mod types;
mod dwarf;
mod app;

use anyhow::{anyhow, Context, Result};
use object::read::File as ObjectFile;
use object::{Endianness, Object};
use gimli::RunTimeEndian;
use std::env;
use std::fs;

use crate::app::DwarfApp;
use crate::dwarf::load_dwarf;
use crate::dwarf::collect_cus;

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
    let app = DwarfApp::new(cus);
    eframe::run_native("DWARF Variable Tree", eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(app))))
        .map_err(|e| anyhow!("{}", e))?;
    Ok(())
}
