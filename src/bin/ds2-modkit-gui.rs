#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[path = "ds2-modkit-gui/app.rs"]
mod app;
#[path = "ds2-modkit-gui/assets.rs"]
mod assets;
#[path = "ds2-modkit-gui/control.rs"]
mod control;
#[path = "ds2-modkit-gui/job.rs"]
mod job;
#[path = "ds2-modkit-gui/map.rs"]
mod map;
#[path = "ds2-modkit-gui/mods.rs"]
mod mods;
#[path = "ds2-modkit-gui/render.rs"]
mod render;
#[path = "ds2-modkit-gui/save.rs"]
mod save;
#[path = "ds2-modkit-gui/status.rs"]
mod status;

use app::{App, Tab};
use eframe::egui;
use std::sync::mpsc;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<(String, mpsc::Sender<String>)>();
    std::thread::spawn(move || control::control_server(ctrl_tx));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 960.0])
            .with_title("Darksiders 2 Modkit"),
        ..Default::default()
    };
    eframe::run_native(
        "Darksiders 2 Modkit",
        options,
        Box::new(move |_cc| {
            let mut app = App::new();
            app.ctrl_rx = Some(ctrl_rx);
            if let Some(tab) = arg("--tab") {
                app.tab = match tab.to_lowercase().as_str() {
                    "map" => Tab::Map,
                    "assets" => Tab::Assets,
                    "status" => Tab::Status,
                    "save" => Tab::Save,
                    _ => Tab::Mods,
                };
            }
            if let Some(world) = arg("--world") {
                let name = world.to_lowercase();
                if let Some(i) = app
                    .map_world_names
                    .iter()
                    .position(|n| n.to_lowercase().contains(&name))
                {
                    app.map_world = Some(i);
                    app.load_map_objects(i);
                    app.select_first_object();
                }
            }
            Ok(Box::new(app))
        }),
    )
}
