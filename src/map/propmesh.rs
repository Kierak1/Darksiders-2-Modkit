//! Static prop meshes from media environment assets (`world/environments/...`).
//!
//! Media env assets contain named mesh records in a geometry region after the
//! `Object3D` BOD definitions; each record is:
//!
//! `[9×u32][u8][5×u32 vc, stride, f2, f3, f4][6×f32 bbox][FF <u16> name]`
//!
//! followed by a small block, the index buffer (`vc`-bounded u16 triangle
//! list) and, immediately after the IB, a `vc × 12` byte position array
//! (f32 triples inside the record bbox). Attributes follow the positions
//! (`vc × (stride - 12)` bytes: normal, tangent, uv). Verified on
//! `z2_dlc_rocks.dec` (`Z2_CrowRock_02_Tile_DLC`, vc 882, 482 tris) and
//! `zone01_flora.dec` (single sub-mesh groups).
//!
//! Materials are `Material` BOD objects named exactly like the records; their
//! `Parameters.DiffuseMap1.Value` is the diffuse texture name, from which a
//! representative color is derived (texture pixel binding is still open).

use crate::obsp::payload::{self, Value};
use std::collections::HashMap;

/// A triangle with optional per-corner UVs.
type TriWithUv = ([[f32; 3]; 3], Option<[[f32; 2]; 3]>);

#[derive(Debug, Clone)]
pub struct PropMesh {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u16>,
    pub uvs: Vec<[f32; 2]>,
    pub color: [u8; 3],
}

impl PropMesh {
    /// World-space triangles with the mesh color.
    pub fn triangles(&self) -> Vec<([[f32; 3]; 3], [u8; 3])> {
        let mut out = Vec::with_capacity(self.indices.len() / 3);
        for t in self.indices.as_chunks::<3>().0 {
            let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
            if a >= self.positions.len() || b >= self.positions.len() || c >= self.positions.len() {
                continue;
            }
            out.push((
                [self.positions[a], self.positions[b], self.positions[c]],
                self.color,
            ));
        }
        out
    }

    /// World-space triangles with per-corner UVs when the record carries a
    /// decodable UV stream.
    pub fn triangles_with_uv(&self) -> Vec<TriWithUv> {
        let mut out = Vec::with_capacity(self.indices.len() / 3);
        for t in self.indices.as_chunks::<3>().0 {
            let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
            if a >= self.positions.len() || b >= self.positions.len() || c >= self.positions.len() {
                continue;
            }
            let uv = (self.uvs.len() >= self.positions.len())
                .then(|| [self.uvs[a], self.uvs[b], self.uvs[c]]);
            out.push((
                [self.positions[a], self.positions[b], self.positions[c]],
                uv,
            ));
        }
        out
    }
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    d.get(o..o + 2)
        .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
}

fn f32_at(d: &[u8], o: usize) -> Option<f32> {
    d.get(o..o + 4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
}

fn bbox_ok(b: &[f32; 6]) -> bool {
    if !b.iter().all(|v| v.is_finite() && v.abs() < 1e7) {
        return false;
    }
    (0..3).all(|k| {
        let (lo, hi) = (b[k].min(b[k + 3]), b[k].max(b[k + 3]));
        hi - lo > 0.0
    })
}

fn in_bbox(b: &[f32; 6], p: [f32; 3], pad: f32) -> bool {
    (0..3).all(|k| {
        let (lo, hi) = (b[k].min(b[k + 3]) - pad, b[k].max(b[k + 3]) + pad);
        p[k] >= lo && p[k] <= hi
    })
}

/// Decode every single-record prop mesh in a decompressed media env asset.
pub fn parse_props(dec: &[u8]) -> Vec<PropMesh> {
    let colors = material_colors(dec);
    let mut out = Vec::new();
    let mut o = 0usize;
    while o + 90 < dec.len() {
        let Some(rec) = parse_record(dec, o) else {
            o += 1;
            continue;
        };
        if let Some(mesh) = decode_record(dec, &rec, &colors) {
            out.push(mesh);
        }
        o = rec.name_end;
    }
    out
}

struct Record {
    name: String,
    name_end: usize,
    vc: usize,
    stride: usize,
    bbox: [f32; 6],
}

fn parse_record(d: &[u8], o: usize) -> Option<Record> {
    if *d.get(o + 81)? != 0xFF {
        return None;
    }
    let len = u16_at(d, o + 82)? as usize;
    if !(3..64).contains(&len) || o + 84 + len > d.len() {
        return None;
    }
    let name = std::str::from_utf8(d.get(o + 84..o + 84 + len)?).ok()?;
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    let vc = u32_at(d, o + 37)? as usize;
    if !(1..200_000).contains(&vc) {
        return None;
    }
    let stride = u32_at(d, o + 41)? as usize;
    let mut bbox = [0f32; 6];
    for (k, b) in bbox.iter_mut().enumerate() {
        *b = f32_at(d, o + 57 + k * 4)?;
    }
    if !bbox_ok(&bbox) {
        return None;
    }
    Some(Record {
        name: name.to_string(),
        name_end: o + 84 + len,
        vc,
        stride,
        bbox,
    })
}

/// Find the index buffer (longest u16 run with all values < vc) and the
/// position array right after it.
fn decode_record(d: &[u8], rec: &Record, colors: &HashMap<String, [u8; 3]>) -> Option<PropMesh> {
    let search_end = (rec.name_end + rec.vc * 16 + 4096).min(d.len());
    let mut best: Option<(usize, usize)> = None;
    for parity in 0..2 {
        let mut i = rec.name_end + parity;
        while i + 2 <= search_end {
            let v = u16_at(d, i)?;
            if v as usize >= rec.vc {
                i += 2;
                continue;
            }
            let mut j = i;
            while j + 2 <= search_end && (u16_at(d, j)? as usize) < rec.vc {
                j += 2;
            }
            let n = (j - i) / 2;
            if n >= 12 && n.is_multiple_of(3) && best.is_none_or(|(_, bn)| n > bn) {
                best = Some((i, n));
            }
            i = j.max(i + 2);
        }
    }
    let (ib, ic) = best?;
    let pos_at = ib + ic * 2;
    if pos_at + rec.vc * 12 > search_end {
        return None;
    }
    let mut positions = Vec::with_capacity(rec.vc);
    for k in 0..rec.vc {
        positions.push([
            f32_at(d, pos_at + k * 12)?,
            f32_at(d, pos_at + k * 12 + 4)?,
            f32_at(d, pos_at + k * 12 + 8)?,
        ]);
    }
    let good = positions
        .iter()
        .filter(|p| in_bbox(&rec.bbox, **p, 3.0))
        .count();
    if good * 10 < rec.vc * 9 {
        return None;
    }
    let mut indices = Vec::with_capacity(ic);
    for k in 0..ic {
        indices.push(u16_at(d, ib + k * 2)?);
    }
    // Per-vertex attributes follow the positions as one interleaved block
    // (`stride` includes the 12 position bytes); scan it for the UV pair.
    let uvs = if rec.stride > 12 && rec.stride <= 256 {
        crate::map::roommesh::scan_attr_uvs(
            d,
            pos_at + rec.vc * 12,
            rec.vc,
            rec.stride - 12,
            &indices,
        )
        .map(|(_, uvs)| uvs)
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    Some(PropMesh {
        name: rec.name.clone(),
        positions,
        indices,
        uvs,
        color: colors
            .get(&rec.name)
            .copied()
            .unwrap_or_else(|| color_from_name(&rec.name)),
    })
}

/// A `Material` BOD's diffuse binding: `Parameters.DiffuseMap1` value plus
/// the engine package id the parameter references.
#[derive(Debug, Clone)]
pub struct MaterialTex {
    pub diffuse: String,
    pub package_id: u16,
}

/// Parse `Material` BODs into `material name -> diffuse binding`.
pub fn material_defs(dec: &[u8]) -> HashMap<String, MaterialTex> {
    let mut out = HashMap::new();
    let mut o = 0usize;
    while o + 4 <= dec.len() {
        let Some(rel) = dec[o..].windows(4).position(|w| w == b"BOD\xFD") else {
            break;
        };
        o += rel;
        if o + 8 > dec.len() {
            break;
        }
        if let Ok(p) = payload::parse_payload(&dec[o..]) {
            if p.class_ref.as_deref() == Some("Material") {
                let Some(name) = p.get("Name").and_then(prop_str) else {
                    o += 4;
                    continue;
                };
                let param = p
                    .get("Parameters")
                    .and_then(|q| child(&q.value, "DiffuseMap1"));
                let diffuse = param
                    .and_then(|v| child(v, "Value"))
                    .and_then(value_str)
                    .unwrap_or_default();
                let package_id = param
                    .and_then(|v| child(v, "PackageID"))
                    .and_then(|v| match v {
                        Value::Int(id) => Some(*id as u16),
                        _ => None,
                    })
                    .unwrap_or(0);
                out.insert(
                    name,
                    MaterialTex {
                        diffuse,
                        package_id,
                    },
                );
            }
        }
        o += 4;
    }
    out
}

/// Parse `Material` BODs into `material name -> diffuse-derived color`.
pub fn material_colors(dec: &[u8]) -> HashMap<String, [u8; 3]> {
    material_defs(dec)
        .into_iter()
        .map(|(name, def)| {
            let c = if def.diffuse.is_empty() {
                color_from_name(&name)
            } else {
                tex_color(&def.diffuse)
            };
            (name, c)
        })
        .collect()
}

fn prop_str(v: &payload::Property) -> Option<String> {
    value_str(&v.value)
}

fn value_str(v: &Value) -> Option<String> {
    match v {
        Value::Ref(s) | Value::Str(s) => Some(s.clone()),
        _ => None,
    }
}

/// Child value by name, from an object or a map.
fn child<'a>(v: &'a Value, name: &str) -> Option<&'a Value> {
    match v {
        Value::Object(o) => o
            .properties
            .iter()
            .find(|p| p.name == name)
            .map(|p| &p.value),
        Value::Map(pairs) => pairs.iter().find_map(|(k, val)| {
            let key = match k {
                Value::Ref(s) | Value::Str(s) => s.as_str(),
                _ => return None,
            };
            (key == name).then_some(val)
        }),
        _ => None,
    }
}

/// Representative color from a diffuse texture name.
fn tex_color(tex: &str) -> [u8; 3] {
    let t = tex.to_ascii_lowercase();
    if t.contains("stump") || t.contains("bark") || t.contains("tree") {
        [104, 72, 48]
    } else if t.contains("moss") || t.contains("grass") || t.contains("flora") {
        [86, 112, 58]
    } else if t.contains("snow") || t.contains("ice") {
        [226, 232, 238]
    } else if t.contains("bone") {
        [214, 206, 182]
    } else if t.contains("rock")
        || t.contains("cliff")
        || t.contains("stone")
        || t.contains("crow")
        || t.contains("wall")
        || t.contains("spire")
    {
        [122, 118, 112]
    } else if t.contains("sand") || t.contains("dirt") || t.contains("ground") {
        [150, 126, 92]
    } else {
        color_from_name(tex)
    }
}

fn color_from_name(name: &str) -> [u8; 3] {
    let mut h = 0u32;
    for b in name.bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as u32);
    }
    [
        90 + (h % 80) as u8,
        85 + ((h >> 8) % 80) as u8,
        70 + ((h >> 16) % 70) as u8,
    ]
}

/// Euler XYZ degrees + translation for one placed instance.
pub fn transform(positions: &[[f32; 3]], rot_deg: [f32; 3], pos: [f32; 3]) -> Vec<[f32; 3]> {
    let (rx, ry, rz) = (
        rot_deg[0].to_radians(),
        rot_deg[1].to_radians(),
        rot_deg[2].to_radians(),
    );
    let (sx, cx) = rx.sin_cos();
    let (sy, cy) = ry.sin_cos();
    let (sz, cz) = rz.sin_cos();
    positions
        .iter()
        .map(|p| {
            // Rz * Ry * Rx
            let (x1, y1, z1) = (p[0], cx * p[1] - sx * p[2], sx * p[1] + cx * p[2]);
            let (x2, y2, z2) = (cy * x1 + sy * z1, y1, -sy * x1 + cy * z1);
            [
                cz * x2 - sz * y2 + pos[0],
                sz * x2 + cz * y2 + pos[1],
                z2 + pos[2],
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_roundtrip_identity() {
        let p = [[1.0f32, 2.0, 3.0]];
        let t = transform(&p, [0.0, 0.0, 0.0], [10.0, -5.0, 2.0]);
        assert!((t[0][0] - 11.0).abs() < 1e-5);
        assert!((t[0][1] + 3.0).abs() < 1e-5);
        assert!((t[0][2] - 5.0).abs() < 1e-5);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_props(&[0u8; 256]).is_empty());
    }
}
