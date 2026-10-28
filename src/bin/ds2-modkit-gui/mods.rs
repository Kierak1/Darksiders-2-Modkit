use eframe::egui;

use ds2_modkit::modkit;

use crate::app::App;
use crate::job::{Action, Msg};

impl App {
    pub(crate) fn scan_mods(&mut self) {
        self.mods.clear();
        if let Ok(entries) = std::fs::read_dir(self.mods_dir()) {
            for entry in entries.flatten() {
                let manifest = entry.path().join("mod.toml");
                if manifest.is_file() {
                    self.mods.push(manifest);
                }
            }
        }
        self.mods.sort();
        self.selected_mod = if self.mods.is_empty() { None } else { Some(0) };
    }

    fn mod_label(&self, index: usize) -> String {
        let path = &self.mods[index];
        let folder = path
            .parent()
            .and_then(|d| d.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        match modkit::load_manifest(path) {
            Ok(m) => format!(
                "{folder}   ({} patch, {} add, {} import, {} asset)",
                m.patch.len(),
                m.add.len(),
                m.import.len(),
                m.asset.len()
            ),
            Err(e) => format!("{folder}   [error: {e}]"),
        }
    }

    pub(crate) fn apply_mod(&mut self, index: usize) {
        let path = self.mods[index].clone();
        let media = self.media.clone();
        self.start_job(format!("applying {}", path.display()), true, move |tx| {
            let result = modkit::apply_with_progress(&media, &path, &mut |p| {
                let msg = match p {
                    modkit::Progress::Step(label) => Msg::Log(format!("  {label}")),
                    modkit::Progress::Bytes { label, done, total } => {
                        Msg::Progress { label, done, total }
                    }
                };
                let _ = tx.send(msg);
            });
            match result {
                Ok(report) => {
                    for line in report
                        .patched
                        .iter()
                        .chain(&report.added)
                        .chain(&report.imported)
                        .chain(&report.assets)
                    {
                        let _ = tx.send(Msg::Log(format!("  {line}")));
                    }
                    let _ = tx.send(Msg::Log(format!("applied {}", report.name)));
                }
                Err(e) => {
                    let _ = tx.send(Msg::Log(format!("apply failed: {e}")));
                }
            }
        });
    }
}

pub(crate) fn draw(app: &mut App, ui: &mut egui::Ui, idle: bool) -> Option<Action> {
    let mut action = Action::None;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(idle, egui::Button::new("Rescan mods"))
            .clicked()
        {
            action = Action::RefreshAll;
        }
    });
    let labels: Vec<String> = (0..app.mods.len()).map(|i| app.mod_label(i)).collect();
    egui::ScrollArea::vertical()
        .max_height(260.0)
        .show(ui, |ui| {
            for (i, label) in labels.iter().enumerate() {
                if ui
                    .selectable_label(app.selected_mod == Some(i), label)
                    .clicked()
                {
                    app.selected_mod = Some(i);
                }
            }
        });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if let Some(i) = app.selected_mod {
            if ui
                .add_enabled(idle, egui::Button::new("Apply selected mod"))
                .clicked()
            {
                action = Action::ApplyMod(i);
            }
        }
        if ui
            .add_enabled(idle, egui::Button::new("Restore scripts.obsp"))
            .clicked()
        {
            action = Action::RestoreScripts;
        }
        if ui
            .add_enabled(idle, egui::Button::new("Restore pak/manifest backups"))
            .clicked()
        {
            action = Action::RestorePaks;
        }
    });
    ui.add_space(6.0);
    ui.label(
        "A mod is a folder in mods/ with a mod.toml: [[patch]], [[add]], [[import]], [[asset]].",
    );
    match action {
        Action::None => None,
        a => Some(a),
    }
}
