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
        eframe::NativeOptions::default(),
        Box::new(|cc| {
            app::setup_fonts(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow!("{}", e))?;
    Ok(())
}
