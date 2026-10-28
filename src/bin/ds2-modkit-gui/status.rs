use eframe::egui;

use crate::app::{human, App};
use crate::job::Action;

impl App {
    fn status_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(format!("media: {}", self.media.display()));
        lines.push(format!("project root: {}", self.root().display()));
        lines.push(format!(
            "mods: {} (from {})",
            self.mods.len(),
            self.mods_dir().display()
        ));
        let scripts = self.media.join("scripts.obsp");
        match std::fs::metadata(&scripts) {
            Ok(meta) => {
                let master = self.media.join("_modkit_work").join("scripts.stock.obsp");
                let state = match std::fs::metadata(&master) {
                    Ok(ms) if ms.len() == meta.len() => "stock (size matches master)",
                    Ok(_) => "MODIFIED",
                    Err(_) => {
                        if scripts.with_extension("obsp.bak").is_file() {
                            "MODIFIED (backup present)"
                        } else {
                            "unknown (no master)"
                        }
                    }
                };
                lines.push(format!("scripts.obsp: {} [{state}]", human(meta.len())));
            }
            Err(_) => lines.push("scripts.obsp: not found".into()),
        }
        let master = self.media.join("_modkit_work").join("scripts.stock.obsp");
        lines.push(format!(
            "scripts master: {}",
            if master.is_file() {
                "present"
            } else {
                "MISSING"
            }
        ));
        let mut backups: Vec<String> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.media) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".bak") {
                    backups.push(format!(
                        "{} ({})",
                        name,
                        human(entry.metadata().map(|m| m.len()).unwrap_or(0))
                    ));
                }
            }
        }
        lines.push(format!(
            "backups: {}",
            if backups.is_empty() {
                "none".to_string()
            } else {
                backups.join(", ")
            }
        ));
        lines
    }
}

pub(crate) fn draw(app: &mut App, ui: &mut egui::Ui, idle: bool) -> Option<Action> {
    let mut action = Action::None;
    for line in app.status_lines() {
        ui.monospace(line);
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
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
    match action {
        Action::None => None,
        a => Some(a),
    }
}
