//! Texture resource resolution: material `TextureParameter` name -> DDS.
//!
//! A material's `Parameters.DiffuseMap1` is a `TextureParameter{Value,
//! PackageID}`: `Value` names the texture resource, `PackageID` is the
//! manifest per-file package id. The pixels live in whichever media asset
//! lists the resource in its OBPK header resource directory (crc64 of the
//! lowercased name, group loader id 6), not necessarily in the referenced
//! package. The DDS blobs in the payload follow the group-6 entry order
//! (verified: `zone01_ld_rm00` 6/6, `zone01_modulardungeon_common` 124/124,
//! `zone01_rocks` 21/21, `gbl_shadowplane` 2/2).
//!
//! `TextureLoader::load_many` resolves a batch of texture names, carves and
//! decompresses each contributing asset once and decodes the DDS via
//! [`crate::pack::dds::decode`].

use crate::hash::crc64;
use crate::pack::dds;
use crate::pack::manifest::Manifest;
use crate::pack::obpk;
use crate::pack::upak;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// First bytes of an entry needed to reach object A's resource directory.
pub const HEADER_SCAN_BYTES: usize = 4096;

#[derive(Debug, Clone)]
pub struct ResourceGroup {
    pub field0: u32,
    pub hashes: Vec<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct ResourceDir {
    pub groups: Vec<ResourceGroup>,
}

impl ResourceDir {
    /// Parse the OBPK resource directory (object A) from the first bytes of
    /// a media asset.
    pub fn parse(header: &[u8]) -> Option<ResourceDir> {
        if header.len() < 30 || &header[..4] != b"OBPK" {
            return None;
        }
        let obj = u32::from_le_bytes(header.get(9..13)?.try_into().ok()?) as usize;
        let obj_size = u32::from_le_bytes(header.get(13..17)?.try_into().ok()?) as usize;
        if obj < 30 || obj_size < 16 || obj + obj_size > header.len() {
            return None;
        }
        let d = &header[..obj + obj_size];
        let u32_at = |o: usize| -> Option<u32> {
            d.get(o..o + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        };
        let (group_count, _entry_count, blob_size) =
            (u32_at(obj)?, u32_at(obj + 4)?, u32_at(obj + 8)?);
        if group_count > 4096 || blob_size > (1 << 20) {
            return None;
        }
        let blob = d.get(obj + 16..obj + 16 + blob_size as usize)?;
        let mut o = obj + 16 + blob_size as usize + 16;
        let mut groups = Vec::with_capacity(group_count as usize);
        for _ in 0..group_count {
            let field0 = u32_at(o)?;
            let count = u32_at(o + 4)?;
            let blob_off1 = u32_at(o + 8)? as usize;
            o += 16;
            let flags = d.get(o..o + count as usize)?;
            o += flags.len();
            let mut hashes = Vec::with_capacity(count as usize);
            for i in 0..count as usize {
                let b = blob.get(blob_off1 + i * 8..blob_off1 + i * 8 + 8)?;
                hashes.push(u64::from_le_bytes(b.try_into().unwrap()));
            }
            groups.push(ResourceGroup { field0, hashes });
        }
        Some(ResourceDir { groups })
    }

    /// Position of a texture resource in its texture group (loader id 6).
    /// The payload's DDS blobs follow this order.
    pub fn texture_slot(&self, name: &str) -> Option<u16> {
        let h = crc64(name.to_lowercase().as_bytes());
        for g in &self.groups {
            if g.field0 != 6 {
                continue;
            }
            if let Some(i) = g.hashes.iter().position(|x| *x == h) {
                return Some(i as u16);
            }
        }
        None
    }

    /// Number of texture resources (group 6 entries).
    pub fn texture_count(&self) -> usize {
        self.groups
            .iter()
            .filter(|g| g.field0 == 6)
            .map(|g| g.hashes.len())
            .sum()
    }
}

/// Global texture name -> (manifest file index, DDS slot) index, built from
/// every media asset's OBPK header (a 4 KiB read per asset).
pub struct TextureIndex {
    map: HashMap<u64, (usize, u16)>,
    scanned: usize,
}

impl TextureIndex {
    pub fn build(media: &Path, m: &Manifest) -> TextureIndex {
        let media_pkg = m.packages.iter().position(|p| p == "media");
        let mut map = HashMap::new();
        let mut scanned = 0usize;
        for (fi, f) in m.files.iter().enumerate() {
            if f.pkg_idx == 0xFF || media_pkg != Some(f.pkg_idx as usize) {
                continue;
            }
            let Ok(header) = upak::carve_range(media, m, f, 64 * 1024) else {
                continue;
            };
            let Some(dir) = ResourceDir::parse(&header) else {
                continue;
            };
            scanned += 1;
            for g in &dir.groups {
                if g.field0 != 6 {
                    continue;
                }
                for (i, h) in g.hashes.iter().enumerate() {
                    map.entry(*h).or_insert((fi, i as u16));
                }
            }
        }
        TextureIndex { map, scanned }
    }

    pub fn scanned(&self) -> usize {
        self.scanned
    }

    pub fn find(&self, name: &str) -> Option<(usize, u16)> {
        self.map
            .get(&crc64(name.to_lowercase().as_bytes()))
            .copied()
    }
}

/// Decoded RGBA8 texture.
pub struct TexImage {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[u8; 4]>,
}

/// Caches resolved names and decoded textures for one session.
#[derive(Default)]
pub struct TextureLoader {
    index: Option<TextureIndex>,
    cache: HashMap<String, Option<Arc<TexImage>>>,
}

impl TextureLoader {
    /// Resolve and decode every requested texture name; unresolved names are
    /// absent from the result. Carves each contributing asset once.
    pub fn load_many(
        &mut self,
        media: &Path,
        m: &Manifest,
        names: &[String],
    ) -> HashMap<String, Arc<TexImage>> {
        if self.index.is_none() {
            self.index = Some(TextureIndex::build(media, m));
        }
        let index = self.index.as_ref().unwrap();
        let mut by_file: HashMap<usize, Vec<(String, u16)>> = HashMap::new();
        for name in names {
            if self.cache.contains_key(name) {
                continue;
            }
            if let Some((fi, slot)) = index.find(name) {
                by_file.entry(fi).or_default().push((name.clone(), slot));
            } else {
                self.cache.insert(name.clone(), None);
            }
        }
        for (fi, wanted) in by_file {
            let mut loaded: Vec<(String, Option<Arc<TexImage>>)> = Vec::new();
            let payload = m
                .files
                .get(fi)
                .and_then(|f| upak::carve(media, m, f).ok())
                .and_then(|carved| obpk::decompress(&carved).ok())
                .map(|(_, payload)| payload);
            match payload {
                Some(payload) => {
                    let offsets = dds::find_textures(&payload);
                    for (name, slot) in wanted {
                        let tex = offsets.get(slot as usize).and_then(|off| {
                            let size = dds::dds_size_at(&payload, *off) as usize;
                            let end = (*off + size).min(payload.len());
                            dds::decode(&payload[*off..end])
                                .ok()
                                .map(|(w, h, px)| Arc::new(TexImage { w, h, px }))
                        });
                        loaded.push((name, tex));
                    }
                }
                None => loaded.extend(wanted.into_iter().map(|(n, _)| (n, None))),
            }
            for (name, tex) in loaded {
                self.cache.insert(name, tex);
            }
        }
        let mut out = HashMap::new();
        for name in names {
            if let Some(Some(tex)) = self.cache.get(name) {
                out.insert(name.clone(), tex.clone());
            }
        }
        out
    }

    /// Resolve texture names to their raw (undecoded) DDS blobs, for
    /// exporting `map_Kd` files. Carves each contributing asset once.
    pub fn raw_dds_many(
        &mut self,
        media: &Path,
        m: &Manifest,
        names: &[String],
    ) -> HashMap<String, Vec<u8>> {
        if self.index.is_none() {
            self.index = Some(TextureIndex::build(media, m));
        }
        let index = self.index.as_ref().unwrap();
        let mut by_file: HashMap<usize, Vec<(String, u16)>> = HashMap::new();
        for name in names {
            if let Some((fi, slot)) = index.find(name) {
                by_file.entry(fi).or_default().push((name.clone(), slot));
            }
        }
        let mut out = HashMap::new();
        for (fi, wanted) in by_file {
            let Some(payload) = m
                .files
                .get(fi)
                .and_then(|f| upak::carve(media, m, f).ok())
                .and_then(|carved| obpk::decompress(&carved).ok())
                .map(|(_, payload)| payload)
            else {
                continue;
            };
            let offsets = dds::find_textures(&payload);
            for (name, slot) in wanted {
                if let Some(off) = offsets.get(slot as usize) {
                    let size = dds::dds_size_at(&payload, *off) as usize;
                    let end = (*off + size).min(payload.len());
                    out.insert(name, payload[*off..end].to_vec());
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synthetic_resource_dir() {
        // object A: 2 groups (loader 5 and 6), blob with hash rows.
        let mut d = vec![0u8; 30];
        d[0..4].copy_from_slice(b"OBPK");
        let obj = 30usize;
        let mut obj_a = Vec::new();
        obj_a.extend_from_slice(&2u32.to_le_bytes()); // groupCount
        obj_a.extend_from_slice(&3u32.to_le_bytes()); // entryCount
        obj_a.extend_from_slice(&40u32.to_le_bytes()); // blobSize
        obj_a.extend_from_slice(&0u32.to_le_bytes());
        let blob_at = obj_a.len();
        obj_a.extend_from_slice(&[0u8; 40]);
        obj_a.extend_from_slice(&[0u8; 16]); // a b c d
                                             // group 5: 1 entry, blobOff1 0
        obj_a.extend_from_slice(&5u32.to_le_bytes());
        obj_a.extend_from_slice(&1u32.to_le_bytes());
        obj_a.extend_from_slice(&0u32.to_le_bytes());
        obj_a.extend_from_slice(&0u32.to_le_bytes());
        obj_a.push(0);
        // group 6: 2 entries, blobOff1 8
        obj_a.extend_from_slice(&6u32.to_le_bytes());
        obj_a.extend_from_slice(&2u32.to_le_bytes());
        obj_a.extend_from_slice(&8u32.to_le_bytes());
        obj_a.extend_from_slice(&0u32.to_le_bytes());
        obj_a.extend_from_slice(&[0, 0]);
        let tex = crc64(b"my_texture_1");
        obj_a[blob_at + 8..blob_at + 16].copy_from_slice(&tex.to_le_bytes());
        d[9..13].copy_from_slice(&(obj as u32).to_le_bytes());
        d[13..17].copy_from_slice(&(obj_a.len() as u32).to_le_bytes());
        d.extend_from_slice(&obj_a);
        let dir = ResourceDir::parse(&d).unwrap();
        assert_eq!(dir.texture_count(), 2);
        assert_eq!(dir.texture_slot("My_Texture_1"), Some(0));
        assert_eq!(dir.texture_slot("My_Texture_2"), None);
    }
}
