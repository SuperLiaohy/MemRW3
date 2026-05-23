use eframe::egui::{self, Color32, RichText, Ui};
use crate::types::ExtendType;
use super::legend::preset_colors;

pub enum DialogAction {
    Confirm,
    Cancel,
    Delete,
}

pub fn line_dialog_ui(
    ui: &mut Ui,
    curve_name: &mut String,
    color: &mut Color32,
    buffer_size: &mut usize,
    visible: &mut bool,
    ext_name: &str,
    ext_address: u64,
    ext_type: &ExtendType,
    ext_size: u32,
    running: bool,
    logging_active: bool,
) -> Option<DialogAction> {
    egui::Grid::new("line_dialog_grid")
        .num_columns(2).spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("曲线名称:");
            ui.text_edit_singleline(curve_name);
            ui.end_row();
            ui.label("颜色:");
            color_pick(ui, color);
            ui.end_row();
            ui.label("缓冲区:");
            ui.add_enabled_ui(!running, |ui| {
                ui.add(egui::Slider::new(buffer_size, 1000..=50000).step_by(1000.0).text("points"));
            });
            ui.end_row();
            ui.label("可见:");
            ui.checkbox(visible, "");
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
        ui.add_enabled_ui(!logging_active, |ui| {
            if ui.button(RichText::new("删除").color(Color32::from_rgb(220,60,50))).clicked() {
                result = Some(DialogAction::Delete);
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("确定").clicked() {
                result = Some(DialogAction::Confirm);
            }
            if ui.button("取消").clicked() {
                result = Some(DialogAction::Cancel);
            }
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

fn color_pick(ui: &mut Ui, current: &mut Color32) {
    ui.vertical(|ui| {
        ui.color_edit_button_srgba(current);
        let colors = preset_colors();
        egui::Grid::new("dialog_color_grid").show(ui, |ui| {
            for (i, &c) in colors.iter().enumerate() {
                let fill = if *current == c { c } else { c.linear_multiply(0.5) };
                if ui.add_sized([18.0, 18.0], egui::Button::new("").fill(fill)).clicked() {
                    *current = c;
                }
                if (i + 1) % 6 == 0 {
                    ui.end_row();
                }
            }
        });
    });
}
