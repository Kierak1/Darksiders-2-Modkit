//! DDS re-bake: encode an RGBA image (or a flat color) into a stock DDS's
//! format/size, keeping the header and mip layout byte-identical.

use crate::Result;

struct DdsInfo {
    header_len: usize,
    width: usize,
    height: usize,
    mips: usize,
    bc3: bool,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn dds_info(stock: &[u8]) -> Result<DdsInfo> {
    if stock.len() < 128 || &stock[0..4] != b"DDS " {
        return Err("not a DDS".into());
    }
    let height = u32_at(stock, 12) as usize;
    let width = u32_at(stock, 16) as usize;
    let mips = (u32_at(stock, 28) as usize).max(1);
    let fourcc = &stock[84..88];
    let header_len = if fourcc == b"DX10" { 148 } else { 128 };
    Ok(DdsInfo {
        header_len,
        width,
        height,
        mips,
        bc3: fourcc == b"DXT5" || fourcc == b"DXT3",
    })
}

fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    (((r as u16) >> 3) << 11) | (((g as u16) >> 2) << 5) | ((b as u16) >> 3)
}

fn expand565(v: u16) -> [u8; 3] {
    let r = (v >> 11) & 0x1F;
    let g = (v >> 5) & 0x3F;
    let b = v & 0x1F;
    [
        ((r << 3) | (r >> 2)) as u8,
        ((g << 2) | (g >> 4)) as u8,
        ((b << 3) | (b >> 2)) as u8,
    ]
}

fn encode_bc1_block(px: &[[u8; 4]]) -> [u8; 8] {
    let mut lo = [255u8; 3];
    let mut hi = [0u8; 3];
    for p in px {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let mut c0 = rgb565(hi[0], hi[1], hi[2]);
    let mut c1 = rgb565(lo[0], lo[1], lo[2]);
    if c0 == c1 {
        let mut out = [0u8; 8];
        out[0..2].copy_from_slice(&c0.to_le_bytes());
        out[2..4].copy_from_slice(&c0.to_le_bytes());
        return out;
    }
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
    }
    let p0 = expand565(c0);
    let p1 = expand565(c1);
    let mut pal = [[0u8; 3]; 4];
    pal[0] = p0;
    pal[1] = p1;
    for k in 0..3 {
        pal[2][k] = ((2 * p0[k] as u16 + p1[k] as u16) / 3) as u8;
        pal[3][k] = ((p0[k] as u16 + 2 * p1[k] as u16) / 3) as u8;
    }
    let mut idx = 0u32;
    for (k, p) in px.iter().enumerate() {
        let mut best = 0usize;
        let mut bd = u32::MAX;
        for (j, q) in pal.iter().enumerate() {
            let d = (p[0] as i32 - q[0] as i32).pow(2) as u32
                + (p[1] as i32 - q[1] as i32).pow(2) as u32
                + (p[2] as i32 - q[2] as i32).pow(2) as u32;
            if d < bd {
                bd = d;
                best = j;
            }
        }
        idx |= (best as u32) << (2 * k);
    }
    let mut out = [0u8; 8];
    out[0..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    out[4..8].copy_from_slice(&idx.to_le_bytes());
    out
}

fn encode_bc3_alpha_block(alphas: &[u8]) -> [u8; 8] {
    let a0 = *alphas.iter().max().unwrap();
    let a1 = *alphas.iter().min().unwrap();
    let mut out = [0u8; 8];
    out[0] = a0;
    out[1] = a1;
    if a0 == a1 {
        return out;
    }
    let mut pal = [0u16; 8];
    pal[0] = a0 as u16;
    pal[1] = a1 as u16;
    for i in 1..7u16 {
        pal[(i + 1) as usize] = (i * a0 as u16 + (7 - i) * a1 as u16) / 7;
    }
    let mut bits = 0u64;
    for (k, &a) in alphas.iter().enumerate() {
        let mut best = 0usize;
        let mut bd = u32::MAX;
        for (j, &q) in pal.iter().enumerate() {
            let d = (a as i32 - q as i32).unsigned_abs();
            if d < bd {
                bd = d;
                best = j;
            }
        }
        bits |= (best as u64) << (3 * k);
    }
    out[2..8].copy_from_slice(&bits.to_le_bytes()[0..6]);
    out
}

fn block_pixels(src: &[u8], w: usize, h: usize, bx: usize, by: usize) -> Vec<[u8; 4]> {
    let mut out = Vec::with_capacity(16);
    for y in by..(by + 4).min(h) {
        for x in bx..(bx + 4).min(w) {
            let o = (y * w + x) * 4;
            out.push([src[o], src[o + 1], src[o + 2], src[o + 3]]);
        }
    }
    out
}

fn encode_mips(rgba: &[u8], mut w: usize, mut h: usize, mips: usize, bc3: bool) -> Vec<u8> {
    let mut src = rgba.to_vec();
    let mut out = Vec::new();
    for _ in 0..mips {
        for by in (0..h).step_by(4) {
            for bx in (0..w).step_by(4) {
                let px = block_pixels(&src, w, h, bx, by);
                if bc3 {
                    let alphas: Vec<u8> = px.iter().map(|p| p[3]).collect();
                    out.extend_from_slice(&encode_bc3_alpha_block(&alphas));
                    out.extend_from_slice(&encode_bc1_block(&px));
                } else {
                    out.extend_from_slice(&encode_bc1_block(&px));
                }
            }
        }
        if w == 1 && h == 1 {
            break;
        }
        let nw = (w / 2).max(1);
        let nh = (h / 2).max(1);
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                let mut acc = [0u32; 4];
                let mut n = 0u32;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let sx = (x * 2 + dx).min(w - 1);
                        let sy = (y * 2 + dy).min(h - 1);
                        let o = (sy * w + sx) * 4;
                        for k in 0..4 {
                            acc[k] += src[o + k] as u32;
                        }
                        n += 1;
                    }
                }
                let o = (y * nw + x) * 4;
                for k in 0..4 {
                    next[o + k] = (acc[k] / n) as u8;
                }
            }
        }
        src = next;
        w = nw;
        h = nh;
    }
    out
}

/// Bake `rgba` (or a flat color when `rgba` is smaller than the target) into
/// the stock DDS's format, keeping the same mip layout. The result is padded
/// to the stock file's length.
pub fn bake(rgba: &[u8], stock: &[u8], fill: Option<[u8; 4]>) -> Result<Vec<u8>> {
    let info = dds_info(stock)?;
    let target = info.width * info.height * 4;
    let data: Vec<u8> = match fill {
        Some(c) => {
            let mut v = Vec::with_capacity(target);
            for _ in 0..info.width * info.height {
                v.extend_from_slice(&c);
            }
            v
        }
        None => {
            if rgba.len() < target {
                return Err("rgba smaller than the target texture".into());
            }
            rgba[..target].to_vec()
        }
    };
    let mut out = stock[..info.header_len].to_vec();
    out.extend_from_slice(&encode_mips(
        &data,
        info.width,
        info.height,
        info.mips,
        info.bc3,
    ));
    if out.len() > stock.len() {
        return Err(format!(
            "baked DDS larger than stock ({} > {})",
            out.len(),
            stock.len()
        )
        .into());
    }
    out.resize(stock.len(), 0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bc1_block_roundtrip_flat() {
        let px = [[255u8, 0, 0, 255]; 16];
        let b = encode_bc1_block(&px);
        let c0 = u16::from_le_bytes([b[0], b[1]]);
        assert_eq!(expand565(c0), [255, 0, 0]);
    }

    #[test]
    fn bake_keeps_stock_size() {
        let mut stock = vec![0u8; 128];
        stock[0..4].copy_from_slice(b"DDS ");
        stock[12..16].copy_from_slice(&4u32.to_le_bytes());
        stock[16..20].copy_from_slice(&4u32.to_le_bytes());
        stock[28..32].copy_from_slice(&1u32.to_le_bytes());
        stock[80..84].copy_from_slice(&0x4u32.to_le_bytes());
        stock[84..88].copy_from_slice(b"DXT1");
        stock.resize(128 + 8, 0);
        let baked = bake(&[], &stock, Some([128, 128, 255, 255])).unwrap();
        assert_eq!(baked.len(), stock.len());
        assert_eq!(&baked[0..4], b"DDS ");
    }
}
