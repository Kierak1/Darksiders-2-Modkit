//! Localized-string table (`LocTable`).
//!
//! # Where the game's text actually lives (verified 2026-10-11)
//!
//! The premise that `Game\Darksiders II Deathinitive Edition\darksiders2.dsl`
//! holds localized text is **wrong**: the `.dsl` file is a D3D9 shader library
//! (`DSL\x04` magic, `vs_3_0`/`CTAB` blobs; the exe declares
//! `DX9.Filename = /Darksiders2.dsl` next to `DX11.Filename = /Darksiders2.wsl`,
//! IDA string at `0x14137a63d`). No plaintext loc table exists in the exe,
//! `scripts.obsp`, `worlds.bin` or the raw `.upak` bytes.
//!
//! The real English text lives in the OBPK asset **`ui/ui_core`** of
//! `media.upak`. Its payload is a serialized dictionary: runs of 40-byte
//! records followed by one NUL-separated string pool per language section:
//!
//! ```text
//! record (40 bytes):
//!   +0x00 u64  crc64(key)          key without the leading '$'
//!   +0x08 u64  a                   string offset: pool_base + a
//!   +0x10 u64  id                  sequential id (0x10 * record index + base)
//!   +0x18 u64  0xFFFFFFFFFFFFFFFF  sentinel
//!   +0x20 u32  0
//!   +0x24 u16  language mask       1 normally, 0x0F on one record
//!   +0x26 u16  0x0002
//! ```
//!
//! The key hash is the same CRC-64 as [`crate::hash::crc64`] (IDA
//! `sub_1400C1710`, table at `0x14137BA40`). The engine's lookup
//! (`getLocalizedString` -> `sub_1400BAB90`) strips the leading `$` and tries
//! `key + "_GAMEPAD"` (or `"_KB"` for the keyboard layout), then `key + "_PC"`,
//! then the bare key; [`LocTable::resolve`] follows the same order.
//!
//! [`LocTable::from_ui_core`] recovers a table from a decompressed ui_core
//! payload. It finds the record runs, solves each run's pool base by matching
//! records against NUL-start string positions (verified: all 5478 records of
//! the first run resolve exactly), and keeps the sections whose
//! `Item.Attributes.Damage` probe reads `"Damage"` (the English tables).
//! **Confidence: the English section is verified against the stock asset; the
//! per-run base solver is a heuristic and other languages are not exported.**
//!
//! [`LocTable::parse`] additionally reads a plain key/value text dump so the
//! editing tools can consume extracted tables (or hand-written overrides)
//! without the game files.

use crate::hash::crc64;
use crate::Result;
use std::collections::HashMap;
use std::path::Path;

/// A key -> text table keyed by the CRC-64 of the key (leading `$` stripped),
/// i.e. the same hash the engine and the OBSP name table use.
#[derive(Debug, Clone, Default)]
pub struct LocTable {
    entries: HashMap<u64, String>,
}

impl LocTable {
    /// Empty table.
    pub fn new() -> LocTable {
        LocTable::default()
    }

    /// Build a table from `(key, text)` pairs. Keys are normalized by
    /// stripping a leading `$` and hashing with [`crc64`].
    pub fn from_pairs<'a, I>(pairs: I) -> LocTable
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let mut entries = HashMap::new();
        for (key, text) in pairs {
            let k = key.strip_prefix('$').unwrap_or(key);
            entries.insert(crc64(k.as_bytes()), text.to_owned());
        }
        LocTable { entries }
    }

    /// Parse a key/value text table.
    ///
    /// Format (UTF-8 with optional BOM; UTF-16 with BOM is converted):
    /// one entry per line, `key = value` or `key<TAB>value`, surrounding
    /// whitespace trimmed; lines whose first non-space character is `#` or
    /// `;` are comments; empty lines are skipped. Keys are normalized like
    /// [`LocTable::from_pairs`]. No escape sequences are interpreted, so
    /// values cannot contain newlines.
    pub fn parse(bytes: &[u8]) -> Result<LocTable> {
        let text = decode_text(bytes)?;
        let mut pairs = Vec::new();
        for (lineno, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let (key, value) = match line.find('=') {
                Some(i) => (line[..i].trim(), line[i + 1..].trim()),
                None => match line.find('\t') {
                    Some(i) => (line[..i].trim(), line[i + 1..].trim()),
                    None => {
                        return Err(format!("line {}: expected key=value", lineno + 1).into());
                    }
                },
            };
            if key.is_empty() {
                return Err(format!("line {}: empty key", lineno + 1).into());
            }
            pairs.push((key, value));
        }
        Ok(LocTable::from_pairs(pairs))
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Resolve a loc key. The leading `$` is optional; the engine's
    /// platform-suffixed candidates (`_KB`/`_GAMEPAD`, `_PC`) are tried before
    /// the bare key.
    pub fn resolve(&self, key: &str) -> Option<&str> {
        let k = key.strip_prefix('$').unwrap_or(key);
        for suffix in ["_KB", "_GAMEPAD", "_PC", ""] {
            let mut candidate = String::with_capacity(k.len() + suffix.len());
            candidate.push_str(k);
            candidate.push_str(suffix);
            if let Some(text) = self.entries.get(&crc64(candidate.as_bytes())) {
                return Some(text);
            }
        }
        None
    }

    /// Resolve a pre-hashed key (`crc64` of the key without `$`).
    pub fn resolve_hash(&self, hash: u64) -> Option<&str> {
        self.entries.get(&hash).map(String::as_str)
    }

    /// Recover the English table from a decompressed `ui/ui_core` OBPK payload.
    ///
    /// Best-effort: see the module docs for the format and the confidence
    /// notes. Returns an empty table when nothing probes as English.
    pub fn from_ui_core(payload: &[u8]) -> LocTable {
        let mut entries = HashMap::new();
        let starts = string_starts(payload);
        let mut accepted = 0usize;
        for run in record_runs(payload) {
            let records = run_records(payload, run.start, run.count);
            let Some(base) = solve_pool_base(payload, &starts, &records) else {
                continue;
            };
            match probe_result(payload, base, &records) {
                Some(true) => accepted += 1,
                // Sections are contiguous: once a known key reads as a
                // translation, the English section is over.
                Some(false) if accepted > 0 => break,
                Some(false) | None => continue,
            }
            for (hash, a) in &records {
                if let Some(text) = read_cstr(payload, base + *a as usize) {
                    entries.entry(*hash).or_insert_with(|| text.to_owned());
                }
            }
        }
        LocTable { entries }
    }

    /// Load the English [`LocTable`] from a media directory (`manifest.bin` +
    /// the `ui/ui_core` asset inside the media package).
    pub fn load_from_media(media: &Path) -> Result<LocTable> {
        use crate::pack::{manifest, obpk, upak};
        let data = std::fs::read(media.join("manifest.bin"))?;
        let m = manifest::parse(&data)?;
        let entry = m
            .find("ui/ui_core")
            .ok_or("manifest has no ui/ui_core asset")?;
        let carved = upak::carve(media, &m, entry)?;
        let payload = obpk::unchunk(&carved)?;
        Ok(LocTable::from_ui_core(&payload))
    }
}

fn decode_text(bytes: &[u8]) -> Result<String> {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        if rest.len() % 2 != 0 {
            return Err("truncated UTF-16LE input".into());
        }
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        return String::from_utf16(&units).map_err(|e| format!("bad UTF-16: {e}").into());
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        if rest.len() % 2 != 0 {
            return Err("truncated UTF-16BE input".into());
        }
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        return String::from_utf16(&units).map_err(|e| format!("bad UTF-16: {e}").into());
    }
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

// ---------------------------------------------------------------------------
// ui_core record-run recovery

const REC_SIZE: usize = 40;

/// One maximal run of 40-byte dictionary records.
#[derive(Debug, Clone, Copy)]
struct RecordRun {
    start: usize,
    count: usize,
}

fn is_record(p: &[u8], i: usize) -> bool {
    i + REC_SIZE <= p.len()
        && p[i + 24..i + 32] == [0xFF; 8]
        && p[i + 32..i + 36] == [0, 0, 0, 0]
        && p[i + 38..i + 40] == [0x02, 0x00]
}

fn record_runs(p: &[u8]) -> Vec<RecordRun> {
    let mut runs = Vec::new();
    let mut i = 0usize;
    while i + REC_SIZE <= p.len() {
        if is_record(p, i) && (i < REC_SIZE || !is_record(p, i - REC_SIZE)) {
            let mut n = 1usize;
            while is_record(p, i + n * REC_SIZE) {
                n += 1;
            }
            if n >= 32 {
                runs.push(RecordRun { start: i, count: n });
            }
            i += n * REC_SIZE;
        } else {
            i += 1;
        }
    }
    runs
}

fn run_records(p: &[u8], start: usize, count: usize) -> Vec<(u64, u64)> {
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        let at = start + k * REC_SIZE;
        let hash = u64::from_le_bytes(p[at..at + 8].try_into().unwrap_or([0; 8]));
        let a = u64::from_le_bytes(p[at + 8..at + 16].try_into().unwrap_or([0; 8]));
        out.push((hash, a));
    }
    out
}

/// A text byte: any printable ASCII or UTF-8 lead/continuation byte.
fn is_text_byte(b: u8) -> bool {
    (0x20..0x7F).contains(&b) || b >= 0x80
}

/// A string starts at `off` if it is NUL-terminated and the first byte is
/// printable.
fn is_string_start(p: &[u8], off: usize) -> bool {
    off > 0 && off < p.len() && p[off - 1] == 0 && is_text_byte(p[off])
}

/// Positions in `p` that look like the start of a NUL-terminated string.
fn string_starts(p: &[u8]) -> Vec<u32> {
    (1..p.len())
        .filter(|&i| p[i - 1] == 0 && is_text_byte(p[i]))
        .map(|i| i as u32)
        .collect()
}

fn read_cstr(p: &[u8], off: usize) -> Option<&str> {
    if off >= p.len() {
        return None;
    }
    let end = p[off..].iter().position(|&b| b == 0).map(|i| off + i)?;
    std::str::from_utf8(&p[off..end]).ok()
}

/// Solve `pool_base` such that `pool_base + record.a` lands on a string start
/// for (nearly) every record. One anchor pair rejects wrong candidates in
/// the candidate loop, remaining anchors and a full validation confirm.
fn solve_pool_base(p: &[u8], starts: &[u32], records: &[(u64, u64)]) -> Option<usize> {
    let (_, a0) = *records.first()?;
    let a0 = usize::try_from(a0).ok()?;
    let n = records.len();
    let anchors = [
        records[n / 4].1,
        records[n / 2].1,
        records[3 * n / 4].1,
        records[n - 1].1,
    ];
    let a_mid = usize::try_from(records[n / 2].1).ok()?;
    let dm = a_mid.saturating_sub(a0);
    let first = starts.partition_point(|&x| (x as usize) < a0);
    for &xs in &starts[first..] {
        let x = xs as usize;
        let base = x - a0;
        let second = x + dm;
        if !is_string_start(p, second) {
            continue;
        }
        let mut ok = true;
        for a in &anchors {
            let Ok(a) = usize::try_from(*a) else {
                ok = false;
                break;
            };
            let pos = base.checked_add(a);
            if !pos.is_some_and(|pos| is_string_start(p, pos)) {
                ok = false;
                break;
            }
        }
        if !ok {
            continue;
        }
        // Validate the whole run; the real base resolves every record.
        let good = records
            .iter()
            .filter(|(_, a)| {
                usize::try_from(*a)
                    .ok()
                    .and_then(|a| base.checked_add(a))
                    .is_some_and(|pos| is_string_start(p, pos))
            })
            .count();
        if good * 100 >= records.len() * 98 {
            return Some(base);
        }
    }
    None
}

/// Keys used to recognize the English section. If a run's records contain one
/// of these keys, its value must equal the known English text for the run to
/// be accepted; once such a key reads as a translation the scan stops.
const PROBES: &[(&[u8], &str)] = &[
    (b"Item.Attributes.Damage", "Damage"),
    (
        b"Item.Armor.Boots.PickupDetails",
        "This armor protects the wearer's feet and legs from attack.",
    ),
    (
        b"Item.Weapons.Legendary.Scythe.BheithirsTalons.PickupTitle",
        "Bheithir's Talons",
    ),
];

fn probe_result(p: &[u8], base: usize, records: &[(u64, u64)]) -> Option<bool> {
    for (key, want) in PROBES {
        let hash = crc64(key);
        if let Some((_, a)) = records.iter().find(|(h, _)| *h == hash) {
            let value = usize::try_from(*a)
                .ok()
                .and_then(|a| base.checked_add(a))
                .and_then(|off| read_cstr(p, off));
            return Some(value == Some(*want));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_key_value_text() {
        let text = b"# comment\r\n$Item.Weapons.Test.PickupTitle = Bheithir's Talons\r\nItem.Armor.Boots.PickupDetails=Protects the feet.\n\n; another\nItem.Attributes.Damage\tDamage\n";
        let table = LocTable::parse(text).unwrap();
        assert_eq!(table.len(), 3);
        assert_eq!(
            table.resolve("$Item.Weapons.Test.PickupTitle"),
            Some("Bheithir's Talons")
        );
        assert_eq!(
            table.resolve("Item.Weapons.Test.PickupTitle"),
            Some("Bheithir's Talons")
        );
        assert_eq!(
            table.resolve("$Item.Armor.Boots.PickupDetails"),
            Some("Protects the feet.")
        );
        assert_eq!(table.resolve("Item.Attributes.Damage"), Some("Damage"));
        assert_eq!(table.resolve("Item.Missing"), None);
    }

    #[test]
    fn rejects_bad_text() {
        assert!(LocTable::parse(b"no separator here\n").is_err());
        assert!(LocTable::parse(b"= value\n").is_err());
    }

    #[test]
    fn utf16_text_is_decoded() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "Key=Value".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let table = LocTable::parse(&bytes).unwrap();
        assert_eq!(table.resolve("Key"), Some("Value"));
    }

    /// Build a synthetic ui_core payload: 32 records whose `a` offsets point
    /// into a NUL-separated pool, plus the English probe key.
    fn synthetic_ui_core() -> (Vec<u8>, Vec<(String, String)>) {
        let mut pairs = vec![("Item.Attributes.Damage".to_string(), "Damage".to_string())];
        for i in 0..31 {
            pairs.push((format!("Item.Test.Key{i}"), format!("Value number {i}")));
        }
        let mut pool = Vec::new();
        let mut offsets = Vec::new();
        for (_, text) in &pairs {
            offsets.push(pool.len() as u64);
            pool.extend_from_slice(text.as_bytes());
            pool.push(0);
        }
        let run_start = 0x100usize;
        let pool_base = 0x1000usize;
        let mut p = vec![0u8; pool_base + pool.len() + 16];
        p[run_start..run_start + pool.len()].fill(0);
        for (i, (key, _)) in pairs.iter().enumerate() {
            let at = run_start + i * REC_SIZE;
            let hash = crc64(key.strip_prefix('$').unwrap_or(key).as_bytes());
            p[at..at + 8].copy_from_slice(&hash.to_le_bytes());
            p[at + 8..at + 16].copy_from_slice(&offsets[i].to_le_bytes());
            p[at + 16..at + 24].copy_from_slice(&((i as u64 + 1) * 0x10).to_le_bytes());
            p[at + 24..at + 32].fill(0xFF);
            p[at + 38..at + 40].copy_from_slice(&[0x02, 0x00]);
        }
        p[pool_base..pool_base + pool.len()].copy_from_slice(&pool);
        (p, pairs)
    }

    #[test]
    fn recovers_synthetic_ui_core() {
        let (payload, pairs) = synthetic_ui_core();
        let table = LocTable::from_ui_core(&payload);
        assert_eq!(table.len(), pairs.len());
        for (key, text) in &pairs {
            assert_eq!(table.resolve(key), Some(text.as_str()), "key {key}");
        }
    }

    #[test]
    fn empty_payload_is_empty_table() {
        assert!(LocTable::from_ui_core(&[]).is_empty());
        assert!(LocTable::from_ui_core(&[0u8; 4096]).is_empty());
    }
}
