use eframe::egui::{self, Color32, RichText, Ui};
use crate::model::VariablePool;
use super::legend::ChartLegend;
use std::time::Instant;

#[derive(PartialEq)]
pub enum PanelAction { None, OpenTree }

pub struct ChartPluginState {
    pub legends: Vec<ChartLegend>,
    pub editing_legend: Option<usize>,
    start_time: Instant,
    elapsed_time: f64,
}

impl Default for ChartPluginState {
    fn default() -> Self {
        Self { legends: Vec::new(), editing_legend: None, start_time: Instant::now(), elapsed_time: 0.0 }
    }
}

impl ChartPluginState {
    pub fn add_from_pool(&mut self, pool: &VariablePool, variable_id: usize) {
        if let Some(var) = pool.get(variable_id) {
            self.legends.push(ChartLegend::new(variable_id, var.tree_node.name.clone()));
        }
    }

    pub fn remove_legend(&mut self, index: usize) {
        if index < self.legends.len() {
            self.legends.remove(index);
            if self.editing_legend == Some(index) { self.editing_legend = None; }
        }
    }

    pub fn legend_ids(&self) -> Vec<usize> {
        self.legends.iter().map(|l| l.variable_id).collect()
    }
}

pub fn chart_add_config_ui(ui: &mut Ui, node_name: &str, out_curve_name: &mut String, out_color: &mut Color32) {
    if out_curve_name.is_empty() { *out_curve_name = node_name.to_string(); }
    ui.horizontal(|ui| {
        ui.label("曲线名:");
        ui.text_edit_singleline(out_curve_name);
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| { ui.label("颜色:"); color_pick(ui, out_color); });
}

fn color_pick(ui: &mut Ui, current: &mut Color32) {
    let colors = crate::ui::chart_plugin::legend::preset_colors();
    egui::Grid::new("add_color_grid").show(ui, |ui| {
        for (i, &c) in colors.iter().enumerate() {
            let fill = if *current == c { c } else { c.linear_multiply(0.5) };
            if ui.add_sized([18.0, 18.0], egui::Button::new("").fill(fill)).clicked() { *current = c; }
            if (i + 1) % 6 == 0 { ui.end_row(); }
        }
    });
}

pub fn chart_panel(ui: &mut Ui, state: &mut ChartPluginState, pool: &VariablePool, running: bool) -> PanelAction {
    let mut action = PanelAction::None;

    if running {
        state.elapsed_time = state.start_time.elapsed().as_secs_f64();
        for legend in &mut state.legends {
            if let Some(var) = pool.get(legend.variable_id) {
                let val = if var.current_value.len() >= 4 {
                    u32::from_le_bytes([var.current_value[0], var.current_value[1], var.current_value[2], var.current_value[3]]) as f64
                } else { 0.0 };
                legend.push_value(state.elapsed_time, val);
            }
        }
    }

    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("📈 实时数据图表").size(16.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(RichText::new("📋 打开变量树").size(12.0)).clicked() {
                    action = PanelAction::OpenTree;
                }
            });
        });
        ui.add_space(4.0);

        if let Some(edit_idx) = state.editing_legend {
            let legend = &mut state.legends[edit_idx];
            ui.separator();
            ui.label(RichText::new(format!("编辑: {}", legend.curve_name)).size(13.0));
            if super::line_dialog::line_dialog_ui(ui, legend) { state.remove_legend(edit_idx); }
            if ui.button("完成").clicked() { state.editing_legend = None; }
            ui.separator();
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            if state.legends.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(RichText::new("暂无监控变量").size(13.0).color(Color32::from_rgb(150, 150, 150)));
                    ui.label(RichText::new("点击右上角「打开变量树」添加变量").size(12.0).color(Color32::from_rgb(130, 130, 130)));
                    if ui.button("📋 打开变量树").clicked() { action = PanelAction::OpenTree; }
                });
            } else {
                legend_list(ui, state, pool);
                ui.add_space(12.0);
                chart_area(ui, state);
            }
        });
    });

    action
}

fn legend_list(ui: &mut Ui, state: &mut ChartPluginState, pool: &VariablePool) {
    egui::Frame::NONE
        .stroke(egui::Stroke::new(1.0, Color32::from_gray(80)))
        .corner_radius(6)
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("曲线列表").size(12.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{} 条", state.legends.len()));
                });
            });
            ui.separator();

            let mut to_remove = None;
            let mut toggle_idx = None;
            let mut edit_idx = None;

            for (i, legend) in state.legends.iter().enumerate() {
                let opacity = if legend.visible { 1.0 } else { 0.35 };
                let text_color = if legend.visible { Color32::WHITE } else { Color32::from_gray(100) };

                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 24.0),
                    egui::Sense::click(),
                );

                if ui.is_rect_visible(rect) {
                    let bg = if response.hovered() { Color32::from_gray(50) } else { Color32::from_gray(35) };
                    ui.painter().rect_filled(rect, egui::CornerRadius::same(3), bg);

                    let bar_x = rect.left() + 4.0;
                    let bar = egui::Rect::from_min_size(egui::pos2(bar_x, rect.top() + 6.0), egui::vec2(4.0, rect.height() - 12.0));
                    ui.painter().rect_filled(bar, egui::CornerRadius::same(2), legend.color.linear_multiply(opacity));

                    let label = if let Some(var) = pool.get(legend.variable_id) {
                        let val_str = format_value(&var.current_value);
                        format!("{}  = {}", legend.curve_name, val_str)
                    } else {
                        legend.curve_name.clone()
                    };
                    let text_pos = egui::pos2(bar_x + 10.0, rect.center().y - 8.0);
                    ui.painter().text(text_pos, egui::Align2::LEFT_TOP, &label, egui::FontId::proportional(12.0), text_color);

                    let del_x = rect.right() - 22.0;
                    let del_rect = egui::Rect::from_center_size(egui::pos2(del_x, rect.center().y), egui::vec2(16.0, 16.0));
                    let del_r = ui.interact(del_rect, ui.next_auto_id(), egui::Sense::click());
                    if del_r.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                    if del_r.clicked() { to_remove = Some(i); }
                    ui.painter().text(del_rect.left_top(), egui::Align2::LEFT_CENTER, "✕", egui::FontId::proportional(12.0), Color32::from_rgb(200, 60, 60));
                }

                if response.clicked() { toggle_idx = Some(i); }
                if response.double_clicked() { edit_idx = Some(i); }
            }

            if let Some(i) = to_remove { state.remove_legend(i); }
            if let Some(i) = toggle_idx { state.legends[i].visible = !state.legends[i].visible; }
            if let Some(i) = edit_idx { state.editing_legend = Some(i); }
        });
}

fn format_value(data: &[u8]) -> String {
    if data.len() >= 4 {
        let val = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        format!("0x{val:08X} ({val})")
    } else if data.is_empty() {
        "--".into()
    } else {
        format!("{data:02X?}")
    }
}

fn chart_area(ui: &mut Ui, state: &ChartPluginState) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ui.available_height().max(150.0)), egui::Sense::hover());
    if !ui.is_rect_visible(rect) { return; }
    let painter = ui.painter();
    let dark = ui.visuals().dark_mode;

    let ml = (rect.width() * 0.08).clamp(40.0, 70.0);
    let mb = (rect.height() * 0.08).clamp(22.0, 40.0);
    let pl = rect.left() + ml; let pr = rect.right() - 5.0;
    let pt = rect.top() + 5.0; let pb = rect.bottom() - mb;
    let pw = (pr - pl).max(1.0); let ph = (pb - pt).max(1.0);

    let bg = if dark { Color32::from_rgb(18, 18, 24) } else { Color32::from_rgb(250, 250, 255) };
    painter.rect_filled(egui::Rect::from_min_max(egui::pos2(pl, pt), egui::pos2(pr, pb)), egui::CornerRadius::same(0), bg);

    let grid = if dark { Color32::from_rgb(35, 35, 45) } else { Color32::from_rgb(220, 220, 225) };
    let tc = if dark { Color32::from_rgb(100, 100, 110) } else { Color32::from_rgb(150, 150, 160) };
    let txc = if dark { Color32::from_rgb(180, 180, 190) } else { Color32::from_rgb(80, 80, 90) };

    let has_data = state.legends.iter().any(|l| l.data_history.len() >= 2);
    let (x_min, x_max, y_min, y_max) = if has_data {
        let t_max = state.legends.iter().filter_map(|l| l.data_history.back().map(|p| p.0)).fold(0.0f64, f64::max);
        let t_min = state.legends.iter().filter_map(|l| l.data_history.front().map(|p| p.0)).fold(f64::MAX, f64::min);
        let x_rng = (t_max - t_min).max(1.0);
        let (ymin, ymax) = state.legends.iter().flat_map(|l| l.data_history.iter().map(|p| p.1))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), y| (lo.min(y), hi.max(y)));
        let y_pad = ((ymax - ymin).max(1.0) * 0.1).max(0.5);
        (t_min - x_rng * 0.02, t_max + x_rng * 0.02, ymin - y_pad, ymax + y_pad)
    } else {
        (0.0, 10.0, -1.0, 1.0)
    };

    let ny = 6;
    for i in 0..=ny {
        let f = i as f32 / ny as f32;
        let y = pb - f * ph;
        painter.line_segment([egui::pos2(pl, y), egui::pos2(pr, y)], (1.0, grid));
        let val = y_min + (y_max - y_min) * (1.0 - i as f64 / ny as f64);
        painter.text(egui::pos2(pl - 4.0, y), egui::Align2::RIGHT_CENTER, format_val_axis(val), egui::FontId::proportional(10.0), txc);
        painter.line_segment([egui::pos2(pl - 5.0, y), egui::pos2(pl, y)], (1.0, tc));
    }

    let nx = 5;
    for i in 0..=nx {
        let f = i as f32 / nx as f32;
        let x = pl + f * pw;
        painter.line_segment([egui::pos2(x, pt), egui::pos2(x, pb)], (1.0, grid));
        let val = x_min + (x_max - x_min) * i as f64 / nx as f64;
        painter.text(egui::pos2(x, pb + 6.0), egui::Align2::CENTER_TOP, format!("{val:.1}s"), egui::FontId::proportional(10.0), txc);
        painter.line_segment([egui::pos2(x, pb), egui::pos2(x, pb + 5.0)], (1.0, tc));
    }

    for legend in &state.legends {
        if !legend.visible || legend.data_history.len() < 2 { continue; }
        let pts: Vec<egui::Pos2> = legend.data_history.iter().map(|&(t, val)| {
            let fx = ((t - x_min) / (x_max - x_min)) as f32;
            let fy = ((val - y_min) / (y_max - y_min)) as f32;
            egui::pos2(pl + fx * pw, pb - fy * ph)
        }).collect();
        for w in pts.windows(2) { painter.line_segment([w[0], w[1]], (1.5, legend.color)); }
    }

    let border = if dark { Color32::from_rgb(60, 60, 70) } else { Color32::from_rgb(180, 180, 190) };
    painter.rect_stroke(egui::Rect::from_min_max(egui::pos2(pl, pt), egui::pos2(pr, pb)), egui::CornerRadius::same(0), (1.0, border), egui::StrokeKind::Middle);
}

fn format_val_axis(v: f64) -> String {
    if v.abs() < 10000.0 && v == (v as i64) as f64 { format!("{v:.0}") }
    else if v.abs() < 0.01 && v != 0.0 { format!("{v:.4}") }
    else { format!("{v:.2}") }
}
