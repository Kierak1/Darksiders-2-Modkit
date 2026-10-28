//! CJ body writer for the armor base-body meshes.
//!
//! The engine binds skin weights per *vertex index* (a stock weight/bone pair
//! stored per index), so CJ's own vertex order can not be used directly: any
//! vertex must land in a stock index slot whose binding makes sense for its
//! position.
//!
//! This writer keeps CJ's real topology but remaps every *face corner*
//! (position + UV + normal) to a unique stock index slot chosen by nearest
//! position, so UV seams survive (one UV per index cannot represent the
//! tank-top/skin boundary otherwise). Each slot keeps its stock `id`, weight
//! block and influence count. CJ's T-posed arms are rotated down about the
//! shoulder joints first so they roughly match the stock A-pose that the bind
//! skeleton expects.
//!
//! Vertical fit: above the knee the mesh matches the stock bare body (which
//! stops at the ankle because Death's boots supplied the ground); the shin is
//! stretched and the foot keeps its natural size anchored at z 0, so CJ's
//! shoes land on the floor.

use crate::Result;
use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const OBJ_GROUP: &str = "Object001";

struct Target {
    name: &'static str,
    data: usize,
    b: usize,
}

const TARGETS: [Target; 2] = [
    Target {
        name: "base_bodya",
        data: 0x189500B,
        b: 0x18B0AF7,
    },
    Target {
        name: "base_bodyb",
        data: 0x1922CF4,
        b: 0x193E0A4,
    },
];

pub(crate) struct Obj {
    pub verts: Vec<[f32; 3]>,
    /// triangles as indices into `corners`
    pub faces: Vec<[u32; 3]>,
    /// (vertex, vt, vn, atlas cell)
    pub corners: Vec<(i32, i32, i32, u8)>,
    pub corner_uv: Vec<[f32; 2]>,
    pub corner_nrm: Vec<[f32; 3]>,
    pub joints: Vec<(String, [f32; 3])>,
}

/// UV atlas cells for the CJ materials: 2x2 grid, (col, row) in image space.
fn material_cell(name: &str) -> u8 {
    match name {
        "head" => 0,
        "body_upper" => 1,
        "legs" => 2,
        "shoes" => 3,
        _ => 1,
    }
}

pub(crate) fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 0.0 {
        [v[0] / l, v[1] / l, v[2] / l]
    } else {
        [0.0, 0.0, 1.0]
    }
}

pub(crate) fn parse_obj(path: &Path, group: &str) -> Result<Obj> {
    let text = std::fs::read_to_string(path)?;
    let mut verts = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut normals = Vec::new();
    let mut faces: Vec<[u32; 3]> = Vec::new();
    let mut corners: Vec<(i32, i32, i32, u8)> = Vec::new();
    let mut corner_map: HashMap<(i32, i32, i32), u32> = HashMap::new();
    let mut vnormals: HashMap<i32, [f32; 3]> = HashMap::new();
    let mut cur = String::new();
    let mut material = String::new();
    let mut sets: HashMap<String, HashSet<i32>> = HashMap::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("v ") {
            let v: Vec<f32> = rest
                .split_whitespace()
                .filter_map(|t| t.parse().ok())
                .collect();
            if v.len() >= 3 {
                verts.push([v[0], v[1], v[2]]);
            }
        } else if let Some(rest) = line.strip_prefix("vt ") {
            let v: Vec<f32> = rest
                .split_whitespace()
                .filter_map(|t| t.parse().ok())
                .collect();
            if v.len() >= 2 {
                uvs.push([v[0], v[1]]);
            }
        } else if let Some(rest) = line.strip_prefix("vn ") {
            let v: Vec<f32> = rest
                .split_whitespace()
                .filter_map(|t| t.parse().ok())
                .collect();
            if v.len() >= 3 {
                normals.push([v[0], v[1], v[2]]);
            }
        } else if let Some(rest) = line.strip_prefix("usemtl ") {
            material = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("g ") {
            cur = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("f ") {
            let set = sets.entry(cur.clone()).or_default();
            if cur != group {
                for t in rest.split_whitespace() {
                    if let Some(vi) = t.split('/').next().and_then(|s| s.parse::<i32>().ok()) {
                        set.insert(vi - 1);
                    }
                }
                continue;
            }
            let poly: Vec<(i32, i32, i32)> = rest
                .split_whitespace()
                .map(|t| {
                    let p: Vec<&str> = t.split('/').collect();
                    let vi = p[0].parse::<i32>().unwrap_or(0) - 1;
                    let vti = p
                        .get(1)
                        .and_then(|s| {
                            if s.is_empty() {
                                None
                            } else {
                                s.parse::<i32>().ok()
                            }
                        })
                        .unwrap_or(-1)
                        - 1;
                    let vni = p
                        .get(2)
                        .and_then(|s| {
                            if s.is_empty() {
                                None
                            } else {
                                s.parse::<i32>().ok()
                            }
                        })
                        .unwrap_or(-1)
                        - 1;
                    (vi, vti, vni)
                })
                .collect();
            if poly.len() == 3 || poly.len() == 4 {
                let cell = material_cell(&material);
                let mut cid = [0u32; 4];
                for (k, &(vi, vti, vni)) in poly.iter().enumerate() {
                    set.insert(vi);
                    if vni >= 0 && vni < normals.len() as i32 {
                        let e = vnormals.entry(vi).or_insert([0.0; 3]);
                        for j in 0..3 {
                            e[j] += normals[vni as usize][j];
                        }
                    }
                    cid[k] = *corner_map.entry((vi, vti, vni)).or_insert_with(|| {
                        corners.push((vi, vti, vni, cell));
                        corners.len() as u32 - 1
                    });
                }
                faces.push([cid[0], cid[1], cid[2]]);
                if poly.len() == 4 {
                    faces.push([cid[0], cid[2], cid[3]]);
                }
            }
        }
    }
    let mut vnormals_full: Vec<[f32; 3]> = vec![[0.0, 0.0, 1.0]; verts.len()];
    for (&vi, n) in &vnormals {
        vnormals_full[vi as usize] = normalize(*n);
    }
    let mut joints = Vec::new();
    for (name, set) in &sets {
        if set.len() == 9 && name != group {
            let mut c = [0.0f32; 3];
            for &vi in set {
                for k in 0..3 {
                    c[k] += verts[vi as usize][k];
                }
            }
            for ck in &mut c {
                *ck /= 9.0;
            }
            joints.push((name.clone(), c));
        }
    }
    let corner_uv: Vec<[f32; 2]> = corners
        .iter()
        .map(|&(_, vti, _, _)| {
            if vti >= 0 && (vti as usize) < uvs.len() {
                uvs[vti as usize]
            } else {
                [0.0, 0.0]
            }
        })
        .collect();
    let corner_nrm: Vec<[f32; 3]> = corners
        .iter()
        .map(|&(vi, _, vni, _)| {
            if vni >= 0 && (vni as usize) < normals.len() {
                normalize(normals[vni as usize])
            } else {
                vnormals_full[vi as usize]
            }
        })
        .collect();
    Ok(Obj {
        verts,
        faces,
        corners,
        corner_uv,
        corner_nrm,
        joints,
    })
}

fn read_at(f: &mut std::fs::File, off: usize, len: usize) -> Result<Vec<u8>> {
    f.seek(SeekFrom::Start(off as u64))?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

struct ArmPose {
    left: HashSet<i32>,
    right: HashSet<i32>,
    sl: [f32; 3],
    sr: [f32; 3],
    al: [f32; 3],
    ar: [f32; 3],
    len_l: f32,
    len_r: f32,
    angle: f32,
}

/// Skin-like falloff along the shoulder->elbow axis: only the shoulder ball
/// blends (short ramp), the rest of the arm rotates rigidly so it stays
/// straight. A wide blend bent the whole arm into a banana.
fn arm_weight(p: [f32; 3], shoulder: [f32; 3], axis: [f32; 3], _len: f32) -> f32 {
    const R: f32 = 2.5;
    let d = (p[0] - shoulder[0]) * axis[0]
        + (p[1] - shoulder[1]) * axis[1]
        + (p[2] - shoulder[2]) * axis[2];
    (d / R).clamp(0.0, 1.0)
}

fn rotate_arm_point(p: &mut [f32; 3], vi: i32, pose: &ArmPose) {
    if pose.left.contains(&vi) {
        let (a, b) = (pose.angle * arm_weight(*p, pose.sl, pose.al, pose.len_l)).sin_cos();
        let dx = p[0] - pose.sl[0];
        let dy = p[1] - pose.sl[1];
        p[0] = pose.sl[0] + dx * b + dy * a;
        p[1] = pose.sl[1] - dx * a + dy * b;
    } else if pose.right.contains(&vi) {
        let (a, b) = (pose.angle * arm_weight(*p, pose.sr, pose.ar, pose.len_r)).sin_cos();
        let dx = p[0] - pose.sr[0];
        let dy = p[1] - pose.sr[1];
        p[0] = pose.sr[0] + dx * b - dy * a;
        p[1] = pose.sr[1] + dx * a + dy * b;
    }
}

fn rotate_arm_normal(n: &mut [f32; 3], p: [f32; 3], vi: i32, pose: &ArmPose) {
    if pose.left.contains(&vi) {
        let (a, b) = (pose.angle * arm_weight(p, pose.sl, pose.al, pose.len_l)).sin_cos();
        let (nx, ny) = (n[0], n[1]);
        n[0] = nx * b + ny * a;
        n[1] = -nx * a + ny * b;
    } else if pose.right.contains(&vi) {
        let (a, b) = (pose.angle * arm_weight(p, pose.sr, pose.ar, pose.len_r)).sin_cos();
        let (nx, ny) = (n[0], n[1]);
        n[0] = nx * b - ny * a;
        n[1] = nx * a + ny * b;
    }
}

/// Transfer CJ's mesh into both base-body meshes: CJ topology, CJ positions
/// remapped into stock index slots (stock binding kept per slot). `dec` is
/// patched in place; `stock` is the untouched payload.
///
/// Returns diagnostic report lines for the caller to print.
pub fn write_body(
    obj_path: &Path,
    dec: &Path,
    stock: &Path,
    extra_arm_deg: f32,
) -> Result<Vec<String>> {
    let obj = parse_obj(obj_path, OBJ_GROUP)?;
    let mut report = Vec::new();
    report.push(format!(
        "CJ body: {} verts, {} corners, {} tris, {} joint boxes",
        obj.verts.len(),
        obj.corners.len(),
        obj.faces.len(),
        obj.joints.len()
    ));

    // joint assignment for the arm repose
    let mut left_joints: HashSet<i32> = HashSet::new();
    let mut right_joints: HashSet<i32> = HashSet::new();
    let mut head_verts: HashSet<i32> = HashSet::new();
    let mut shoulder_l = None;
    let mut shoulder_r = None;
    let mut elbow_l = None;
    let mut elbow_r = None;
    for (name, c) in &obj.joints {
        match name.as_str() {
            "arm_left_shoulder_2" => shoulder_l = Some(*c),
            "arm_right_shoulder_2" => shoulder_r = Some(*c),
            "arm_left_elbow" => elbow_l = Some(*c),
            "arm_right_elbow" => elbow_r = Some(*c),
            _ => {}
        }
    }
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
        let name = &obj.joints[best].0;
        if name.starts_with("arm_left") {
            left_joints.insert(i as i32);
        } else if name.starts_with("arm_right") {
            right_joints.insert(i as i32);
        }
        if name.starts_with("head") {
            head_verts.insert(i as i32);
        }
    }
    let shoulder_l = shoulder_l.ok_or("missing arm_left_shoulder_2 joint")?;
    let shoulder_r = shoulder_r.ok_or("missing arm_right_shoulder_2 joint")?;
    let elbow_l = elbow_l.ok_or("missing arm_left_elbow joint")?;
    let elbow_r = elbow_r.ok_or("missing arm_right_elbow joint")?;

    // diagnostic: CJ head (body faces bound to head_* joints) y range, used to
    // pick the head-scale factor that matches the body fit
    {
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let mut used_head: HashSet<i32> = HashSet::new();
        for &c in obj.faces.iter().flatten() {
            let vi = obj.corners[c as usize].0;
            if head_verts.contains(&vi) {
                used_head.insert(vi);
            }
        }
        for &vi in &used_head {
            let y = obj.verts[vi as usize][1];
            lo = lo.min(y);
            hi = hi.max(y);
        }
        report.push(format!(
            "CJ head y range {:.2}..{:.2} ({:.2} units; body-fit height {:.2}), {} unique verts",
            lo,
            hi,
            hi - lo,
            (hi - lo) * 0.72385,
            used_head.len()
        ));
    }
    // the mask (characters/death) is the visible head; drop the body's hidden
    // head faces so it cannot poke through the smaller mask
    let body_faces: Vec<[u32; 3]> = obj
        .faces
        .iter()
        .filter(|t| {
            !t.iter()
                .all(|&c| head_verts.contains(&obj.corners[c as usize].0))
        })
        .copied()
        .collect();
    report.push(format!(
        "  body faces {} ({} head faces dropped)",
        body_faces.len(),
        obj.faces.len() - body_faces.len()
    ));

    let mut stock_f = std::fs::File::open(stock)?;
    let mut dec_f = OpenOptions::new().write(true).open(dec)?;

    // stock arm angle: shoulder is the highest arm vertex, the hand the widest
    let ref_b = read_at(&mut stock_f, TARGETS[0].b, 3844 * 52)?;
    let angle: f32;
    {
        let mut shoulder_z = f32::MIN;
        let mut shoulder_x = 0.0;
        let mut hand_x = f32::MIN;
        let mut hand_z = 0.0;
        for i in 0..3844usize {
            let o = i * 52 + 4;
            let (x, z) = (f32_at(&ref_b, o), f32_at(&ref_b, o + 8));
            if x > 8.0 {
                if z > shoulder_z {
                    shoulder_z = z;
                    shoulder_x = x;
                }
                if x > hand_x {
                    hand_x = x;
                    hand_z = z;
                }
            }
        }
        angle = (shoulder_z - hand_z).atan2(hand_x - shoulder_x) + extra_arm_deg.to_radians();
        report.push(format!(
            "stock arm angle {:.1} deg + {:.1} deg extra; reposing CJ arms to {:.1} deg",
            (angle - extra_arm_deg.to_radians()).to_degrees(),
            extra_arm_deg,
            angle.to_degrees()
        ));
    }

    // per-corner pose (positions and normals), arms rotated to the A-pose
    let axis = |a: [f32; 3], b: [f32; 3]| -> ([f32; 3], f32) {
        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
        ([d[0] / l, d[1] / l, d[2] / l], l)
    };
    let (al, len_l) = axis(shoulder_l, elbow_l);
    let (ar, len_r) = axis(shoulder_r, elbow_r);
    let pose = ArmPose {
        left: left_joints,
        right: right_joints,
        sl: shoulder_l,
        sr: shoulder_r,
        al,
        ar,
        len_l,
        len_r,
        angle,
    };
    let mut corner_pos: Vec<[f32; 3]> = obj
        .corners
        .iter()
        .map(|&(vi, _, _, _)| obj.verts[vi as usize])
        .collect();
    let mut corner_nrm = obj.corner_nrm.clone();
    for (k, &(vi, _, _, _)) in obj.corners.iter().enumerate() {
        rotate_arm_point(&mut corner_pos[k], vi, &pose);
        rotate_arm_normal(&mut corner_nrm[k], corner_pos[k], vi, &pose);
    }

    // reposed CJ bbox, used to fit into each target's stock bbox
    let fit_lo = [0, 1, 2].map(|k| corner_pos.iter().map(|p| p[k]).fold(f32::MAX, f32::min));
    let fit_hi = [0, 1, 2].map(|k| corner_pos.iter().map(|p| p[k]).fold(f32::MIN, f32::max));

    for t in &TARGETS {
        let hdr = read_at(&mut stock_f, t.data, 28)?;
        let vc = u32_at(&hdr, 4) as usize;
        let ic = u32_at(&hdr, 12) as usize;
        assert_eq!(u32_at(&hdr, 0), 0);
        assert_eq!(u32_at(&hdr, 8), 0);
        assert_eq!(u32_at(&hdr, 20), (ic * 2) as u32);
        assert_eq!(u32_at(&hdr, 24), 2);
        assert!(
            body_faces.len() * 3 <= ic,
            "too many indices for {}",
            t.name
        );
        assert!(
            obj.corners.len() <= vc,
            "{} corners > {} stock slots for {}",
            obj.corners.len(),
            vc,
            t.name
        );

        // start from the stock object region (data header, IB, A, B, C) so the
        // binding data is byte-exact stock; IB and B get rewritten
        let region_end = t.b + vc * 72;
        let region = read_at(&mut stock_f, t.data, region_end - t.data)?;
        dec_f.seek(SeekFrom::Start(t.data as u64))?;
        dec_f.write_all(&region)?;
        let stock_b = read_at(&mut stock_f, t.b, vc * 52)?;

        let d_lo = [0, 1, 2].map(|k| {
            (0..vc)
                .map(|i| f32_at(&stock_b, i * 52 + 4 + k * 4))
                .fold(f32::MAX, f32::min)
        });
        let d_hi = [0, 1, 2].map(|k| {
            (0..vc)
                .map(|i| f32_at(&stock_b, i * 52 + 4 + k * 4))
                .fold(f32::MIN, f32::max)
        });
        let scale = (d_hi[2] - d_lo[2]) / (fit_hi[1] - fit_lo[1]);
        let cj_c = [0, 1, 2].map(|k| (fit_lo[k] + fit_hi[k]) * 0.5);
        let d_c = [0, 1, 2].map(|k| (d_lo[k] + d_hi[k]) * 0.5);
        // The stock bare body stops at the ankle (its lowest z), the boots
        // provide the ground; CJ's body must reach the ground itself. Fit the
        // upper body to the stock bare body as before, but stretch the shin so
        // the shoe (natural size) lands on the floor:
        //   below ankle  -> unscaled foot anchored at ground
        //   ankle..knee  -> stretched shin
        //   above knee   -> stock bare-body fit
        let joint_y = |name: &str| {
            obj.joints
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, c)| c[1])
        };
        let ankle_y = joint_y("leg_left_ankle")
            .or_else(|| joint_y("leg_right_ankle"))
            .ok_or("missing ankle joint in CJ rig")?;
        let knee_y = joint_y("leg_left_knee")
            .or_else(|| joint_y("leg_right_knee"))
            .ok_or("missing knee joint in CJ rig")?;
        let ground_y = fit_lo[1].min(joint_y("root_ground").unwrap_or(0.0));
        let z_up = |y: f32| (y - cj_c[1]) * scale + d_c[2];
        let ank_z = (ankle_y - ground_y) * scale;
        let s_shin = (z_up(knee_y) - ank_z) / (knee_y - ankle_y);
        let z_of = move |y: f32| {
            if y <= ankle_y {
                (y - ground_y) * scale
            } else if y < knee_y {
                ank_z + (y - ankle_y) * s_shin
            } else {
                z_up(y)
            }
        };
        let xf = |p: [f32; 3]| -> [f32; 3] {
            [
                -(p[0] - cj_c[0]) * scale + d_c[0],
                (p[2] - cj_c[2]) * scale + d_c[1],
                z_of(p[1]),
            ]
        };
        report.push(format!(
            "  {}: ankle y {:.2} -> z {:.2}, knee y {:.2} -> z {:.2}, shin stretch {:.2}x",
            t.name,
            ankle_y,
            ank_z,
            knee_y,
            z_up(knee_y),
            s_shin / scale
        ));

        // fitted CJ corner positions
        let body_pos: Vec<[f32; 3]> = corner_pos.iter().map(|p| xf(*p)).collect();

        // greedy assignment: each CJ corner gets a unique stock slot, nearest
        // position first so the binding of that slot fits the vertex
        let mut cands: Vec<(f32, usize, usize)> = Vec::new();
        for (i, p) in body_pos.iter().enumerate() {
            let mut best: Vec<(f32, usize)> = Vec::new();
            for s in 0..vc {
                let o = s * 52 + 4;
                let sp = [
                    f32_at(&stock_b, o),
                    f32_at(&stock_b, o + 4),
                    f32_at(&stock_b, o + 8),
                ];
                let d = (p[0] - sp[0]).powi(2) + (p[1] - sp[1]).powi(2) + (p[2] - sp[2]).powi(2);
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
        let mut slot_of_cj = vec![usize::MAX; body_pos.len()];
        let mut used = vec![false; vc];
        let mut assigned = 0usize;
        for (_, cj, s) in &cands {
            if slot_of_cj[*cj] == usize::MAX && !used[*s] {
                slot_of_cj[*cj] = *s;
                used[*s] = true;
                assigned += 1;
            }
        }
        // any corner without a slot takes the nearest still-unused stock slot
        for (i, p) in body_pos.iter().enumerate() {
            if slot_of_cj[i] != usize::MAX {
                continue;
            }
            let mut best_s = usize::MAX;
            let mut bd = f32::MAX;
            for (s, taken) in used.iter().enumerate() {
                if *taken {
                    continue;
                }
                let o = s * 52 + 4;
                let sp = [
                    f32_at(&stock_b, o),
                    f32_at(&stock_b, o + 4),
                    f32_at(&stock_b, o + 8),
                ];
                let d = (p[0] - sp[0]).powi(2) + (p[1] - sp[1]).powi(2) + (p[2] - sp[2]).powi(2);
                if d < bd {
                    bd = d;
                    best_s = s;
                }
            }
            if best_s != usize::MAX {
                slot_of_cj[i] = best_s;
                used[best_s] = true;
                assigned += 1;
            }
        }
        for v in slot_of_cj.iter_mut() {
            if *v == usize::MAX {
                *v = 0;
            }
        }
        // B stream: move the used slots to the CJ positions/normals
        let mut b_out = stock_b.clone();
        let mut dist_mean = 0.0f64;
        let mut dist_max = 0.0f32;
        for (i, p) in body_pos.iter().enumerate() {
            let s = slot_of_cj[i];
            let o = s * 52 + 4;
            let sp = [
                f32_at(&stock_b, o),
                f32_at(&stock_b, o + 4),
                f32_at(&stock_b, o + 8),
            ];
            let d =
                ((p[0] - sp[0]).powi(2) + (p[1] - sp[1]).powi(2) + (p[2] - sp[2]).powi(2)).sqrt();
            dist_mean += d as f64;
            dist_max = dist_max.max(d);
            let target = &mut b_out[s * 52..s * 52 + 52];
            target[4..8].copy_from_slice(&p[0].to_le_bytes());
            target[8..12].copy_from_slice(&p[1].to_le_bytes());
            target[12..16].copy_from_slice(&p[2].to_le_bytes());
            let n0 = corner_nrm[i];
            // same yaw mapping rotation for the normal
            let mut n = [-n0[0], n0[2], n0[1]];
            // the stretched shin is scaled along model z: correct the normal
            let y0 = corner_pos[i][1];
            if y0 > ankle_y && y0 < knee_y {
                n[2] *= scale / s_shin;
            }
            let n = normalize(n);
            target[20..24].copy_from_slice(&n[0].to_le_bytes());
            target[24..28].copy_from_slice(&n[1].to_le_bytes());
            target[28..32].copy_from_slice(&n[2].to_le_bytes());
            target[32..36].copy_from_slice(&n[2].to_le_bytes());
            let (mut tx, mut ty) = (n[1], -n[0]);
            let lt = (tx * tx + ty * ty).sqrt();
            if lt > 0.0 {
                tx /= lt;
                ty /= lt;
            }
            target[36..40].copy_from_slice(&tx.to_le_bytes());
            target[40..44].copy_from_slice(&ty.to_le_bytes());
            target[44..48].copy_from_slice(&0.0f32.to_le_bytes());
        }

        // A stream: keep the stock record per slot, but remap the CJ corner UVs
        // into the 2x2 atlas cell of the corner's material. The engine samples
        // the stored v as top-down (V = S mod 1, verified against the stock
        // body mesh + stock diffuse), so a corner at OBJ v needs the cell's
        // top-down row (1 - v) offset by the cell row: u at +0, -v at +4.
        let a_off = t.data + 28 + ic * 2;
        let a_base = a_off - t.data;
        let mut a_out = region[a_base..a_base + vc * 20].to_vec();
        for (i, s) in slot_of_cj.iter().enumerate() {
            let uv = obj.corner_uv[i];
            let cell = obj.corners[i].3;
            let (col, row) = ((cell % 2) as f32, (cell / 2) as f32);
            let u = uv[0] * 0.5 + col * 0.5;
            let neg_v = -0.5 - uv[1] * 0.5 + row * 0.5;
            let at = s * 20;
            a_out[at..at + 4].copy_from_slice(&u.to_le_bytes());
            a_out[at + 4..at + 8].copy_from_slice(&neg_v.to_le_bytes());
        }

        // IB: CJ triangles through the slot mapping, reversed winding (CW
        // front faces), padded with degenerate triangles
        let mut ib = vec![0u8; ic * 2];
        for (ti, tri) in body_faces.iter().enumerate() {
            let mut mapped = [0u16; 3];
            for (k, &cid) in tri.iter().enumerate() {
                mapped[k] = slot_of_cj[cid as usize] as u16;
            }
            for (k, &vi) in [mapped[0], mapped[2], mapped[1]].iter().enumerate() {
                ib[(ti * 3 + k) * 2..(ti * 3 + k) * 2 + 2].copy_from_slice(&vi.to_le_bytes());
            }
        }

        dec_f.seek(SeekFrom::Start(a_off as u64))?;
        dec_f.write_all(&a_out)?;
        dec_f.seek(SeekFrom::Start((t.data + 28) as u64))?;
        dec_f.write_all(&ib)?;
        dec_f.seek(SeekFrom::Start(t.b as u64))?;
        dec_f.write_all(&b_out)?;
        report.push(format!(
            "  {}: {} CJ corners -> unique stock slots ({} assigned), {} tris, match mean {:.2} max {:.2}",
            t.name,
            body_pos.len(),
            assigned,
            body_faces.len(),
            dist_mean / body_pos.len() as f64,
            dist_max
        ));
    }
    Ok(report)
}
