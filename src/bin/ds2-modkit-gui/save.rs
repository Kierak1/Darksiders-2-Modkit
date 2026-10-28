use eframe::egui;
use std::path::Path;

use ds2_modkit::dsave;

use crate::app::App;
use crate::job::Msg;

/// Bulk-op labels indexed by `App::save_bulk_op`.
const BULK_OPS: [&str; 5] = [
    "set stat value",
    "scale stat %",
    "clear container",
    "duplicate item",
    "set level",
];

/// Item DB + attribute catalog + localization for the Save tab, built in the
/// background after a save loads.
pub(crate) struct SaveDb {
    pub(crate) item_db: dsave::ItemDb,
    pub(crate) catalog: dsave::AttributeCatalog,
    pub(crate) loc: dsave::LocTable,
}

impl SaveDb {
    /// Friendly attribute label (`$Item.Attributes.Damage` -> "Damage").
    pub(crate) fn label(&self, attribute: &str) -> String {
        self.loc
            .resolve(&format!("$Item.Attributes.{attribute}"))
            .map(str::to_owned)
            .unwrap_or_else(|| attribute.to_string())
    }
}

/// One pending edit submitted by the Save tab; applied after the UI closures
/// (so `self` is free for the action functions).
pub(crate) enum SaveEdit {
    Clone(usize),
    Remove(usize),
    SetField(usize, String, String),
    SetStat(usize, String, i64),
    SetRow(usize, String, u32),
    SetRowOffset(usize, String, i32),
    AddDef(String, String),
    AddMod(usize, usize, String, i64),
    DelMod(usize, usize, usize),
    AddEffect(usize, usize),
    DelEffect(usize, usize, usize),
    Move(usize, String),
    Equip(usize),
    Unequip(usize),
    SetLevel(usize, u32),
    MaxPossess(usize),
    /// One bulk operation (set-stat / scale / clear / duplicate / set-level).
    Bulk(Box<dsave::BulkOp>),
    /// Apply a `save.toml` preset to the loaded save (no write).
    Preset(String),
    /// Copy the item into another slot file on disk (`<path>`, item index).
    CopyToSlot(String, usize),
    Write(bool),
}

impl App {
    pub(crate) fn save_load(&mut self) {
        self.save_file = None;
        self.save_original = None;
        self.save_db = None;
        self.save_selected = None;
        self.save_confirm_diff.clear();
        self.save_stat_edits.clear();
        match std::fs::read(&self.save_path) {
            Ok(data) => match dsave::parse(&data) {
                Ok(save) => {
                    self.save_status = format!(
                        "loaded {}: module v{}, {} objects, {} items",
                        self.save_path,
                        save.module.version,
                        save.module.objects.len(),
                        save.items().len()
                    );
                    self.log(self.save_status.clone());
                    self.save_original = Some(save.clone());
                    self.save_file = Some(save);
                    self.save_load_db();
                    self.save_refresh_slots();
                }
                Err(e) => {
                    self.save_status = format!("parse failed: {e}");
                    self.log(self.save_status.clone());
                }
            },
            Err(e) => {
                self.save_status = format!("read failed: {e}");
                self.log(self.save_status.clone());
            }
        }
    }

    /// Build `ItemDb` + `AttributeCatalog` + `LocTable` in a background job.
    pub(crate) fn save_load_db(&mut self) {
        if self.save_db.is_some() || self.job.is_some() {
            return;
        }
        let Some(save) = self.save_file.clone() else {
            return;
        };
        let media = self.media.clone();
        self.start_job("item db".to_string(), false, move |tx| {
            let path = media.join("scripts.obsp");
            let data = match std::fs::read(&path) {
                Ok(d) => d,
                Err(e) => {
                    let _ = tx.send(Msg::Log(format!(
                        "item db: cannot read {}: {e}",
                        path.display()
                    )));
                    return;
                }
            };
            let obsp = match ds2_modkit::obsp::parse(&data) {
                Ok(o) => o,
                Err(e) => {
                    let _ = tx.send(Msg::Log(format!("item db: parse failed: {e}")));
                    return;
                }
            };
            let item_db = dsave::ItemDb::build(&obsp, &data);
            let catalog = dsave::AttributeCatalog::build(&obsp, &data, &[&save]);
            let loc = dsave::LocTable::load_from_media(&media).unwrap_or_default();
            let _ = tx.send(Msg::SaveDb(Box::new(SaveDb {
                item_db,
                catalog,
                loc,
            })));
        });
    }

    /// Diff of the edited save against the loaded state.
    pub(crate) fn save_diff_lines(&self) -> Vec<String> {
        match (&self.save_file, &self.save_original) {
            (Some(a), Some(b)) => a.diff(b),
            _ => Vec::new(),
        }
    }

    /// Refresh the slot list from the loaded save's directory (or `dir`).
    pub(crate) fn save_refresh_slots_with(&mut self, dir: Option<&str>) {
        let dir = match dir {
            Some(d) => std::path::PathBuf::from(d),
            None => match std::path::Path::new(&self.save_path).parent() {
                Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
                _ => match dsave::default_save_dir() {
                    Some(p) => p,
                    None => {
                        self.save_slots.clear();
                        return;
                    }
                },
            },
        };
        // The slot list is informational; an unreadable dir just empties it.
        self.save_slots = dsave::list_slots(&dir).unwrap_or_default();
    }

    pub(crate) fn save_refresh_slots(&mut self) {
        self.save_refresh_slots_with(None);
    }

    /// Write the save. Without `confirmed`, a first click with pending changes
    /// only shows the diff and asks for a second click; the control channel
    /// (`savewrite`) passes `confirmed = true`.
    pub(crate) fn save_write(&mut self, confirmed: bool) {
        let Some(save) = self.save_file.clone() else {
            return;
        };
        let diff = self.save_diff_lines();
        if !confirmed && !diff.is_empty() && self.save_confirm_diff != diff {
            self.save_confirm_diff = diff.clone();
            self.save_status = format!(
                "{} pending change(s) — click \"Write save\" again to confirm:\n{}",
                diff.len(),
                diff.iter().take(12).cloned().collect::<Vec<_>>().join("\n")
            );
            self.log(self.save_status.clone());
            return;
        }
        match dsave::write_file(&save, Path::new(&self.save_path)) {
            Ok(report) => {
                let mut text = format!("wrote {} bytes (re-parsed cleanly)", report.bytes);
                if let Some(b) = &report.backup {
                    text += &format!(", backup {}", b.display());
                }
                if let Some(b) = &report.timestamped_backup {
                    text += &format!(", timestamped {}", b.display());
                }
                text += &format!(", {} diff line(s)", report.diff.len());
                self.save_status = text;
                self.log(self.save_status.clone());
                for line in report.diff.iter().take(10) {
                    self.log(format!("  {line}"));
                }
                self.save_original = Some(save);
                self.save_confirm_diff.clear();
            }
            Err(e) => {
                self.save_status = format!("write refused: {e}");
                self.log(self.save_status.clone());
            }
        }
    }

    /// Apply one queued Save-tab edit and set `save_status`.
    pub(crate) fn save_apply(&mut self, edit: SaveEdit) {
        match &edit {
            SaveEdit::Write(confirmed) => {
                let confirmed = *confirmed;
                self.save_write(confirmed);
                return;
            }
            SaveEdit::Preset(path) => {
                let path = path.clone();
                self.save_apply_preset(&path);
                return;
            }
            SaveEdit::CopyToSlot(target, idx) => {
                let target = target.clone();
                let idx = *idx;
                self.save_copy_to_slot(&target, idx);
                return;
            }
            _ => {}
        }
        let Some(save) = self.save_file.as_mut() else {
            self.save_status = "no save loaded".to_string();
            return;
        };
        let db = self.save_db.as_ref();
        let result: ds2_modkit::Result<String> = match edit {
            SaveEdit::Write(_) => Err("write is handled before the item edits".into()),
            SaveEdit::Preset(_) => Err("preset is handled before the item edits".into()),
            SaveEdit::CopyToSlot(..) => Err("slot copy is handled before the item edits".into()),
            SaveEdit::Clone(idx) => {
                if let Some(r) = save.items().into_iter().find(|r| r.index == idx) {
                    match save.item(idx).cloned() {
                        Some(item) => {
                            let inserted = match r.source {
                                dsave::ItemSource::Inventory => {
                                    let list = &mut save.module.inventory[r.list_idx];
                                    list.items.insert(r.item_idx + 1, item);
                                    list.items.len()
                                }
                                dsave::ItemSource::Equipped => {
                                    let list = &mut save.module.equipped[r.list_idx];
                                    list.items.insert(r.item_idx + 1, item);
                                    list.items.len()
                                }
                                dsave::ItemSource::Record => {
                                    let list = &mut save.module.records[r.source_name_idx].lists
                                        [r.list_idx];
                                    list.items.insert(r.item_idx + 1, item);
                                    list.items.len()
                                }
                            };
                            Ok(format!("cloned item {idx} (list now {inserted} items)"))
                        }
                        None => Err(format!("no item at index {idx}").into()),
                    }
                } else {
                    Err(format!("no item at index {idx}").into())
                }
            }
            SaveEdit::Remove(idx) => match save.remove_item(idx) {
                Some(item) => Ok(format!("removed {:?}", item.name)),
                None => Err(format!("no item at index {idx}").into()),
            },
            SaveEdit::SetField(idx, field, value) => match save.item_mut(idx) {
                Some(item) => dsave::set_item_field(item, &field, &value),
                None => Err(format!("no item at index {idx}").into()),
            },
            SaveEdit::SetStat(idx, attr, value) => match db {
                Some(db) => match save.item_mut(idx) {
                    Some(item) => dsave::set_stat(item, &db.catalog, &attr, value),
                    None => Err(format!("no item at index {idx}").into()),
                },
                None => Err("item DB is still loading".into()),
            },
            SaveEdit::SetRow(idx, attr, row) => match db {
                Some(db) => match save.item_mut(idx) {
                    Some(item) => dsave::set_stat_row(item, &db.catalog, &attr, row),
                    None => Err(format!("no item at index {idx}").into()),
                },
                None => Err("item DB is still loading".into()),
            },
            SaveEdit::SetRowOffset(idx, attr, offset) => match db {
                Some(db) => match save.item_mut(idx) {
                    Some(item) => dsave::set_stat_row_offset(item, &db.catalog, &attr, offset),
                    None => Err(format!("no item at index {idx}").into()),
                },
                None => Err("item DB is still loading".into()),
            },
            SaveEdit::AddDef(def, container) => {
                let item_db = db.map(|d| &d.item_db);
                match dsave::add_item(save, item_db, &container, &def, None, None) {
                    Ok(index) => {
                        self.save_selected = Some(index);
                        Ok(format!("added {def} as item {index} in {container}"))
                    }
                    Err(e) => Err(e),
                }
            }
            SaveEdit::AddMod(idx, stat, attr, value) => match save.item_mut(idx) {
                Some(item) => {
                    dsave::possessed_add_mod(item, db.map(|d| &d.catalog), stat, &attr, Some(value))
                }
                None => Err(format!("no item at index {idx}").into()),
            },
            SaveEdit::DelMod(idx, stat, md) => match save.item_mut(idx) {
                Some(item) => dsave::possessed_remove_mod(item, stat, md),
                None => Err(format!("no item at index {idx}").into()),
            },
            SaveEdit::AddEffect(idx, stat) => match save.item_mut(idx) {
                Some(item) => dsave::possessed_add_effect(item, stat),
                None => Err(format!("no item at index {idx}").into()),
            },
            SaveEdit::DelEffect(idx, stat, effect) => match save.item_mut(idx) {
                Some(item) => dsave::possessed_remove_effect(item, stat, effect),
                None => Err(format!("no item at index {idx}").into()),
            },
            SaveEdit::Move(idx, target) => {
                let item_db = db.map(|d| &d.item_db);
                match dsave::move_item(save, idx, &target, item_db) {
                    Ok(new_index) => {
                        self.save_selected = Some(new_index);
                        Ok(format!(
                            "moved item {idx} to {target} (now item {new_index})"
                        ))
                    }
                    Err(e) => Err(e),
                }
            }
            SaveEdit::Equip(idx) => {
                let item_db = db.map(|d| &d.item_db);
                match dsave::equip_item(save, idx, item_db) {
                    Ok(new_index) => {
                        self.save_selected = Some(new_index);
                        Ok(format!("equipped item {idx} (now item {new_index})"))
                    }
                    Err(e) => Err(e),
                }
            }
            SaveEdit::Unequip(idx) => match dsave::unequip_item(save, idx) {
                Ok(new_index) => {
                    self.save_selected = Some(new_index);
                    Ok(format!("unequipped item {idx} (still item {new_index})"))
                }
                Err(e) => Err(e),
            },
            SaveEdit::SetLevel(idx, level) => match save.item_mut(idx) {
                Some(item) => dsave::possessed_set_level(item, level),
                None => Err(format!("no item at index {idx}").into()),
            },
            SaveEdit::MaxPossess(idx) => match save.item_mut(idx) {
                Some(item) => dsave::possessed_max(item),
                None => Err(format!("no item at index {idx}").into()),
            },
            SaveEdit::Bulk(op) => match db {
                Some(db) => match dsave::apply_bulk(save, Some(&db.catalog), &op) {
                    Ok(report) => {
                        let mut text = format!(
                            "bulk {}: {} changed, {} skipped",
                            op.name(),
                            report.changed,
                            report.skipped
                        );
                        for line in report.lines.iter().take(5) {
                            text += &format!("\n  {line}");
                        }
                        Ok(text)
                    }
                    Err(e) => Err(e),
                },
                None => Err("item DB is still loading".into()),
            },
        };
        match result {
            Ok(message) => {
                self.save_status = message.clone();
                self.log(message);
            }
            Err(e) => {
                self.save_status = format!("edit failed: {e}");
                self.log(self.save_status.clone());
            }
        }
    }

    /// Apply a `save.toml` preset to the loaded save (in memory only; the
    /// Write button persists it after the usual diff confirmation).
    pub(crate) fn save_apply_preset(&mut self, path: &str) {
        let Some(save) = self.save_file.as_mut() else {
            self.save_status = "no save loaded".to_string();
            return;
        };
        let preset = match dsave::load_preset(Path::new(path)) {
            Ok(p) => p,
            Err(e) => {
                self.save_status = format!("preset failed: {e}");
                self.log(self.save_status.clone());
                return;
            }
        };
        let item_db = self.save_db.as_ref().map(|d| &d.item_db);
        let catalog = self.save_db.as_ref().map(|d| &d.catalog);
        match dsave::apply_preset(save, item_db, catalog, &preset) {
            Ok(lines) => {
                for line in lines.iter().take(20) {
                    self.log(format!("preset: {line}"));
                }
                self.save_status = format!(
                    "preset applied: {} operation(s) — click \"Write save\" to persist",
                    lines.len()
                );
                self.log(self.save_status.clone());
            }
            Err(e) => {
                self.save_status = format!("preset failed: {e}");
                self.log(self.save_status.clone());
            }
        }
    }

    /// Copy the selected item into another slot file on disk, keeping the
    /// item's container. Writes the destination through `write_file` (its own
    /// backups), so the loaded file is untouched.
    pub(crate) fn save_copy_to_slot(&mut self, target: &str, idx: usize) {
        let Some(save) = &self.save_file else {
            self.save_status = "no save loaded".to_string();
            return;
        };
        let Some(item) = save.item(idx) else {
            self.save_status = format!("no item at index {idx}");
            return;
        };
        let item_name = item.name.clone();
        // The destination container: the item's current inventory list, else
        // the def's container.
        let mut container = save
            .items()
            .into_iter()
            .find(|r| r.index == idx && r.source == dsave::ItemSource::Inventory)
            .map(|r| save.module.inventory[r.list_idx].name.clone());
        if container.is_none() {
            container = self
                .save_db
                .as_ref()
                .and_then(|d| d.item_db.get(&item_name))
                .and_then(|def| def.container.clone());
        }
        let Some(container) = container else {
            self.save_status = format!("item {idx} has no container to copy into");
            return;
        };
        let dst_data = match std::fs::read(target) {
            Ok(d) => d,
            Err(e) => {
                self.save_status = format!("slot copy failed: cannot read {target}: {e}");
                return;
            }
        };
        let mut dst = match dsave::parse(&dst_data) {
            Ok(d) => d,
            Err(e) => {
                self.save_status = format!("slot copy failed: {target}: {e}");
                return;
            }
        };
        let db = self.save_db.as_ref().map(|d| &d.item_db);
        let new_index = match dsave::copy_item(save, idx, &mut dst, &container, db) {
            Ok(i) => i,
            Err(e) => {
                self.save_status = format!("slot copy failed: {e}");
                return;
            }
        };
        match dsave::write_file(&dst, Path::new(target)) {
            Ok(report) => {
                self.save_status = format!(
                    "copied item {idx} ({item_name}) to {target} as item {new_index} in {container} ({} bytes written, {} diff line(s))",
                    report.bytes,
                    report.diff.len()
                );
            }
            Err(e) => {
                self.save_status = format!("slot write refused: {e}");
            }
        }
        self.log(self.save_status.clone());
        self.save_refresh_slots();
    }

    fn save_clone(&mut self, index: usize) -> String {
        let Some(save) = &mut self.save_file else {
            return "no save loaded".to_string();
        };
        let Some(r) = save.items().into_iter().find(|r| r.index == index) else {
            return format!("no item at index {index}");
        };
        let Some(item) = save.item(index).cloned() else {
            return format!("no item at index {index}");
        };
        let inserted = match r.source {
            dsave::ItemSource::Inventory => {
                let list = &mut save.module.inventory[r.list_idx];
                list.items.insert(r.item_idx + 1, item);
                list.items.len()
            }
            dsave::ItemSource::Equipped => {
                let list = &mut save.module.equipped[r.list_idx];
                list.items.insert(r.item_idx + 1, item);
                list.items.len()
            }
            dsave::ItemSource::Record => {
                let list = &mut save.module.records[r.source_name_idx].lists[r.list_idx];
                list.items.insert(r.item_idx + 1, item);
                list.items.len()
            }
        };
        format!("cloned item {index} (list now {inserted} items)")
    }

    pub(crate) fn save_remove_item(&mut self, index: usize) {
        let Some(save) = &mut self.save_file else {
            return;
        };
        match save.remove_item(index) {
            Some(item) => {
                self.save_status = format!("removed {:?}", item.name);
                self.save_selected = None;
            }
            None => self.save_status = format!("no item at index {index}"),
        }
    }

    pub(crate) fn save_add_clone(&mut self, index: usize) {
        let message = self.save_clone(index);
        self.log(message.clone());
        self.save_status = message;
    }
}

// ---------------------------------------------------------------------------
// Save tab UI

impl App {
    pub(crate) fn draw_save_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("save:");
            ui.add(egui::TextEdit::singleline(&mut self.save_path).desired_width(380.0));
            if ui.button("Browse").clicked() {
                if let Some(f) = rfd::FileDialog::new()
                    .add_filter("Darksiders 2 save", &["dsav"])
                    .pick_file()
                {
                    self.save_path = f.display().to_string();
                    self.save_load();
                }
            }
            // Slot picker: every dssave*.dsav next to the loaded file.
            let slot_name = |s: &dsave::SlotSummary| {
                s.path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            };
            let current_slot = self
                .save_slots
                .iter()
                .find(|s| s.path.display().to_string() == self.save_path)
                .map(|s| format!("{} ({} items)", slot_name(s), s.items))
                .unwrap_or_else(|| "slot".to_string());
            let mut slot_pick: Option<String> = None;
            egui::ComboBox::from_id_salt("save-slot")
                .selected_text(current_slot)
                .show_ui(ui, |ui| {
                    for s in &self.save_slots {
                        let path = s.path.display().to_string();
                        let label = format!(
                            "{} ({} items, {} equipped)",
                            slot_name(s),
                            s.items,
                            s.equipped
                        );
                        if ui.selectable_label(self.save_path == path, label).clicked() {
                            slot_pick = Some(path);
                        }
                    }
                });
            if ui
                .button("Slots")
                .on_hover_text("refresh the dssave*.dsav slot list")
                .clicked()
            {
                self.save_refresh_slots();
            }
            if let Some(path) = slot_pick {
                self.save_path = path;
                self.save_load();
            }
            if ui
                .button("Load")
                .on_hover_text("reload; picking a slot in the combo loads it directly")
                .clicked()
            {
                self.save_load();
            }
            if ui
                .add_enabled(self.save_file.is_some(), egui::Button::new("Write save"))
                .on_hover_text(
                    "writes the loaded save; a first click with pending changes shows the diff",
                )
                .clicked()
            {
                self.save_apply(SaveEdit::Write(false));
            }
            if ui
                .add_enabled(
                    self.save_file.is_some() && self.save_original.is_some(),
                    egui::Button::new("Diff"),
                )
                .clicked()
            {
                let diff = self.save_diff_lines();
                self.save_status = if diff.is_empty() {
                    "no changes vs the loaded file".to_string()
                } else {
                    format!(
                        "{} change(s) vs the loaded file:\n{}",
                        diff.len(),
                        diff.join("\n")
                    )
                };
            }
        });
        ui.label(&self.save_status);
        ui.separator();
        let Some(save) = &mut self.save_file else {
            ui.label("Load a .dsav file to browse and edit its items.");
            return;
        };
        let list_names: Vec<String> = save
            .module
            .inventory
            .iter()
            .map(|l| l.name.clone())
            .collect();
        ui.horizontal(|ui| {
            ui.label("filter:");
            ui.add(egui::TextEdit::singleline(&mut self.save_filter).desired_width(180.0));
            ui.label("container:");
            egui::ComboBox::from_id_salt("save-filter-container")
                .selected_text(if self.save_filter_container.is_empty() {
                    "any".to_string()
                } else {
                    self.save_filter_container.clone()
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.save_filter_container, String::new(), "any");
                    for name in &list_names {
                        ui.selectable_value(&mut self.save_filter_container, name.clone(), name);
                    }
                });
            ui.label("rarity:");
            egui::ComboBox::from_id_salt("save-filter-rarity")
                .selected_text(if self.save_filter_rarity.is_empty() {
                    "any".to_string()
                } else {
                    self.save_filter_rarity.clone()
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.save_filter_rarity, String::new(), "any");
                    let mut rarities: Vec<u32> = self
                        .save_db
                        .as_ref()
                        .map(|db| db.item_db.defs().iter().filter_map(|d| d.rarity).collect())
                        .unwrap_or_default();
                    rarities.sort_unstable();
                    rarities.dedup();
                    for r in rarities {
                        ui.selectable_value(
                            &mut self.save_filter_rarity,
                            r.to_string(),
                            r.to_string(),
                        );
                    }
                });
            ui.label("sort:");
            egui::ComboBox::from_id_salt("save-sort")
                .selected_text(if self.save_sort_attr.is_empty() {
                    "none".to_string()
                } else {
                    self.save_sort_attr.clone()
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.save_sort_attr, String::new(), "none");
                    if let Some(db) = &self.save_db {
                        for a in db.catalog.attrs() {
                            ui.selectable_value(
                                &mut self.save_sort_attr,
                                a.name.clone(),
                                a.name.clone(),
                            );
                        }
                    }
                });
            ui.label(format!("{} items in file", save.items().len()));
        });
        // ---- equipped/container totals panel
        if let Some(db) = self.save_db.as_ref() {
            ui.collapsing("equipped totals", |ui| {
                ui.horizontal(|ui| {
                    ui.label("scope:");
                    egui::ComboBox::from_id_salt("save-totals-scope")
                        .selected_text(if self.save_totals_scope.is_empty() {
                            "equipped".to_string()
                        } else {
                            self.save_totals_scope.clone()
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.save_totals_scope,
                                String::new(),
                                "equipped",
                            );
                            ui.selectable_value(
                                &mut self.save_totals_scope,
                                "all".to_string(),
                                "all items",
                            );
                            for name in &list_names {
                                ui.selectable_value(
                                    &mut self.save_totals_scope,
                                    name.clone(),
                                    name,
                                );
                            }
                        });
                });
                let scope = if self.save_totals_scope.is_empty() {
                    dsave::TotalScope::Equipped
                } else if self.save_totals_scope == "all" {
                    dsave::TotalScope::All
                } else {
                    dsave::TotalScope::Container(&self.save_totals_scope)
                };
                let totals = dsave::totals(save, &db.catalog, scope);
                if totals.is_empty() {
                    ui.label("no static stat values in this scope");
                }
                for t in &totals {
                    ui.monospace(dsave::total_line(&db.label(&t.attribute), t));
                }
            });
        }
        let filter = self.save_filter.to_lowercase();
        let container_filter = self.save_filter_container.clone();
        let rarity_filter: Option<u32> = self.save_filter_rarity.parse().ok();
        let sort_attr = self.save_sort_attr.clone();
        let sort_positions: std::collections::HashMap<usize, usize> =
            match (self.save_db.as_ref(), sort_attr.is_empty()) {
                (Some(db), false) => dsave::sorted_items(save, &db.catalog, &sort_attr)
                    .iter()
                    .enumerate()
                    .map(|(p, idx)| (*idx, p))
                    .collect(),
                _ => std::collections::HashMap::new(),
            };
        let sort_values: std::collections::HashMap<usize, i64> =
            match (self.save_db.as_ref(), sort_attr.is_empty()) {
                (Some(db), false) => {
                    let want_id: Option<u32> = sort_attr.trim().parse().ok();
                    dsave::sorted_items(save, &db.catalog, &sort_attr)
                        .into_iter()
                        .filter_map(|idx| {
                            let item = save.item(idx)?;
                            let v = dsave::stat_views(item, &db.catalog).into_iter().find(|v| {
                                v.attribute.eq_ignore_ascii_case(&sort_attr)
                                    || want_id == Some(v.enum_id)
                            })?;
                            Some((idx, v.value))
                        })
                        .collect()
                }
                _ => std::collections::HashMap::new(),
            };
        // (index, display label, rarity, origin)
        let mut rows: Vec<(usize, String, Option<u32>, String)> = Vec::new();
        for r in save.items() {
            let Some(item) = save.item(r.index) else {
                continue;
            };
            let origin = save.origin(&r);
            let def = self
                .save_db
                .as_ref()
                .and_then(|db| db.item_db.get(&item.name));
            let item_rarity = def.and_then(|d| d.rarity);
            let label = match self.save_db.as_ref() {
                Some(db) => dsave::display_name(&item.name, item_rarity, &db.loc, &db.item_db),
                None => dsave::pretty_name(&item.name),
            };
            if !container_filter.is_empty() {
                let origin_match = origin.eq_ignore_ascii_case(&container_filter);
                let def_match = def
                    .and_then(|d| d.container.as_deref())
                    .is_some_and(|c| c.eq_ignore_ascii_case(&container_filter));
                if !origin_match && !def_match {
                    continue;
                }
            }
            if let Some(want) = rarity_filter {
                if item_rarity != Some(want) {
                    continue;
                }
            }
            if !filter.is_empty()
                && !item.name.to_lowercase().contains(&filter)
                && !label.to_lowercase().contains(&filter)
            {
                continue;
            }
            rows.push((r.index, label, item_rarity, origin));
        }
        if !sort_attr.is_empty() {
            rows.sort_by_key(|(idx, ..)| sort_positions.get(idx).copied().unwrap_or(usize::MAX));
        }
        let mut selected = self.save_selected;
        let mut edits: Vec<SaveEdit> = Vec::new();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                egui::ScrollArea::vertical()
                    .id_salt("save-items")
                    .max_height(420.0)
                    .max_width(520.0)
                    .show(ui, |ui| {
                        for (idx, label, item_rarity, origin) in &rows {
                            let tag = item_rarity
                                .map(|r| format!(" [R{r}]"))
                                .unwrap_or_default();
                            let stat = if sort_attr.is_empty() {
                                String::new()
                            } else {
                                sort_values
                                    .get(idx)
                                    .map(|v| format!("  {sort_attr}={v}"))
                                    .unwrap_or_default()
                            };
                            if ui
                                .selectable_label(
                                    selected == Some(*idx),
                                    format!("[{idx}] {label}{tag}  ({origin}){stat}"),
                                )
                                .clicked()
                            {
                                selected = Some(*idx);
                            }
                        }
                    });
            });
            ui.separator();
            ui.vertical(|ui| {
                let Some(idx) = selected else {
                    ui.label("select an item");
                    return;
                };
                ui.horizontal(|ui| {
                    if ui.button("Clone item").clicked() {
                        edits.push(SaveEdit::Clone(idx));
                    }
                    if ui.button("Remove item").clicked() {
                        edits.push(SaveEdit::Remove(idx));
                    }
                    if ui.button("Equip").clicked() {
                        edits.push(SaveEdit::Equip(idx));
                    }
                    if ui.button("Unequip").clicked() {
                        edits.push(SaveEdit::Unequip(idx));
                    }
                });
                // ---- copy the item into another slot file (writes that file)
                ui.horizontal(|ui| {
                    ui.label("copy to slot:");
                    let target_text = if self.save_copy_target.is_empty() {
                        "slot".to_string()
                    } else {
                        Path::new(&self.save_copy_target)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| self.save_copy_target.clone())
                    };
                    egui::ComboBox::from_id_salt("save-copy-slot")
                        .selected_text(target_text)
                        .show_ui(ui, |ui| {
                            for s in &self.save_slots {
                                let path = s.path.display().to_string();
                                if path == self.save_path {
                                    continue;
                                }
                                let label = format!(
                                    "{} ({} items)",
                                    s.path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_default(),
                                    s.items
                                );
                                ui.selectable_value(&mut self.save_copy_target, path, label);
                            }
                        });
                    if ui
                        .add_enabled(
                            !self.save_copy_target.is_empty(),
                            egui::Button::new("Copy"),
                        )
                        .on_hover_text(
                            "writes a clone into the target slot file (its own backups); the loaded file is untouched",
                        )
                        .clicked()
                    {
                        edits.push(SaveEdit::CopyToSlot(
                            self.save_copy_target.clone(),
                            idx,
                        ));
                    }
                });
                // ---- semantic stats
                match self.save_db.as_ref() {
                    Some(db) => {
                        let views = save
                            .item(idx)
                            .map(|i| dsave::stat_views(i, &db.catalog))
                            .unwrap_or_default();
                        ui.label(format!("stats ({}):", views.len()));
                        for v in &views {
                            ui.horizontal(|ui| {
                                let table = match v.table.as_deref() {
                                    Some(t) => format!(
                                        " table {t} ({})",
                                        dsave::row_display(v.row.unwrap_or(0))
                                    ),
                                    None => String::new(),
                                };
                                ui.label(format!(
                                    "{} [{}] value {}{}",
                                    db.label(&v.attribute),
                                    v.enum_id,
                                    v.value,
                                    table
                                ));
                                let key = format!("{idx}:{}", v.value_path);
                                let buf = self.save_stat_edits.entry(key).or_insert(v.value);
                                ui.add(egui::DragValue::new(buf).speed(1.0));
                                if ui.button("Set").clicked() {
                                    edits.push(SaveEdit::SetStat(idx, v.attribute.clone(), *buf));
                                }
                                if v.is_table_backed() {
                                    let rkey = format!("{idx}:{}!row", v.row_path);
                                    // rows are 1..=40; real weapons store a
                                    // signed level offset here instead.
                                    let initial = match v.row {
                                        Some(r) if r > 40 => i64::from(r as i32),
                                        Some(r) => i64::from(r),
                                        None => 1,
                                    };
                                    let rbuf = self.save_stat_edits.entry(rkey).or_insert(initial);
                                    ui.label("row/offset");
                                    ui.add(egui::DragValue::new(rbuf).speed(1.0));
                                    if ui.button("Set row").clicked() {
                                        let row = *rbuf;
                                        if (1..=40).contains(&row) {
                                            edits.push(SaveEdit::SetRow(
                                                idx,
                                                v.attribute.clone(),
                                                row as u32,
                                            ));
                                        } else {
                                            let off = row.clamp(
                                                i64::from(i32::MIN),
                                                i64::from(i32::MAX),
                                            ) as i32;
                                            edits.push(SaveEdit::SetRowOffset(
                                                idx,
                                                v.attribute.clone(),
                                                off,
                                            ));
                                        }
                                    }
                                }
                            });
                        }
                        // new upgrade entry (possessed wizard)
                        ui.horizontal(|ui| {
                            ui.label("new stat:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.save_new_attr)
                                    .hint_text("FireDamage|80")
                                    .desired_width(120.0),
                            );
                            ui.add(egui::DragValue::new(&mut self.save_new_value).speed(1.0));
                            if ui.button("Add mod").clicked() {
                                let attr = self.save_new_attr.clone();
                                let value = self.save_new_value;
                                let stat = save
                                    .item(idx)
                                    .and_then(|i| {
                                        dsave::find_stat(i, &attr).or_else(|| first_mod_stat(i))
                                    })
                                    .unwrap_or(0);
                                edits.push(SaveEdit::AddMod(idx, stat, attr, value));
                            }
                        });
                    }
                    None => {
                        ui.label(if self.job.is_some() {
                            "item DB loading..."
                        } else {
                            "item DB not loaded (scripts.obsp missing?)"
                        });
                    }
                }
                // ---- possessed wizard
                if let Some(info) = save.item(idx).and_then(dsave::possessed_info) {
                    ui.separator();
                    ui.label(format!(
                        "possessed: xp {} level {} cap {} fed {}",
                        info.cur_xp,
                        info.level,
                        info.level_cap,
                        info.fed.len()
                    ));
                    ui.horizontal(|ui| {
                        let key = format!("{idx}:!possess_level");
                        let lv = self
                            .save_stat_edits
                            .entry(key)
                            .or_insert(i64::from(info.level));
                        ui.add(
                            egui::DragValue::new(lv)
                                .range(1..=i64::from(info.level_cap))
                                .speed(1.0),
                        );
                        if ui.button("Set level").clicked() {
                            edits.push(SaveEdit::SetLevel(idx, (*lv).max(1) as u32));
                        }
                        if ui.button("Max").clicked() {
                            edits.push(SaveEdit::MaxPossess(idx));
                        }
                        if ui.button("Add effect").clicked() {
                            edits.push(SaveEdit::AddEffect(idx, 0));
                        }
                        if ui.button("Remove last effect").clicked() {
                            let last = save.item(idx).and_then(|i| {
                                i.save_body()
                                    .and_then(|s| s.nested.first())
                                    .and_then(|n| match &n.body {
                                        dsave::ItemBody::Modifier(m) => {
                                            Some(m.effects.len().saturating_sub(1))
                                        }
                                        _ => None,
                                    })
                            });
                            if let Some(effect) = last {
                                edits.push(SaveEdit::DelEffect(idx, 0, effect));
                            }
                        }
                    });
                }
                // ---- move to
                ui.horizontal(|ui| {
                    ui.label("move to:");
                    egui::ComboBox::from_id_salt("save-move")
                        .selected_text(if self.save_move_target.is_empty() {
                            "container".to_string()
                        } else {
                            self.save_move_target.clone()
                        })
                        .show_ui(ui, |ui| {
                            for name in &list_names {
                                ui.selectable_value(&mut self.save_move_target, name.clone(), name);
                            }
                        });
                    if ui.button("Move").clicked() && !self.save_move_target.is_empty() {
                        edits.push(SaveEdit::Move(idx, self.save_move_target.clone()));
                    }
                });
                // ---- raw field row
                ui.horizontal(|ui| {
                    ui.label("set field:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.save_field)
                            .hint_text("mod[0].mods[0].a[2]")
                            .desired_width(200.0),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.save_value)
                            .hint_text("value")
                            .desired_width(90.0),
                    );
                    if ui.button("Apply").clicked() {
                        edits.push(SaveEdit::SetField(
                            idx,
                            self.save_field.clone(),
                            self.save_value.clone(),
                        ));
                    }
                });
                // ---- bulk operations (all items in a scope)
                ui.collapsing("bulk ops", |ui| {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("save-bulk-op")
                            .selected_text(
                                BULK_OPS[self.save_bulk_op.min(BULK_OPS.len() - 1)],
                            )
                            .show_ui(ui, |ui| {
                                for (i, name) in BULK_OPS.iter().enumerate() {
                                    ui.selectable_value(&mut self.save_bulk_op, i, *name);
                                }
                            });
                        match self.save_bulk_op {
                            2 => {
                                egui::ComboBox::from_id_salt("save-bulk-container")
                                    .selected_text(if self.save_move_target.is_empty() {
                                        "container".to_string()
                                    } else {
                                        self.save_move_target.clone()
                                    })
                                    .show_ui(ui, |ui| {
                                        for name in &list_names {
                                            ui.selectable_value(
                                                &mut self.save_move_target,
                                                name.clone(),
                                                name,
                                            );
                                        }
                                    });
                            }
                            3 => {
                                let max =
                                    save.items().len().saturating_sub(1) as u64;
                                ui.label("item");
                                ui.add(
                                    egui::DragValue::new(&mut self.save_bulk_item)
                                        .range(0..=max as usize)
                                        .speed(1.0),
                                );
                            }
                            4 => {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.save_bulk_attr)
                                        .hint_text("Damage|7")
                                        .desired_width(110.0),
                                );
                                ui.add(
                                    egui::DragValue::new(&mut self.save_bulk_level)
                                        .range(0..=u32::MAX as i64)
                                        .speed(1.0),
                                );
                                ui.checkbox(&mut self.save_bulk_possessed, "possessed level");
                            }
                            _ => {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.save_bulk_attr)
                                        .hint_text("Damage|7")
                                        .desired_width(110.0),
                                );
                                if self.save_bulk_op == 0 {
                                    ui.add(
                                        egui::DragValue::new(&mut self.save_bulk_value)
                                            .speed(1.0),
                                    );
                                } else {
                                    ui.add(
                                        egui::DragValue::new(&mut self.save_bulk_percent)
                                            .speed(5.0),
                                    );
                                    ui.label("%");
                                }
                            }
                        }
                    });
                    if self.save_bulk_op != 2 && self.save_bulk_op != 3 {
                        ui.horizontal(|ui| {
                            ui.label("scope:");
                            egui::ComboBox::from_id_salt("save-bulk-scope")
                                .selected_text(self.save_bulk_scope.clone())
                                .show_ui(ui, |ui| {
                                    for s in ["all", "equipped", "possessed"] {
                                        ui.selectable_value(
                                            &mut self.save_bulk_scope,
                                            s.to_string(),
                                            s,
                                        );
                                    }
                                    for name in &list_names {
                                        ui.selectable_value(
                                            &mut self.save_bulk_scope,
                                            name.clone(),
                                            name,
                                        );
                                    }
                                });
                        });
                    }
                    if ui
                        .button(if self.save_bulk_armed {
                            "Confirm bulk op"
                        } else {
                            "Apply bulk op"
                        })
                        .clicked()
                    {
                        if self.save_bulk_armed {
                            let scope = dsave::BulkScope::parse(&self.save_bulk_scope);
                            let op = match self.save_bulk_op {
                                0 => dsave::BulkOp::SetStat {
                                    attribute: self.save_bulk_attr.clone(),
                                    value: self.save_bulk_value,
                                    scope,
                                },
                                1 => dsave::BulkOp::Scale {
                                    attribute: self.save_bulk_attr.clone(),
                                    percent: self.save_bulk_percent,
                                    scope,
                                },
                                2 => dsave::BulkOp::Clear {
                                    container: self.save_move_target.clone(),
                                },
                                3 => dsave::BulkOp::Duplicate {
                                    item_index: self.save_bulk_item,
                                },
                                _ => dsave::BulkOp::SetLevel {
                                    level: self.save_bulk_level.clamp(0, u32::MAX as i64)
                                        as u32,
                                    possessed: self.save_bulk_possessed,
                                    scope,
                                },
                            };
                            edits.push(SaveEdit::Bulk(Box::new(op)));
                            self.save_bulk_armed = false;
                        } else {
                            self.save_bulk_armed = true;
                            self.save_status =
                                "bulk op armed — click Confirm to apply".to_string();
                        }
                    }
                });
                // ---- preset file
                ui.collapsing("preset (save.toml)", |ui| {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.save_preset_path)
                                .hint_text("save.toml")
                                .desired_width(200.0),
                        );
                        if ui.button("Browse").clicked() {
                            if let Some(f) = rfd::FileDialog::new()
                                .add_filter("preset", &["toml"])
                                .pick_file()
                            {
                                self.save_preset_path = f.display().to_string();
                            }
                        }
                        if ui
                            .add_enabled(
                                !self.save_preset_path.is_empty(),
                                egui::Button::new("Apply preset"),
                            )
                            .clicked()
                        {
                            edits.push(SaveEdit::Preset(self.save_preset_path.clone()));
                        }
                    });
                    ui.label(
                        "all-or-nothing; the Write button shows the diff before saving",
                    );
                });
                egui::ScrollArea::vertical()
                    .id_salt("save-details")
                    .max_height(300.0)
                    .show(ui, |ui| {
                        if let Some(item) = save.item(idx) {
                            for line in dsave::item_lines(item, 0) {
                                ui.monospace(line);
                            }
                        }
                    });
            });
        });
        self.save_selected = selected;
        ui.separator();
        self.draw_save_browser(ui, &mut edits, &list_names);
        for edit in edits {
            self.save_apply(edit);
        }
    }

    /// Item-def browser (search + container/rarity filters + Add).
    fn draw_save_browser(
        &mut self,
        ui: &mut egui::Ui,
        edits: &mut Vec<SaveEdit>,
        list_names: &[String],
    ) {
        let Some(db) = self.save_db.as_ref() else {
            return;
        };
        ui.collapsing("item browser", |ui| {
            ui.horizontal(|ui| {
                ui.label("search:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.save_browser_filter).desired_width(160.0),
                );
                ui.label("container:");
                egui::ComboBox::from_id_salt("save-browse-container")
                    .selected_text(if self.save_browser_container.is_empty() {
                        "any".to_string()
                    } else {
                        self.save_browser_container.clone()
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.save_browser_container, String::new(), "any");
                        let mut containers: Vec<&str> = db
                            .item_db
                            .defs()
                            .iter()
                            .filter_map(|d| d.container.as_deref())
                            .collect();
                        containers.sort_unstable();
                        containers.dedup();
                        for c in containers {
                            ui.selectable_value(&mut self.save_browser_container, c.to_string(), c);
                        }
                    });
                ui.label("rarity:");
                egui::ComboBox::from_id_salt("save-browse-rarity")
                    .selected_text(if self.save_browser_rarity.is_empty() {
                        "any".to_string()
                    } else {
                        self.save_browser_rarity.clone()
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.save_browser_rarity, String::new(), "any");
                        let mut rarities: Vec<u32> =
                            db.item_db.defs().iter().filter_map(|d| d.rarity).collect();
                        rarities.sort_unstable();
                        rarities.dedup();
                        for r in rarities {
                            ui.selectable_value(
                                &mut self.save_browser_rarity,
                                r.to_string(),
                                r.to_string(),
                            );
                        }
                    });
                ui.label("into:");
                egui::ComboBox::from_id_salt("save-browse-target")
                    .selected_text(if self.save_browser_target.is_empty() {
                        "container".to_string()
                    } else {
                        self.save_browser_target.clone()
                    })
                    .show_ui(ui, |ui| {
                        for name in list_names {
                            ui.selectable_value(&mut self.save_browser_target, name.clone(), name);
                        }
                    });
            });
            let needle = self.save_browser_filter.to_lowercase();
            let container = self.save_browser_container.clone();
            let rarity = self.save_browser_rarity.parse::<u32>().ok();
            let matches: Vec<(&dsave::ItemDef, String)> = db
                .item_db
                .defs()
                .iter()
                .filter_map(|d| {
                    if !container.is_empty()
                        && !d
                            .container
                            .as_deref()
                            .is_some_and(|c| c.eq_ignore_ascii_case(&container))
                    {
                        return None;
                    }
                    if let Some(r) = rarity {
                        if d.rarity != Some(r) {
                            return None;
                        }
                    }
                    let name = dsave::display_name(&d.name, None, &db.loc, &db.item_db);
                    if !needle.is_empty()
                        && !d.name.to_lowercase().contains(&needle)
                        && !name.to_lowercase().contains(&needle)
                    {
                        return None;
                    }
                    Some((d, name))
                })
                .collect();
            let per_page = 50usize;
            let pages = matches.len().div_ceil(per_page).max(1);
            if self.save_browser_page >= pages {
                self.save_browser_page = pages - 1;
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.save_browser_page > 0, egui::Button::new("<"))
                    .clicked()
                {
                    self.save_browser_page -= 1;
                }
                ui.label(format!(
                    "{} defs, page {}/{}",
                    matches.len(),
                    self.save_browser_page + 1,
                    pages
                ));
                if ui
                    .add_enabled(self.save_browser_page + 1 < pages, egui::Button::new(">"))
                    .clicked()
                {
                    self.save_browser_page += 1;
                }
            });
            egui::ScrollArea::vertical()
                .id_salt("save-browser")
                .max_height(240.0)
                .show(ui, |ui| {
                    for (def, name) in matches
                        .iter()
                        .skip(self.save_browser_page * per_page)
                        .take(per_page)
                    {
                        ui.horizontal(|ui| {
                            if ui.button("Add").clicked() && !self.save_browser_target.is_empty() {
                                edits.push(SaveEdit::AddDef(
                                    def.name.clone(),
                                    self.save_browser_target.clone(),
                                ));
                            }
                            ui.monospace(format!(
                                "{:<40} {:<14} {:<3} {}",
                                def.name,
                                def.container.as_deref().unwrap_or("-"),
                                def.rarity
                                    .map(|r| r.to_string())
                                    .unwrap_or_else(|| "-".into()),
                                name
                            ));
                        });
                    }
                });
        });
    }
}

pub(crate) fn first_mod_stat(item: &dsave::Item) -> Option<usize> {
    let body = item.save_body()?;
    body.nested
        .iter()
        .position(|n| matches!(n.body, dsave::ItemBody::Modifier(_)))
}
