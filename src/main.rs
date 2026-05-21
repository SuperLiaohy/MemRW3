mod app;
mod dwarf;
mod model;
mod probe;
mod types;
mod ui;

use anyhow::{anyhow, Context, Result};
use gimli::RunTimeEndian;
use object::read::File as ObjectFile;
use object::{Endianness, Object};
use std::env;
use std::fs;

use crate::dwarf::{collect_cus, load_dwarf};
use crate::app::MemRW3App;

fn main() -> Result<()> {
    let elf_path = env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("Usage: cargo run -- <firmware.elf>"))?;

    let data = fs::read(&elf_path).with_context(|| format!("Failed to read {}", elf_path))?;
    let object = ObjectFile::parse(&*data)
        .with_context(|| format!("Failed to parse ELF file {}", elf_path))?;
    if object.format() != object::BinaryFormat::Elf {
        anyhow::bail!("Input file is not an ELF file");
    }

    let endian = match object.endianness() {
        Endianness::Little => RunTimeEndian::Little,
        Endianness::Big => RunTimeEndian::Big,
    };

    let dwarf = load_dwarf(&object, endian)?;
    let cus = collect_cus(&dwarf)?;
    let app = MemRW3App::new(types::DwarfApp::new(cus));

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
