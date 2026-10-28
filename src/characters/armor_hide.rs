//! Collapse whole armor pieces to a point so they stop rendering.
//!
//! The engine binds skin weights per vertex index and we cannot change a
//! piece's visibility flag cheaply, so the CJ mod hides Death's base boots
//! and gloves by moving every vertex of their B streams onto the mesh
//! centroid (degenerate triangles). The IBs and per-index bindings stay.

use crate::pack::obpk;
use crate::Result;
use std::path::Path;

fn f32_at(d: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

fn u32_at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

/// One 52-byte armor vertex: `{u32 id <= 64, pos, 1.0, normal, normal.z,
/// tangent, tangent sign}`; the tangent sign is +-1.
fn is_b_record(c: &[u8], o: usize) -> bool {
    o + 52 <= c.len()
        && u32_at(c, o) <= 64
        && (f32_at(c, o + 16) - 1.0).abs() < 1e-4
        && (f32_at(c, o + 48).abs() - 1.0).abs() < 1e-4
}

/// Collapse every B stream (>= 3 records) in `chunk` to its centroid.
fn collapse_runs(chunk: &mut [u8]) -> usize {
    let mut collapsed = 0usize;
    let mut o = 0usize;
    while o + 52 <= chunk.len() {
        if !is_b_record(chunk, o) {
            o += 1;
            continue;
        }
        let start = o;
        let mut n = 0usize;
        while is_b_record(chunk, o) {
            n += 1;
            o += 52;
        }
        if n >= 3 {
            let mut c = [0f64; 3];
            for i in 0..n {
                for (k, ck) in c.iter_mut().enumerate() {
                    *ck += f32_at(chunk, start + i * 52 + 4 + k * 4) as f64;
                }
            }
            let c = c.map(|v| (v / n as f64) as f32);
            for i in 0..n {
                for (k, ck) in c.iter().enumerate() {
                    let at = start + i * 52 + 4 + k * 4;
                    chunk[at..at + 4].copy_from_slice(&ck.to_le_bytes());
                }
            }
            collapsed += 1;
        }
    }
    collapsed
}

/// Longest `{u64 hash, u32 len, name}` chain in the OBPK header.
fn name_table(d: &[u8]) -> Vec<String> {
    let mut best: (usize, usize) = (0, 0);
    for start in 0..d.len().min(1 << 20) {
        let mut p = start;
        let mut n = 0usize;
        while p + 12 <= d.len() {
            let len = u32_at(d, p + 8) as usize;
            if !(2..=120).contains(&len) || p + 12 + len > d.len() {
                break;
            }
            let s = &d[p + 12..p + 12 + len];
            if !(s.iter().all(|&b| (32..127).contains(&b)) && s[0].is_ascii_alphanumeric()) {
                break;
            }
            n += 1;
            p += 12 + len;
        }
        if n > best.1 {
            best = (start, n);
        }
    }
    let (start, n) = best;
    let mut out = Vec::with_capacity(n);
    let mut p = start;
    for _ in 0..n {
        let len = u32_at(d, p + 8) as usize;
        out.push(String::from_utf8_lossy(&d[p + 12..p + 12 + len]).into_owned());
        p += 12 + len;
    }
    out
}

/// Resolve each `name` to its OBPK chunks and collapse every B stream inside.
/// Every occurrence is processed (the small visual entry and the geometry
/// chunk), so the command can be re-run safely.
///
/// Returns one report line per processed record for the caller to print.
pub fn hide_pieces(obpk_path: &Path, dec_path: &Path, names: &[String]) -> Result<Vec<String>> {
    let obpk_data = std::fs::read(obpk_path)?;
    let chunks = obpk::scan_chunks(&obpk_data)?;
    let table = name_table(&obpk_data);
    if chunks.len() != table.len() {
        return Err(format!(
            "{}: {} chunks but {} name records",
            obpk_path.display(),
            chunks.len(),
            table.len()
        )
        .into());
    }
    let mut off = vec![0usize; chunks.len() + 1];
    for (i, c) in chunks.iter().enumerate() {
        off[i + 1] = off[i] + c.usize_;
    }
    let mut dec = std::fs::read(dec_path)?;
    if dec.len() != off[chunks.len()] {
        return Err(format!(
            "{} is {} bytes, expected {}",
            dec_path.display(),
            dec.len(),
            off[chunks.len()]
        )
        .into());
    }
    let mut report = Vec::new();
    for name in names {
        let hits: Vec<usize> = table
            .iter()
            .enumerate()
            .filter(|(_, r)| r.as_str() == name)
            .map(|(i, _)| i)
            .collect();
        if hits.is_empty() {
            return Err(format!("armor part {name} not found").into());
        }
        for idx in hits {
            let n = collapse_runs(&mut dec[off[idx]..off[idx + 1]]);
            report.push(format!(
                "  {name}: rec {idx} dec {:#X}..{:#X}: {n} B run(s) collapsed",
                off[idx],
                off[idx + 1]
            ));
        }
    }
    std::fs::write(dec_path, &dec)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: u32, x: f32) -> Vec<u8> {
        let mut r = vec![0u8; 52];
        r[0..4].copy_from_slice(&id.to_le_bytes());
        r[4..8].copy_from_slice(&x.to_le_bytes());
        r[8..12].copy_from_slice(&(x * 2.0).to_le_bytes());
        r[12..16].copy_from_slice(&(x * 3.0).to_le_bytes());
        r[16..20].copy_from_slice(&1.0f32.to_le_bytes());
        r[48..52].copy_from_slice(&1.0f32.to_le_bytes());
        r
    }

    #[test]
    fn collapses_only_b_runs() {
        let mut c = vec![0xF0u8; 17];
        for x in 1..5 {
            c.extend(record(2, x as f32));
        }
        c.extend([0xAB; 10]);
        let n = collapse_runs(&mut c);
        assert_eq!(n, 1);
        // all four positions collapsed to the same point
        let p: Vec<f32> = (0..4).map(|i| f32_at(&c, 17 + i * 52 + 4)).collect();
        assert!(p.iter().all(|&v| (v - 2.5).abs() < 1e-4));
        // idempotent: a second pass keeps the centroid
        let n2 = collapse_runs(&mut c);
        assert_eq!(n2, 1);
        let p2: Vec<f32> = (0..4).map(|i| f32_at(&c, 17 + i * 52 + 4)).collect();
        assert_eq!(p, p2);
    }
}
