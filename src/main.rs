#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod dwarf;
mod model;
mod probe;
mod sync;
mod types;
mod ui;

use anyhow::{anyhow, Result};

use crate::app::MemRW3App;

fn main() -> Result<()> {
    let app = MemRW3App::new(types::DwarfApp::new(Vec::new()));

    eframe::run_native(
        "MemRW3 - Memory Read/Write Monitor",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([1280.0, 720.0])
                .with_min_inner_size([800.0, 500.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            app::setup_fonts(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow!("{}", e))?;
    Ok(())
}
