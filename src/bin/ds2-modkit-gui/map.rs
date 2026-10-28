use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ds2_modkit::map::{propmesh, roommat, roommesh, texmap, worldmap};
use ds2_modkit::obsp::payload as obsp_payload;
use ds2_modkit::pack::{dds, manifest, obpk, upak};

use crate::app::App;
use crate::render;

/// One decoded maps room: full payload name, mesh, material keys, material
/// diffuse bindings.
pub(crate) type DecodedRoom = (
    String,
    roommesh::RoomMesh,
    HashSet<String>,
    HashMap<String, propmesh::MaterialTex>,
);

/// A triangle with optional per-corner UVs (room/prop raster input).
pub(crate) type TriWithUv = ([[f32; 3]; 3], Option<[[f32; 2]; 3]>);

/// Map-view camera cache key: `(yaw, pitch, dist, target, screen size)`.
pub(crate) type MapRoomCam = (f32, f32, f32, [f32; 3], [usize; 2]);

#[derive(Clone, Copy)]
pub(crate) struct MapEdit {
    pub(crate) pos: [f32; 3],
    pub(crate) rot: Option<[f32; 3]>,
}

/// One decoded maps room, ready for the z-buffer rasterizer.
pub(crate) struct RoomLayer {
    pub(crate) tris: Vec<TriWithUv>,
    /// Material color per triangle, `None` while unclassified.
    pub(crate) tri_colors: Vec<Option<[u8; 3]>>,
    /// Diffuse texture per triangle (index into the app texture table,
    /// `u32::MAX` = no texture).
    pub(crate) tri_tex: Vec<u32>,
    /// Per-batch triangle ranges into `tris`.
    pub(crate) batch_ranges: Vec<(usize, usize)>,
    /// Triangle-shape fingerprints per batch (for re-classification).
    pub(crate) batch_fps: Vec<roommat::Fingerprint>,
    /// Material name per batch after classification.
    pub(crate) batch_mats: Vec<Option<String>>,
    /// Material keys the room defines (classification filter).
    pub(crate) mats: HashSet<String>,
    pub(crate) center: [f32; 3],
    pub(crate) radius: f32,
}

pub(crate) struct PropLayer {
    pub(crate) tris: Vec<([[f32; 3]; 3], [u8; 3])>,
    pub(crate) tri_uv: Vec<Option<[[f32; 2]; 3]>>,
    pub(crate) tri_tex: Vec<u32>,
    pub(crate) center: [f32; 3],
    pub(crate) radius: f32,
}

impl PropLayer {
    fn new(tris: Vec<([[f32; 3]; 3], [u8; 3])>) -> Self {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for (p, _) in &tris {
            for v in p {
                for k in 0..3 {
                    lo[k] = lo[k].min(v[k]);
                    hi[k] = hi[k].max(v[k]);
                }
            }
        }
        let center = [0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5);
        let radius = (0..3)
            .map(|k| (hi[k] - center[k]).abs())
            .fold(0.0f32, f32::max);
        Self {
            tris,
            tri_uv: Vec::new(),
            tri_tex: Vec::new(),
            center,
            radius,
        }
    }
}

impl RoomLayer {
    fn new(room: &roommesh::RoomMesh) -> Self {
        let tris = room.triangles_with_uv();
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for (p, _) in &tris {
            for v in p {
                for k in 0..3 {
                    lo[k] = lo[k].min(v[k]);
                    hi[k] = hi[k].max(v[k]);
                }
            }
        }
        let center = if tris.is_empty() {
            room.origin
        } else {
            [0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5)
        };
        let radius = if tris.is_empty() {
            0.0
        } else {
            (0..3)
                .map(|k| (hi[k] - center[k]).abs())
                .fold(0.0f32, f32::max)
        };
        let mut batch_ranges = Vec::with_capacity(room.batches.len());
        let mut batch_fps = Vec::with_capacity(room.batches.len());
        let mut at = 0usize;
        for b in &room.batches {
            let n = b.indices.as_chunks::<3>().0.len();
            batch_ranges.push((at, at + n));
            batch_fps.push(roommat::fingerprint(&b.positions, &b.indices));
            at += n;
        }
        let tri_colors = vec![None; tris.len()];
        let tri_tex = vec![u32::MAX; tris.len()];
        Self {
            tris,
            tri_colors,
            tri_tex,
            batch_ranges,
            batch_fps,
            batch_mats: vec![None; room.batches.len()],
            mats: HashSet::new(),
            center,
            radius,
        }
    }

    /// Assign a flat material color per batch from the zone database.
    fn apply_materials(&mut self, db: &roommat::SourceDb) {
        for (bi, fp) in self.batch_fps.iter().enumerate() {
            let (start, end) = self.batch_ranges[bi];
            let class = roommat::classify_fp(fp, db, Some(&self.mats));
            let color = class
                .as_ref()
                .map(|(name, _)| db.colors.get(name).copied().unwrap_or([150, 150, 150]));
            self.batch_mats[bi] = class.map(|(name, _)| name);
            let n = self.tri_colors.len();
            for c in &mut self.tri_colors[start..end.min(n)] {
                *c = color;
            }
        }
    }

    /// Bind per-batch diffuse textures from the app texture table, matching
    /// the classified source-mesh name to the room's material definitions
    /// through the same normalized key.
    fn apply_textures(&mut self, by_name: &HashMap<String, u32>) {
        for (bi, mat) in self.batch_mats.iter().enumerate() {
            let Some(name) = mat else { continue };
            let Some(id) = by_name.get(&roommat::material_key(name)) else {
                continue;
            };
            let (start, end) = self.batch_ranges[bi];
            let n = self.tri_tex.len();
            for t in &mut self.tri_tex[start..end.min(n)] {
                *t = *id;
            }
        }
    }
}

impl App {
    pub(crate) fn load_map_worlds(&mut self) {
        self.map_world_names.clear();
        let Ok(data) = std::fs::read(self.media.join("worlds.bin")) else {
            self.log("map: worlds.bin not found");
            return;
        };
        match worldmap::parse_worlds(&data) {
            Ok(worlds) => {
                self.map_world_names = worlds.iter().map(|w| w.name.clone()).collect();
                self.log(format!("map: {} worlds", worlds.len()));
            }
            Err(e) => self.log(format!("map: {e}")),
        }
    }

    pub(crate) fn load_map_objects(&mut self, idx: usize) {
        self.map_objects.clear();
        self.map_selected = None;
        self.map_edits.clear();
        let Ok(data) = std::fs::read(self.media.join("worlds.bin")) else {
            self.log("map: worlds.bin not found");
            return;
        };
        let Ok(worlds) = worldmap::parse_worlds(&data) else {
            self.log("map: bad worlds.bin");
            return;
        };
        let Some(w) = worlds.get(idx) else { return };
        match worldmap::collect_objects(&data, w) {
            Ok(objects) => {
                self.log(format!("map: {} objects in {}", objects.len(), w.name));
                if !objects.is_empty() {
                    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                    for o in &objects {
                        for k in 0..3 {
                            lo[k] = lo[k].min(o.position[k]);
                            hi[k] = hi[k].max(o.position[k]);
                        }
                    }
                    self.cam_target = [0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5);
                    let span = (0..3).map(|k| hi[k] - lo[k]).fold(0.0f32, f32::max);
                    self.cam_dist = span.max(2000.0) * 1.2;
                }
                self.map_objects = objects;
                self.map_classes.clear();
                let mut hist: std::collections::HashMap<&str, usize> =
                    std::collections::HashMap::new();
                for o in &self.map_objects {
                    *hist.entry(o.class.as_str()).or_default() += 1;
                }
                let mut list: Vec<(String, usize)> =
                    hist.into_iter().map(|(c, n)| (c.to_string(), n)).collect();
                list.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
                self.map_classes = list;
                self.map_class_on.clear();
            }
            Err(e) => self.log(format!("map: {e}")),
        }
    }

    pub(crate) fn class_enabled(&self, class: &str) -> bool {
        *self.map_class_on.get(class).unwrap_or(&true)
    }

    pub(crate) fn select_first_object(&mut self) {
        self.map_selected = self
            .map_objects
            .iter()
            .position(|o| self.class_enabled(&o.class));
    }

    /// Re-read the selected object's blob and snapshot all of its properties.
    pub(crate) fn refresh_selected_props(&mut self) {
        self.selected_props.clear();
        let Some(i) = self.map_selected else {
            return;
        };
        let o = &self.map_objects[i];
        let Ok(data) = std::fs::read(self.media.join("worlds.bin")) else {
            return;
        };
        let (boff, bsize) = (o.blob_offset, o.blob_size);
        let Some(blob) = data.get(boff..boff + bsize) else {
            return;
        };
        let Ok(parsed) = obsp_payload::parse_payload(blob) else {
            return;
        };
        let Some(target) = obsp_payload::resolve_path(blob, &parsed, &o.path) else {
            return;
        };
        if let obsp_payload::Value::Object(obj) = &target.value {
            for p in &obj.properties {
                let v: String = format!("{}", p.value).chars().take(120).collect();
                self.selected_props.push((p.name.clone(), v));
            }
        }
    }

    pub(crate) fn object_position(&self, i: usize) -> [f32; 3] {
        self.map_edits
            .get(&i)
            .map(|e| e.pos)
            .unwrap_or(self.map_objects[i].position)
    }

    fn object_rotation(&self, i: usize) -> Option<[f32; 3]> {
        self.map_edits
            .get(&i)
            .and_then(|e| e.rot)
            .or(self.map_objects[i].rotation)
    }

    pub(crate) fn set_position_edit(&mut self, i: usize, pos: [f32; 3]) {
        let rot = self.map_edits.get(&i).and_then(|e| e.rot);
        self.map_edits.insert(i, MapEdit { pos, rot });
    }

    pub(crate) fn set_rotation_edit(&mut self, i: usize, rot: [f32; 3]) {
        let pos = self.object_position(i);
        self.map_edits.insert(
            i,
            MapEdit {
                pos,
                rot: Some(rot),
            },
        );
    }

    /// Patch the pending position edits into a copy of worlds.bin.
    pub(crate) fn save_map(&mut self) -> String {
        let path = self.media.join("worlds.bin");
        let Ok(mut data) = std::fs::read(&path) else {
            self.log("map: worlds.bin not found");
            return "err: worlds.bin not found".into();
        };
        let mut patched = 0usize;
        let mut failed = 0usize;
        let mut by_blob: std::collections::HashMap<(usize, usize), Vec<usize>> =
            std::collections::HashMap::new();
        for &i in self.map_edits.keys() {
            let o = &self.map_objects[i];
            by_blob
                .entry((o.blob_offset, o.blob_size))
                .or_default()
                .push(i);
        }
        for ((boff, bsize), ids) in by_blob {
            let Some(mut blob) = data.get(boff..boff + bsize).map(|b| b.to_vec()) else {
                failed += ids.len();
                continue;
            };
            let Ok(parsed) = obsp_payload::parse_payload(&blob) else {
                failed += ids.len();
                continue;
            };
            for i in ids {
                let o = &self.map_objects[i];
                let pos = self.map_edits[&i].pos;
                let mut ok = true;
                for (k, pk) in pos.iter().enumerate() {
                    let p = format!("{}.Position[{k}]", o.path);
                    let Some(target) = obsp_payload::resolve_path(&blob, &parsed, &p) else {
                        ok = false;
                        break;
                    };
                    if obsp_payload::patch_target(&mut blob, &target, &format!("{}", pk)).is_err() {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    patched += 1;
                } else {
                    failed += 1;
                }
                if let Some(rot) = self.map_edits[&i].rot {
                    if o.rotation.is_some() {
                        for (k, r) in rot.iter().enumerate() {
                            let p = format!("{}.Rotation[{k}]", o.path);
                            if let Some(target) = obsp_payload::resolve_path(&blob, &parsed, &p) {
                                let _ = obsp_payload::patch_target(
                                    &mut blob,
                                    &target,
                                    &format!("{}", r),
                                );
                            }
                        }
                    }
                }
            }
            data[boff..boff + bsize].copy_from_slice(&blob);
        }
        let bak = self.media.join("worlds.bin.bak");
        if !bak.exists() {
            if let Err(e) = std::fs::copy(&path, &bak) {
                self.log(format!("map: backup failed: {e}"));
                return format!("err: backup failed: {e}");
            }
        }
        match std::fs::write(&path, &data) {
            Ok(()) => {
                self.log(format!(
                    "map: saved worlds.bin ({patched} objects patched, {failed} failed); backup at {}",
                    bak.display()
                ));
                self.map_edits.clear();
                format!("saved worlds.bin patched {patched} failed {failed}")
            }
            Err(e) => {
                self.log(format!("map: save failed: {e}"));
                format!("err: save failed: {e}")
            }
        }
    }

    /// Spawn a clone of the selected object at `pos`, or delete the selected
    /// object (`spawn = None`). Writes worlds.bin immediately with a one-time
    /// .bak and reloads the object list.
    pub(crate) fn map_struct_edit(&mut self, spawn: Option<[f32; 3]>) -> String {
        if !self.map_edits.is_empty() {
            return "err: save or revert pending moves first".into();
        }
        let Some(i) = self.map_selected else {
            return "err: no selection".into();
        };
        let Some(world_idx) = self.map_world else {
            return "err: no world".into();
        };
        let o = &self.map_objects[i];
        let (boff, bsize, path, cls) =
            (o.blob_offset, o.blob_size, o.path.clone(), o.class.clone());
        let path_file = self.media.join("worlds.bin");
        let Ok(mut data) = std::fs::read(&path_file) else {
            return "err: worlds.bin not found".into();
        };
        let result = match spawn {
            Some(pos) => worldmap::spawn_object(&mut data, boff, bsize, &path, pos),
            None => worldmap::delete_object(&mut data, boff, bsize, &path).map(|()| path.clone()),
        };
        let new_path = match result {
            Ok(new_path) => new_path,
            Err(e) => {
                self.log(format!("map: edit failed: {e}"));
                return format!("err: {e}");
            }
        };
        let bak = self.media.join("worlds.bin.bak");
        if !bak.exists() {
            if let Err(e) = std::fs::copy(&path_file, &bak) {
                self.log(format!("map: backup failed: {e}"));
                return format!("err: backup failed: {e}");
            }
        }
        if let Err(e) = std::fs::write(&path_file, &data) {
            self.log(format!("map: write failed: {e}"));
            return format!("err: write failed: {e}");
        }
        let action = if spawn.is_some() {
            "spawned"
        } else {
            "deleted"
        };
        self.log(format!(
            "map: {action} {cls} {path} -> {new_path}; worlds.bin {} B",
            data.len()
        ));
        self.load_map_objects(world_idx);
        if spawn.is_some() {
            self.map_selected = self
                .map_objects
                .iter()
                .position(|o| o.path == new_path && o.class == cls);
            if let Some(idx) = self.map_selected {
                self.cam_target = self.map_objects[idx].position;
            }
        } else {
            self.select_first_object();
        }
        format!("{action} {new_path}")
    }

    /// Maps-package environment assets as `(lowercased tail, full name)`,
    /// longest tail first (for prefix matching).
    fn maps_assets(&self) -> Result<Vec<(String, String)>, String> {
        let data = std::fs::read(self.media.join("manifest.bin"))
            .map_err(|e| format!("err: manifest.bin: {e}"))?;
        let m = manifest::parse(&data).map_err(|e| format!("err: bad manifest.bin: {e}"))?;
        let mut assets: Vec<(String, String)> = Vec::new();
        for f in m.files.iter().filter(|f| f.pkg_idx != 0xFF) {
            let pkg = m
                .packages
                .get(f.pkg_idx as usize)
                .map(String::as_str)
                .unwrap_or("");
            if pkg != "maps" {
                continue;
            }
            let full = m.full_name(f);
            if let Some(tail) = full.strip_prefix("environments/") {
                if tail.starts_with("terrain") {
                    continue;
                }
                assets.push((tail.to_lowercase(), full));
            }
        }
        assets.sort_by_key(|(t, _)| std::cmp::Reverse(t.len()));
        Ok(assets)
    }

    /// Asset for a region name: tail equals the name or ends with `_<name>`.
    fn region_asset<'a>(assets: &'a [(String, String)], name: &str) -> Option<&'a str> {
        let name = name.to_lowercase();
        assets
            .iter()
            .filter(|(t, _)| *t == name || t.ends_with(&format!("_{name}")))
            .min_by_key(|(t, _)| t.len())
            .map(|(_, full)| full.as_str())
    }

    /// Load the maps room whose region contains (or is nearest to) the camera
    /// target. Region membership comes from `RegionData` + the `RegionCell` /
    /// `RegionPortal` references in `worlds.bin`, so it works for worlds whose
    /// portal names are generic (`dlc1_world`).
    pub(crate) fn load_nearest_room(&mut self) -> String {
        if self.map_objects.is_empty() {
            return "err: load a world first".into();
        }
        let assets = match self.maps_assets() {
            Ok(a) => a,
            Err(e) => return e,
        };

        // 1. Region-based lookup: RegionData name -> asset whose tail is
        //    `<name>` or ends with `_<name>`.
        if let Some(world_idx) = self.map_world {
            let wname = self.map_world_names[world_idx].clone();
            if let Ok(wfile) = std::fs::read(self.media.join("worlds.bin")) {
                if let Ok(worlds) = worldmap::parse_worlds(&wfile) {
                    if let Some(w) = worlds.iter().find(|w| w.name == wname) {
                        if let Some(id) = worldmap::nearest_region(&wfile, w, self.cam_target) {
                            if let Some(region) = worldmap::regions(&wfile, w)
                                .ok()
                                .and_then(|rs| rs.into_iter().find(|r| r.id == id))
                            {
                                if let Some(full) = Self::region_asset(&assets, &region.name) {
                                    let full = full.to_string();
                                    let msg = self.load_room_mesh(&full);
                                    return format!("{msg} (region {} {})", region.id, region.name);
                                }
                                self.log(format!(
                                    "map: region {} {} has no maps asset",
                                    region.id, region.name
                                ));
                            }
                        }
                    }
                }
            }
        }

        // 2. Fallback: match region/portal object names against asset tails.
        let mut best: Option<(f32, String)> = None;
        for o in &self.map_objects {
            let class = o.class.to_lowercase();
            if !class.contains("region") && !class.contains("portal") {
                continue;
            }
            let name = o.name.to_lowercase();
            let Some((_, asset)) = assets.iter().find(|(t, _)| name.starts_with(t)) else {
                continue;
            };
            let d = [0, 1, 2]
                .map(|k| (o.position[k] - self.cam_target[k]).powi(2))
                .into_iter()
                .sum::<f32>();
            if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                best = Some((d, asset.clone()));
            }
        }
        let Some((d2, asset)) = best else {
            return "err: no region matches a maps asset".into();
        };
        let msg = self.load_room_mesh(&asset);
        format!("{msg} (nearest portal at {:.0} units)", d2.sqrt())
    }

    /// Decode every room asset referenced by the current world's regions on a
    /// pool of background worker threads; results stream into `map_rooms`.
    pub(crate) fn queue_world_rooms(&mut self) -> String {
        let Some(world_idx) = self.map_world else {
            return "err: no world".into();
        };
        let wname = self.map_world_names[world_idx].clone();
        let Ok(wfile) = std::fs::read(self.media.join("worlds.bin")) else {
            return "err: worlds.bin not found".into();
        };
        let Ok(worlds) = worldmap::parse_worlds(&wfile) else {
            return "err: bad worlds.bin".into();
        };
        let Some(w) = worlds.iter().find(|w| w.name == wname) else {
            return format!("err: world {wname} not found");
        };
        let Ok(regions) = worldmap::regions(&wfile, w) else {
            return "err: no region table".into();
        };
        let assets = match self.maps_assets() {
            Ok(a) => a,
            Err(e) => return e,
        };
        self.map_rooms.clear();
        self.map_room_rx = None;
        let mut jobs: Vec<(String, String)> = Vec::new();
        let mut missing = Vec::new();
        for r in &regions {
            match Self::region_asset(&assets, &r.name) {
                Some(full) => jobs.push((full.to_string(), format!("{} {}", r.id, r.name))),
                None => missing.push(format!("{} {}", r.id, r.name)),
            }
        }
        let note = if missing.is_empty() {
            String::new()
        } else {
            format!(", {} regions without asset", missing.len())
        };
        if jobs.is_empty() {
            return format!("no rooms to load{note}");
        }
        let Ok(data) = std::fs::read(self.media.join("manifest.bin")) else {
            return "err: manifest.bin not found".into();
        };
        let Ok(m) = manifest::parse(&data) else {
            return "err: bad manifest.bin".into();
        };
        // Decode rooms on all cores; results stream back over a channel and
        // are appended to `map_rooms` as they arrive.
        let total = jobs.len();
        self.map_room_loading = total;
        self.map_auto_fit = true;
        self.map_room_cam = None;
        let m = std::sync::Arc::new(m);
        let media = self.media.clone();
        let (tx, rx) = mpsc::channel();
        self.map_room_rx = Some(rx);
        // Start the zone material scan alongside the room decode; rooms are
        // colored as soon as the database and their batches are both ready.
        if let Some(folder) = jobs
            .iter()
            .find_map(|(asset, _)| m.find(asset).and_then(|e| roommat::zone_folder(&m, e)))
        {
            if self.map_mat_folder != folder {
                self.map_materials = None;
                self.map_mat_rx = None;
                self.map_mat_folder = folder.clone();
            }
            if self.map_materials.is_none() && self.map_mat_rx.is_none() {
                let (mtx, mrx) = mpsc::channel();
                self.map_mat_rx = Some(mrx);
                let m2 = m.clone();
                let media2 = media.clone();
                std::thread::spawn(move || {
                    let res = roommat::build(&media2, &m2, &folder, None)
                        .map(std::sync::Arc::new)
                        .map_err(|e| e.to_string());
                    let _ = mtx.send(res);
                });
            }
        }
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .clamp(1, 8)
            .min(total);
        let chunk = total.div_ceil(threads);
        for slice in jobs.chunks(chunk) {
            let tx = tx.clone();
            let m = m.clone();
            let media = media.clone();
            let slice = slice.to_vec();
            std::thread::spawn(move || {
                for (asset, label) in slice {
                    let res = decode_room_asset(&media, &m, &asset);
                    if tx.send((label, res)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        format!(
            "loading {total} rooms of {} regions on {threads} threads{note}",
            regions.len()
        )
    }

    /// Append room results from the loader pool (non-blocking unless `block`).
    fn drain_room_results(&mut self, block: bool) {
        let Some(rx) = self.map_room_rx.take() else {
            return;
        };
        let mut closed = false;
        loop {
            let (label, res) = if block {
                match rx.recv() {
                    Ok(m) => m,
                    Err(_) => {
                        closed = true;
                        break;
                    }
                }
            } else {
                match rx.try_recv() {
                    Ok(m) => m,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        closed = true;
                        break;
                    }
                }
            };
            self.map_room_loading = self.map_room_loading.saturating_sub(1);
            match res {
                Ok((name, room, mats, defs)) => {
                    self.log(format!("map: room {name}: {} tris", room.triangles().len()));
                    self.map_mat_defs.extend(
                        defs.into_iter()
                            .map(|(n, d)| (roommat::material_key(&n), d)),
                    );
                    let mut layer = RoomLayer::new(&room);
                    layer.mats = mats;
                    if let Some(db) = &self.map_materials {
                        layer.apply_materials(db);
                        layer.apply_textures(&self.map_tex_by_name);
                    }
                    self.map_rooms.push(layer);
                }
                Err(e) => self.log(format!("map: room {label} skipped: {e}")),
            }
        }
        if closed {
            self.map_room_loading = 0;
        }
        if self.map_room_loading == 0 {
            if self.map_auto_fit && !self.map_rooms.is_empty() {
                self.map_auto_fit = false;
                let msg = self.load_props();
                self.log(format!("map: {msg}"));
                self.fit_view();
            }
            self.map_room_cam = None;
        } else {
            self.map_room_rx = Some(rx);
        }
    }

    /// Poll the zone material scan and color every loaded room when it lands.
    fn drain_material_db(&mut self) {
        let Some(rx) = self.map_mat_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(db)) => {
                self.log(format!(
                    "map: material db {}: {} materials, {} source meshes from {}",
                    self.map_mat_folder,
                    db.fingerprints.len(),
                    db.meshes,
                    db.assets
                ));
                for layer in &mut self.map_rooms {
                    layer.apply_materials(&db);
                }
                self.map_room_cam = None;
                self.map_materials = Some(db);
                let msg = self.load_room_textures();
                self.log(format!("map: {msg}"));
            }
            Ok(Err(e)) => self.log(format!("map: material scan failed: {e}")),
            Err(mpsc::TryRecvError::Empty) => self.map_mat_rx = Some(rx),
            Err(mpsc::TryRecvError::Disconnected) => {}
        }
    }

    /// Apply or report the zone material database for the current rooms.
    pub(crate) fn load_materials(&mut self) -> String {
        if let Some(db) = self.map_materials.clone() {
            for layer in &mut self.map_rooms {
                layer.apply_materials(&db);
            }
            self.map_room_cam = None;
            let tex = self.load_room_textures();
            return format!(
                "applied {} materials to {} rooms; {tex}",
                db.fingerprints.len(),
                self.map_rooms.len()
            );
        }
        if self.map_mat_rx.is_some() {
            return format!("material scan of {} still running", self.map_mat_folder);
        }
        "err: no material database (load world rooms first)".into()
    }

    /// Resolve every known material's diffuse DDS and bind it per batch.
    fn load_room_textures(&mut self) -> String {
        let defs: Vec<(String, propmesh::MaterialTex)> = self
            .map_mat_defs
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let names: Vec<String> = defs
            .iter()
            .filter(|(_, d)| !d.diffuse.is_empty())
            .map(|(_, d)| d.diffuse.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if names.is_empty() {
            return "no material diffuse bindings".into();
        }
        let Ok(data) = std::fs::read(self.media.join("manifest.bin")) else {
            return "err: manifest.bin not found".into();
        };
        let Ok(m) = manifest::parse(&data) else {
            return "err: bad manifest.bin".into();
        };
        let imgs = self.map_tex_loader.load_many(&self.media, &m, &names);
        let mut bound = 0usize;
        for (key, def) in &defs {
            if def.diffuse.is_empty() || self.map_tex_by_name.contains_key(key) {
                continue;
            }
            let Some(img) = imgs.get(&def.diffuse) else {
                continue;
            };
            let id = self.map_textures.len() as u32;
            self.map_textures.push(img.clone());
            self.map_tex_by_name.insert(key.clone(), id);
            bound += 1;
        }
        for layer in &mut self.map_rooms {
            layer.apply_textures(&self.map_tex_by_name);
        }
        self.map_room_cam = None;
        self.map_tex_status = format!(
            "{} diffuse names, {} decoded, {} materials bound, {} textures bound",
            names.len(),
            imgs.len(),
            bound,
            self.map_tex_by_name.len()
        );
        self.map_tex_status.clone()
    }

    /// Load every room now (control-channel path) and report.
    pub(crate) fn flush_room_queue(&mut self) -> String {
        while self.map_room_loading > 0 {
            self.drain_room_results(true);
        }
        while self.map_mat_rx.is_some() {
            self.drain_material_db();
            if self.map_mat_rx.is_some() {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        let tris: usize = self.map_rooms.iter().map(|r| r.tris.len()).sum();
        format!("loaded {} rooms ({} tris)", self.map_rooms.len(), tris)
    }

    pub(crate) fn clear_rooms(&mut self) {
        self.map_rooms.clear();
        self.map_props.clear();
        self.map_room_rx = None;
        self.map_room_loading = 0;
        self.map_auto_fit = false;
        self.map_room_tex = None;
        self.map_room_cam = None;
        self.map_room_low = false;
        self.map_interact_until = None;
    }

    /// Match the filter against the maps package's environment assets.
    pub(crate) fn find_mesh_assets(&mut self) -> String {
        self.map_mesh_matches.clear();
        let Ok(data) = std::fs::read(self.media.join("manifest.bin")) else {
            return "err: manifest.bin not found".into();
        };
        let Ok(m) = manifest::parse(&data) else {
            return "err: bad manifest.bin".into();
        };
        let filter = self.map_mesh_filter.to_lowercase();
        for f in m.files.iter().filter(|f| f.pkg_idx != 0xFF) {
            let pkg = m
                .packages
                .get(f.pkg_idx as usize)
                .map(String::as_str)
                .unwrap_or("");
            if pkg != "maps" {
                continue;
            }
            let full = m.full_name(f);
            if !full.starts_with("environments/")
                || (!filter.is_empty() && !full.to_lowercase().contains(&filter))
            {
                continue;
            }
            self.map_mesh_matches.push(full);
            if self.map_mesh_matches.len() >= 60 {
                break;
            }
        }
        format!("{} matches", self.map_mesh_matches.len())
    }

    /// Carve + decode a room asset and set it as the visible mesh layer.
    pub(crate) fn load_room_mesh(&mut self, asset: &str) -> String {
        let Ok(data) = std::fs::read(self.media.join("manifest.bin")) else {
            return "err: manifest.bin not found".into();
        };
        let Ok(m) = manifest::parse(&data) else {
            return "err: bad manifest.bin".into();
        };
        match decode_room_asset(&self.media, &m, asset) {
            Ok((full, room, mats, defs)) => {
                let uv_count = room
                    .triangles_with_uv()
                    .iter()
                    .filter(|(_, uv)| uv.is_some())
                    .count();
                self.map_rooms.clear();
                self.map_room_rx = None;
                self.map_room_loading = 0;
                self.map_mat_defs.extend(
                    defs.into_iter()
                        .map(|(n, d)| (roommat::material_key(&n), d)),
                );
                let mut layer = RoomLayer::new(&room);
                layer.mats = mats;
                if let Some(db) = &self.map_materials {
                    layer.apply_materials(db);
                    layer.apply_textures(&self.map_tex_by_name);
                }
                self.map_rooms.push(layer);
                self.map_room_cam = None;
                self.map_show_meshes = true;
                format!(
                    "mesh {full} tris {} uvs {uv_count}",
                    self.map_rooms[0].tris.len()
                )
            }
            Err(e) => format!("err: {e}"),
        }
    }

    /// Decode the zone's static-prop assets and place every matching world
    /// object (trees, rocks, statues, arches) as textured triangles.
    pub(crate) fn load_props(&mut self) -> String {
        self.map_props.clear();
        let world = self
            .map_world
            .and_then(|i| self.map_world_names.get(i))
            .cloned()
            .unwrap_or_default()
            .to_lowercase();
        let (folder, base): (&str, &[&str]) = if self.map_mat_folder.is_empty() {
            if world.starts_with("z2") || world.starts_with("dlc1") {
                ("Zone_02", &["z2_dlc_rocks", "z2_dlc_props", "z2_rocks"])
            } else if world.starts_with("dlc2") {
                ("Zone_DLC02", &["dlc02_rocks"])
            } else if world.starts_with("dlc3") {
                ("Zone_DLC03", &["dlc03_rocks"])
            } else {
                ("Zone_01", &["zone01_flora", "zone01_rocks"])
            }
        } else {
            ("", &[])
        };
        let Ok(data) = std::fs::read(self.media.join("manifest.bin")) else {
            return "err: manifest.bin not found".into();
        };
        let Ok(m) = manifest::parse(&data) else {
            return "err: bad manifest.bin".into();
        };
        // The room scan knows the world's media zone folder; enumerate its
        // vegetation/rock/prop assets instead of a hardcoded list.
        let zone = if folder.is_empty() {
            self.map_mat_folder.clone()
        } else {
            folder.to_string()
        };
        let mut assets: Vec<String> = base
            .iter()
            .map(|n| format!("world/environments/{zone}/{n}"))
            .collect();
        let prefix = format!("world/environments/{}/", zone.to_lowercase());
        for f in m.files.iter().filter(|f| f.pkg_idx != 0xFF) {
            let full = m.full_name(f).replace('\\', "/");
            let low = full.to_lowercase();
            if !low.starts_with(&prefix) {
                continue;
            }
            let fbase = f.name.to_lowercase();
            let wanted = [
                "flora", "flower", "tree", "grass", "fern", "bush", "moss", "leaf", "wood", "rock",
                "prop", "statue",
            ]
            .iter()
            .any(|k| fbase.contains(k));
            if wanted && !fbase.starts_with("cs") && !assets.contains(&full) {
                assets.push(full);
            }
        }
        let mut decoded: Vec<propmesh::PropMesh> = Vec::new();
        let mut defs: HashMap<String, propmesh::MaterialTex> = HashMap::new();
        self.log(format!(
            "map: prop assets {} ({})",
            assets.len(),
            assets.join(", ")
        ));
        for a in &assets {
            match decode_asset_payload(&self.media, &m, a) {
                Ok((_, payload)) => {
                    let n = decoded.len();
                    decoded.extend(propmesh::parse_props(&payload));
                    self.log(format!("map:   {a}: {} records", decoded.len() - n));
                    for (n, d) in propmesh::material_defs(&payload) {
                        defs.entry(roommat::material_key(&n)).or_insert(d);
                    }
                }
                Err(e) => self.log(format!("map: prop asset {a}: {e}")),
            }
        }
        self.map_mat_defs.extend(defs);
        let tex_msg = self.load_room_textures();
        self.log(format!("map: {tex_msg}"));
        // Match objects to records: the object name shorter than the record,
        // or a prefix of it after dropping instance suffixes (`_A_03`).
        let mut placed = 0usize;
        let mut tris = 0usize;
        let mut textured = 0usize;
        let mut uvs = 0usize;
        for obj in &self.map_objects {
            if obj.class != "StaticObject" {
                continue;
            }
            let lower = obj.name.to_lowercase();
            let mesh = decoded.iter().find(|p| {
                let rec = p.name.to_lowercase();
                rec.starts_with(&lower) || {
                    let mut base = lower.as_str();
                    while let Some(pos) = base.rfind('_') {
                        base = &base[..pos];
                        if base.len() >= 5 && rec.starts_with(base) {
                            return true;
                        }
                    }
                    false
                }
            });
            let Some(mesh) = mesh else { continue };
            let rot = obj.rotation.unwrap_or([0.0; 3]);
            let world_pos = propmesh::transform(&mesh.positions, rot, obj.position);
            let tex_id = self
                .map_tex_by_name
                .get(&roommat::material_key(&mesh.name))
                .copied()
                .unwrap_or(u32::MAX);
            let mesh_uvs = (mesh.uvs.len() >= mesh.positions.len()).then_some(&mesh.uvs);
            let mut layer_tris = Vec::with_capacity(mesh.indices.len() / 3);
            let mut tri_uv = Vec::with_capacity(mesh.indices.len() / 3);
            for t in mesh.indices.as_chunks::<3>().0 {
                let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
                if a < world_pos.len() && b < world_pos.len() && c < world_pos.len() {
                    layer_tris.push(([world_pos[a], world_pos[b], world_pos[c]], mesh.color));
                    tri_uv.push(mesh_uvs.map(|uv| [uv[a], uv[b], uv[c]]));
                }
            }
            if !layer_tris.is_empty() {
                placed += 1;
                tris += layer_tris.len();
                if tex_id != u32::MAX {
                    textured += layer_tris.len();
                }
                uvs += tri_uv.iter().filter(|u| u.is_some()).count();
                let mut layer = PropLayer::new(layer_tris);
                layer.tri_uv = tri_uv;
                layer.tri_tex = vec![tex_id; layer.tris.len()];
                self.map_props.push(layer);
            }
        }
        self.map_room_cam = None;
        format!(
            "props {placed} placed, {tris} tris ({textured} textured), {uvs} with uvs, {} records",
            decoded.len()
        )
    }

    /// Decode a DDS file and use it as a full-mesh texture override.
    pub(crate) fn load_room_texture(&mut self, path: &str) -> String {
        let Ok(bytes) = std::fs::read(path) else {
            return format!("err: cannot read {path}");
        };
        let (w, h, px) = match dds::decode(&bytes) {
            Ok(v) => v,
            Err(e) => return format!("err: decode {path}: {e}"),
        };
        self.map_tex_override = Some((path.to_string(), Arc::new(texmap::TexImage { w, h, px })));
        self.map_room_cam = None;
        format!("texture {path} {w}x{h}")
    }

    /// Orbit-camera eye position (`target + dir * dist`).
    pub(crate) fn camera_eye(&self) -> [f32; 3] {
        let (sp, cp) = self.cam_pitch.sin_cos();
        let (sy, cy) = self.cam_yaw.sin_cos();
        [
            self.cam_target[0] + cp * cy * self.cam_dist,
            self.cam_target[1] + cp * sy * self.cam_dist,
            self.cam_target[2] + sp * self.cam_dist,
        ]
    }

    /// Project a world point to screen with the current orbit camera.
    fn project(&self, p: [f32; 3], rect: egui::Rect) -> Option<(egui::Pos2, f32)> {
        let d = [
            p[0] - self.cam_target[0],
            p[1] - self.cam_target[1],
            p[2] - self.cam_target[2],
        ];
        let (sy, cy) = self.cam_yaw.sin_cos();
        let (sp, cp) = self.cam_pitch.sin_cos();
        // camera direction (target -> camera)
        let dir = [cp * cy, cp * sy, sp];
        // right = normalize(cross(dir, up_z)); up = cross(right, dir)
        let mut right = [-dir[1], dir[0], 0.0];
        let rl = (right[0] * right[0] + right[1] * right[1]).sqrt().max(1e-6);
        right = [right[0] / rl, right[1] / rl, 0.0];
        let up = [
            right[1] * dir[2] - right[2] * dir[1],
            right[2] * dir[0] - right[0] * dir[2],
            right[0] * dir[1] - right[1] * dir[0],
        ];
        let vx = d[0] * right[0] + d[1] * right[1] + d[2] * right[2];
        let vy = d[0] * up[0] + d[1] * up[1] + d[2] * up[2];
        let vz = d[0] * dir[0] + d[1] * dir[1] + d[2] * dir[2] - self.cam_dist;
        let depth = -vz; // distance in front of the camera
        if depth < 10.0 {
            return None;
        }
        let focal = rect.height() * 0.9;
        let sx = rect.center().x + vx * focal / depth;
        let sy_ = rect.center().y - vy * focal / depth;
        Some((egui::pos2(sx, sy_), depth))
    }

    pub(crate) fn fit_view(&mut self) {
        // Fit from above, looking down at the map (pitch +1.3 is nearly
        // straight down; the orbit camera sits above the target).
        self.cam_pitch = 1.3;
        // Prefer the loaded room meshes; fall back to object positions.
        if !self.map_rooms.is_empty() || !self.map_props.is_empty() {
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for r in &self.map_rooms {
                for k in 0..3 {
                    lo[k] = lo[k].min(r.center[k] - r.radius);
                    hi[k] = hi[k].max(r.center[k] + r.radius);
                }
            }
            for p in &self.map_props {
                for k in 0..3 {
                    lo[k] = lo[k].min(p.center[k] - p.radius);
                    hi[k] = hi[k].max(p.center[k] + p.radius);
                }
            }
            self.cam_target = [0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5);
            let span = (0..3).map(|k| hi[k] - lo[k]).fold(0.0f32, f32::max);
            self.cam_dist = span.max(2000.0) * 1.2;
            return;
        }
        if self.map_objects.is_empty() {
            return;
        }
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for i in 0..self.map_objects.len() {
            if !self.class_enabled(&self.map_objects[i].class) {
                continue;
            }
            let p = self.object_position(i);
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        self.cam_target = [0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5);
        let span = (0..3).map(|k| hi[k] - lo[k]).fold(0.0f32, f32::max);
        self.cam_dist = span.max(2000.0) * 1.2;
    }

    pub(crate) fn map_ui(&mut self, ui: &mut egui::Ui) {
        let mut load_world: Option<usize> = None;
        ui.horizontal(|ui| {
            ui.label(format!("{} worlds", self.map_world_names.len()));
            egui::ComboBox::from_id_salt("map_world")
                .selected_text(
                    self.map_world
                        .and_then(|i| self.map_world_names.get(i))
                        .cloned()
                        .unwrap_or_else(|| "-".into()),
                )
                .show_ui(ui, |ui| {
                    for (i, name) in self.map_world_names.iter().enumerate() {
                        if ui
                            .selectable_label(self.map_world == Some(i), name)
                            .clicked()
                        {
                            load_world = Some(i);
                        }
                    }
                });
            ui.label("name filter:");
            ui.add(egui::TextEdit::singleline(&mut self.map_filter).desired_width(150.0));
            if ui.button("All classes").clicked() {
                self.map_class_on.clear();
            }
            if ui.button("No classes").clicked() {
                for (c, _) in &self.map_classes {
                    self.map_class_on.insert(c.clone(), false);
                }
            }
            if ui.button("Fit view").clicked() {
                self.fit_view();
            }
            if ui
                .button("Flip 180")
                .on_hover_text("Rotate the view 180 deg (match the in-game map)")
                .clicked()
            {
                self.cam_yaw += std::f32::consts::PI;
                self.map_room_cam = None;
            }
            if ui
                .add_enabled(
                    !self.map_edits.is_empty(),
                    egui::Button::new("Save worlds.bin"),
                )
                .clicked()
            {
                self.save_map();
            }
        });
        if let Some(i) = load_world {
            self.map_world = Some(i);
            self.load_map_objects(i);
            self.select_first_object();
            let msg = self.queue_world_rooms();
            self.log(format!("map: {msg}"));
        }
        // append room results from the background loader pool
        self.drain_room_results(false);
        self.drain_material_db();

        ui.horizontal(|ui| {
            ui.label("mesh:");
            ui.add(
                egui::TextEdit::singleline(&mut self.map_mesh_filter)
                    .desired_width(150.0)
                    .hint_text("z1_ld_rm00"),
            );
            if ui.button("Find").clicked() {
                let msg = self.find_mesh_assets();
                self.log(format!("map: {msg}"));
            }
            if ui
                .button("Auto room")
                .on_hover_text("load the room nearest the view centre")
                .clicked()
            {
                let msg = self.load_nearest_room();
                self.log(format!("map: {msg}"));
            }
            if ui
                .button("Load world rooms")
                .on_hover_text("load every region's room mesh for this world")
                .clicked()
            {
                let msg = self.queue_world_rooms();
                self.log(format!("map: {msg}"));
            }
            if ui
                .button("Materials")
                .on_hover_text("color room batches from their source meshes (zone material scan)")
                .clicked()
            {
                let msg = self.load_materials();
                self.log(format!("map: {msg}"));
            }
            if ui.button("Clear").clicked() {
                self.clear_rooms();
            }
            ui.checkbox(&mut self.map_show_meshes, "show meshes");
            if ui.button("Texture...").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("DDS", &["dds"])
                    .pick_file()
                {
                    let msg = self.load_room_texture(&path.display().to_string());
                    self.log(format!("map: {msg}"));
                }
            }
            if self.map_tex_override.is_some() && ui.button("tex off").clicked() {
                self.map_tex_override = None;
                self.map_room_cam = None;
            }
            if let Some((name, tex)) = &self.map_tex_override {
                ui.label(format!(
                    "{}x{} {}",
                    tex.w,
                    tex.h,
                    name.rsplit(['/', '\\']).next().unwrap_or(name)
                ));
            }
            if !self.map_rooms.is_empty() {
                let tris: usize = self.map_rooms.iter().map(|r| r.tris.len()).sum();
                ui.label(format!("{} rooms ({} tris)", self.map_rooms.len(), tris));
            }
            if self.map_room_loading > 0 {
                ui.spinner();
                ui.label(format!(
                    "rooms {}/{}",
                    self.map_rooms.len(),
                    self.map_rooms.len() + self.map_room_loading
                ));
            }
        });
        if !self.map_mesh_matches.is_empty() {
            let mut load: Option<String> = None;
            ui.horizontal_wrapped(|ui| {
                for (i, name) in self.map_mesh_matches.iter().enumerate() {
                    if i >= 12 {
                        ui.label(format!("... {} more", self.map_mesh_matches.len() - 12));
                        break;
                    }
                    if ui.small_button(name).clicked() {
                        load = Some(name.clone());
                    }
                }
            });
            if let Some(name) = load {
                let msg = self.load_room_mesh(&name);
                self.map_mesh_matches.clear();
                self.log(format!("map: {msg}"));
            }
        }

        let enabled_classes = self
            .map_classes
            .iter()
            .filter(|(c, _)| self.class_enabled(c))
            .count();
        ui.horizontal(|ui| {
            ui.label(format!(
                "objects: {}  classes: {}/{}  pending edits: {}",
                self.map_objects.len(),
                enabled_classes,
                self.map_classes.len(),
                self.map_edits.len()
            ));
        });

        ui.horizontal(|ui| {
            // left column: class toggles + object list
            let total_h = ui.available_height();
            let view_h = (total_h - 175.0).max(260.0);
            let list_h = (view_h - 40.0).max(160.0);
            ui.vertical(|ui| {
                ui.set_min_width(340.0);
                ui.set_max_width(430.0);
                ui.strong("Classes");
                ui.allocate_ui(egui::vec2(ui.available_width(), 130.0), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("map_classes")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (c, n) in &self.map_classes {
                                let mut on = self.class_enabled(c);
                                ui.horizontal(|ui| {
                                    if ui.checkbox(&mut on, "").changed() {
                                        self.map_class_on.insert(c.clone(), on);
                                    }
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(10.0, 10.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().circle_filled(
                                        rect.center(),
                                        5.0,
                                        class_color(c),
                                    );
                                    ui.label(format!("{c} ({n})"));
                                });
                            }
                        });
                });
                ui.strong("Objects");
                let name_filter = self.map_filter.to_lowercase();
                egui::ScrollArea::vertical()
                    .id_salt("map_objects")
                    .max_height(list_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut shown = 0;
                        for (i, o) in self.map_objects.iter().enumerate() {
                            if !self.class_enabled(&o.class) {
                                continue;
                            }
                            if !name_filter.is_empty()
                                && !o.name.to_lowercase().contains(&name_filter)
                                && !o.path.to_lowercase().contains(&name_filter)
                            {
                                continue;
                            }
                            let edited = self.map_edits.contains_key(&i);
                            let label = if o.name == o.class || o.name.is_empty() {
                                format!("{}{}", if edited { "* " } else { "" }, o.class)
                            } else {
                                format!("{}{} | {}", if edited { "* " } else { "" }, o.class, o.name)
                            };
                            if ui.selectable_label(self.map_selected == Some(i), label).clicked() {
                                self.map_selected = Some(i);
                            }
                            shown += 1;
                            if shown >= 2000 {
                                break;
                            }
                        }
                        if shown == 0 {
                            ui.label("(no matches)");
                        }
                    });
            });
            ui.separator();
            // right: viewport + inspector
            ui.vertical(|ui| {
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), view_h),
                    egui::Sense::click_and_drag(),
                );
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 2.0, egui::Color32::from_gray(16));
                // This frame's raw wheel delta (egui's smoothed delta decays
                // over several frames and would multiply the zoom step).
                let wheel = ui.ctx().input(|i| {
                    i.events
                        .iter()
                        .filter_map(|e| match e {
                            egui::Event::MouseWheel { unit, delta, .. } => {
                                Some(match unit {
                                    egui::MouseWheelUnit::Line => delta.y * 50.0,
                                    egui::MouseWheelUnit::Point => delta.y,
                                    egui::MouseWheelUnit::Page => delta.y * 500.0,
                                })
                            }
                            _ => None,
                        })
                        .sum::<f32>()
                });
                let interacting = response.dragged()
                    || wheel != 0.0
                    || self
                        .map_interact_until
                        .is_some_and(|t| Instant::now() < t);
                if self.map_show_meshes && (!self.map_rooms.is_empty() || !self.map_props.is_empty())
                {
                    let ppp = ui.ctx().pixels_per_point();
                    let px_size = [
                        (rect.width() * ppp).round() as usize,
                        (rect.height() * ppp).round() as usize,
                    ];
                    let key = (
                        self.cam_yaw,
                        self.cam_pitch,
                        self.cam_dist,
                        self.cam_target,
                        px_size,
                    );
                    // Re-render when the camera/size changes, at 30% resolution
                    // while dragging/scrolling, then once more at full res.
                    if self.map_room_cam != Some(key) || (self.map_room_low && !interacting) {
                        let render_ppp = if interacting {
                            (ppp * 0.3).max(0.15)
                        } else {
                            ppp
                        };
                        let tex = self
                            .map_tex_override
                            .as_ref()
                            .map(|(_, t)| t.as_ref());
                        let img = render::rasterize_room(
                            &self.map_rooms,
                            &self.map_props,
                            &self.map_textures,
                            tex,
                            (self.cam_yaw, self.cam_pitch, self.cam_dist, self.cam_target),
                            rect,
                            render_ppp,
                        );
                        match &mut self.map_room_tex {
                            Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                            None => {
                                self.map_room_tex = Some(ui.ctx().load_texture(
                                    "room_mesh",
                                    img,
                                    egui::TextureOptions::NEAREST,
                                ))
                            }
                        }
                        self.map_room_cam = Some(key);
                        self.map_room_low = interacting;
                    }
                    if let Some(t) = &self.map_room_tex {
                        painter.image(
                            t.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                }
                // grid on the ground plane (z = target z)
                let gz = self.cam_target[2];
                let step = (self.cam_dist / 6.0).max(500.0);
                for g in -8..=8 {
                    let a = [g as f32 * step, -8.0 * step, gz];
                    let b = [g as f32 * step, 8.0 * step, gz];
                    if let (Some((pa, _)), Some((pb, _))) = (self.project(a, rect), self.project(b, rect)) {
                        painter.line_segment([pa, pb], egui::Stroke::new(1.0, egui::Color32::from_gray(38)));
                    }
                    let a = [-8.0 * step, g as f32 * step, gz];
                    let b = [8.0 * step, g as f32 * step, gz];
                    if let (Some((pa, _)), Some((pb, _))) = (self.project(a, rect), self.project(b, rect)) {
                        painter.line_segment([pa, pb], egui::Stroke::new(1.0, egui::Color32::from_gray(38)));
                    }
                }
                if let Some((o, _)) = self.project(self.cam_target, rect) {
                    painter.circle_stroke(o, 7.0, egui::Stroke::new(1.0, egui::Color32::from_gray(90)));
                }
                // interactions: LMB orbit, RMB/MMB pan, WASD/QE fly, shift fast
                let drag = response.drag_delta();
                let mut cam_input = false;
                if response.dragged_by(egui::PointerButton::Primary) {
                    self.cam_yaw -= drag.x * 0.006;
                    // Continuous orbit: past a pole, flip yaw so the user can
                    // keep rotating instead of hitting a hard clamp.
                    let mut p = self.cam_pitch + drag.y * 0.006;
                    if p > std::f32::consts::FRAC_PI_2 {
                        p = std::f32::consts::PI - p;
                        self.cam_yaw += std::f32::consts::PI;
                    } else if p < -std::f32::consts::FRAC_PI_2 {
                        p = -std::f32::consts::PI - p;
                        self.cam_yaw += std::f32::consts::PI;
                    }
                    self.cam_pitch = p.clamp(-1.55, 1.55);
                    cam_input = true;
                }
                if response.dragged_by(egui::PointerButton::Secondary)
                    || response.dragged_by(egui::PointerButton::Middle)
                {
                    let k = self.cam_dist * 0.0016;
                    let (sy, cy) = self.cam_yaw.sin_cos();
                    let (sp, cp) = self.cam_pitch.sin_cos();
                    let dir = [cp * cy, cp * sy, sp];
                    let mut right = [-dir[1], dir[0], 0.0];
                    let rl = (right[0] * right[0] + right[1] * right[1]).sqrt().max(1e-6);
                    right = [right[0] / rl, right[1] / rl, 0.0];
                    let up = [
                        right[1] * dir[2] - right[2] * dir[1],
                        right[2] * dir[0] - right[0] * dir[2],
                        right[0] * dir[1] - right[1] * dir[0],
                    ];
                    for kk in 0..3 {
                        self.cam_target[kk] -= right[kk] * drag.x * k;
                        self.cam_target[kk] += up[kk] * drag.y * k;
                    }
                    cam_input = true;
                }
                // Keyboard fly: W/S forward-back, A/D strafe, Q/E down-up,
                // shift for 8x speed. Movement is horizontal except Q/E.
                let (sy, cy) = self.cam_yaw.sin_cos();
                let (fwd, right) = ([cy, sy, 0.0], [-sy, cy, 0.0]);
                let (mut mx, mut my, mut mz) = (0.0f32, 0.0f32, 0.0f32);
                ui.ctx().input(|i| {
                    let f = (i.key_down(egui::Key::W) || i.key_down(egui::Key::ArrowUp)) as i32
                        - (i.key_down(egui::Key::S) || i.key_down(egui::Key::ArrowDown)) as i32;
                    let r = (i.key_down(egui::Key::D) || i.key_down(egui::Key::ArrowRight)) as i32
                        - (i.key_down(egui::Key::A) || i.key_down(egui::Key::ArrowLeft)) as i32;
                    let u = i.key_down(egui::Key::E) as i32 - i.key_down(egui::Key::Q) as i32;
                    if f != 0 || r != 0 || u != 0 {
                        let fast = i.modifiers.shift;
                        let speed = if fast { 8000.0 } else { 1000.0 } * i.stable_dt.min(0.1);
                        mx = (fwd[0] * f as f32 + right[0] * r as f32) * speed;
                        my = (fwd[1] * f as f32 + right[1] * r as f32) * speed;
                        mz = u as f32 * speed;
                    }
                });
                if mx != 0.0 || my != 0.0 || mz != 0.0 {
                    for k in 0..3 {
                        self.cam_target[k] += [mx, my, mz][k];
                    }
                    cam_input = true;
                }
                if response.hovered() && wheel != 0.0 {
                    // One step per scroll event, exponential so a notch changes
                    // the distance by a fixed percentage at any scale.
                    let factor = (1.0 - wheel * 0.0015).clamp(0.5, 2.0);
                    self.cam_dist = (self.cam_dist * factor).clamp(1.0, 500_000.0);
                    cam_input = true;
                }
                if cam_input {
                    self.map_interact_until =
                        Some(Instant::now() + Duration::from_millis(250));
                }
                let mut nearest: Option<(f32, usize)> = None;
                let click_pos = response.interact_pointer_pos();
                let mut drawn = 0usize;
                // Egui tessellates every marker each frame; cap the marker
                // count for huge worlds so the frame stays cheap.
                let max_dots = if self.map_objects.len() > 20_000 {
                    5000
                } else {
                    15000
                };
                for (i, o) in self.map_objects.iter().enumerate() {
                    if !self.class_enabled(&o.class) {
                        continue;
                    }
                    let p = self.object_position(i);
                    let Some((sp, depth)) = self.project(p, rect) else {
                        continue;
                    };
                    if !rect.contains(sp) || depth > self.cam_dist * 12.0 {
                        continue;
                    }
                    let edited = self.map_edits.contains_key(&i);
                    let selected = self.map_selected == Some(i);
                    let color = if selected {
                        egui::Color32::YELLOW
                    } else if edited {
                        egui::Color32::LIGHT_RED
                    } else {
                        class_color(&o.class)
                    };
                    let r = if selected { 6.0 } else { 3.0 };
                    painter.circle_filled(sp, r, color);
                    drawn += 1;
                    if let Some(click) = click_pos {
                        let d = (click - sp).length();
                        if d < 8.0 && nearest.map(|(nd, _)| d < nd).unwrap_or(true) {
                            nearest = Some((d, i));
                        }
                    }
                    if drawn >= max_dots {
                        break;
                    }
                }
                if response.clicked() {
                    if let Some((_, i)) = nearest {
                        self.map_selected = Some(i);
                    }
                }
                // selected marker + label
                if let Some(i) = self.map_selected {
                    let p = self.object_position(i);
                    if let Some((sp, _)) = self.project(p, rect) {
                        painter.circle_stroke(sp, 9.0, egui::Stroke::new(1.5, egui::Color32::YELLOW));
                        painter.text(
                            sp + egui::vec2(12.0, -12.0),
                            egui::Align2::LEFT_TOP,
                            format!("{} {}", self.map_objects[i].class, self.map_objects[i].name),
                            egui::FontId::monospace(11.0),
                            egui::Color32::YELLOW,
                        );
                    }
                }
                let eye = self.camera_eye();
                painter.text(
                    rect.left_top() + egui::vec2(8.0, 6.0),
                    egui::Align2::LEFT_TOP,
                    format!(
                        "{drawn} drawn | LMB orbit | RMB/MMB pan | scroll zoom | WASD move | Q/E down/up | Shift fast\n\
                         eye ({:.0}, {:.0}, {:.0})  ->  target ({:.0}, {:.0}, {:.0})   yaw {:.2} pitch {:.2} dist {:.0}",
                        eye[0], eye[1], eye[2],
                        self.cam_target[0], self.cam_target[1], self.cam_target[2],
                        self.cam_yaw, self.cam_pitch, self.cam_dist
                    ),
                    egui::FontId::monospace(12.0),
                    egui::Color32::GRAY,
                );

                // On-screen camera controls for mouse-only navigation.
                ui.horizontal(|ui| {
                    ui.label("camera:");
                    let (sy, cy) = self.cam_yaw.sin_cos();
                    let (fwd, right) = ([cy, sy, 0.0f32], [-sy, cy, 0.0f32]);
                    let step = (self.cam_dist * 0.25).max(200.0);
                    let mut mv = [0.0f32; 3];
                    if ui.button("Left").clicked() {
                        mv = [-right[0] * step, -right[1] * step, 0.0];
                    }
                    if ui.button("Right").clicked() {
                        mv = [right[0] * step, right[1] * step, 0.0];
                    }
                    if ui.button("Fwd").clicked() {
                        mv = [fwd[0] * step, fwd[1] * step, 0.0];
                    }
                    if ui.button("Back").clicked() {
                        mv = [-fwd[0] * step, -fwd[1] * step, 0.0];
                    }
                    if ui.button("Up").clicked() {
                        mv[2] = step;
                    }
                    if ui.button("Down").clicked() {
                        mv[2] = -step;
                    }
                    if ui
                        .add_enabled(self.map_selected.is_some(), egui::Button::new("Center"))
                        .clicked()
                    {
                        if let Some(i) = self.map_selected {
                            self.cam_target = self.object_position(i);
                        }
                    }
                    if mv != [0.0; 3] {
                        for (k, m) in mv.iter().enumerate() {
                            self.cam_target[k] += m;
                        }
                        self.map_interact_until =
                            Some(Instant::now() + Duration::from_millis(250));
                    }
                });

                if let Some(i) = self.map_selected {
                    let o = &self.map_objects[i];
                    ui.add_space(4.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.strong(format!("{} [{}]", o.name, o.class));
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 5.0, class_color(&o.class));
                    });
                    ui.monospace(format!(
                        "{}  blob {:#X}..{:#X}",
                        o.path,
                        o.blob_offset,
                        o.blob_offset + o.blob_size
                    ));
                    if let Some(rot) = o.rotation {
                        ui.monospace(format!(
                            "rotation ({:.2}, {:.2}, {:.2})",
                            rot[0], rot[1], rot[2]
                        ));
                    }
                    let mut pos = self.object_position(i);
                    let mut changed = false;
                    ui.horizontal(|ui| {
                        for k in 0..3 {
                            ui.label(["X", "Y", "Z"][k]);
                            changed |= ui
                                .add(egui::DragValue::new(&mut pos[k]).speed(10.0))
                                .changed();
                        }
                    });
                    if changed {
                        self.set_position_edit(i, pos);
                    }
                    if let Some(mut rot) = self.object_rotation(i) {
                        let mut rchanged = false;
                        ui.horizontal(|ui| {
                            ui.label("rot");
                            for r in &mut rot {
                                rchanged |= ui
                                    .add(
                                        egui::DragValue::new(r)
                                            .speed(0.5)
                                            .suffix(" deg"),
                                    )
                                    .changed();
                            }
                        });
                        if rchanged {
                            self.set_rotation_edit(i, rot);
                        }
                    }
                    if !self.selected_props.is_empty() {
                        ui.add_space(2.0);
                        egui::ScrollArea::vertical()
                            .id_salt("selprops")
                            .max_height(150.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for (k, v) in &self.selected_props {
                                    ui.monospace(format!("{k} = {v}"));
                                }
                            });
                    }
                    ui.horizontal(|ui| {
                        let mut op: Option<Option<[f32; 3]>> = None;
                        if ui
                            .button("Spawn clone")
                            .on_hover_text("clone at the view centre")
                            .clicked()
                        {
                            op = Some(Some(self.cam_target));
                        }
                        if ui
                            .button("Delete")
                            .on_hover_text("remove this object from worlds.bin")
                            .clicked()
                        {
                            op = Some(None);
                        }
                        if let Some(op) = op {
                            self.map_struct_edit(op);
                        }
                        if self.map_edits.contains_key(&i) {
                            if ui.button("revert").clicked() {
                                self.map_edits.remove(&i);
                            }
                            if ui.button("focus").clicked() {
                                self.cam_target = pos;
                            }
                        }
                    });
                }
            });
        });
    }

    pub(crate) fn describe_selected(&self) -> String {
        match self.map_selected {
            Some(i) => {
                let o = &self.map_objects[i];
                let p = self.object_position(i);
                format!(
                    "selected {i} {} {} pos {:.1} {:.1} {:.1}{}",
                    o.class,
                    o.name,
                    p[0],
                    p[1],
                    p[2],
                    if self.map_edits.contains_key(&i) {
                        " (edited)"
                    } else {
                        ""
                    }
                )
            }
            None => "selected none".into(),
        }
    }
}

/// Carve + decompress one asset payload (thread-safe).
fn decode_asset_payload(
    media: &Path,
    m: &manifest::Manifest,
    asset: &str,
) -> Result<(String, Vec<u8>), String> {
    let entry = m
        .find(asset)
        .ok_or_else(|| format!("asset not found: {asset}"))?;
    let full = m.full_name(entry);
    let carved = upak::carve(media, m, entry).map_err(|e| format!("carve {full}: {e}"))?;
    let payload = obpk::decompress(&carved).map_err(|e| format!("decompress {full}: {e}"))?;
    Ok((full, payload.1))
}

/// Carve + decompress + parse one maps room asset (thread-safe).
fn decode_room_asset(
    media: &Path,
    m: &manifest::Manifest,
    asset: &str,
) -> Result<DecodedRoom, String> {
    let (full, payload) = decode_asset_payload(media, m, asset)?;
    let room = roommesh::parse_room(&payload).map_err(|e| format!("parse {full}: {e}"))?;
    let defs = propmesh::material_defs(&payload);
    let mats = defs.keys().map(|n| roommat::material_key(n)).collect();
    Ok((full, room, mats, defs))
}

fn class_color(class: &str) -> egui::Color32 {
    match class {
        "spawnpoint" => return egui::Color32::from_rgb(235, 80, 80),
        "StaticObject" => return egui::Color32::from_rgb(120, 180, 255),
        "omnilight" => return egui::Color32::from_rgb(240, 220, 120),
        "waypoint" => return egui::Color32::from_rgb(120, 230, 140),
        "blocker" => return egui::Color32::from_rgb(150, 110, 200),
        "EffectObject" => return egui::Color32::from_rgb(255, 160, 220),
        _ => {}
    }
    let mut h = 0u32;
    for b in class.bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as u32);
    }
    let hue = (h % 360) as f32 / 360.0;
    let (r, g, b) = hsl_to_rgb(hue, 0.55, 0.62);
    egui::Color32::from_rgb(r, g, b)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h * 6.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h * 6.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}
