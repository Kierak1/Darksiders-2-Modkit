use eframe::egui;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Instant;

use ds2_modkit::dsave;
use ds2_modkit::map::{propmesh, roommat, texmap, worldmap};

use crate::job::{Action, Job};
use crate::map::{DecodedRoom, MapEdit, MapRoomCam, PropLayer, RoomLayer};
use crate::{assets, mods, render, save, status};

#[derive(PartialEq, Clone, Copy, Debug)]
pub(crate) enum Tab {
    Mods,
    Save,
    Map,
    Assets,
    Status,
}

pub(crate) struct AssetRow {
    pub(crate) path: String,
    pub(crate) size: u64,
    pub(crate) package: String,
}

pub(crate) struct App {
    pub(crate) media: PathBuf,
    pub(crate) media_input: String,
    pub(crate) tab: Tab,
    pub(crate) mods: Vec<PathBuf>,
    pub(crate) selected_mod: Option<usize>,
    pub(crate) assets: Vec<AssetRow>,
    pub(crate) asset_filter: String,
    pub(crate) selected_asset: Option<usize>,
    pub(crate) log: Vec<String>,
    pub(crate) job: Option<Job>,
    // map tab
    pub(crate) map_world_names: Vec<String>,
    pub(crate) map_world: Option<usize>,
    pub(crate) map_objects: Vec<worldmap::PlacedObject>,
    pub(crate) map_filter: String,
    pub(crate) map_classes: Vec<(String, usize)>,
    pub(crate) map_class_on: std::collections::HashMap<String, bool>,
    pub(crate) map_selected: Option<usize>,
    pub(crate) map_edits: std::collections::HashMap<usize, MapEdit>,
    pub(crate) cam_yaw: f32,
    pub(crate) cam_pitch: f32,
    pub(crate) cam_dist: f32,
    pub(crate) cam_target: [f32; 3],
    pub(crate) ctrl_rx: Option<mpsc::Receiver<(String, mpsc::Sender<String>)>>,
    pub(crate) ctx: Option<egui::Context>,
    pub(crate) shot_path: Option<String>,
    pub(crate) selected_props: Vec<(String, String)>,
    pub(crate) props_for: Option<usize>,
    // room meshes
    pub(crate) map_mesh_filter: String,
    pub(crate) map_mesh_matches: Vec<String>,
    /// Loaded room-mesh layers (one per maps asset).
    pub(crate) map_rooms: Vec<RoomLayer>,
    /// Decoded static-prop triangles (trees, rocks, statues, ...).
    pub(crate) map_props: Vec<PropLayer>,
    /// Background room-loading pool: `(label, decoded room)` results.
    pub(crate) map_room_rx: Option<mpsc::Receiver<(String, Result<DecodedRoom, String>)>>,
    /// Zone material database (room batch -> source mesh fingerprint match).
    pub(crate) map_materials: Option<std::sync::Arc<roommat::SourceDb>>,
    pub(crate) map_mat_rx:
        Option<mpsc::Receiver<Result<std::sync::Arc<roommat::SourceDb>, String>>>,
    pub(crate) map_mat_folder: String,
    /// Rooms still being decoded by the pool.
    pub(crate) map_room_loading: usize,
    /// Fit the view once the pending world rooms finish loading.
    pub(crate) map_auto_fit: bool,
    pub(crate) map_room_tex: Option<egui::TextureHandle>,
    pub(crate) map_room_cam: Option<MapRoomCam>,
    /// True when `map_room_tex` was rendered at reduced resolution while the
    /// camera is being dragged; triggers a full-res re-render once idle.
    pub(crate) map_room_low: bool,
    /// Low-res rendering stays active until this instant (covers a burst of
    /// scroll events so the full-res pass runs once, when input stops).
    pub(crate) map_interact_until: Option<Instant>,
    pub(crate) map_show_meshes: bool,
    pub(crate) map_tex_override: Option<(String, Arc<texmap::TexImage>)>,
    /// Material name (roommat key) -> diffuse binding, merged over rooms.
    pub(crate) map_mat_defs: HashMap<String, propmesh::MaterialTex>,
    /// Global per-room-pixel texture table (deduped by name).
    pub(crate) map_textures: Vec<Arc<texmap::TexImage>>,
    pub(crate) map_tex_by_name: HashMap<String, u32>,
    pub(crate) map_tex_loader: texmap::TextureLoader,
    pub(crate) map_tex_status: String,
    // save tab
    pub(crate) save_path: String,
    pub(crate) save_file: Option<dsave::Save>,
    /// The save as loaded/last written, for pre-write diffs.
    pub(crate) save_original: Option<dsave::Save>,
    /// Item DB + catalog + localization, built in the background per save.
    pub(crate) save_db: Option<save::SaveDb>,
    /// Diff shown once before a write with pending changes; a second click
    /// confirms.
    pub(crate) save_confirm_diff: Vec<String>,
    /// Per-stat DragValue buffers, keyed by `<item>:<value_path>`.
    pub(crate) save_stat_edits: HashMap<String, i64>,
    pub(crate) save_new_attr: String,
    pub(crate) save_new_value: i64,
    pub(crate) save_browser_filter: String,
    pub(crate) save_browser_container: String,
    pub(crate) save_browser_rarity: String,
    pub(crate) save_browser_target: String,
    pub(crate) save_browser_page: usize,
    pub(crate) save_move_target: String,
    pub(crate) save_filter: String,
    pub(crate) save_filter_container: String,
    pub(crate) save_filter_rarity: String,
    pub(crate) save_sort_attr: String,
    pub(crate) save_totals_scope: String,
    pub(crate) save_slots: Vec<dsave::SlotSummary>,
    pub(crate) save_copy_target: String,
    pub(crate) save_bulk_op: usize,
    pub(crate) save_bulk_attr: String,
    pub(crate) save_bulk_value: i64,
    pub(crate) save_bulk_percent: i64,
    pub(crate) save_bulk_scope: String,
    pub(crate) save_bulk_level: i64,
    pub(crate) save_bulk_possessed: bool,
    pub(crate) save_bulk_item: usize,
    pub(crate) save_bulk_armed: bool,
    pub(crate) save_preset_path: String,
    pub(crate) save_selected: Option<usize>,
    pub(crate) save_status: String,
    pub(crate) save_field: String,
    pub(crate) save_value: String,
}

fn detect_media() -> PathBuf {
    if let Some(dir) = std::env::var_os("DS2_MEDIA") {
        return PathBuf::from(dir);
    }
    let game = Path::new("Game")
        .join("Darksiders II Deathinitive Edition")
        .join("media");
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut candidates = vec![cwd.join(&game)];
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().skip(1).take(5) {
            candidates.push(ancestor.join(&game));
        }
    }
    candidates
        .into_iter()
        .find(|c| c.join("manifest.bin").is_file())
        .unwrap_or_else(|| cwd.join(game))
}

pub(crate) fn human(n: u64) -> String {
    if n >= 1 << 30 {
        format!("{:.2} GB", n as f64 / (1 << 30) as f64)
    } else if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / (1 << 20) as f64)
    } else if n >= 1 << 10 {
        format!("{:.0} KB", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}

fn default_save_path() -> String {
    dsave::default_save_dir()
        .map(|dir| dir.join("dssave0.dsav"))
        .filter(|p| p.is_file())
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

impl App {
    pub(crate) fn new() -> Self {
        let media = detect_media();
        let mut app = Self {
            media_input: media.display().to_string(),
            media,
            tab: Tab::Mods,
            mods: Vec::new(),
            selected_mod: None,
            assets: Vec::new(),
            asset_filter: String::new(),
            selected_asset: None,
            log: Vec::new(),
            job: None,
            map_world_names: Vec::new(),
            map_world: None,
            map_objects: Vec::new(),
            map_filter: String::new(),
            map_classes: Vec::new(),
            map_class_on: std::collections::HashMap::new(),
            map_selected: None,
            map_edits: std::collections::HashMap::new(),
            cam_yaw: 0.6,
            cam_pitch: 0.5,
            cam_dist: 12000.0,
            cam_target: [0.0, 0.0, 0.0],
            ctrl_rx: None,
            ctx: None,
            shot_path: None,
            selected_props: Vec::new(),
            props_for: None,
            map_mesh_filter: String::new(),
            map_mesh_matches: Vec::new(),
            map_rooms: Vec::new(),
            map_props: Vec::new(),
            map_room_rx: None,
            map_materials: None,
            map_mat_rx: None,
            map_mat_folder: String::new(),
            map_room_loading: 0,
            map_auto_fit: false,
            map_room_tex: None,
            map_room_cam: None,
            map_room_low: false,
            map_interact_until: None,
            map_show_meshes: true,
            map_tex_override: None,
            map_mat_defs: HashMap::new(),
            map_textures: Vec::new(),
            map_tex_by_name: HashMap::new(),
            map_tex_loader: texmap::TextureLoader::default(),
            map_tex_status: String::new(),
            save_path: default_save_path(),
            save_file: None,
            save_original: None,
            save_db: None,
            save_confirm_diff: Vec::new(),
            save_stat_edits: HashMap::new(),
            save_new_attr: String::new(),
            save_new_value: 0,
            save_browser_filter: String::new(),
            save_browser_container: String::new(),
            save_browser_rarity: String::new(),
            save_browser_target: String::new(),
            save_browser_page: 0,
            save_move_target: String::new(),
            save_filter: String::new(),
            save_filter_container: String::new(),
            save_filter_rarity: String::new(),
            save_sort_attr: String::new(),
            save_totals_scope: String::new(),
            save_slots: Vec::new(),
            save_copy_target: String::new(),
            save_bulk_op: 0,
            save_bulk_attr: String::new(),
            save_bulk_value: 0,
            save_bulk_percent: 100,
            save_bulk_scope: "equipped".to_string(),
            save_bulk_level: 1,
            save_bulk_possessed: false,
            save_bulk_item: 0,
            save_bulk_armed: false,
            save_preset_path: String::new(),
            save_selected: None,
            save_status: "no save loaded".to_string(),
            save_field: String::new(),
            save_value: String::new(),
        };
        app.scan_mods();
        app.refresh_assets();
        app.load_map_worlds();
        app.log("ready");
        app
    }

    pub(crate) fn log(&mut self, message: impl Into<String>) {
        self.log.push(message.into());
        if self.log.len() > 1000 {
            self.log.remove(0);
        }
    }

    pub(crate) fn root(&self) -> PathBuf {
        self.media
            .ancestors()
            .nth(3)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.media.clone())
    }

    pub(crate) fn mods_dir(&self) -> PathBuf {
        self.root().join("mods")
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.ctx = Some(ui.ctx().clone());
        // an in-app screenshot requested via the control channel
        if let Some(path) = self.shot_path.clone() {
            let image = ui.ctx().input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::Screenshot { image, .. } = e {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
            });
            if let Some(img) = image {
                match render::write_png(std::path::Path::new(&path), &img) {
                    Ok(()) => self.log(format!("shot saved to {path}")),
                    Err(e) => self.log(format!("shot failed: {e}")),
                }
                self.shot_path = None;
            }
        }
        if let Some(rx) = self.ctrl_rx.take() {
            while let Ok((line, reply)) = rx.try_recv() {
                let resp = self.handle_command(&line);
                let _ = reply.send(resp);
            }
            self.ctrl_rx = Some(rx);
        }
        if self.map_selected != self.props_for {
            self.refresh_selected_props();
            self.props_for = self.map_selected;
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(200));
        self.poll_job();
        let idle = self.job.is_none();
        let mut action = Action::None;

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("Darksiders 2 Modkit");
                ui.separator();
                ui.label("media:");
                ui.add(egui::TextEdit::singleline(&mut self.media_input).desired_width(430.0));
                if ui.add_enabled(idle, egui::Button::new("Browse")).clicked() {
                    action = Action::BrowseMedia;
                }
                if ui.add_enabled(idle, egui::Button::new("Reload")).clicked() {
                    action = Action::RefreshAll;
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Mods, "Mods");
                ui.selectable_value(&mut self.tab, Tab::Save, "Save");
                ui.selectable_value(&mut self.tab, Tab::Map, "Map");
                ui.selectable_value(&mut self.tab, Tab::Assets, "Assets");
                ui.selectable_value(&mut self.tab, Tab::Status, "Status");
            });
            if let Some(job) = &self.job {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.label(&job.label);
                    match &job.progress {
                        Some((label, done, total)) if *total > 0 => {
                            let fraction = *done as f32 / *total as f32;
                            ui.add(
                                egui::ProgressBar::new(fraction)
                                    .desired_width(260.0)
                                    .text(format!("{label} {:.0}%", fraction * 100.0)),
                            );
                        }
                        _ => {
                            ui.label("working...");
                        }
                    }
                    ui.label(format!("{}s", job.started.elapsed().as_secs()));
                });
            }
            ui.add_space(4.0);
        });

        egui::Panel::bottom("log")
            .resizable(true)
            .default_size(150.0)
            .show(ui, |ui| {
                ui.label("Log");
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for line in &self.log {
                            ui.monospace(line);
                        }
                    });
            });

        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Save => {
                self.draw_save_tab(ui);
            }
            Tab::Mods => {
                if let Some(a) = mods::draw(self, ui, idle) {
                    action = a;
                }
            }
            Tab::Map => {
                self.map_ui(ui);
            }
            Tab::Assets => {
                if let Some(a) = assets::draw(self, ui, idle) {
                    action = a;
                }
            }
            Tab::Status => {
                if let Some(a) = status::draw(self, ui, idle) {
                    action = a;
                }
            }
        });
        match action {
            Action::None => {}
            Action::ApplyMod(i) => self.apply_mod(i),
            Action::ExtractAsset(i) => self.extract_asset(i),
            Action::ReplaceAsset(i) => self.replace_asset(i),
            Action::RefreshAll => {
                self.media = PathBuf::from(self.media_input.trim());
                self.scan_mods();
                self.refresh_assets();
            }
            Action::RefreshAssets => self.refresh_assets(),
            Action::RestoreScripts => self.restore_scripts(),
            Action::RestorePaks => self.restore_paks(),
            Action::BrowseMedia => {
                if let Some(dir) = rfd::FileDialog::new()
                    .set_title("Select the game media folder")
                    .pick_folder()
                {
                    self.media = dir;
                    self.media_input = self.media.display().to_string();
                    self.scan_mods();
                    self.refresh_assets();
                }
            }
        }

        if self.job.is_some() {
            ui.ctx().request_repaint();
        }
    }
}
