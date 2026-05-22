use crate::types::{ExtendConfig, ExtendType, TreeNode, extend_type_label};
use eframe::egui::{self, ComboBox, RichText, TextEdit, Ui};

pub fn vari_properties_ui(
    ui: &mut Ui,
    node: &TreeNode,
    config: &mut ExtendConfig,
    add_config_ui: impl FnOnce(&mut Ui, &str) -> bool,
) -> bool {
    ui.heading("属性");
    ui.separator();

    let mut return_val = false;

    egui::ScrollArea::vertical()
        .id_salt("vari_properties_scroll")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            // ── BASIC section (read-only, shows DWARF raw offset) ──
            ui.label(RichText::new("Basic").strong().size(13.0));
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.label(&node.name);
            });
            ui.horizontal(|ui| {
                ui.label("Address (offset):");
                ui.label(format!("0x{:X}", node.address));
            });
            ui.horizontal(|ui| {
                ui.label("Size:");
                ui.label(node.size.to_string());
            });
            ui.horizontal(|ui| {
                ui.label("Type:");
                ui.label(&node.type_name);
            });

            ui.add_space(8.0);

            // ── EXTEND section (editable except size) ──
            ui.label(RichText::new("Extend").strong().size(13.0));
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.label(&config.name);
            });

            ui.horizontal(|ui| {
                ui.label("Address:");
                let mut addr_hex = format!("0x{:X}", config.address);
                if ui
                    .add(TextEdit::singleline(&mut addr_hex).desired_width(160.0))
                    .changed()
                {
                    let cleaned = addr_hex.trim_start_matches("0x").trim_start_matches("0X");
                    if let Ok(parsed) = u64::from_str_radix(cleaned, 16) {
                        config.address = parsed;
                    } else if let Ok(parsed) = cleaned.parse::<u64>() {
                        config.address = parsed;
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label("Size:");
                ui.label(config.size.to_string());
            });

            ui.horizontal(|ui| {
                ui.label("Type:");
                let prev_type = config.ext_type.clone();
                let selected = extend_type_label(&config.ext_type);
                ComboBox::from_id_salt("extend_type_combo")
                    .selected_text(selected)
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut config.ext_type, ExtendType::U8, "u8");
                        ui.selectable_value(&mut config.ext_type, ExtendType::U16, "u16");
                        ui.selectable_value(&mut config.ext_type, ExtendType::U32, "u32");
                        ui.selectable_value(&mut config.ext_type, ExtendType::U64, "u64");
                        ui.selectable_value(&mut config.ext_type, ExtendType::I8, "i8");
                        ui.selectable_value(&mut config.ext_type, ExtendType::I16, "i16");
                        ui.selectable_value(&mut config.ext_type, ExtendType::I32, "i32");
                        ui.selectable_value(&mut config.ext_type, ExtendType::I64, "i64");
                        ui.selectable_value(&mut config.ext_type, ExtendType::Float, "float");
                        ui.selectable_value(&mut config.ext_type, ExtendType::Double, "double");
                        ui.selectable_value(&mut config.ext_type, ExtendType::Other, "other");
                    });
                if config.ext_type != prev_type && config.ext_type != ExtendType::Other {
                    config.size = extend_type_default_size(&config.ext_type);
                }
            });

            ui.add_space(8.0);

            // ── ADD section ──
            ui.label(RichText::new("Add").strong().size(13.0));
            ui.separator();

            if config.ext_type == ExtendType::Other {
                ui.label(
                    RichText::new("type 为 \"other\"，不可添加到 Chart 或 Table")
                        .color(egui::Color32::from_rgb(200, 80, 80))
                        .size(12.0),
                );
            } else {
                let final_name = format!("{} @ 0x{:X}", config.name, config.address);
                return_val = add_config_ui(ui, &final_name);
            }
        });

    return_val
}

fn extend_type_default_size(et: &ExtendType) -> u32 {
    match et {
        ExtendType::U8 | ExtendType::I8 => 1,
        ExtendType::U16 | ExtendType::I16 => 2,
        ExtendType::U32 | ExtendType::I32 | ExtendType::Float => 4,
        ExtendType::U64 | ExtendType::I64 | ExtendType::Double => 8,
        ExtendType::Other => 0,
    }
}
