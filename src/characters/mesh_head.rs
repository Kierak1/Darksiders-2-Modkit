//! Remap the CJ head meshes into stock vertex slots.
//!
//! The head meshes (G6 m4 mask main, G1 m2 maskless) carry the same per-index
//! skin binding as the body (the 20-byte weight block, plus the influence
//! count in the last dword of the 52-byte vertex). The earlier head writers
//! replaced the topology in place and left those per-index bindings stock,
//! which flings a few vertices when posed.
//!
//! This remaps each written CJ vertex to a unique stock slot by nearest
//! position and rewrites the index buffer through that mapping, so every
//! slot keeps its stock binding/count. Positions, normals, tangents and
//! UVs of the used slots come from the CJ data already in `dec`.

use crate::characters::mesh_body::{normalize, parse_obj};
use crate::Result;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

struct Target {
    name: &'static str,
    ib: usize,
    a: usize,
    b: usize,
    vc: usize,
    ic: usize,
}

const TARGETS: [Target; 2] = [
    Target {
        name: "G6 m4 mask main",
        ib: 0x4342F9,
        a: 0x435425,
        b: 0x4375E5,
        vc: 432,
        ic: 2196,
    },
    Target {
        name: "G1 m2 maskless",
        ib: 0x178A30,
        a: 0x17A312,
        b: 0x17E5CA,
        vc: 854,
        ic: 3183,
    },
];

/// Decimated mesh output: positions, normals, UVs, triangles.
type Decimated = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<[u32; 3]>);

fn read_at(f: &mut std::fs::File, off: usize, len: usize) -> Result<Vec<u8>> {
    f.seek(SeekFrom::Start(off as u64))?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}

/// Collapse the stock `death_head` sub-meshes that must not show under the CJ
/// mask: the long head+hair shell (G6 m3) and both eye meshes (G1 m1, G6 m1 —
/// they carry Death's glowing eye material). The IBs and per-index bindings
/// stay; every B position is moved to the mesh centroid, so the triangles
/// become degenerate and render nothing.
///
/// Returns one report line per collapsed mesh for the caller to print.
pub fn hide_stock_head(dec: &Path) -> Result<Vec<String>> {
    const TARGETS: [(&str, usize, usize); 3] = [
        ("G1 m1 eyes", 0x178252, 28),
        ("G6 m1 eyes", 0x41BBF9, 28),
        ("G6 m3 head+hair", 0x4252C5, 854),
    ];
    let mut f = OpenOptions::new().read(true).write(true).open(dec)?;
    let mut report = Vec::new();
    for (name, b, vc) in TARGETS {
        let mut buf = read_at(&mut f, b, vc * 52)?;
        let mut c = [0.0f64; 3];
        for i in 0..vc {
            for (k, ck) in c.iter_mut().enumerate() {
                *ck += f32_at(&buf, i * 52 + k * 4) as f64;
            }
        }
        let c = c.map(|v| (v / vc as f64) as f32);
        for i in 0..vc {
            for (k, ck) in c.iter().enumerate() {
                buf[i * 52 + k * 4..i * 52 + k * 4 + 4].copy_from_slice(&ck.to_le_bytes());
            }
        }
        f.seek(SeekFrom::Start(b as u64))?;
        f.write_all(&buf)?;
        report.push(format!(
            "  {name}: {vc} verts collapsed to ({:.2}, {:.2}, {:.2})",
            c[0], c[1], c[2]
        ));
    }
    Ok(report)
}

/// Scale the two CJ head meshes (G6 m4 mask, G1 m2 maskless) about their
/// neck (bbox bottom centre) so the head matches the CJ body's scale instead
/// of Death's oversized head bbox. The binding is per index and the head is
/// rigid, so scaling in place is safe; the B record layout is
/// `{pos, 1.0, normal, normal.z, tangent, 1.0, u32 influenceCount}`.
///
/// Returns one report line per scaled mesh for the caller to print.
pub fn scale_heads(dec: &Path, factor: f32, dz: f32) -> Result<Vec<String>> {
    const TARGETS: [(&str, usize, usize); 2] = [
        ("G6 m4 mask", 0x4375E5, 432),
        ("G1 m2 maskless", 0x17E5CA, 854),
    ];
    let mut f = OpenOptions::new().read(true).write(true).open(dec)?;
    let mut report = Vec::new();
    for (name, b, vc) in TARGETS {
        let mut buf = read_at(&mut f, b, vc * 52)?;
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for i in 0..vc {
            for k in 0..3 {
                let v = f32_at(&buf, i * 52 + k * 4);
                lo[k] = lo[k].min(v);
                hi[k] = hi[k].max(v);
            }
        }
        let anchor = [0.0, (lo[1] + hi[1]) * 0.5, lo[2]];
        for i in 0..vc {
            for (k, a) in anchor.iter().enumerate() {
                let at = i * 52 + k * 4;
                let v = f32_at(&buf, at);
                let mut s = a + (v - a) * factor;
                if k == 2 {
                    s += dz;
                }
                buf[at..at + 4].copy_from_slice(&s.to_le_bytes());
            }
        }
        f.seek(SeekFrom::Start(b as u64))?;
        f.write_all(&buf)?;
        report.push(format!(
            "  {name}: {vc} verts scaled x{factor} about ({:.2}, {:.2}, {:.2}); z {:.2}..{:.2}",
            anchor[0],
            anchor[1],
            anchor[2],
            anchor[2] + (lo[2] - anchor[2]) * factor,
            anchor[2] + (hi[2] - anchor[2]) * factor
        ));
    }
    Ok(report)
}

/// Rewrite the visible mask (G6 m4) with CJ's full head, decimated in a
/// seam-aware way: corners only merge inside the same 3D cell *and* the same
/// UV island (quantized UV cell), so the face texture is not smeared across
/// seams like the old spatial-grid decimation did.
///
/// Returns the decimation-trial and result lines for the caller to print.
pub fn write_mask(obj_path: &Path, dec: &Path) -> Result<Vec<String>> {
    const M4_IB: usize = 0x4342F9;
    const M4_A: usize = 0x435425;
    const M4_B: usize = 0x4375E5;
    const M4_VC: usize = 432;
    const M4_IC: usize = 2196;

    let obj = parse_obj(obj_path, "Object001")?;
    // body vertices bound to head_* joints
    let mut is_head: HashMap<i32, bool> = HashMap::new();
    for (i, v) in obj.verts.iter().enumerate() {
        let mut best = 0usize;
        let mut bd = f32::MAX;
        for (j, (_, c)) in obj.joints.iter().enumerate() {
            let d = (v[0] - c[0]).powi(2) + (v[1] - c[1]).powi(2) + (v[2] - c[2]).powi(2);
            if d < bd {
                bd = d;
                best = j;
            }
        }
        is_head.insert(i as i32, obj.joints[best].0.starts_with("head"));
    }
    let head_faces: Vec<[u32; 3]> = obj
        .faces
        .iter()
        .filter(|f| {
            f.iter()
                .all(|&c| *is_head.get(&obj.corners[c as usize].0).unwrap_or(&false))
        })
        .copied()
        .collect();
    if head_faces.is_empty() {
        return Err("no head faces found in the CJ OBJ".into());
    }

    // collapse corners into clusters (3D cell + UV cell)
    let decimate = |grid: f32, uq: f32| -> Decimated {
        let mut map: HashMap<(i32, i32, i32, i32, i32), u32> = HashMap::new();
        let mut pos: Vec<[f64; 3]> = Vec::new();
        let mut nrm: Vec<[f64; 3]> = Vec::new();
        let mut uv: Vec<[f64; 2]> = Vec::new();
        let mut cnt: Vec<u32> = Vec::new();
        let mut cmap: HashMap<u32, u32> = HashMap::new();
        for f in &head_faces {
            for &c in f {
                if cmap.contains_key(&c) {
                    continue;
                }
                let (vi, _, _, _) = obj.corners[c as usize];
                let p = obj.verts[vi as usize];
                let u = obj.corner_uv[c as usize];
                let key = (
                    (p[0] / grid).floor() as i32,
                    (p[1] / grid).floor() as i32,
                    (p[2] / grid).floor() as i32,
                    (u[0] * uq).floor() as i32,
                    (u[1] * uq).floor() as i32,
                );
                let n = obj.corner_nrm[c as usize];
                let id = *map.entry(key).or_insert_with(|| {
                    pos.push([0.0; 3]);
                    nrm.push([0.0; 3]);
                    uv.push([0.0; 2]);
                    cnt.push(0);
                    pos.len() as u32 - 1
                });
                for k in 0..3 {
                    pos[id as usize][k] += p[k] as f64;
                    nrm[id as usize][k] += n[k] as f64;
                }
                uv[id as usize][0] += u[0] as f64;
                uv[id as usize][1] += u[1] as f64;
                cnt[id as usize] += 1;
                cmap.insert(c, id);
            }
        }
        let n = pos.len();
        let mut out_p = Vec::with_capacity(n);
        let mut out_n = Vec::with_capacity(n);
        let mut out_uv = Vec::with_capacity(n);
        for i in 0..n {
            let c = cnt[i] as f64;
            out_p.push([
                (pos[i][0] / c) as f32,
                (pos[i][1] / c) as f32,
                (pos[i][2] / c) as f32,
            ]);
            out_n.push(normalize([
                (nrm[i][0] / c) as f32,
                (nrm[i][1] / c) as f32,
                (nrm[i][2] / c) as f32,
            ]));
            out_uv.push([(uv[i][0] / c) as f32, (uv[i][1] / c) as f32]);
        }
        let mut tris: Vec<[u32; 3]> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for f in &head_faces {
            let t = [cmap[&f[0]], cmap[&f[1]], cmap[&f[2]]];
            if t[0] == t[1] || t[1] == t[2] || t[0] == t[2] {
                continue;
            }
            let mut key = t;
            key.sort_unstable();
            if !seen.insert(key) {
                continue;
            }
            tris.push(t);
        }
        (out_p, out_n, out_uv, tris)
    };

    let mut chosen = None;
    let mut report = Vec::new();
    for (grid, uq) in [
        (0.06f32, 64.0f32),
        (0.08, 48.0),
        (0.1, 32.0),
        (0.12, 32.0),
        (0.12, 24.0),
        (0.15, 24.0),
        (0.15, 16.0),
        (0.2, 16.0),
        (0.25, 16.0),
        (0.3, 16.0),
        (0.25, 12.0),
        (0.3, 12.0),
        (0.35, 12.0),
        (0.4, 12.0),
        (0.45, 10.0),
        (0.5, 10.0),
        (0.4, 8.0),
        (0.5, 8.0),
    ] {
        let (p, n, u, t) = decimate(grid, uq);
        report.push(format!(
            "  grid {grid} uq {uq}: {} verts, {} tris",
            p.len(),
            t.len()
        ));
        if p.len() <= M4_VC && t.len() * 3 <= M4_IC {
            chosen = Some((p, n, u, t));
            break;
        }
    }
    let (dpos, dnrm, duv, dtris) =
        chosen.ok_or("cannot decimate the CJ head into the mask budget")?;

    // place the decimated head where the current (already sized) head sits:
    // bbox-centre affine derived from the OBJ head bbox and the live m4 bbox
    let mut obj_lo = [f32::MAX; 3];
    let mut obj_hi = [f32::MIN; 3];
    for (i, v) in obj.verts.iter().enumerate() {
        if *is_head.get(&(i as i32)).unwrap_or(&false) {
            for k in 0..3 {
                obj_lo[k] = obj_lo[k].min(v[k]);
                obj_hi[k] = obj_hi[k].max(v[k]);
            }
        }
    }
    let mut f = OpenOptions::new().read(true).write(true).open(dec)?;
    let cur_b = read_at(&mut f, M4_B, M4_VC * 52)?;
    let mut cur_lo = [f32::MAX; 3];
    let mut cur_hi = [f32::MIN; 3];
    for i in 0..M4_VC {
        for k in 0..3 {
            let v = f32_at(&cur_b, i * 52 + k * 4);
            cur_lo[k] = cur_lo[k].min(v);
            cur_hi[k] = cur_hi[k].max(v);
        }
    }
    let s = (cur_hi[2] - cur_lo[2]) / (obj_hi[1] - obj_lo[1]);
    let oc = [0, 1, 2].map(|k| (obj_lo[k] + obj_hi[k]) * 0.5);
    let tc = [0, 1, 2].map(|k| (cur_lo[k] + cur_hi[k]) * 0.5);
    let xf = |p: [f32; 3]| -> [f32; 3] {
        [
            -(p[0] - oc[0]) * s + tc[0],
            (p[2] - oc[2]) * s + tc[1],
            (p[1] - oc[1]) * s + tc[2],
        ]
    };
    let world: Vec<[f32; 3]> = dpos.iter().map(|p| xf(*p)).collect();

    // unique slot assignment by nearest live m4 position
    let mut cands: Vec<(f32, usize, usize)> = Vec::new();
    for (i, p) in world.iter().enumerate() {
        let mut best: Vec<(f32, usize)> = Vec::new();
        for s in 0..M4_VC {
            let o = s * 52;
            let q = [
                f32_at(&cur_b, o),
                f32_at(&cur_b, o + 4),
                f32_at(&cur_b, o + 8),
            ];
            let d = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2);
            if best.len() < 32 {
                best.push((d, s));
                best.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            } else if d < best[31].0 {
                best[31] = (d, s);
                best.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            }
        }
        for (d, s) in best {
            cands.push((d, i, s));
        }
    }
    cands.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut slot_of = vec![usize::MAX; world.len()];
    let mut used = vec![false; M4_VC];
    let mut assigned = 0usize;
    for (_, i, s) in &cands {
        if slot_of[*i] == usize::MAX && !used[*s] {
            slot_of[*i] = *s;
            used[*s] = true;
            assigned += 1;
        }
    }
    // any vertex without a slot takes the nearest still-unused slot
    for (i, p) in world.iter().enumerate() {
        if slot_of[i] != usize::MAX {
            continue;
        }
        let mut best_s = usize::MAX;
        let mut bd = f32::MAX;
        for (s, taken) in used.iter().enumerate() {
            if *taken {
                continue;
            }
            let o = s * 52;
            let q = [
                f32_at(&cur_b, o),
                f32_at(&cur_b, o + 4),
                f32_at(&cur_b, o + 8),
            ];
            let d = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2);
            if d < bd {
                bd = d;
                best_s = s;
            }
        }
        if best_s != usize::MAX {
            slot_of[i] = best_s;
            used[best_s] = true;
            assigned += 1;
        }
    }
    for v in slot_of.iter_mut() {
        if *v == usize::MAX {
            *v = 0;
        }
    }

    let mut b_out = cur_b.clone();
    let mut a_out = read_at(&mut f, M4_A, M4_VC * 20)?;
    let mut dist_mean = 0.0f64;
    for (i, p) in world.iter().enumerate() {
        let s = slot_of[i];
        let o = s * 52;
        let q = [
            f32_at(&cur_b, o),
            f32_at(&cur_b, o + 4),
            f32_at(&cur_b, o + 8),
        ];
        dist_mean +=
            (((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)) as f64).sqrt();
        for k in 0..3 {
            b_out[o + k * 4..o + k * 4 + 4].copy_from_slice(&p[k].to_le_bytes());
        }
        let n0 = dnrm[i];
        let n = normalize([-n0[0], n0[2], n0[1]]);
        for k in 0..3 {
            b_out[o + 16 + k * 4..o + 16 + k * 4 + 4].copy_from_slice(&n[k].to_le_bytes());
        }
        b_out[o + 28..o + 32].copy_from_slice(&n[2].to_le_bytes());
        let (mut tx, mut ty) = (n[1], -n[0]);
        let lt = (tx * tx + ty * ty).sqrt();
        if lt > 0.0 {
            tx /= lt;
            ty /= lt;
        }
        b_out[o + 32..o + 36].copy_from_slice(&tx.to_le_bytes());
        b_out[o + 36..o + 40].copy_from_slice(&ty.to_le_bytes());
        b_out[o + 40..o + 44].copy_from_slice(&0.0f32.to_le_bytes());
        let a = s * 20;
        a_out[a..a + 4].copy_from_slice(&(-duv[i][1]).to_le_bytes());
        a_out[a + 16..a + 20].copy_from_slice(&duv[i][0].to_le_bytes());
    }

    let mut ib = vec![0u8; M4_IC * 2];
    for (ti, tri) in dtris.iter().enumerate() {
        for (k, &c) in [tri[0], tri[2], tri[1]].iter().enumerate() {
            let v = slot_of[c as usize] as u16;
            ib[(ti * 3 + k) * 2..(ti * 3 + k) * 2 + 2].copy_from_slice(&v.to_le_bytes());
        }
    }

    f.seek(SeekFrom::Start(M4_A as u64))?;
    f.write_all(&a_out)?;
    f.seek(SeekFrom::Start(M4_IB as u64))?;
    f.write_all(&ib)?;
    f.seek(SeekFrom::Start(M4_B as u64))?;
    f.write_all(&b_out)?;
    report.push(format!(
        "  G6 m4 mask: {} verts -> unique slots ({} assigned), {} tris, match mean {:.2}",
        world.len(),
        assigned,
        dtris.len(),
        dist_mean / world.len() as f64
    ));
    Ok(report)
}

/// Remap both head meshes in `dec` (patched in place) against `stock`.
///
/// Returns one report line per remapped mesh for the caller to print.
pub fn remap_heads(dec: &Path, stock: &Path) -> Result<Vec<String>> {
    let mut dec_f = OpenOptions::new().read(true).write(true).open(dec)?;
    let mut stock_f = std::fs::File::open(stock)?;
    let mut report = Vec::new();

    for t in &TARGETS {
        let ib_data = read_at(&mut dec_f, t.ib, t.ic * 2)?;
        let mut used = 0usize;
        let mut tris: Vec<[u16; 3]> = Vec::with_capacity(t.ic / 3);
        for c in ib_data.as_chunks::<6>().0 {
            let tri = [u16_at(c, 0), u16_at(c, 2), u16_at(c, 4)];
            for &i in &tri {
                used = used.max(i as usize + 1);
            }
            tris.push(tri);
        }
        let cj_a = read_at(&mut dec_f, t.a, used * 20)?;
        let cj_b = read_at(&mut dec_f, t.b, used * 52)?;
        let stock_a = read_at(&mut stock_f, t.a, t.vc * 20)?;
        let stock_b = read_at(&mut stock_f, t.b, t.vc * 52)?;
        let stock_pos: Vec<[f32; 3]> = (0..t.vc)
            .map(|i| {
                [
                    f32_at(&stock_b, i * 52),
                    f32_at(&stock_b, i * 52 + 4),
                    f32_at(&stock_b, i * 52 + 8),
                ]
            })
            .collect();
        let cj_pos: Vec<[f32; 3]> = (0..used)
            .map(|i| {
                [
                    f32_at(&cj_b, i * 52),
                    f32_at(&cj_b, i * 52 + 4),
                    f32_at(&cj_b, i * 52 + 8),
                ]
            })
            .collect();

        // greedy unique assignment, nearest position first
        let mut cands: Vec<(f32, usize, usize)> = Vec::new();
        for (i, p) in cj_pos.iter().enumerate() {
            let mut best: Vec<(f32, usize)> = Vec::new();
            for (s, q) in stock_pos.iter().enumerate() {
                let d = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2);
                if best.len() < 64 {
                    best.push((d, s));
                    best.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                } else if d < best[63].0 {
                    best[63] = (d, s);
                    best.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                }
            }
            for (d, s) in best {
                cands.push((d, i, s));
            }
        }
        cands.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut slot_of = vec![usize::MAX; used];
        let mut taken = vec![false; t.vc];
        let mut assigned = 0usize;
        for (_, cj, s) in &cands {
            if slot_of[*cj] == usize::MAX && !taken[*s] {
                slot_of[*cj] = *s;
                taken[*s] = true;
                assigned += 1;
            }
        }
        for (i, p) in cj_pos.iter().enumerate() {
            if slot_of[i] != usize::MAX {
                continue;
            }
            let mut best_s = usize::MAX;
            let mut bd = f32::MAX;
            for s in 0..t.vc {
                if taken[s] {
                    continue;
                }
                let q = stock_pos[s];
                let d = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2);
                if d < bd {
                    bd = d;
                    best_s = s;
                }
            }
            if best_s != usize::MAX {
                slot_of[i] = best_s;
                taken[best_s] = true;
                assigned += 1;
            }
        }
        let mut dist_mean = 0.0f64;
        let mut dist_max = 0.0f32;
        for (i, p) in cj_pos.iter().enumerate() {
            let s = slot_of[i];
            if taken[s] && s < t.vc {
                let q = stock_pos[s];
                let d =
                    ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
                dist_mean += d as f64;
                dist_max = dist_max.max(d);
            }
        }

        // rewrite the IB through the mapping
        let mut ib_out = ib_data.clone();
        for (ti, tri) in tris.iter().enumerate() {
            for k in 0..3 {
                let cj = tri[k] as usize;
                let s = if cj < slot_of.len() && slot_of[cj] != usize::MAX {
                    slot_of[cj] as u16
                } else {
                    0
                };
                ib_out[(ti * 3 + k) * 2..(ti * 3 + k) * 2 + 2].copy_from_slice(&s.to_le_bytes());
            }
        }

        // A: stock record per slot, CJ UVs patched in (-v at +0, u at +16)
        let mut a_out = stock_a.clone();
        // B: stock record per slot, CJ position/normal/tangent patched in
        let mut b_out = stock_b.clone();
        for (i, s) in slot_of.iter().enumerate() {
            if *s >= t.vc || !taken[*s] {
                continue;
            }
            let s = *s;
            a_out[s * 20..s * 20 + 4].copy_from_slice(&cj_a[i * 20..i * 20 + 4]);
            a_out[s * 20 + 16..s * 20 + 20].copy_from_slice(&cj_a[i * 20 + 16..i * 20 + 20]);
            let dst = &mut b_out[s * 52..s * 52 + 52];
            dst[0..12].copy_from_slice(&cj_b[i * 52..i * 52 + 12]);
            dst[16..32].copy_from_slice(&cj_b[i * 52 + 16..i * 52 + 32]);
            dst[32..44].copy_from_slice(&cj_b[i * 52 + 32..i * 52 + 44]);
        }

        dec_f.seek(SeekFrom::Start(t.ib as u64))?;
        dec_f.write_all(&ib_out)?;
        dec_f.seek(SeekFrom::Start(t.a as u64))?;
        dec_f.write_all(&a_out)?;
        dec_f.seek(SeekFrom::Start(t.b as u64))?;
        dec_f.write_all(&b_out)?;
        report.push(format!(
            "  {}: {} CJ verts -> {} unique stock slots ({} assigned), match mean {:.2} max {:.2}",
            t.name,
            used,
            t.vc,
            assigned,
            dist_mean / used as f64,
            dist_max
        ));
    }
    Ok(report)
}
