use eframe::egui::{self, Color32, RichText, Ui};
use crate::model::VariablePool;
use crate::types::ExtendType;
use super::legend::ChartLegend;
use std::time::Instant;

#[derive(PartialEq)]
pub enum PanelAction { None, OpenTree }

pub struct ChartPluginState {
    pub legends: Vec<ChartLegend>,
    pub editing_legend: Option<usize>,
    pub show_line_dialog: bool,
    start_time: Instant,
    elapsed_time: f64,
}

impl Default for ChartPluginState {
    fn default() -> Self {
        Self { legends: Vec::new(), editing_legend: None, show_line_dialog: false,
            start_time: Instant::now(), elapsed_time: 0.0 }
    }
}

impl ChartPluginState {
    pub fn add_from_pool(&mut self, pool: &VariablePool, variable_id: usize) {
        if let Some(var) = pool.get(variable_id) {
            self.legends.push(ChartLegend::new(variable_id, var.tree_node.name.clone()));
        }
    }
    pub fn remove_legend(&mut self, index: usize) {
        if index < self.legends.len() { self.legends.remove(index); if self.editing_legend == Some(index) { self.editing_legend = None; } }
    }
    pub fn legend_ids(&self) -> Vec<usize> { self.legends.iter().map(|l| l.variable_id).collect() }
}

pub fn chart_add_config_ui(ui: &mut Ui, node_name: &str, out_name: &mut String, out_color: &mut Color32) {
    if out_name.is_empty() { *out_name = node_name.to_string(); }
    ui.horizontal(|ui| { ui.label("曲线名:"); ui.text_edit_singleline(out_name); });
    ui.add_space(4.0);
    ui.horizontal(|ui| { ui.label("颜色:"); color_pick(ui, out_color); });
}

fn color_pick(ui: &mut Ui, current: &mut Color32) {
    let colors = super::legend::preset_colors();
    egui::Grid::new("add_color_grid").show(ui, |ui| {
        for (i, &c) in colors.iter().enumerate() {
            let fill = if *current == c { c } else { c.linear_multiply(0.5) };
            if ui.add_sized([18.0, 18.0], egui::Button::new("").fill(fill)).clicked() { *current = c; }
            if (i+1)%6==0 { ui.end_row(); }
        }
    });
}

pub fn chart_panel(ui: &mut Ui, state: &mut ChartPluginState, pool: &VariablePool, running: bool) -> PanelAction {
    let mut action = PanelAction::None;

    if running {
        state.elapsed_time = state.start_time.elapsed().as_secs_f64();
        for legend in &mut state.legends {
            if let Some(var) = pool.get(legend.variable_id) {
                let val = decode_value_f64(&var.current_value, &var.tree_node.extend_type);
                legend.push_value(state.elapsed_time, val);
            }
        }
    }

    ui.vertical(|ui| {
        let dialog_is_open = state.show_line_dialog;

        ui.add_enabled_ui(!dialog_is_open, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new("📈 实时数据图表").size(16.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new("📋 打开变量树").size(12.0)).clicked() { action = PanelAction::OpenTree; }
                });
            });
            ui.add_space(2.0);

            if state.legends.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(RichText::new("暂无监控变量").size(13.0).color(Color32::from_rgb(150,150,150)));
                    if ui.button("📋 打开变量树").clicked() { action = PanelAction::OpenTree; }
                });
            } else {
                render_chart(ui, state);
            }
        });

        // Dialog (rendered at top layer, always interactive)
        if state.show_line_dialog {
            if let Some(edit_idx) = state.editing_legend {
                let mut remove = false; let mut done = false;
                let legend = &mut state.legends[edit_idx];
                let ext_info = pool.get(legend.variable_id).map(|v| {
                    (
                        v.tree_node.extend_name.as_deref(),
                        v.tree_node.extend_address,
                        v.tree_node.extend_type.as_ref(),
                        v.tree_node.extend_size,
                    )
                });
                egui::Window::new(format!("曲线属性 - {}", legend.curve_name))
                    .collapsible(false).resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ui.ctx(), |ui| {
                        let (ext_name, ext_addr, ext_type, ext_size) = ext_info.unwrap_or((None, None, None, None));
                        if let Some(r) = super::line_dialog::line_dialog_ui(ui, legend, ext_name, ext_addr, ext_type, ext_size) {
                            remove = r; done = true;
                        }
                    });
                if done {
                    state.show_line_dialog = false;
                    if remove { state.remove_legend(edit_idx); }
                }
            } else { state.show_line_dialog = false; }
        }
    });
    action
}

fn render_chart(ui: &mut Ui, state: &mut ChartPluginState) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ui.available_height().max(150.0)), egui::Sense::hover());
    if !ui.is_rect_visible(rect) { return; }
    let painter = ui.painter();
    let dark = ui.visuals().dark_mode;

    let ml = (rect.width()*0.08).clamp(40.0,70.0);
    let mb = (rect.height()*0.08).clamp(22.0,40.0);
    let pl = rect.left()+ml; let pr = rect.right()-5.0;
    let pt = rect.top()+5.0; let pb = rect.bottom()-mb;
    let pw = (pr-pl).max(1.0); let ph = (pb-pt).max(1.0);

    let bg = if dark { Color32::from_rgb(18,18,24) } else { Color32::from_rgb(250,250,255) };
    painter.rect_filled(egui::Rect::from_min_max(egui::pos2(pl,pt), egui::pos2(pr,pb)), egui::CornerRadius::same(0), bg);

    let grid = if dark { Color32::from_rgb(35,35,45) } else { Color32::from_rgb(220,220,225) };
    let tc = if dark { Color32::from_rgb(100,100,110) } else { Color32::from_rgb(150,150,160) };
    let txc = if dark { Color32::from_rgb(180,180,190) } else { Color32::from_rgb(80,80,90) };

    let has_data = state.legends.iter().any(|l| l.data_history.len() >= 2);
    let (x_min, x_max, y_min, y_max) = if has_data {
        let t_max = state.legends.iter().filter_map(|l| l.data_history.back().map(|p| p.0)).fold(0.0f64, f64::max);
        let t_min = state.legends.iter().filter_map(|l| l.data_history.front().map(|p| p.0)).fold(f64::MAX, f64::min);
        let xr = (t_max-t_min).max(1.0);
        let (ymin,ymax) = state.legends.iter().flat_map(|l| l.data_history.iter().map(|p| p.1)).fold((f64::INFINITY,f64::NEG_INFINITY),|(lo,hi),y|(lo.min(y),hi.max(y)));
        let yp = ((ymax-ymin).max(1.0)*0.1).max(0.5);
        (t_min-xr*0.02, t_max+xr*0.02, ymin-yp, ymax+yp)
    } else { (0.0, 10.0, -1.0, 1.0) };

    for i in 0..=6 {
        let f = i as f32/6.0; let y = pb-f*ph;
        painter.line_segment([egui::pos2(pl,y), egui::pos2(pr,y)], (1.0, grid));
        let val = y_min+(y_max-y_min)*(1.0-i as f64/6.0);
        painter.text(egui::pos2(pl-4.0,y), egui::Align2::RIGHT_CENTER, format_axis(val), egui::FontId::proportional(10.0), txc);
        painter.line_segment([egui::pos2(pl-5.0,y), egui::pos2(pl,y)], (1.0, tc));
    }
    for i in 0..=5 {
        let f = i as f32/5.0; let x = pl+f*pw;
        painter.line_segment([egui::pos2(x,pt), egui::pos2(x,pb)], (1.0, grid));
        let val = x_min+(x_max-x_min)*i as f64/5.0;
        painter.text(egui::pos2(x,pb+6.0), egui::Align2::CENTER_TOP, format!("{val:.1}s"), egui::FontId::proportional(10.0), txc);
        painter.line_segment([egui::pos2(x,pb), egui::pos2(x,pb+5.0)], (1.0, tc));
    }

    for legend in &state.legends {
        if !legend.visible || legend.data_history.len() < 2 { continue; }
        let pts: Vec<egui::Pos2> = legend.data_history.iter().map(|&(t,val)| {
            let fx = ((t-x_min)/(x_max-x_min)) as f32;
            let fy = ((val-y_min)/(y_max-y_min)) as f32;
            egui::pos2(pl+fx*pw, pb-fy*ph)
        }).collect();
        for w in pts.windows(2) { painter.line_segment([w[0],w[1]], (1.5, legend.color)); }
    }

    let border = if dark { Color32::from_rgb(60,60,70) } else { Color32::from_rgb(180,180,190) };
    painter.rect_stroke(egui::Rect::from_min_max(egui::pos2(pl,pt), egui::pos2(pr,pb)), egui::CornerRadius::same(0), (1.0,border), egui::StrokeKind::Middle);

    legend_overlay(ui, state, egui::pos2(pr, pt));
}

fn legend_overlay(ui: &mut Ui, state: &mut ChartPluginState, anchor: egui::Pos2) {
    let mut toggle = None; let mut edit = None;
    let mut y = anchor.y + 4.0;
    let x = (anchor.x - 160.0).max(0.0);
    for (i, legend) in state.legends.iter().enumerate() {
        let op = if legend.visible { 1.0 } else { 0.35 };
        let tc = if legend.visible { Color32::WHITE } else { Color32::from_gray(100) };
        let lv = legend.data_history.back().map(|&(_, v)| format_axis(v)).unwrap_or_else(|| "--".into());
        let text = format!("{} = {}", legend.curve_name, lv);
        let g = ui.painter().layout_no_wrap(text, egui::FontId::proportional(11.0), tc);
        let w = g.rect.width() + 24.0; let h = g.rect.height() + 6.0;
        let r = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
        let resp = ui.interact(r, egui::Id::new(("chart_legend", i)), egui::Sense::click());
        let bg = if resp.hovered() { Color32::from_rgba_premultiplied(40,40,50,200) } else { Color32::from_rgba_premultiplied(20,20,30,180) };
        ui.painter().rect_filled(r, egui::CornerRadius::same(3), bg);
        let bar = egui::Rect::from_min_size(egui::pos2(r.left()+4.0, r.top()+3.0), egui::vec2(8.0, h-6.0));
        ui.painter().rect_filled(bar, egui::CornerRadius::same(2), legend.color.linear_multiply(op));
        ui.painter().galley(egui::pos2(r.left()+14.0, r.top()+3.0), g, tc);
        if resp.clicked() { toggle = Some(i); }
        if resp.double_clicked() { edit = Some(i); }
        y += h + 3.0;
    }
    if let Some(i) = toggle { state.legends[i].visible = !state.legends[i].visible; }
    if let Some(i) = edit { state.editing_legend = Some(i); state.show_line_dialog = true; }
}

fn decode_value_f64(data: &[u8], ext_type: &Option<ExtendType>) -> f64 {
    use ExtendType::*;
    if data.is_empty() { return 0.0; }
    match ext_type {
        Some(U8) | None => *data.first().unwrap_or(&0) as f64,
        Some(I8) => *data.first().unwrap_or(&0) as i8 as f64,
        Some(U16) => if data.len() >= 2 { u16::from_le_bytes([data[0], data[1]]) as f64 } else { 0.0 },
        Some(I16) => if data.len() >= 2 { i16::from_le_bytes([data[0], data[1]]) as f64 } else { 0.0 },
        Some(U32) => if data.len() >= 4 { u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64 } else { 0.0 },
        Some(I32) => if data.len() >= 4 { i32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64 } else { 0.0 },
        Some(U64) => if data.len() >= 8 { u64::from_le_bytes([data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]]) as f64 } else { 0.0 },
        Some(I64) => if data.len() >= 8 { i64::from_le_bytes([data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]]) as f64 } else { 0.0 },
        Some(Float) => if data.len() >= 4 { f32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64 } else { 0.0 },
        Some(Double) => if data.len() >= 8 { f64::from_le_bytes([data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]]) } else { 0.0 },
        Some(Other) => 0.0,
    }
}

fn format_axis(v: f64) -> String {
    if v.abs() < 10000.0 && v == (v as i64) as f64 { format!("{v:.0}") }
    else if v.abs() < 0.01 && v != 0.0 { format!("{v:.4}") }
    else { format!("{v:.2}") }
}
