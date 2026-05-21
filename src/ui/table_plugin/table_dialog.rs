use eframe::egui::{self, RichText, Ui};
use crate::types::ExtendType;

pub struct TableEntry {
    pub variable_id: usize,
    pub display_name: String,
    pub current_value: String,
    pub edit_buffer: String,
}

impl TableEntry {
    pub fn new(variable_id: usize, display_name: String) -> Self {
        Self { variable_id, display_name, current_value: String::from("--"), edit_buffer: String::new() }
    }
}

pub fn table_add_config_ui(ui: &mut Ui, node_name: &str, out_display_name: &mut String) {
    if out_display_name.is_empty() { *out_display_name = node_name.to_string(); }
    ui.horizontal(|ui| { ui.label("显示名:"); ui.text_edit_singleline(out_display_name); });
}

pub fn table_entry_dialog_ui(
    ui: &mut Ui,
    entry: &mut TableEntry,
    ext_name: &str,
    ext_address: u64,
    ext_type: &ExtendType,
    ext_size: u32,
) -> Option<bool> {
    egui::Grid::new("table_entry_dialog")
        .num_columns(2).spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("显示名:");
            ui.text_edit_singleline(&mut entry.display_name);
            ui.end_row();
            ui.label("当前值:");
            ui.label(&entry.current_value);
            ui.end_row();

            ui.separator();
            ui.label(RichText::new("变量属性").strong());
            ui.end_row();

            ui.label("名称:");
            ui.label(ext_name);
            ui.end_row();

            ui.label("地址:");
            ui.label(format!("0x{:X}", ext_address));
            ui.end_row();

            ui.label("类型:");
            ui.label(extend_type_label(ext_type));
            ui.end_row();

            ui.label("大小:");
            ui.label(ext_size.to_string());
            ui.end_row();
        });

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);

    let mut result = None;
    ui.horizontal(|ui| {
        if ui.button(RichText::new("删除").color(egui::Color32::from_rgb(220, 60, 50))).clicked() {
            result = Some(true);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("确定").clicked() { result = Some(false); }
            if ui.button("取消").clicked() { result = Some(false); }
        });
    });

    result
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
