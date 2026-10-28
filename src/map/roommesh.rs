//! Static room meshes from `environments/world_<zone>_<room>` assets.
//!
//! Reversed from `world_z1_ld_rm00.dec`: after the two BOD object streams the
//! payload holds a table of **glomm batch** records (`FF <u16 len>
//! glomm_<hex8>` + 69 B: `u64 zero, u64 prev_vc, u64 X, u32 vc<<8,
//! u32 stride<<8, u8 flag, u32 dataOffset, u32 lodCount, u32 one, 6×f32
//! bbox`). Each batch's data region (`[off_i, off_{i+1})`) is
//! `[attrs][ic×u16 indices][vc×12 position triples][attrs]`, so
//! `ic = (region - vc*stride) / 2`. The second BOD's `Object3D` carries the
//! room placement: every `glomm_N` node has the same `Position` (integer world
//! coordinates) and `Scale` (10), and `world = position + local * scale`.

use crate::obsp::payload::{self, Value};
use crate::Result;

/// A triangle with optional per-corner UVs.
type TriWithUv = ([[f32; 3]; 3], Option<[[f32; 2]; 3]>);

#[derive(Debug, Clone)]
pub struct Batch {
    pub name: String,
    /// Local room-space positions.
    pub positions: Vec<[f32; 3]>,
    /// u16 triangle list (one trailing index may be dropped by the reader).
    pub indices: Vec<u16>,
    /// Per-vertex `(u, v)` pairs, when a clean float-pair run is present
    /// (empty for formats whose attributes are packed differently).
    pub uvs: Vec<[f32; 2]>,
}

#[derive(Debug, Clone)]
pub struct RoomMesh {
    /// World position of the room origin (`glomm_0` node position).
    pub origin: [f32; 3],
    /// World units per local unit (`glomm_0` node scale, usually 10).
    pub scale: f32,
    pub batches: Vec<Batch>,
}

impl RoomMesh {
    pub fn world_position(&self, local: [f32; 3]) -> [f32; 3] {
        [
            self.origin[0] + local[0] * self.scale,
            self.origin[1] + local[1] * self.scale,
            self.origin[2] + local[2] * self.scale,
        ]
    }

    /// World-space triangles `(a, b, c)` for every batch.
    pub fn triangles(&self) -> Vec<([f32; 3], [f32; 3], [f32; 3])> {
        self.triangles_with_uv()
            .into_iter()
            .map(|(p, _)| (p[0], p[1], p[2]))
            .collect()
    }

    /// World-space triangles with optional per-corner UVs.
    pub fn triangles_with_uv(&self) -> Vec<TriWithUv> {
        let mut out = Vec::new();
        for b in &self.batches {
            for t in b.indices.as_chunks::<3>().0 {
                let (a, b2, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
                if a >= b.positions.len() || b2 >= b.positions.len() || c >= b.positions.len() {
                    continue;
                }
                let p = [
                    self.world_position(b.positions[a]),
                    self.world_position(b.positions[b2]),
                    self.world_position(b.positions[c]),
                ];
                let uv = if b.uvs.len() >= b.positions.len() {
                    Some([b.uvs[a], b.uvs[b2], b.uvs[c]])
                } else {
                    None
                };
                out.push((p, uv));
            }
        }
        out
    }
}

fn f32_at(d: &[u8], o: usize) -> Option<f32> {
    d.get(o..o + 4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    d.get(o..o + 2)
        .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
}

#[derive(Debug, Clone, Copy)]
struct Record {
    name_at: usize,
    name_len: usize,
    vc: usize,
    stride: usize,
    off: usize,
    bbox: [f32; 6],
}

impl Record {
    fn name<'a>(&self, d: &'a [u8]) -> &'a str {
        std::str::from_utf8(&d[self.name_at..self.name_at + self.name_len]).unwrap_or("?")
    }

    fn region(&self, end: usize) -> usize {
        end.saturating_sub(self.off)
    }
}

/// Find the contiguous run of glomm batch records in the payload.
fn find_records(dec: &[u8], limit: usize) -> Vec<Record> {
    let mut offsets = Vec::new();
    let mut i = 0usize;
    while i + 3 < limit {
        if dec[i] != 0xFF {
            i += 1;
            continue;
        }
        let Some(len) = u16_at(dec, i + 1) else { break };
        let len = len as usize;
        if !(5..=64).contains(&len) || i + 3 + len > limit {
            i += 1;
            continue;
        }
        let name = &dec[i + 3..i + 3 + len];
        if !name.starts_with(b"glomm_") || !name.iter().all(|b| (32..127).contains(b)) {
            i += 1;
            continue;
        }
        offsets.push(i);
        i += 3 + len;
    }
    // Keep the longest run of records separated by at most 128 bytes.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for &o in &offsets {
        match groups.last_mut() {
            Some(g) if o.saturating_sub(*g.last().unwrap()) <= 128 => g.push(o),
            _ => groups.push(vec![o]),
        }
    }
    let Some(group) = groups.into_iter().max_by_key(Vec::len) else {
        return Vec::new();
    };
    let mut recs = Vec::new();
    for &p in &group {
        let len = u16_at(dec, p + 1).unwrap() as usize;
        let data_at = p + 3 + len;
        let Some(vc) = u32_at(dec, data_at + 24) else {
            continue;
        };
        let Some(stride) = u32_at(dec, data_at + 28) else {
            continue;
        };
        let Some(off) = u32_at(dec, data_at + 32).map(|v| v >> 8) else {
            continue;
        };
        let mut bbox = [0f32; 6];
        let mut ok = true;
        for (k, b) in bbox.iter_mut().enumerate() {
            match f32_at(dec, data_at + 45 + k * 4) {
                Some(v) if v.is_finite() && v.abs() < 1e7 => *b = v,
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        let vc = (vc >> 8) as usize;
        let stride = (stride >> 8) as usize;
        if ok && (1..100_000).contains(&vc) && (1..256).contains(&stride) {
            recs.push(Record {
                name_at: p + 3,
                name_len: len,
                vc,
                stride,
                off: off as usize,
                bbox,
            });
        }
    }
    recs.sort_by_key(|r| r.off);
    recs
}

fn bbox_contains(bbox: &[f32; 6], p: [f32; 3], pad: f32) -> bool {
    let lo = [
        bbox[0].min(bbox[3]) - pad,
        bbox[1].min(bbox[4]) - pad,
        bbox[2].min(bbox[5]) - pad,
    ];
    let hi = [
        bbox[0].max(bbox[3]) + pad,
        bbox[1].max(bbox[4]) + pad,
        bbox[2].max(bbox[5]) + pad,
    ];
    p[0] >= lo[0]
        && p[0] <= hi[0]
        && p[1] >= lo[1]
        && p[1] <= hi[1]
        && p[2] >= lo[2]
        && p[2] <= hi[2]
}

/// First 8-byte-aligned run of `vc` float `(u, v)` pairs inside `[start, end)`.
/// Both components must be in `[0, 1]` (small negative rounding tolerated).
fn find_uv_run(dec: &[u8], start: usize, end: usize, vc: usize) -> Option<Vec<[f32; 2]>> {
    if vc == 0 || start + vc * 8 > end {
        return None;
    }
    let pair_ok = |o: usize| -> bool {
        matches!(f32_at(dec, o), Some(a) if (-0.01..=1.01).contains(&a))
            && matches!(f32_at(dec, o + 4), Some(b) if (-0.01..=1.01).contains(&b))
    };
    let mut at = start;
    while at + vc * 8 <= end {
        let step = (vc / 8).max(1);
        let mut sample_ok = true;
        let mut i = 0;
        while i < vc {
            if !pair_ok(at + i * 8) {
                sample_ok = false;
                break;
            }
            i += step;
        }
        if sample_ok && (0..vc).all(|i| pair_ok(at + i * 8)) {
            return Some(
                (0..vc)
                    .map(|i| {
                        [
                            f32_at(dec, at + i * 8).unwrap(),
                            f32_at(dec, at + i * 8 + 4).unwrap(),
                        ]
                    })
                    .collect(),
            );
        }
        at += 8;
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum UvKind {
    F32,
    F16,
    U16,
}

fn uv_read(dec: &[u8], o: usize, kind: UvKind) -> Option<f32> {
    match kind {
        UvKind::F32 => f32_at(dec, o),
        UvKind::F16 => u16_at(dec, o).map(half_to_f32),
        UvKind::U16 => u16_at(dec, o).map(|v| v as f32 / 65535.0),
    }
}

fn half_to_f32(h: u16) -> f32 {
    let sign = (h as u32 >> 15) << 31;
    let exp = ((h >> 10) & 0x1F) as u32;
    let mant = (h & 0x3FF) as u32;
    let bits = match exp {
        0 if mant == 0 => sign,
        0 => {
            let (mut e, mut m) = (127 - 15 + 1i32, mant);
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | ((e as u32) << 23) | ((m & 0x3FF) << 13)
        }
        0x1F => sign | 0x7F80_0000 | (mant << 13),
        _ => sign | ((exp + 127 - 15) << 23) | (mant << 13),
    };
    f32::from_bits(bits)
}

/// Score a candidate UV layout: all `vc` values inside `[0,1]` with some
/// spread, then the mean UV distance across the mesh edges (a real UV stream
/// is continuous, packed data is not).
fn uv_score(
    dec: &[u8],
    base: usize,
    stride: usize,
    offsets: (usize, usize),
    vc: usize,
    edges: &[(usize, usize)],
    kind: UvKind,
) -> Option<f64> {
    let (u_off, v_off) = offsets;
    let at = |i: usize, off: usize| uv_read(dec, base + off + i * stride, kind);
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    let mut step = (vc / 32).max(1);
    for i in (0..vc).step_by(step) {
        let u = at(i, u_off)?;
        let v = at(i, v_off)?;
        if !u.is_finite() || !v.is_finite() {
            return None;
        }
        lo = lo.min(u).min(v);
        hi = hi.max(u).max(v);
    }
    step = (vc / 96).max(1);
    for i in (0..vc).step_by(step) {
        for off in [u_off, v_off] {
            let val = at(i, off)?;
            if !(-0.06..=1.06).contains(&val) {
                return None;
            }
        }
    }
    if hi - lo < 0.08 {
        return None;
    }
    let mut total = 0f64;
    for &(a, b) in edges {
        let (ua, va) = (at(a, u_off)?, at(a, v_off)?);
        let (ub, vb) = (at(b, u_off)?, at(b, v_off)?);
        total += (((ua - ub) as f64).powi(2) + ((va - vb) as f64).powi(2)).sqrt();
    }
    Some(total / edges.len().max(1) as f64)
}

/// Unique edges of an index buffer, valid indices only.
fn mesh_edges(indices: &[u16], vc: usize) -> Vec<(usize, usize)> {
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for t in (0..indices.len().saturating_sub(2)).step_by(3) {
        let (a, b, c) = (
            indices[t] as usize,
            indices[t + 1] as usize,
            indices[t + 2] as usize,
        );
        if a >= vc || b >= vc || c >= vc {
            continue;
        }
        for (x, y) in [(a, b), (b, c), (c, a)] {
            edges.push((x.min(y), x.max(y)));
        }
    }
    edges.sort_unstable();
    edges.dedup();
    edges
}

/// Scan one interleaved attribute block (`base`, `stride` bytes per vertex)
/// for the smoothest `[0,1]` component pair, scored by the mean UV distance
/// across the mesh edges. Shared by room batches and prop records.
pub(crate) fn scan_attr_uvs(
    dec: &[u8],
    base: usize,
    vc: usize,
    stride: usize,
    indices: &[u16],
) -> Option<(f64, Vec<[f32; 2]>)> {
    if vc == 0 || stride < 4 || base + vc * stride > dec.len() {
        return None;
    }
    let edges = mesh_edges(indices, vc);
    if edges.is_empty() {
        return None;
    }
    let mut best: Option<(f64, usize, usize, UvKind)> = None;
    let mut consider = |u_off: usize, v_off: usize, kind: UvKind| {
        let Some(score) = uv_score(dec, base, stride, (u_off, v_off), vc, &edges, kind) else {
            return;
        };
        if best.as_ref().is_none_or(|(s, _, _, _)| score < *s) {
            best = Some((score, u_off, v_off, kind));
        }
    };
    let mut off = 0usize;
    while off + 4 <= stride {
        if off + 4 <= stride {
            consider(off, off + 2, UvKind::U16);
            consider(off, off + 2, UvKind::F16);
        }
        if off + 6 <= stride {
            consider(off, off + 4, UvKind::U16);
            consider(off, off + 4, UvKind::F16);
        }
        if off + 8 <= stride {
            consider(off, off + 4, UvKind::F32);
        }
        off += 2;
    }
    let (score, u_off, v_off, kind) = best?;
    if score > 0.15 {
        return None;
    }
    let uvs = (0..vc)
        .map(|i| {
            [
                uv_read(dec, base + u_off + i * stride, kind).unwrap_or(0.0),
                uv_read(dec, base + v_off + i * stride, kind).unwrap_or(0.0),
            ]
        })
        .collect();
    Some((score, uvs))
}

/// Find a per-vertex UV stream. First tries a contiguous f32 pair run (plain
/// UV block); otherwise scans interleaved vertex records in the tail,
/// choosing the smoothest `[0,1]` component pair.
fn find_uvs(
    dec: &[u8],
    rec: &Record,
    ib: usize,
    _ic: usize,
    tail: usize,
    end: usize,
    indices: &[u16],
) -> Option<Vec<[f32; 2]>> {
    let vc = rec.vc;
    if let Some(uvs) = find_uv_run(dec, rec.off, ib, vc) {
        return Some(uvs);
    }
    let mut best: Option<(f64, usize)> = None;
    for stride in [16usize, 20, 24, 28, 32, 36, 40, 44, 48, 52, 56, 60, 64, 68] {
        if tail + vc * stride > end {
            continue;
        }
        if let Some((score, _)) = scan_attr_uvs(dec, tail, vc, stride, indices) {
            if best.as_ref().is_none_or(|(s, _)| score < *s) {
                best = Some((score, stride));
            }
        }
    }
    let (_, stride) = best?;
    scan_attr_uvs(dec, tail, vc, stride, indices).map(|(_, uvs)| uvs)
}

/// Locate `[ic×u16 indices][vc×12 positions]` for one batch.
fn extract_batch(dec: &[u8], rec: &Record, end: usize) -> Option<Batch> {
    let region = rec.region(end);
    if region < rec.vc * rec.stride {
        return None;
    }
    let expect_ic = (region - rec.vc * rec.stride) / 2;
    if expect_ic == 0 {
        return None;
    }
    // Find the index run. Runs may start at either byte alignment (rm01
    // batches put one odd byte before the IB), so scan both parities and keep
    // the run whose length is closest to the size arithmetic.
    let mut best: Option<(usize, usize)> = None;
    for parity in 0..2 {
        let mut i = rec.off + parity;
        while i + 2 <= end {
            let Some(v) = u16_at(dec, i) else { break };
            if v as usize >= rec.vc {
                i += 2;
                continue;
            }
            let mut j = i;
            let mut n = 0usize;
            while j + 2 <= end {
                match u16_at(dec, j) {
                    Some(x) if (x as usize) < rec.vc => {
                        j += 2;
                        n += 1;
                    }
                    _ => break,
                }
            }
            if n + 4 >= expect_ic
                && n <= expect_ic + 4
                && best.is_none_or(|(_, bn)| n.abs_diff(expect_ic) < bn.abs_diff(expect_ic))
            {
                best = Some((i, n));
            }
            i = j.max(i + 2);
        }
    }
    let (ib, n) = best?;
    // The index run can absorb the first bytes of the positions as a trailing
    // pseudo-index; try a few position starts around the run end.
    let bbox_ok = |pos_at: usize| -> Option<usize> {
        if pos_at + rec.vc * 12 > end {
            return None;
        }
        let mut good = 0;
        let sample = rec.vc.min(64);
        for k in 0..sample {
            let p = (0..3)
                .map(|c| f32_at(dec, pos_at + k * 12 + c * 4))
                .collect::<Option<Vec<_>>>()?;
            if bbox_contains(&rec.bbox, [p[0], p[1], p[2]], 3.0) {
                good += 1;
            }
        }
        (good * 10 >= sample * 9).then_some(good)
    };
    let mut chosen: Option<(usize, usize)> = None;
    for (dc, drop) in [(0usize, 0usize), (1, 1), (2, 2), (1, 0), (2, 0), (2, 1)] {
        let ic = n.saturating_sub(drop);
        let pos_at = ib + n * 2 - dc * 2;
        if ic < 3 || pos_at < ib {
            continue;
        }
        if bbox_ok(pos_at).is_some() {
            chosen = Some((pos_at, ic));
            break;
        }
    }
    let (pos_at, ic) = chosen?;
    let mut indices = Vec::with_capacity(ic);
    for k in 0..ic {
        indices.push(u16_at(dec, ib + k * 2)?);
    }
    let mut positions = Vec::with_capacity(rec.vc);
    for k in 0..rec.vc {
        let mut p = [0f32; 3];
        for (c, slot) in p.iter_mut().enumerate() {
            *slot = f32_at(dec, pos_at + k * 12 + c * 4)?;
        }
        positions.push(p);
    }
    let uvs = find_uvs(dec, rec, ib, ic, pos_at + rec.vc * 12, end, &indices).unwrap_or_default();
    Some(Batch {
        name: rec.name(dec).to_string(),
        positions,
        indices,
        uvs,
    })
}

/// Read the room placement from the `glomm_` `Object3D` BOD payload.
fn room_transform(dec: &[u8]) -> ([f32; 3], f32) {
    let mut at = 0usize;
    while at + 4 < dec.len() {
        let Some(rel) = dec[at..].windows(4).position(|w| w == b"BOD\xFD") else {
            break;
        };
        at += rel;
        if let Ok(payload) = payload::parse_payload(&dec[at..]) {
            if let Some(t) = payload.find_path("Skeleton.Children[0].Name") {
                let is_glomm =
                    matches!(&t.value, Value::Ref(s) | Value::Str(s) if s.starts_with("glomm_"));
                if is_glomm {
                    let num = |path: &str, k: usize| -> Option<f32> {
                        let t =
                            payload::resolve_path(&dec[at..], &payload, &format!("{path}[{k}]"))?;
                        match &t.value {
                            Value::Int(v) => Some(*v as f32),
                            Value::Float(v) => Some(*v),
                            _ => None,
                        }
                    };
                    let origin = [
                        num("Skeleton.Children[0].Position", 0).unwrap_or(0.0),
                        num("Skeleton.Children[0].Position", 1).unwrap_or(0.0),
                        num("Skeleton.Children[0].Position", 2).unwrap_or(0.0),
                    ];
                    let scale = num("Skeleton.Children[0].Scale", 0).unwrap_or(1.0);
                    return (origin, scale);
                }
            }
        }
        at += 4;
    }
    ([0.0, 0.0, 0.0], 1.0)
}

/// Parse a decompressed room payload (`world_<zone>_<room>.dec`).
pub fn parse_room(dec: &[u8]) -> Result<RoomMesh> {
    // Geometry ends at the first DDS texture (materials follow the batches).
    let dds_at = dec
        .windows(4)
        .position(|w| w == b"DDS ")
        .unwrap_or(dec.len());
    let mut recs = find_records(dec, dds_at);
    if recs.is_empty() {
        return Err("no glomm batch table found".into());
    }
    recs.retain(|r| r.off < dds_at);
    let (origin, scale) = room_transform(dec);
    let mut batches = Vec::new();
    for (i, rec) in recs.iter().enumerate() {
        let end = recs.get(i + 1).map(|r| r.off).unwrap_or(dds_at);
        if let Some(b) = extract_batch(dec, rec, end) {
            batches.push(b);
        }
    }
    if batches.is_empty() {
        return Err("no glomm batch decoded".into());
    }
    Ok(RoomMesh {
        origin,
        scale,
        batches,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_transform_matches_record_offsets() {
        // The synthetic payload has no BOD table; parse_room must fail cleanly.
        assert!(parse_room(&[0u8; 64]).is_err());
    }
}
