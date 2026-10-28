use eframe::egui;

use ds2_modkit::modkit;
use ds2_modkit::pack::{manifest, upak};

use crate::app::{human, App, AssetRow};
use crate::job::{Action, Msg};

impl App {
    pub(crate) fn refresh_assets(&mut self) {
        self.assets.clear();
        let bytes = match std::fs::read(self.media.join("manifest.bin")) {
            Ok(b) => b,
            Err(e) => {
                self.log(format!("manifest.bin read failed: {e}"));
                return;
            }
        };
        let m = match manifest::parse(&bytes) {
            Ok(m) => m,
            Err(e) => {
                self.log(format!("manifest parse failed: {e}"));
                return;
            }
        };
        let mut package_len: Vec<u64> = vec![0; m.packages.len()];
        for (i, name) in m.packages.iter().enumerate() {
            package_len[i] = std::fs::metadata(self.media.join(format!("{name}.upak")))
                .map(|md| md.len())
                .unwrap_or(0);
        }
        for file in &m.files {
            if file.pkg_idx == 0xFF {
                continue;
            }
            let upak_len = package_len.get(file.pkg_idx as usize).copied().unwrap_or(0);
            self.assets.push(AssetRow {
                path: m.full_name(file),
                size: upak::entry_size(&m, upak_len, file),
                package: m
                    .packages
                    .get(file.pkg_idx as usize)
                    .cloned()
                    .unwrap_or_default(),
            });
        }
        self.assets.sort_by(|a, b| a.path.cmp(&b.path));
        self.selected_asset = if self.assets.is_empty() {
            None
        } else {
            Some(0)
        };
        let count = self.assets.len();
        self.log(format!("loaded {count} assets"));
    }

    pub(crate) fn extract_asset(&mut self, index: usize) {
        let asset = self.assets[index].path.clone();
        let media = self.media.clone();
        self.start_job(
            format!("extracting {asset}"),
            false,
            move |tx| match modkit::extract_asset(&media, &asset) {
                Ok(report) => {
                    let textures = if report.texture_count > 0 {
                        format!(
                            ", {} textures -> {}",
                            report.texture_count,
                            report.textures.as_ref().unwrap().display()
                        )
                    } else {
                        String::new()
                    };
                    let _ = tx.send(Msg::Log(format!(
                        "{asset}: {} -> {}{}",
                        human(report.bytes as u64),
                        report.obpk.display(),
                        textures
                    )));
                }
                Err(e) => {
                    let _ = tx.send(Msg::Log(format!("extract failed: {e}")));
                }
            },
        );
    }

    pub(crate) fn replace_asset(&mut self, index: usize) {
        let asset = self.assets[index].path.clone();
        let Some(file) = rfd::FileDialog::new()
            .set_title(format!("Replace {asset}"))
            .pick_file()
        else {
            return;
        };
        let bytes = match std::fs::read(&file) {
            Ok(b) => b,
            Err(e) => {
                self.log(format!("read failed: {e}"));
                return;
            }
        };
        let media = self.media.clone();
        self.start_job(
            format!(
                "replacing {asset} with {} ({})",
                file.display(),
                human(bytes.len() as u64)
            ),
            true,
            move |tx| {
                let manifest_file = media.join("manifest.bin");
                let mut manifest_bytes = match std::fs::read(&manifest_file) {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = tx.send(Msg::Log(format!("manifest read failed: {e}")));
                        return;
                    }
                };
                let m = match manifest::parse(&manifest_bytes) {
                    Ok(m) => m,
                    Err(e) => {
                        let _ = tx.send(Msg::Log(format!("manifest parse failed: {e}")));
                        return;
                    }
                };
                let Some(index) = m.find_index(&asset) else {
                    let _ = tx.send(Msg::Log(format!("asset not found: {asset}")));
                    return;
                };
                let results = upak::replace_assets_with_progress(
                    &media,
                    &m,
                    &mut manifest_bytes,
                    &[(index, bytes)],
                    &mut |label, done, total| {
                        let _ = tx.send(Msg::Progress {
                            label: label.to_string(),
                            done,
                            total,
                        });
                    },
                );
                match results {
                    Ok(results) => {
                        for r in results {
                            match r.replaced.first() {
                                Some((_, old, new)) => {
                                    let _ = tx.send(Msg::Log(format!(
                                        "{asset}: {} -> {} ({} following assets shifted{})",
                                        human(*old),
                                        human(*new),
                                        r.shifted_entries,
                                        if r.in_place { ", in place" } else { "" }
                                    )));
                                }
                                None => {
                                    let _ = tx.send(Msg::Log(format!(
                                        "{}: {} asset(s) rewritten",
                                        r.package,
                                        r.replaced.len()
                                    )));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Msg::Log(format!("replace failed: {e}")));
                    }
                }
            },
        );
    }

    pub(crate) fn restore_scripts(&mut self) {
        let target = self.media.join("scripts.obsp");
        let master = self.media.join("_modkit_work").join("scripts.stock.obsp");
        let backup = self.media.join("scripts.obsp.bak");
        let source = if master.is_file() {
            master
        } else if backup.is_file() {
            backup
        } else {
            self.log("no scripts.obsp master or backup to restore");
            return;
        };
        match std::fs::copy(&source, &target) {
            Ok(_) => {
                let _ = std::fs::remove_file(self.media.join("scripts.obsp.bak"));
                self.log(format!("scripts.obsp restored from {}", source.display()));
            }
            Err(e) => self.log(format!("restore failed: {e}")),
        }
    }

    pub(crate) fn restore_paks(&mut self) {
        let media = self.media.clone();
        self.start_job("restoring pak/manifest backups".into(), true, move |tx| {
            let mut restored = 0;
            if let Ok(entries) = std::fs::read_dir(&media) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if !name.ends_with(".bak") {
                        continue;
                    }
                    let target = path.with_file_name(name.trim_end_matches(".bak"));
                    let _ = tx.send(Msg::Log(format!(
                        "restoring {} ({}) ...",
                        target.display(),
                        human(entry.metadata().map(|m| m.len()).unwrap_or(0))
                    )));
                    match std::fs::copy(&path, &target) {
                        Ok(_) => {
                            let _ = std::fs::remove_file(&path);
                            restored += 1;
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::Log(format!(
                                "restore failed for {}: {e}",
                                target.display()
                            )));
                        }
                    }
                }
            }
            let _ = tx.send(Msg::Log(format!(
                "restored {restored} file(s) from backups"
            )));
        });
    }
}

pub(crate) fn draw(app: &mut App, ui: &mut egui::Ui, idle: bool) -> Option<Action> {
    let mut action = Action::None;
    ui.horizontal(|ui| {
        ui.label("filter:");
        ui.add(egui::TextEdit::singleline(&mut app.asset_filter).desired_width(300.0));
        if ui.add_enabled(idle, egui::Button::new("Reload")).clicked() {
            action = Action::RefreshAssets;
        }
    });
    let filter = app.asset_filter.to_lowercase();
    let rows: Vec<usize> = app
        .assets
        .iter()
        .enumerate()
        .filter(|(_, a)| filter.is_empty() || a.path.to_lowercase().contains(&filter))
        .map(|(i, _)| i)
        .take(3000)
        .collect();
    let shown = rows.len();
    egui::ScrollArea::vertical()
        .max_height(340.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("assets")
                .striped(true)
                .num_columns(3)
                .show(ui, |ui| {
                    ui.strong("asset");
                    ui.strong("size");
                    ui.strong("pak");
                    ui.end_row();
                    for i in rows {
                        let row = &app.assets[i];
                        if ui
                            .selectable_label(app.selected_asset == Some(i), &row.path)
                            .clicked()
                        {
                            app.selected_asset = Some(i);
                        }
                        ui.label(human(row.size));
                        ui.label(&row.package);
                        ui.end_row();
                    }
                });
        });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(format!("{shown} shown / {} total", app.assets.len()));
        if let Some(i) = app.selected_asset {
            if ui
                .add_enabled(idle, egui::Button::new("Extract selected"))
                .clicked()
            {
                action = Action::ExtractAsset(i);
            }
            if ui
                .add_enabled(idle, egui::Button::new("Replace selected with file..."))
                .clicked()
            {
                action = Action::ReplaceAsset(i);
            }
        }
    });
    match action {
        Action::None => None,
        a => Some(a),
    }
}
