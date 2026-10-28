//! Room batch -> material binding via triangle-shape fingerprints.
//!
//! Maps room batches (`glomm_<hex8>` records) are baked merges of modular
//! source meshes. Those source meshes live in the zone's media environment
//! assets (`world/environments/Zone_XX/<zone>_<room>` etc.) as named prop
//! records, and the room's materials are named after them. Rigid transforms
//! preserve triangle edge-length ratios, so a scale/rotation-invariant
//! fingerprint (the two shorter edge lengths of each triangle divided by the
//! longest) identifies a batch's source meshes even after baking.
//!
//! Verified on `environments/world_z1_ld_rm00` against the `Zone_01` media
//! assets: all 28 batches (54,107 tris) classify to a room material, e.g.
//! floors -> `Z1_FloorSet_B_Stones_A`, columns -> `Z1_Column_E`, brick ->
//! `Z1_MakerBrick`, trim -> `Z1_Trim_E_low_spec` (`room-materials` CLI and the
//! GUI Map tab's **Materials** button).

use crate::map::propmesh;
use crate::map::roommesh::RoomMesh;
use crate::pack::manifest::{FileEntry, Manifest};
use crate::pack::obpk;
use crate::pack::upak;
use crate::Result;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Histogram of quantized triangle shapes: `[shorter/max, middle/max] * 100`.
pub type Fingerprint = HashMap<[u8; 2], u32>;

/// Minimum share of a batch's triangles a material must explain to be used.
pub const MIN_SHARE: f32 = 0.25;

#[derive(Debug, Default)]
pub struct SourceDb {
    /// Representative color per material name (from `DiffuseMap1`).
    pub colors: HashMap<String, [u8; 3]>,
    /// Merged triangle fingerprints per material name.
    pub fingerprints: HashMap<String, Fingerprint>,
    /// Number of source meshes merged.
    pub meshes: usize,
    /// Number of assets scanned.
    pub assets: usize,
}

fn edge2(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|k| (a[k] - b[k]).powi(2)).sum()
}

/// Scale/rotation-invariant histogram of `(positions, indices)` triangles.
pub fn fingerprint(positions: &[[f32; 3]], indices: &[u16]) -> Fingerprint {
    let mut fp = Fingerprint::new();
    for t in indices.as_chunks::<3>().0 {
        let (ia, ib, ic) = (t[0] as usize, t[1] as usize, t[2] as usize);
        if ia >= positions.len() || ib >= positions.len() || ic >= positions.len() {
            continue;
        }
        let mut e = [
            edge2(positions[ia], positions[ib]),
            edge2(positions[ib], positions[ic]),
            edge2(positions[ic], positions[ia]),
        ];
        e.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if e[2].partial_cmp(&1e-12) != Some(std::cmp::Ordering::Greater) {
            continue;
        }
        let r0 = ((e[0] / e[2]) * 100.0).round().clamp(0.0, 255.0) as u8;
        let r1 = ((e[1] / e[2]) * 100.0).round().clamp(0.0, 255.0) as u8;
        *fp.entry([r0, r1]).or_insert(0) += 1;
    }
    fp
}

/// Normalized material key: lowercase, with DLC/no-normal/snow/instance
/// decorations removed. Room materials carry suffixes the source meshes do
/// not (`Z2_MetalStrip_DLC` vs `Z2_MetalStrip`, `Z1_Wall_B_Slabs_NN_DLC`),
/// so matching and filtering compare keys, not raw names.
pub fn material_key(name: &str) -> String {
    let mut s = name.to_lowercase();
    for tag in ["_dlc1", "_dlc2", "_dlc"] {
        s = s.replace(tag, "");
    }
    loop {
        let before = s.clone();
        for tag in ["_nn", "_snow", "_mid"] {
            if let Some(t) = s.strip_suffix(tag) {
                s = t.to_string();
            }
        }
        if let Some(pos) = s.rfind('_') {
            if pos + 1 < s.len() && s[pos + 1..].bytes().all(|b| b.is_ascii_digit()) {
                s.truncate(pos);
            }
        }
        if s == before {
            break;
        }
    }
    s
}

/// Fraction of `src` triangles explainable by `batch` (1:1, same shape).
pub fn coverage(src: &Fingerprint, batch: &Fingerprint) -> f32 {
    let total: u32 = src.values().sum();
    if total == 0 {
        return 0.0;
    }
    let common: u32 = src
        .iter()
        .map(|(k, v)| (*v).min(*batch.get(k).unwrap_or(&0)))
        .sum();
    common as f32 / total as f32
}

/// Best material for one batch fingerprint, restricted to `allowed` keys
/// (`material_key` form) when given. Returns `(name, share of batch triangles
/// explained)`.
pub fn classify_fp(
    fp: &Fingerprint,
    db: &SourceDb,
    allowed: Option<&HashSet<String>>,
) -> Option<(String, f32)> {
    let batch_total: u32 = fp.values().sum();
    if batch_total == 0 {
        return None;
    }
    let mut best: Option<(String, f32)> = None;
    for (name, sfp) in &db.fingerprints {
        if let Some(allowed) = allowed {
            if !allowed.is_empty() && !allowed.contains(&material_key(name)) {
                continue;
            }
        }
        let common: u32 = sfp
            .iter()
            .map(|(k, v)| (*v).min(*fp.get(k).unwrap_or(&0)))
            .sum();
        if common == 0 {
            continue;
        }
        let share = common as f32 / batch_total as f32;
        if best.as_ref().is_none_or(|(_, bs)| share > *bs) {
            best = Some((name.clone(), share));
        }
    }
    best.filter(|(_, share)| *share >= MIN_SHARE)
}

/// Best material per batch, ranked by how much of the batch's triangle budget
/// the material accounts for; `None` when the best share is below `MIN_SHARE`.
pub fn classify(room: &RoomMesh, db: &SourceDb) -> Vec<Option<(String, f32)>> {
    room.batches
        .iter()
        .map(|b| classify_fp(&fingerprint(&b.positions, &b.indices), db, None))
        .collect()
}

/// Longest common suffix length, used to find the room's media zone folder.
fn common_suffix_len(a: &str, b: &str) -> usize {
    a.bytes()
        .rev()
        .zip(b.bytes().rev())
        .take_while(|(x, y)| x == y)
        .count()
}

/// Zone media folder (`Zone_01`, ...) whose asset basenames best match the
/// room asset name (e.g. `world_z1_ld_rm00` -> `zone01_ld_rm00`).
pub fn zone_folder(manifest: &Manifest, entry: &FileEntry) -> Option<String> {
    let target = entry.name.to_lowercase();
    let mut best: Option<(usize, String)> = None;
    for f in manifest.files.iter() {
        if f.pkg_idx == 0xFF {
            continue;
        }
        if manifest
            .packages
            .get(f.pkg_idx as usize)
            .map(String::as_str)
            != Some("media")
        {
            continue;
        }
        let full = manifest.full_name(f).to_lowercase();
        let Some(rest) = full.strip_prefix("world/environments/") else {
            continue;
        };
        let Some((folder, base)) = rest.split_once('/') else {
            continue;
        };
        let n = common_suffix_len(&target, base);
        if n >= 6 && best.as_ref().is_none_or(|(bn, _)| n > *bn) {
            best = Some((n, folder.to_string()));
        }
    }
    best.map(|(_, folder)| folder)
}

/// Decode every prop mesh in the zone's media assets and merge their
/// fingerprints/colors by material name. When `wanted` is given, only material
/// keys (`material_key` form) in that set are kept (the room's own material
/// list). Assets that fail to parse are skipped; an error is returned only
/// when nothing could be read.
pub fn build(
    media: &Path,
    manifest: &Manifest,
    folder: &str,
    wanted: Option<&HashSet<String>>,
) -> Result<SourceDb> {
    let prefix = format!("world/environments/{}/", folder.to_lowercase());
    let entries: Vec<&FileEntry> = manifest
        .files
        .iter()
        .filter(|f| f.pkg_idx != 0xFF)
        .filter(|f| {
            manifest
                .packages
                .get(f.pkg_idx as usize)
                .map(String::as_str)
                == Some("media")
        })
        .filter(|f| manifest.full_name(f).to_lowercase().starts_with(&prefix))
        .collect();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 8)
        .min(entries.len().max(1));
    let chunk = entries.len().div_ceil(threads).max(1);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = entries
            .chunks(chunk)
            .map(|slice| {
                scope.spawn(move || {
                    let mut meshes: Vec<(String, Fingerprint)> = Vec::new();
                    let mut colors: HashMap<String, [u8; 3]> = HashMap::new();
                    let mut scanned = 0usize;
                    for entry in slice {
                        let Ok(carved) = upak::carve(media, manifest, entry) else {
                            continue;
                        };
                        let Ok((_, payload)) = obpk::decompress(&carved) else {
                            continue;
                        };
                        scanned += 1;
                        for m in propmesh::parse_props(&payload) {
                            if let Some(wanted) = wanted {
                                if !wanted.contains(&material_key(&m.name)) {
                                    continue;
                                }
                            }
                            meshes.push((m.name.clone(), fingerprint(&m.positions, &m.indices)));
                        }
                        for (n, c) in propmesh::material_colors(&payload) {
                            if wanted.is_none_or(|w| w.contains(&material_key(&n))) {
                                colors.insert(n, c);
                            }
                        }
                    }
                    (meshes, colors, scanned)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect::<Vec<_>>()
    });
    let mut db = SourceDb::default();
    let mut any = false;
    for (meshes, colors, scanned) in results {
        db.assets += scanned;
        any |= scanned > 0;
        db.colors.extend(colors);
        for (name, fp) in meshes {
            let merged = db.fingerprints.entry(name).or_default();
            if !fp.is_empty() {
                db.meshes += 1;
            }
            for (k, v) in fp {
                *merged.entry(k).or_insert(0) += v;
            }
        }
    }
    if !any {
        return Err(format!("no media assets under {prefix} could be read").into());
    }
    Ok(db)
}

/// Flat color for a material: database color, else a name-derived grey.
pub fn material_color(db: &SourceDb, name: &str) -> [u8; 3] {
    db.colors.get(name).copied().unwrap_or([128, 128, 128])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::roommesh::{Batch, RoomMesh};

    fn tri(p: [[f32; 3]; 3]) -> (Vec<[f32; 3]>, Vec<u16>) {
        (p.to_vec(), vec![0, 1, 2])
    }

    #[test]
    fn fingerprint_is_scale_and_rotation_invariant() {
        let (pos, idx) = tri([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 2.0, 0.0]]);
        let fp = fingerprint(&pos, &idx);
        // scale x3 and rotate 90 deg about z
        let scaled = [[0.0, 0.0, 0.0], [0.0, 3.0, 0.0], [-6.0, 0.0, 0.0]];
        assert_eq!(fp, fingerprint(&scaled, &idx));
        assert!((coverage(&fp, &fp) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn classify_picks_matching_source() {
        let (a_pos, a_idx) = tri([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        let (b_pos, b_idx) = tri([[0.0, 0.0, 0.0], [5.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        let mut db = SourceDb::default();
        db.fingerprints
            .insert("A".into(), fingerprint(&a_pos, &a_idx));
        db.fingerprints
            .insert("B".into(), fingerprint(&b_pos, &b_idx));
        let scaled_a = [[0.0, 0.0, 0.0], [7.0, 0.0, 0.0], [0.0, 7.0, 0.0]];
        let room = RoomMesh {
            origin: [0.0; 3],
            scale: 1.0,
            batches: vec![Batch {
                name: "glomm_test".into(),
                positions: scaled_a.to_vec(),
                indices: vec![0, 1, 2],
                uvs: Vec::new(),
            }],
        };
        let out = classify(&room, &db);
        assert_eq!(out[0].as_ref().map(|(n, _)| n.as_str()), Some("A"));
    }
}
