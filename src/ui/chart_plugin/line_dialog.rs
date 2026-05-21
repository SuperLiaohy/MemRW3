use eframe::egui::{self, Color32, RichText, Ui};
use super::legend::{preset_colors, ChartLegend};

pub fn line_dialog_ui(ui: &mut Ui, legend: &mut ChartLegend) -> Option<bool> {
    egui::Grid::new("line_dialog_grid")
        .num_columns(2).spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("曲线名称:");
            ui.text_edit_singleline(&mut legend.curve_name);
            ui.end_row();
            ui.label("颜色:");
            color_picker(ui, &mut legend.color);
            ui.end_row();
            ui.label("缓冲区:");
            ui.add(egui::Slider::new(&mut legend.buffer_size, 1000..=50000).step_by(1000.0).text("points"));
            ui.end_row();
            ui.label("可见:");
            ui.checkbox(&mut legend.visible, "");
            ui.end_row();
        });
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);
    let mut result = None;
    ui.horizontal(|ui| {
        if ui.button(RichText::new("删除").color(Color32::from_rgb(220,60,50))).clicked() { result = Some(true); }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("确定").clicked() { result = Some(false); }
            if ui.button("取消").clicked() { result = Some(false); }
        });
    });
    result
}

fn color_picker(ui: &mut Ui, current: &mut Color32) {
    egui::Grid::new("color_grid").show(ui, |ui| {
        for (i, &c) in preset_colors().iter().enumerate() {
            let bg = if *current == c { c } else { c.linear_multiply(0.6) };
            if ui.add_sized([20.0, 20.0], egui::Button::new("").fill(bg)).clicked() { *current = c; }
            if (i+1)%6==0 { ui.end_row(); }
        }
    });
}
