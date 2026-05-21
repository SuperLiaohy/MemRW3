use eframe::egui::{self, RichText, Ui, ComboBox, DragValue, TextEdit};
use crate::types::{TreeNode, ExtendType};

pub fn vari_properties_ui(
    ui: &mut Ui,
    node: &mut TreeNode,
    add_config_ui: impl FnOnce(&mut Ui, &str) -> bool,
) -> bool {
    ui.heading("属性");
    ui.separator();

    // ── Initialize extend_* from basic when first shown ──
    if node.extend_name.is_none() {
        let name = if let Some(ref sn) = node.struct_name {
            format!("{}.{}", sn, node.name)
        } else {
            node.name.clone()
        };
        node.extend_name = Some(name);
    }
    if node.extend_address.is_none() {
        node.extend_address = Some(node.address);
    }
    if node.extend_type.is_none() {
        node.extend_type = Some(basic_type_to_extend(&node.basic_type));
    }
    if node.extend_size.is_none() {
        node.extend_size = Some(node.size);
    }

    let mut return_val = false;

    egui::ScrollArea::vertical()
        .id_salt("vari_properties_scroll")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            // ════════════════════════════════════════════
            // BASIC section (read-only)
            // ════════════════════════════════════════════
            ui.label(RichText::new("Basic").strong().size(13.0));
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.label(&node.name);
            });
            ui.horizontal(|ui| {
                ui.label("Address:");
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

            // ════════════════════════════════════════════
            // EXTEND section (editable)
            // ════════════════════════════════════════════
            ui.label(RichText::new("Extend").strong().size(13.0));
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("Name:");
                if let Some(mut ext_name) = node.extend_name.clone() {
                    if ui.add(TextEdit::singleline(&mut ext_name).desired_width(160.0)).changed() {
                        node.extend_name = Some(ext_name);
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label("Address:");
                if let Some(ext_addr) = node.extend_address {
                    let mut addr_hex = format!("0x{:X}", ext_addr);
                    if ui.add(TextEdit::singleline(&mut addr_hex).desired_width(160.0)).changed() {
                        let cleaned = addr_hex.trim_start_matches("0x").trim_start_matches("0X");
                        if let Ok(parsed) = u64::from_str_radix(cleaned, 16) {
                            node.extend_address = Some(parsed);
                        } else if let Ok(parsed) = cleaned.parse::<u64>() {
                            node.extend_address = Some(parsed);
                        }
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label("Size:");
                if let Some(mut ext_size) = node.extend_size {
                    if ui.add(DragValue::new(&mut ext_size).speed(1).range(1..=65536)).changed() {
                        node.extend_size = Some(ext_size);
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label("Type:");
                if let Some(ref mut ext_type) = node.extend_type {
                    let prev_type = ext_type.clone();
                    let selected = extend_type_label(ext_type);
                    ComboBox::from_id_salt("extend_type_combo")
                        .selected_text(selected)
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(ext_type, ExtendType::U8, "u8");
                            ui.selectable_value(ext_type, ExtendType::U16, "u16");
                            ui.selectable_value(ext_type, ExtendType::U32, "u32");
                            ui.selectable_value(ext_type, ExtendType::U64, "u64");
                            ui.selectable_value(ext_type, ExtendType::I8, "i8");
                            ui.selectable_value(ext_type, ExtendType::I16, "i16");
                            ui.selectable_value(ext_type, ExtendType::I32, "i32");
                            ui.selectable_value(ext_type, ExtendType::I64, "i64");
                            ui.selectable_value(ext_type, ExtendType::Float, "float");
                            ui.selectable_value(ext_type, ExtendType::Double, "double");
                            ui.selectable_value(ext_type, ExtendType::Other, "other");
                        });
                    // Auto-bind size when type changes
                    if *ext_type != prev_type && *ext_type != ExtendType::Other {
                        node.extend_size = Some(extend_type_default_size(ext_type));
                    }
                }
            });

            ui.add_space(8.0);

            // ════════════════════════════════════════════
            // ADD section
            // ════════════════════════════════════════════
            ui.label(RichText::new("Add").strong().size(13.0));
            ui.separator();

            if node.extend_type == Some(ExtendType::Other) {
                ui.label(
                    RichText::new("type 为 \"other\"，不可添加到 Chart 或 Table")
                        .color(egui::Color32::from_rgb(200, 80, 80))
                        .size(12.0),
                );
            } else {
                let ext_name = node.extend_name.as_ref().unwrap();
                let ext_address = node.extend_address.unwrap();
                let final_name = format!("{} @ 0x{:X}", ext_name, ext_address);
                return_val = add_config_ui(ui, &final_name);
            }
        });

    return_val
}

fn basic_type_to_extend(bt: &crate::types::BasicType) -> ExtendType {
    use crate::types::BasicType;
    match bt {
        BasicType::U8 => ExtendType::U8,
        BasicType::U16 => ExtendType::U16,
        BasicType::U32 => ExtendType::U32,
        BasicType::U64 => ExtendType::U64,
        BasicType::I8 => ExtendType::I8,
        BasicType::I16 => ExtendType::I16,
        BasicType::I32 => ExtendType::I32,
        BasicType::I64 => ExtendType::I64,
        BasicType::Float => ExtendType::Float,
        BasicType::Double => ExtendType::Double,
        BasicType::Pointer => ExtendType::U64,
        BasicType::Struct(_) => ExtendType::Other,
        BasicType::Other(_) => ExtendType::Other,
    }
}

fn extend_type_label(et: &ExtendType) -> &'static str {
    match et {
        ExtendType::U8 => "u8",
        ExtendType::U16 => "u16",
        ExtendType::U32 => "u32",
        ExtendType::U64 => "u64",
        ExtendType::I8 => "i8",
        ExtendType::I16 => "i16",
        ExtendType::I32 => "i32",
        ExtendType::I64 => "i64",
        ExtendType::Float => "float",
        ExtendType::Double => "double",
        ExtendType::Other => "other",
    }
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
