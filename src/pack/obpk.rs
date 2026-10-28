use crate::Result;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{Read, Write};

fn get_u32(data: &[u8], at: usize, big_endian: bool) -> Result<u32> {
    let b: [u8; 4] = data
        .get(at..at + 4)
        .ok_or("OBPK: truncated field")?
        .try_into()
        .unwrap();
    Ok(if big_endian {
        u32::from_be_bytes(b)
    } else {
        u32::from_le_bytes(b)
    })
}

fn set_u32(data: &mut [u8], at: usize, value: u32, big_endian: bool) -> Result<()> {
    let bytes = if big_endian {
        value.to_be_bytes()
    } else {
        value.to_le_bytes()
    };
    data.get_mut(at..at + 4)
        .ok_or("OBPK: truncated field")?
        .copy_from_slice(&bytes);
    Ok(())
}

pub fn is_obpk(data: &[u8]) -> bool {
    data.len() >= 5 && &data[0..4] == b"OBPK"
}

pub fn header(data: &[u8]) -> Result<Vec<u8>> {
    if !is_obpk(data) {
        return Err("not an OBPK archive".into());
    }
    let big_endian = data[4] != 0;
    let data_start = get_u32(data, 25, big_endian)? as usize;
    let end = data_start
        .checked_add(4)
        .filter(|e| *e <= data.len())
        .ok_or("OBPK: header out of range")?;
    Ok(data[..end].to_vec())
}

pub fn decompress(data: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let header = header(data)?;
    let start = header.len();
    let compressed = data
        .get(start..)
        .ok_or("OBPK: compressed payload out of range")?;
    let mut decoder = ZlibDecoder::new(compressed);
    let mut payload = Vec::new();
    decoder.read_to_end(&mut payload)?;
    Ok((header, payload))
}

pub fn compress(header: &[u8], payload: &[u8]) -> Result<Vec<u8>> {
    if !is_obpk(header) || header.len() < 8 {
        return Err("not an OBPK header".into());
    }
    let big_endian = header[4] != 0;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(9));
    encoder.write_all(payload)?;
    let compressed = encoder.finish()?;
    let mut out = header.to_vec();
    // The u32 at data_start is the *decompressed* payload size (stock
    // semantics); the compressed stream length is implied by the zlib end.
    let size_at = out.len() - 4;
    set_u32(&mut out, size_at, payload.len() as u32, big_endian)?;
    out.extend_from_slice(&compressed);
    Ok(out)
}

/// One `[u32 usize][zlib]` chunk of a chunked OBPK payload.
#[derive(Debug, Clone, Copy)]
pub struct Chunk {
    pub pos: usize,
    pub csize: usize,
    pub usize_: usize,
}

/// Walk the `[u32 usize][zlib]` chunk sequence that starts at `data_start`.
pub fn scan_chunks(data: &[u8]) -> Result<Vec<Chunk>> {
    if !is_obpk(data) {
        return Err("not an OBPK archive".into());
    }
    let data_start = get_u32(data, 25, data[4] != 0)? as usize;
    let mut pos = data_start;
    let mut chunks = Vec::new();
    let mut out = Vec::new();
    while pos + 4 <= data.len() {
        let size = get_u32(data, pos, false)? as usize;
        if size == 0 || size > 1 << 30 {
            break;
        }
        let input = data.get(pos + 4..).ok_or("OBPK: chunk out of range")?;
        out.clear();
        out.resize(size, 0);
        let mut d = flate2::Decompress::new(true);
        let status = d.decompress(input, &mut out, flate2::FlushDecompress::Finish)?;
        if status != flate2::Status::StreamEnd || d.total_out() as usize != size {
            return Err(format!("OBPK: chunk at {:#x} does not decompress cleanly", pos).into());
        }
        let csize = d.total_in() as usize;
        chunks.push(Chunk {
            pos,
            csize,
            usize_: size,
        });
        pos += 4 + csize;
    }
    Ok(chunks)
}

/// Stats for one [`repack_chunks`] run.
pub struct RepackReport {
    /// Rebuilt OBPK bytes.
    pub data: Vec<u8>,
    /// Number of chunks in the stock file.
    pub chunks: usize,
    /// Payload bytes consumed (sum of the chunks' uncompressed sizes).
    pub payload_bytes: usize,
    /// Chunks whose compressed bytes had to be rebuilt.
    pub changed: usize,
    /// Stock file size.
    pub stock_bytes: usize,
}

/// Rebuild a chunked OBPK from a decompressed payload. Unchanged chunks keep
/// their exact stock compressed bytes; changed chunks are recompressed (and
/// with `exact`, padded to the stock compressed size so the file layout is
/// byte-for-byte the same length).
pub fn repack_chunks(stock: &[u8], payload: &[u8], exact: bool) -> Result<RepackReport> {
    let chunks = scan_chunks(stock)?;
    let data_start = get_u32(stock, 25, stock[4] != 0)? as usize;
    let end = chunks
        .last()
        .map(|c| c.pos + 4 + c.csize)
        .unwrap_or(data_start);
    let tail = stock
        .get(end..)
        .ok_or("OBPK: chunk table overruns the file")?;
    let mut out = Vec::with_capacity(stock.len());
    out.extend_from_slice(stock.get(..data_start).ok_or("OBPK: header out of range")?);
    let mut o = 0usize;
    let mut changed = 0usize;
    let mut scratch = Vec::new();
    for (i, c) in chunks.iter().enumerate() {
        let slice = payload
            .get(o..o + c.usize_)
            .ok_or_else(|| format!("payload too small at chunk {i}"))?;
        o += c.usize_;
        let compressed = &stock[c.pos + 4..c.pos + 4 + c.csize];
        scratch.clear();
        scratch.resize(c.usize_, 0);
        let mut d = flate2::Decompress::new(true);
        d.decompress(compressed, &mut scratch, flate2::FlushDecompress::Finish)?;
        out.extend_from_slice(&(c.usize_ as u32).to_le_bytes());
        if scratch == slice {
            out.extend_from_slice(compressed);
        } else {
            changed += 1;
            let blob = if exact {
                deflate_exact(slice, c.csize)?
            } else {
                let mut e = ZlibEncoder::new(Vec::new(), Compression::new(9));
                e.write_all(slice)?;
                e.finish()?
            };
            out.extend_from_slice(&blob);
        }
    }
    out.extend_from_slice(tail);
    Ok(RepackReport {
        data: out,
        chunks: chunks.len(),
        payload_bytes: o,
        changed,
        stock_bytes: stock.len(),
    })
}

/// Decompress every chunk of a chunked OBPK into one flat payload.
pub fn unchunk(data: &[u8]) -> Result<Vec<u8>> {
    let chunks = scan_chunks(data)?;
    let total: usize = chunks.iter().map(|c| c.usize_).sum();
    let mut out = Vec::with_capacity(total);
    let mut scratch = Vec::new();
    for c in &chunks {
        scratch.clear();
        scratch.resize(c.usize_, 0);
        let input = data
            .get(c.pos + 4..c.pos + 4 + c.csize)
            .ok_or("OBPK: chunk out of range")?;
        let mut d = flate2::Decompress::new(true);
        d.decompress(input, &mut scratch, flate2::FlushDecompress::Finish)?;
        out.extend_from_slice(&scratch);
    }
    Ok(out)
}

/// Decompress every chunk of `data` and compare it with `payload`.
pub fn verify_chunks(data: &[u8], payload: &[u8]) -> Result<(usize, u64)> {
    let chunks = scan_chunks(data)?;
    let mut o = 0usize;
    let mut out = Vec::new();
    for (i, c) in chunks.iter().enumerate() {
        let slice = payload
            .get(o..o + c.usize_)
            .ok_or_else(|| format!("payload too small at chunk {i}"))?;
        out.clear();
        out.resize(c.usize_, 0);
        let input = data
            .get(c.pos + 4..c.pos + 4 + c.csize)
            .ok_or("OBPK: chunk out of range")?;
        let mut d = flate2::Decompress::new(true);
        d.decompress(input, &mut out, flate2::FlushDecompress::Finish)?;
        if out != slice {
            return Err(format!("chunk {i} content mismatch").into());
        }
        o += c.usize_;
    }
    Ok((chunks.len(), o as u64))
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn empty_fixed_blocks(k: usize) -> Vec<u8> {
    let mut bits = Vec::with_capacity(k * 10);
    for i in 0..k {
        bits.push(if i + 1 == k { 1 } else { 0 });
        bits.push(1);
        bits.push(0);
        bits.extend_from_slice(&[0u8; 7]);
    }
    let mut out = vec![0u8; bits.len().div_ceil(8)];
    for (i, bit) in bits.iter().enumerate() {
        out[i / 8] |= bit << (i % 8);
    }
    out
}

/// A zlib stream of `data` whose total byte length is exactly `target`.
///
/// The deflate stream is cut with a sync flush (non-final, byte aligned) and
/// padded with empty stored/fixed blocks, which produce no output. The
/// decompressed bytes are unchanged.
fn deflate_exact(data: &[u8], target: usize) -> Result<Vec<u8>> {
    use flate2::write::DeflateEncoder;
    if target < 6 {
        return Err("chunk target size too small".into());
    }
    let target_raw = target - 6;
    for level in [9u32, 8, 7, 6, 5, 4, 3, 2, 1] {
        let mut e = DeflateEncoder::new(Vec::new(), Compression::new(level));
        e.write_all(data)?;
        e.flush()?;
        let raw = e.get_mut().clone();
        if raw.len() + 2 > target_raw {
            continue;
        }
        let extra = target_raw - raw.len();
        let mut blob = Vec::with_capacity(target);
        blob.extend_from_slice(&[0x78, 0xDA]);
        blob.extend_from_slice(&raw);
        let mut done = false;
        for k in 1..=8usize {
            let fixed = (10 * k).div_ceil(8);
            if extra >= fixed && (extra - fixed).is_multiple_of(5) {
                for _ in 0..(extra - fixed) / 5 {
                    blob.extend_from_slice(&[0x00, 0x00, 0x00, 0xFF, 0xFF]);
                }
                blob.extend_from_slice(&empty_fixed_blocks(k));
                done = true;
                break;
            }
        }
        if !done {
            continue;
        }
        blob.extend_from_slice(&adler32(data).to_be_bytes());
        if blob.len() != target {
            return Err(format!("padded chunk is {} bytes, wanted {target}", blob.len()).into());
        }
        return Ok(blob);
    }
    Err(format!("cannot pad chunk to the stock size (target {target})").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_with(data_start: u32) -> Vec<u8> {
        let mut h = vec![0u8; data_start as usize + 4];
        h[0..4].copy_from_slice(b"OBPK");
        h[4] = 0;
        h[25..29].copy_from_slice(&data_start.to_le_bytes());
        h
    }

    #[test]
    fn compress_writes_decompressed_size_and_roundtrips() {
        let header = header_with(64);
        let payload = vec![7u8; 1000];
        let out = compress(&header, &payload).unwrap();
        let field = u32::from_le_bytes(out[64..68].try_into().unwrap());
        assert_eq!(
            field,
            payload.len() as u32,
            "field must be the decompressed size"
        );
        let (_, back) = decompress(&out).unwrap();
        assert_eq!(back, payload);
    }
}
