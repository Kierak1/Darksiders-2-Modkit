use crate::cursor::Cursor;
use crate::Result;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path_idx: u16,
    pub pkg_idx: u8,
    /// Engine package id: the `PackageID` values in BOD parameters (e.g.
    /// `TextureParameter.PackageID`) and world region `PackageIDs[]` resolve
    /// to this per-file id.
    pub package_id: u16,
    pub offset: u64,
    pub offset_ptr: usize,
}

#[derive(Debug)]
pub struct Manifest {
    pub version: u32,
    pub paths: Vec<String>,
    pub packages: Vec<String>,
    pub files: Vec<FileEntry>,
}

impl Manifest {
    pub fn full_name(&self, entry: &FileEntry) -> String {
        let prefix = self
            .paths
            .get(entry.path_idx as usize)
            .map(|s| s.as_str())
            .unwrap_or("");
        format!("{}/{}", prefix, entry.name).replace('\\', "/")
    }

    pub fn find(&self, asset: &str) -> Option<&FileEntry> {
        self.find_index(asset).map(|i| &self.files[i])
    }

    pub fn find_index(&self, asset: &str) -> Option<usize> {
        let needle = asset.replace('\\', "/").to_lowercase();
        self.files.iter().position(|f| {
            self.full_name(f).to_lowercase() == needle || f.name.to_lowercase() == needle
        })
    }

    /// Asset carrying an engine package id. Ids are unique across the
    /// manifest (verified on the real `manifest.bin`).
    pub fn find_by_package_id(&self, id: u16) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.package_id == id)
    }
}

pub fn parse(data: &[u8]) -> Result<Manifest> {
    let mut c = Cursor { data, pos: 0 };
    let version = c.u32()?;
    c.skip(40)?;

    let num_paths = c.u32()?;
    c.skip(4)?;
    let mut paths = Vec::with_capacity(num_paths as usize);
    for _ in 0..num_paths {
        c.skip(8)?;
        let len = c.u32()? as usize;
        paths.push(String::from_utf8_lossy(c.take(len)?).into_owned());
    }

    let num_packages = c.u32()?;
    let mut packages = Vec::with_capacity(num_packages as usize);
    for _ in 0..num_packages {
        c.skip(1)?;
        let len = c.u16()? as usize;
        packages.push(String::from_utf8_lossy(c.take(len)?).into_owned());
    }

    let num_files = c.u32()?;
    let mut files = Vec::with_capacity(num_files as usize);
    for _ in 0..num_files {
        let fn_len = c.u8()? as usize;
        let name = String::from_utf8_lossy(c.take(fn_len)?).into_owned();
        c.skip(8)?;
        let package_id = c.u16()?;
        let us1 = c.u16()?;
        let align = c.u8()? as usize;
        c.skip(align * 2)?;
        let _uc0 = c.u8()?;
        let uc1 = c.u8()?;
        c.skip(2)?;
        let offset_ptr = c.pos;
        let offset = if version >= 13 {
            c.u64()?
        } else {
            c.u32()? as u64
        };
        files.push(FileEntry {
            name,
            path_idx: us1,
            pkg_idx: uc1,
            package_id,
            offset,
            offset_ptr,
        });
    }

    Ok(Manifest {
        version,
        paths,
        packages,
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_str(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(s.as_bytes());
    }

    #[test]
    fn parses_package_id_and_looks_it_up() {
        let mut d = Vec::new();
        d.extend_from_slice(&13u32.to_le_bytes());
        d.extend_from_slice(&[0u8; 40]);
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(&[0u8; 8]);
        d.extend_from_slice(&3u32.to_le_bytes());
        push_str(&mut d, "abc");
        d.extend_from_slice(&1u32.to_le_bytes());
        d.push(0);
        d.extend_from_slice(&5u16.to_le_bytes());
        push_str(&mut d, "media");
        d.extend_from_slice(&1u32.to_le_bytes());
        d.push(9);
        push_str(&mut d, "roomasset");
        d.extend_from_slice(&[0u8; 8]);
        d.extend_from_slice(&10756u16.to_le_bytes());
        d.extend_from_slice(&0u16.to_le_bytes());
        d.push(0); // align
        d.extend_from_slice(&[0u8; 4]); // uc0, uc1, skip
        let off_at = d.len();
        d.extend_from_slice(&0x1234u64.to_le_bytes());
        let m = parse(&d).unwrap();
        assert_eq!(m.files.len(), 1);
        assert_eq!(m.files[0].package_id, 10756);
        assert_eq!(m.files[0].offset, 0x1234);
        assert_eq!(m.files[0].offset_ptr, off_at);
        let e = m.find_by_package_id(10756).unwrap();
        assert_eq!(e.name, "roomasset");
        assert_eq!(m.full_name(e), "abc/roomasset");
        assert!(m.find_by_package_id(1).is_none());
    }
}
