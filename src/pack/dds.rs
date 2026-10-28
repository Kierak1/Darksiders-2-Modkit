use crate::Result;
use std::collections::BTreeMap;
use std::path::Path;

pub fn find_textures(payload: &[u8]) -> Vec<usize> {
    payload
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"DDS ")
        .map(|(i, _)| i)
        .collect()
}

pub fn dds_size_at(data: &[u8], offset: usize) -> u64 {
    if data.len().saturating_sub(offset) < 128 {
        return 0;
    }
    let u32_at = |at: usize| -> u32 {
        data.get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .unwrap_or(0)
    };
    if &data[offset..offset + 4] != b"DDS " {
        return 0;
    }
    let height = u32_at(offset + 12);
    let width = u32_at(offset + 16);
    let mipmaps = u32_at(offset + 28);
    let pixel_flags = u32_at(offset + 80);
    let fourcc = data.get(offset + 84..offset + 88).unwrap_or(&[]);

    let mut header_size = 128u64;
    let mut is_dx10 = false;
    if pixel_flags & 0x4 != 0 && fourcc == b"DX10" {
        is_dx10 = true;
        header_size += 20;
    }

    let num_mips = mipmaps.max(1);
    let mut is_compressed = false;
    let mut block_size = 0u64;

    if pixel_flags & 0x4 != 0 {
        is_compressed = true;
        match fourcc {
            b"DXT1" | b"BC1U" | b"BC4U" => block_size = 8,
            b"DXT2" | b"DXT3" | b"DXT4" | b"DXT5" | b"BC2U" | b"BC3U" | b"BC5U" | b"ATI2" => {
                block_size = 16
            }
            b"DX10" if is_dx10 && data.len().saturating_sub(offset) >= 148 => {
                let dxgi_format = u32_at(offset + 128);
                match dxgi_format {
                    70..=72 | 79..=81 => block_size = 8,
                    73..=78 | 82..=84 | 97..=99 => block_size = 16,
                    _ => is_compressed = false,
                }
            }
            _ => is_compressed = false,
        }
    }

    let mut total = 0u64;
    let (mut w, mut h) = (width, height);
    if is_compressed && block_size > 0 {
        for _ in 0..num_mips {
            total += w.div_ceil(4).max(1) as u64 * h.div_ceil(4).max(1) as u64 * block_size;
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
    } else {
        let bit_count = u32_at(offset + 88);
        let bpp = if bit_count > 0 { bit_count } else { 32 };
        for _ in 0..num_mips {
            let row_pitch = (w * bpp).div_ceil(32) * 4;
            total += row_pitch as u64 * h as u64;
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
    }

    header_size + total
}

/// Decode the first mip of a DXT1/DXT3/DXT5 (BC1/BC2/BC3) DDS into RGBA8.
pub fn decode(dds: &[u8]) -> Result<(usize, usize, Vec<[u8; 4]>)> {
    if dds.len() < 128 || &dds[..4] != b"DDS " {
        return Err("not a DDS file".into());
    }
    let u32_at = |at: usize| u32::from_le_bytes(dds[at..at + 4].try_into().unwrap());
    let h = u32_at(12) as usize;
    let w = u32_at(16) as usize;
    if w == 0 || h == 0 || w > 8192 || h > 8192 {
        return Err("bad DDS dimensions".into());
    }
    let (block, alpha_kind) = match &dds[84..88] {
        b"DXT1" => (8usize, 0u8),
        b"DXT3" => (16, 1),
        b"DXT5" => (16, 2),
        other => {
            return Err(format!(
                "unsupported DDS format {:?}",
                String::from_utf8_lossy(other)
            )
            .into())
        }
    };
    let data = dds.get(128..).ok_or("truncated DDS")?;
    let bw = w.div_ceil(4);
    let bh = h.div_ceil(4);
    if data.len() < bw * bh * block {
        return Err("truncated DDS blocks".into());
    }
    let mut out = vec![[0u8; 4]; w * h];
    let expand = |c: u16| -> [u8; 3] {
        [
            (((c >> 11) & 0x1F) as u32 * 255 / 31) as u8,
            (((c >> 5) & 0x3F) as u32 * 255 / 63) as u8,
            ((c & 0x1F) as u32 * 255 / 31) as u8,
        ]
    };
    for by in 0..bh {
        for bx in 0..bw {
            let o = (by * bw + bx) * block;
            let b = &data[o..o + block];
            let mut alphas = [255u8; 16];
            let color_at = match alpha_kind {
                2 => {
                    let a0 = b[0];
                    let a1 = b[1];
                    let mut idx = 0u64;
                    for (k, byte) in b[2..8].iter().enumerate() {
                        idx |= (*byte as u64) << (8 * k);
                    }
                    let mut table = [0u8; 8];
                    table[0] = a0;
                    table[1] = a1;
                    if a0 > a1 {
                        for i in 0..6u16 {
                            table[2 + i as usize] =
                                ((a0 as u16 * (5 - i) + a1 as u16 * (1 + i)) / 7) as u8;
                        }
                    } else {
                        for i in 0..4u16 {
                            table[2 + i as usize] =
                                ((a0 as u16 * (3 - i) + a1 as u16 * i) / 5) as u8;
                        }
                        table[6] = 0;
                        table[7] = 255;
                    }
                    for (p, alpha) in alphas.iter_mut().enumerate() {
                        *alpha = table[((idx >> (3 * p)) & 7) as usize];
                    }
                    8
                }
                1 => {
                    for (p, alpha) in alphas.iter_mut().enumerate() {
                        let nib = (b[p / 2] >> (4 * (p % 2))) & 0xF;
                        *alpha = (nib as u16 * 17) as u8;
                    }
                    8
                }
                _ => 0,
            };
            let c = &b[color_at..color_at + 8];
            let c0 = u16::from_le_bytes([c[0], c[1]]);
            let c1 = u16::from_le_bytes([c[2], c[3]]);
            let e0 = expand(c0);
            let e1 = expand(c1);
            let mut table = [[0u8; 3]; 4];
            table[0] = e0;
            table[1] = e1;
            if c0 > c1 || alpha_kind != 0 {
                for i in 0..3 {
                    table[2][i] = ((2 * e0[i] as u16 + e1[i] as u16) / 3) as u8;
                    table[3][i] = ((e0[i] as u16 + 2 * e1[i] as u16) / 3) as u8;
                }
            } else {
                for i in 0..3 {
                    table[2][i] = ((e0[i] as u16 + e1[i] as u16) / 2) as u8;
                }
                table[3] = [0, 0, 0];
            }
            let idx = u32::from_le_bytes(c[4..8].try_into().unwrap());
            for p in 0..16 {
                let px = bx * 4 + (p & 3);
                let py = by * 4 + (p >> 2);
                if px >= w || py >= h {
                    continue;
                }
                let col = table[((idx >> (2 * p)) & 3) as usize];
                out[py * w + px] = [col[0], col[1], col[2], alphas[p]];
            }
        }
    }
    Ok((w, h, out))
}

fn texture_index(filename: &str) -> Option<usize> {
    let lower = filename.to_lowercase();
    let rest = lower.strip_prefix("texture_")?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// Result of [`extract`]: the number of texture offsets found and one report
/// line per written file.
#[derive(Debug)]
pub struct ExtractReport {
    pub count: usize,
    pub lines: Vec<String>,
}

/// Extract every DDS blob in `payload` to `out_dir`.
pub fn extract(payload: &[u8], out_dir: &Path) -> Result<ExtractReport> {
    let offsets = find_textures(payload);
    std::fs::create_dir_all(out_dir)?;
    let mut lines = Vec::new();
    for (i, &off) in offsets.iter().enumerate() {
        if off + 20 > payload.len() {
            continue;
        }
        let height = u32::from_le_bytes(payload[off + 12..off + 16].try_into().unwrap());
        let width = u32::from_le_bytes(payload[off + 16..off + 20].try_into().unwrap());
        let calc_size = dds_size_at(payload, off) as usize;
        let next_off = offsets.get(i + 1).copied().unwrap_or(payload.len());
        let end = if calc_size > 0 && calc_size <= next_off - off {
            off + calc_size
        } else {
            next_off
        };
        let path = out_dir.join(format!("texture_{i}_{width}x{height}.dds"));
        std::fs::write(&path, &payload[off..end])?;
        lines.push(format!(
            "  texture_{i}: {width}x{height} -> {} ({} bytes)",
            path.display(),
            end - off
        ));
    }
    Ok(ExtractReport {
        count: offsets.len(),
        lines,
    })
}

pub fn inject(payload: &[u8], textures_dir: &Path) -> Result<Vec<u8>> {
    let mut replacements: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
    for entry in std::fs::read_dir(textures_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.to_lowercase().ends_with(".dds") {
            continue;
        }
        if let Some(idx) = texture_index(&name) {
            replacements.insert(idx, std::fs::read(entry.path())?);
        }
    }
    if replacements.is_empty() {
        return Ok(payload.to_vec());
    }

    let offsets = find_textures(payload);
    let mut out = Vec::with_capacity(payload.len());
    let mut last_off = 0usize;
    for (i, &off) in offsets.iter().enumerate() {
        out.extend_from_slice(&payload[last_off..off]);
        let calc_size = dds_size_at(payload, off) as usize;
        let next_off = offsets.get(i + 1).copied().unwrap_or(payload.len());
        let (orig_len, trailing) = if calc_size > 0 && calc_size <= next_off - off {
            (calc_size, Some((off + calc_size, next_off)))
        } else {
            (next_off - off, None)
        };
        if let Some(new_dds) = replacements.get(&i) {
            if new_dds.len() <= orig_len {
                out.extend_from_slice(new_dds);
                out.extend(std::iter::repeat_n(0u8, orig_len - new_dds.len()));
            } else {
                out.extend_from_slice(new_dds);
            }
        } else {
            out.extend_from_slice(&payload[off..off + orig_len]);
        }
        if let Some((start, end)) = trailing {
            out.extend_from_slice(&payload[start..end]);
        }
        last_off = next_off;
    }
    out.extend_from_slice(&payload[last_off..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dxt1_dds(width: u32, height: u32, mips: u32) -> Vec<u8> {
        let mut d = vec![0u8; 128];
        d[0..4].copy_from_slice(b"DDS ");
        d[12..16].copy_from_slice(&height.to_le_bytes());
        d[16..20].copy_from_slice(&width.to_le_bytes());
        d[28..32].copy_from_slice(&mips.to_le_bytes());
        d[80..84].copy_from_slice(&0x4u32.to_le_bytes());
        d[84..88].copy_from_slice(b"DXT1");
        d
    }

    #[test]
    fn dxt1_size_is_header_plus_blocks() {
        let dds = dxt1_dds(4, 4, 1);
        assert_eq!(dds_size_at(&dds, 0), 128 + 8);
    }

    #[test]
    fn dxt1_mips_halve() {
        let dds = dxt1_dds(8, 8, 3);
        assert_eq!(dds_size_at(&dds, 0), 128 + 32 + 8 + 8);
    }

    #[test]
    fn truncation_returns_zero() {
        assert_eq!(dds_size_at(b"DDS ", 0), 0);
    }

    #[test]
    fn decode_dxt1_solid_color() {
        let mut dds = dxt1_dds(4, 4, 1);
        // One BC1 block, endpoints = pure red (565 0xF800), all indices 0.
        dds.extend_from_slice(&[0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0]);
        let (w, h, px) = decode(&dds).unwrap();
        assert_eq!((w, h), (4, 4));
        assert!(px
            .iter()
            .all(|p| p[0] == 255 && p[1] == 0 && p[2] == 0 && p[3] == 255));
    }
}
