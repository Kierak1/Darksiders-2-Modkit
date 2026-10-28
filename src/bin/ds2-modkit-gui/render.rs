use eframe::egui;
use std::sync::Arc;

use ds2_modkit::map::texmap;

use crate::map::{PropLayer, RoomLayer};

/// Software z-buffer render of world-space room triangles into an egui image.
/// Mirrors `App::project` (same orbit camera, z-up) in pixel space.
/// Camera/view state shared by the rasterizer workers.
struct RasterView {
    w: usize,
    h: usize,
    dir: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    focal: f32,
    target: [f32; 3],
    dist: f32,
    /// Near plane; scales down with the orbit distance so close zooms do not
    /// clip the target away.
    near: f32,
}

impl RasterView {
    fn new(yaw: f32, pitch: f32, dist: f32, target: [f32; 3], w: usize, h: usize) -> Self {
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        let dir = [cp * cy, cp * sy, sp];
        let mut right = [-dir[1], dir[0], 0.0];
        let rl = (right[0] * right[0] + right[1] * right[1]).sqrt().max(1e-6);
        right = [right[0] / rl, right[1] / rl, 0.0];
        let up = [
            right[1] * dir[2] - right[2] * dir[1],
            right[2] * dir[0] - right[0] * dir[2],
            right[0] * dir[1] - right[1] * dir[0],
        ];
        Self {
            w,
            h,
            dir,
            right,
            up,
            focal: h as f32 * 0.9,
            target,
            dist,
            near: (dist * 0.02).clamp(0.5, 10.0),
        }
    }

    #[inline]
    fn project(&self, p: [f32; 3]) -> Option<([f32; 2], f32)> {
        let d = [
            p[0] - self.target[0],
            p[1] - self.target[1],
            p[2] - self.target[2],
        ];
        let vx = d[0] * self.right[0] + d[1] * self.right[1] + d[2] * self.right[2];
        let vy = d[0] * self.up[0] + d[1] * self.up[1] + d[2] * self.up[2];
        let vz = d[0] * self.dir[0] + d[1] * self.dir[1] + d[2] * self.dir[2] - self.dist;
        let depth = -vz;
        if depth < self.near {
            return None;
        }
        Some((
            [
                self.w as f32 / 2.0 + vx * self.focal / depth,
                self.h as f32 / 2.0 - vy * self.focal / depth,
            ],
            depth,
        ))
    }

    /// Room/prop-level cull: behind the camera, tiny on screen, or beyond the
    /// current view distance (roughly target +/- the orbit radius).
    #[inline]
    fn sphere_visible(&self, center: [f32; 3], radius: f32) -> bool {
        let dc = [
            center[0] - self.target[0],
            center[1] - self.target[1],
            center[2] - self.target[2],
        ];
        let depth_c = self.dist - (dc[0] * self.dir[0] + dc[1] * self.dir[1] + dc[2] * self.dir[2]);
        if depth_c + radius < self.near {
            return false;
        }
        if depth_c - radius > self.dist * 1.5 + 8000.0 + radius {
            return false;
        }
        let screen_r = radius * self.focal / (depth_c - radius).max(self.near);
        screen_r >= 1.0
    }
}

/// Nearest-neighbour sample of a texture with wrapping UVs.
fn sample_tex(tex: &texmap::TexImage, u: f32, v: f32) -> [u8; 4] {
    let tx = ((u.rem_euclid(1.0) * (tex.w - 1).max(1) as f32) as usize).min(tex.w - 1);
    let ty = ((v.rem_euclid(1.0) * (tex.h - 1).max(1) as f32) as usize).min(tex.h - 1);
    tex.px[ty * tex.w + tx]
}

/// Rasterize a slice of rooms+props into `color`/`zbuf` (workers own buffers).
fn rasterize_slice(
    rooms: &[RoomLayer],
    props: &[PropLayer],
    view: &RasterView,
    textures: &[Arc<texmap::TexImage>],
    tex_override: Option<&texmap::TexImage>,
    color: &mut [egui::Color32],
    zbuf: &mut [f32],
) {
    let (w, h) = (view.w, view.h);
    for room in rooms {
        if !view.sphere_visible(room.center, room.radius) {
            continue;
        }
        for (ti, (p3, uv3)) in room.tris.iter().enumerate() {
            let (a, b, c) = (&p3[0], &p3[1], &p3[2]);
            let (Some((pa, da)), Some((pb, db)), Some((pc, dc))) =
                (view.project(*a), view.project(*b), view.project(*c))
            else {
                continue;
            };
            let area = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pc[0] - pa[0]) * (pb[1] - pa[1]);
            if area.abs() < 1e-6 {
                continue;
            }
            let (ux, uy, uz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
            let (vx, vy, vz) = (c[0] - a[0], c[1] - a[1], c[2] - a[2]);
            let n = [uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx];
            let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
            let lit = (n[2].abs() * 0.65 + n[0].abs() * 0.2 + n[1].abs() * 0.15) / nl;
            let shade = 0.25 + 0.75 * lit.clamp(0.0, 1.0);
            let col = match room.tri_colors.get(ti).copied().flatten() {
                Some(c) => {
                    let s = 0.35 + 0.65 * shade;
                    egui::Color32::from_rgb(
                        (c[0] as f32 * s) as u8,
                        (c[1] as f32 * s) as u8,
                        (c[2] as f32 * s) as u8,
                    )
                }
                None => egui::Color32::from_rgb(
                    (70.0 + 150.0 * shade) as u8,
                    (85.0 + 135.0 * shade) as u8,
                    (110.0 + 120.0 * shade) as u8,
                ),
            };
            let minx = (pa[0].min(pb[0]).min(pc[0]) as isize).clamp(0, w as isize) as usize;
            let maxx =
                (pa[0].max(pb[0]).max(pc[0]).ceil() as isize + 1).clamp(0, w as isize) as usize;
            let miny = (pa[1].min(pb[1]).min(pc[1]) as isize).clamp(0, h as isize) as usize;
            let maxy =
                (pa[1].max(pb[1]).max(pc[1]).ceil() as isize + 1).clamp(0, h as isize) as usize;
            if maxx <= minx || maxy <= miny {
                continue;
            }
            let inv = 1.0 / area;
            for yy in miny..maxy {
                let py = yy as f32 + 0.5;
                for xx in minx..maxx {
                    let qx = xx as f32 + 0.5;
                    let w0 = ((pb[0] - qx) * (pc[1] - py) - (pc[0] - qx) * (pb[1] - py)) * inv;
                    let w1 = ((pc[0] - qx) * (pa[1] - py) - (pa[0] - qx) * (pc[1] - py)) * inv;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let z = w0 / da + w1 / db + w2 / dc;
                    let o = yy * w + xx;
                    if z > zbuf[o] {
                        zbuf[o] = z;
                        let sampled = tex_override.or_else(|| {
                            let id = room.tri_tex.get(ti).copied().unwrap_or(u32::MAX);
                            (id != u32::MAX)
                                .then(|| textures.get(id as usize))
                                .flatten()
                                .map(|t| t.as_ref())
                        });
                        let pixel = match (sampled, uv3) {
                            (Some(t), Some(uv)) => {
                                let u = w0 * uv[0][0] + w1 * uv[1][0] + w2 * uv[2][0];
                                let v = w0 * uv[0][1] + w1 * uv[1][1] + w2 * uv[2][1];
                                let p = sample_tex(t, u, v);
                                let s = 0.45 + 0.55 * shade;
                                egui::Color32::from_rgb(
                                    (p[0] as f32 * s) as u8,
                                    (p[1] as f32 * s) as u8,
                                    (p[2] as f32 * s) as u8,
                                )
                            }
                            _ => col,
                        };
                        color[o] = pixel;
                    }
                }
            }
        }
    }
    // Static props: flat material color, same projection/depth.
    for layer in props {
        if !view.sphere_visible(layer.center, layer.radius) {
            continue;
        }
        for (ti, (p3, c)) in layer.tris.iter().enumerate() {
            let (a, b, cc) = (&p3[0], &p3[1], &p3[2]);
            let (Some((pa, da)), Some((pb, db)), Some((pc, dc))) =
                (view.project(*a), view.project(*b), view.project(*cc))
            else {
                continue;
            };
            let area = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pc[0] - pa[0]) * (pb[1] - pa[1]);
            if area.abs() < 1e-6 {
                continue;
            }
            let (ux, uy, uz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
            let (vx, vy, vz) = (cc[0] - a[0], cc[1] - a[1], cc[2] - a[2]);
            let n = [uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx];
            let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
            let lit = (n[2].abs() * 0.65 + n[0].abs() * 0.2 + n[1].abs() * 0.15) / nl;
            let s = 0.55 + 0.45 * lit.clamp(0.0, 1.0);
            let col = egui::Color32::from_rgb(
                (c[0] as f32 * s) as u8,
                (c[1] as f32 * s) as u8,
                (c[2] as f32 * s) as u8,
            );
            let uv3 = layer.tri_uv.get(ti).copied().flatten();
            let sampled = tex_override.or_else(|| {
                let id = layer.tri_tex.get(ti).copied().unwrap_or(u32::MAX);
                (id != u32::MAX)
                    .then(|| textures.get(id as usize))
                    .flatten()
                    .map(|t| t.as_ref())
            });
            let minx = (pa[0].min(pb[0]).min(pc[0]) as isize).clamp(0, w as isize) as usize;
            let maxx =
                (pa[0].max(pb[0]).max(pc[0]).ceil() as isize + 1).clamp(0, w as isize) as usize;
            let miny = (pa[1].min(pb[1]).min(pc[1]) as isize).clamp(0, h as isize) as usize;
            let maxy =
                (pa[1].max(pb[1]).max(pc[1]).ceil() as isize + 1).clamp(0, h as isize) as usize;
            if maxx <= minx || maxy <= miny {
                continue;
            }
            let inv = 1.0 / area;
            for yy in miny..maxy {
                let py = yy as f32 + 0.5;
                for xx in minx..maxx {
                    let qx = xx as f32 + 0.5;
                    let w0 = ((pb[0] - qx) * (pc[1] - py) - (pc[0] - qx) * (pb[1] - py)) * inv;
                    let w1 = ((pc[0] - qx) * (pa[1] - py) - (pa[0] - qx) * (pc[1] - py)) * inv;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let z = w0 / da + w1 / db + w2 / dc;
                    let o = yy * w + xx;
                    if z > zbuf[o] {
                        zbuf[o] = z;
                        color[o] = match (sampled, uv3) {
                            (Some(t), Some(uv)) => {
                                let u = w0 * uv[0][0] + w1 * uv[1][0] + w2 * uv[2][0];
                                let v = w0 * uv[0][1] + w1 * uv[1][1] + w2 * uv[2][1];
                                let p = sample_tex(t, u, v);
                                let sh = 0.75 + 0.25 * lit.clamp(0.0, 1.0);
                                egui::Color32::from_rgb(
                                    (p[0] as f32 * sh) as u8,
                                    (p[1] as f32 * sh) as u8,
                                    (p[2] as f32 * sh) as u8,
                                )
                            }
                            _ => col,
                        };
                    }
                }
            }
        }
    }
}

pub(crate) fn rasterize_room(
    rooms: &[RoomLayer],
    props: &[PropLayer],
    textures: &[Arc<texmap::TexImage>],
    tex_override: Option<&texmap::TexImage>,
    cam: (f32, f32, f32, [f32; 3]),
    rect: egui::Rect,
    ppp: f32,
) -> egui::ColorImage {
    let (yaw, pitch, dist, target) = cam;
    let w = ((rect.width() * ppp).round() as usize).clamp(1, 4096);
    let h = ((rect.height() * ppp).round() as usize).clamp(1, 4096);
    let view = RasterView::new(yaw, pitch, dist, target, w, h);
    let mut color = vec![egui::Color32::TRANSPARENT; w * h];
    let mut zbuf = vec![-1.0f32; w * h];
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 8);
    if threads <= 1 || rooms.len() + props.len() < 3 {
        rasterize_slice(
            rooms,
            props,
            &view,
            textures,
            tex_override,
            &mut color,
            &mut zbuf,
        );
    } else {
        // Partition rooms+props across workers, each with its own color/depth
        // buffer; merge by depth (z = 1/depth, larger wins).
        let total = rooms.len() + props.len();
        let chunk = total.div_ceil(threads);
        let mut jobs: Vec<(usize, usize, bool)> = Vec::new();
        let mut s = 0usize;
        while s < total {
            let e = (s + chunk).min(total);
            if s < rooms.len() {
                let re = e.min(rooms.len());
                jobs.push((s, re, true));
                if e > rooms.len() {
                    jobs.push((0, e - rooms.len(), false));
                }
            } else {
                jobs.push((s - rooms.len(), e - rooms.len(), false));
            }
            s = e;
        }
        let results: Vec<(Vec<egui::Color32>, Vec<f32>)> = std::thread::scope(|scope| {
            let view = &view;
            let handles: Vec<_> = jobs
                .into_iter()
                .map(|(lo, hi, is_rooms)| {
                    scope.spawn(move || {
                        let mut c = vec![egui::Color32::TRANSPARENT; w * h];
                        let mut z = vec![-1.0f32; w * h];
                        if is_rooms {
                            rasterize_slice(
                                &rooms[lo..hi],
                                &[],
                                view,
                                textures,
                                tex_override,
                                &mut c,
                                &mut z,
                            );
                        } else {
                            rasterize_slice(
                                &[],
                                &props[lo..hi],
                                view,
                                textures,
                                tex_override,
                                &mut c,
                                &mut z,
                            );
                        }
                        (c, z)
                    })
                })
                .collect();
            handles.into_iter().map(|t| t.join().unwrap()).collect()
        });
        for (c, z) in results {
            for i in 0..color.len() {
                if z[i] > zbuf[i] {
                    zbuf[i] = z[i];
                    color[i] = c[i];
                }
            }
        }
    }
    egui::ColorImage::new([w, h], color)
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Minimal RGBA PNG writer (no extra deps).
pub(crate) fn write_png(path: &std::path::Path, img: &egui::ColorImage) -> std::io::Result<()> {
    use std::io::Write;
    let w = img.width() as u32;
    let h = img.height() as u32;
    let mut raw = Vec::with_capacity((w * h * 4 + h) as usize);
    for y in 0..h {
        raw.push(0u8);
        for x in 0..w {
            let p = img.pixels[(y * w + x) as usize];
            raw.extend_from_slice(&[p.r(), p.g(), p.b(), p.a()]);
        }
    }
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(&raw)?;
    let idat = enc.finish()?;
    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let chunk = |out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(tag);
        out.extend_from_slice(data);
        let mut crc_input = Vec::with_capacity(4 + data.len());
        crc_input.extend_from_slice(tag);
        crc_input.extend_from_slice(data);
        out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    };
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    std::fs::write(path, out)
}
