use crate::cursor::Cursor;
use crate::Result;
use std::collections::HashMap;

pub const HEADER_SIZE: usize = 21;
pub const ENTRY_SIZE: usize = 51;

#[derive(Debug, Clone)]
pub struct Entry {
    pub name_hash: u64,
    pub type_hash: u64,
    pub payload_offset: u32,
    pub payload_size: u32,
    pub field_e: u16,
    pub flags: u8,
    pub refs: [u64; 3],
}

#[derive(Debug)]
pub struct Obsp {
    pub version: u8,
    pub marker: u32,
    pub names_present: bool,
    pub entries_offset: u32,
    pub names: Vec<(u64, String)>,
    pub entries: Vec<Entry>,
    index: HashMap<u64, u32>,
}

impl Obsp {
    pub fn name_of(&self, hash: u64) -> Option<&str> {
        self.index
            .get(&hash)
            .map(|i| self.names[*i as usize].1.as_str())
    }

    pub fn entry_names(&self, entry: &Entry) -> Vec<&str> {
        entry.refs.iter().filter_map(|h| self.name_of(*h)).collect()
    }

    pub fn find(&self, needle: &str) -> Option<&Entry> {
        self.find_index(needle).map(|i| &self.entries[i])
    }

    pub fn find_index(&self, needle: &str) -> Option<usize> {
        let n = needle.to_lowercase();
        self.entries.iter().position(|e| {
            self.name_of(e.name_hash)
                .is_some_and(|x| x.to_lowercase() == n)
                || e.refs
                    .iter()
                    .any(|h| self.name_of(*h).is_some_and(|x| x.to_lowercase() == n))
        })
    }

    pub fn payload<'a>(&self, data: &'a [u8], entry: &Entry) -> &'a [u8] {
        let start = entry.payload_offset as usize;
        &data[start..start + entry.payload_size as usize]
    }
}

pub fn is_obsp(data: &[u8]) -> bool {
    data.len() >= 4 && &data[0..4] == b"OBSP"
}

pub fn parse(data: &[u8]) -> Result<Obsp> {
    let mut c = Cursor::new(data);
    if c.take(4)? != b"OBSP" {
        return Err("not an OBSP archive".into());
    }
    let version = c.u8()?;
    let marker = c.u32()?;
    let names_present = c.u32()? != 0;
    let entry_count = c.u32()? as usize;
    let entries_offset = c.u32()?;

    let mut names = Vec::new();
    if names_present {
        let name_count = c.u32()?;
        let _max_name_len = c.u32()?;
        names.reserve(name_count as usize);
        for _ in 0..name_count {
            let hash = c.u64()?;
            let len = c.u32()? as usize;
            let name = String::from_utf8_lossy(c.take(len)?).into_owned();
            names.push((hash, name));
        }
    }

    let mut index = HashMap::with_capacity(names.len());
    for (i, (hash, _)) in names.iter().enumerate() {
        if *hash != 0 {
            index.insert(*hash, i as u32);
        }
    }

    let mut ec = Cursor::new(
        data.get(entries_offset as usize..)
            .ok_or("OBSP: entries offset out of range")?,
    );
    let mut entries = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        let name_hash = ec.u64()?;
        let type_hash = ec.u64()?;
        let payload_offset = ec.u32()?;
        let payload_size = ec.u32()?;
        let field_e = ec.u16()?;
        let flags = ec.u8()?;
        let refs = [ec.u64()?, ec.u64()?, ec.u64()?];
        entries.push(Entry {
            name_hash,
            type_hash,
            payload_offset,
            payload_size,
            field_e,
            flags,
            refs,
        });
    }

    Ok(Obsp {
        version,
        marker,
        names_present,
        entries_offset,
        names,
        entries,
        index,
    })
}

pub fn rebuild(
    data: &[u8],
    obsp: &Obsp,
    replacements: &HashMap<usize, Vec<u8>>,
) -> Result<Vec<u8>> {
    if obsp.entries.is_empty() {
        if replacements.is_empty() {
            return Ok(data.to_vec());
        }
        return Err("no entries to patch".into());
    }

    let first = obsp.entries.iter().map(|e| e.payload_offset).min().unwrap() as usize;
    if first > data.len() {
        return Err("first payload offset out of range".into());
    }

    let mut order: Vec<usize> = (0..obsp.entries.len()).collect();
    order.sort_by_key(|&i| (obsp.entries[i].payload_offset, i));

    let mut out = data[..first].to_vec();
    let mut new_offsets = vec![0u32; obsp.entries.len()];
    for &i in &order {
        let entry = &obsp.entries[i];
        let old_start = entry.payload_offset as usize;
        let old_end = old_start
            .checked_add(entry.payload_size as usize)
            .filter(|end| *end <= data.len())
            .ok_or("entry payload out of range")?;
        let payload: &[u8] = match replacements.get(&i) {
            Some(bytes) => bytes,
            None => &data[old_start..old_end],
        };
        new_offsets[i] = u32::try_from(out.len()).map_err(|_| "file too large")?;
        out.extend_from_slice(payload);
    }

    let index_at = obsp.entries_offset as usize;
    for (i, new_off) in new_offsets.iter().enumerate() {
        let at = index_at
            .checked_add(i * ENTRY_SIZE + 16)
            .ok_or("entry index out of range")?;
        let size = replacements
            .get(&i)
            .map(|b| b.len())
            .unwrap_or(obsp.entries[i].payload_size as usize);
        out.get_mut(at..at + 4)
            .ok_or("entry index out of range")?
            .copy_from_slice(&new_off.to_le_bytes());
        out.get_mut(at + 4..at + 8)
            .ok_or("entry index out of range")?
            .copy_from_slice(&(size as u32).to_le_bytes());
    }

    Ok(out)
}
