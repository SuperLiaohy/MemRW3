use super::legend::ChartLegend;
use crate::model::VariablePool;
use crate::types::ExtendType;
use eframe::egui::{self, Color32, RichText, Ui};
use egui_plot::{Line, Plot, PlotBounds, PlotPoints};
use std::collections::HashMap;
use std::io::{BufWriter, Write};
use std::time::Instant;

#[derive(PartialEq)]
pub enum PanelAction {
    None,
    OpenTree,
}

#[derive(Clone, PartialEq)]
pub enum YAxisMode {
    Auto,
    Fixed { min: f64, max: f64 },
    None,
}

#[derive(Clone, PartialEq)]
pub enum XAxisMode {
    Auto,
    Fixed(f64),
}

impl XAxisMode {
    fn window(&self, xr: f64) -> f64 {
        match self {
            XAxisMode::Auto => xr.max(6.0),
            XAxisMode::Fixed(w) => *w,
        }
    }
}

pub struct ChartPluginState {
    pub legends: Vec<ChartLegend>,
    pub editing_legend: Option<usize>,
    pub show_line_dialog: bool,
    pub auto_scroll: bool,
    pub x_mode: XAxisMode,
    pub y_mode: YAxisMode,
    pub acq_hz: f64,
    pub removed_var_ids: Vec<usize>,
    pub reset_timer: bool,
    pub edit_curve_name: String,
    pub edit_color: Color32,
    pub edit_buffer_size: usize,
    pub edit_visible: bool,
    pub log_file: Option<std::path::PathBuf>,
    log_writer: Option<BufWriter<std::fs::File>>,
    logging_active: bool,
    pub log_started: bool,
    pub log_stopped: bool,
    pub cursor_txt: String,
    acq_frame_count: u64,
    acq_last_reset: Instant,
    was_running: bool,
}

impl Default for ChartPluginState {
    fn default() -> Self {
        Self {
            legends: Vec::new(),
            editing_legend: None,
            show_line_dialog: false,
            auto_scroll: true,
            x_mode: XAxisMode::Auto,
            y_mode: YAxisMode::Auto,
            acq_hz: 0.0,
            removed_var_ids: Vec::new(),
            reset_timer: false,
            edit_curve_name: String::new(),
            edit_color: Color32::WHITE,
            edit_buffer_size: 10000,
            edit_visible: true,
            log_file: None,
            log_writer: None,
            logging_active: false,
            log_started: false,
            log_stopped: false,
            cursor_txt: String::new(),
            acq_frame_count: 0,
            acq_last_reset: Instant::now(),
            was_running: false,
        }
    }
}

impl ChartPluginState {
    pub fn add_legend(&mut self, variable_id: usize, pool: &VariablePool, curve_name: String, color: Color32) {
        if let Some(var) = pool.get(variable_id) {
            let mut legend = ChartLegend::new(variable_id, var.name.clone());
            legend.curve_name = if curve_name.is_empty() { var.name.clone() } else { curve_name };
            legend.color = color;
            self.legends.push(legend);
        }
    }
    pub fn remove_legend(&mut self, index: usize) {
        if index < self.legends.len() {
            let var_id = self.legends[index].variable_id;
            self.removed_var_ids.push(var_id);
            self.legends.remove(index);
            if self.editing_legend == Some(index) {
                self.editing_legend = None;
            }
        }
    }
    pub fn legend_ids(&self) -> Vec<usize> {
        self.legends.iter().map(|l| l.variable_id).collect()
    }
}

pub fn chart_add_config_ui(
    ui: &mut Ui,
    node_name: &str,
    out_name: &mut String,
    out_color: &mut Color32,
) {
    if out_name.is_empty() {
        *out_name = node_name.to_string();
    }
    ui.horizontal(|ui| {
        ui.label("曲线名:");
        ui.text_edit_singleline(out_name);
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label("颜色:");
        color_pick(ui, out_color);
    });
}

fn color_pick(ui: &mut Ui, current: &mut Color32) {
    ui.vertical(|ui| {
        ui.color_edit_button_srgba(current);
        let colors = super::legend::preset_colors();
        egui::Grid::new("add_color_grid").show(ui, |ui| {
            for (i, &c) in colors.iter().enumerate() {
                let fill = if *current == c {
                    c
                } else {
                    c.linear_multiply(0.5)
                };
                if ui
                    .add_sized([18.0, 18.0], egui::Button::new("").fill(fill))
                    .clicked()
                {
                    *current = c;
                }
                if (i + 1) % 6 == 0 {
                    ui.end_row();
                }
            }
        });
    });
}

pub fn chart_panel(
    ui: &mut Ui,
    state: &mut ChartPluginState,
    pool: &VariablePool,
    frame_data: &HashMap<usize, Vec<(f64, [u8; 8])>>,
    running: bool,
) -> PanelAction {
    let mut action = PanelAction::None;

    if running {
        if !state.was_running {
            state.acq_frame_count = 0;
            state.acq_last_reset = Instant::now();
            state.was_running = true;
            if state.log_file.is_some() {
                match std::fs::File::create(state.log_file.as_ref().unwrap()) {
                    Ok(f) => {
                        let mut w = BufWriter::new(f);
                        let _ = write!(w, "timestamp");
                        for legend in &state.legends {
                            let _ = write!(w, ",{}", legend.curve_name);
                        }
                        let _ = writeln!(w);
                        state.log_writer = Some(w);
                        state.logging_active = true;
                        state.log_started = true;
                    }
                    Err(_) => {
                        state.logging_active = false;
                    }
                }
            }
        }
        for legend in &mut state.legends {
            if let Some(data) = frame_data.get(&legend.variable_id) {
                let var = match pool.get(legend.variable_id) {
                    Some(v) => v,
                    None => continue,
                };
                let n = data.len() as u64;
                for (t, raw) in data {
                    let val = decode_value_f64(raw, &var.ext_type);
                    legend.push_value(*t, val);
                }
                state.acq_frame_count += n;
            }
        }
        let elapsed = state.acq_last_reset.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            state.acq_hz = state.acq_frame_count as f64 / elapsed;
            state.acq_frame_count = 0;
            state.acq_last_reset = Instant::now();
        }
    } else {
        if state.logging_active {
            state.log_writer = None;
            state.logging_active = false;
            state.log_stopped = true;
        }
        state.was_running = false;
    }

    if running && state.logging_active {
        if let Some(ref mut w) = state.log_writer {
            let mut timestamps: Vec<f64> = Vec::new();
            for legend in &state.legends {
                if let Some(data) = frame_data.get(&legend.variable_id) {
                    for (t, _) in data {
                        if !timestamps.contains(t) {
                            timestamps.push(*t);
                        }
                    }
                }
            }
            timestamps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for t in &timestamps {
                let _ = write!(w, "{:.6}", t);
                for legend in &state.legends {
                    if let Some(data) = frame_data.get(&legend.variable_id) {
                        let val = data.iter().find(|(dt, _)| (dt - t).abs() < 1e-9);
                        if let Some((_, raw)) = val {
                            if let Some(var) = pool.get(legend.variable_id) {
                                let f = decode_value_f64(raw, &var.ext_type);
                                let _ = write!(w, ",{:.6}", f);
                                continue;
                            }
                        }
                    }
                    let _ = write!(w, ",");
                }
                let _ = writeln!(w);
            }
            let _ = w.flush();
        }
    }

    ui.vertical(|ui| {
        let dialog_is_open = state.show_line_dialog;

        ui.add_enabled_ui(!dialog_is_open, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new("📈 实时数据图表").size(16.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(!state.logging_active, |ui| {
                        if ui.button(RichText::new("📋 打开变量树").size(12.0)).clicked() {
                            action = PanelAction::OpenTree;
                        }
                    });
                    if !state.cursor_txt.is_empty() {
                        ui.label(RichText::new(&state.cursor_txt).size(11.0).color(Color32::from_rgb(180, 180, 180)));
                    }
                });
            });
            ui.add_space(2.0);

            ui.horizontal(|ui| {
                if ui.button("清空").clicked() {
                    for legend in &mut state.legends {
                        legend.data_history.clear();
                    }
                    state.auto_scroll = true;
                    state.acq_hz = 0.0;
                    state.acq_frame_count = 0;
                    state.reset_timer = true;
                }
                if ui.button("回到最新").clicked() {
                    state.auto_scroll = true;
                }
                ui.separator();
                egui::ComboBox::from_label("X轴")
                    .selected_text(x_mode_label(&state.x_mode))
                    .width(80.0)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(state.x_mode == XAxisMode::Auto, "自动").clicked() {
                            state.x_mode = XAxisMode::Auto;
                        }
                        if ui
                            .selectable_label(
                                matches!(state.x_mode, XAxisMode::Fixed(_)),
                                "固定",
                            )
                            .clicked()
                        {
                            let xr = state
                                .legends
                                .iter()
                                .filter_map(|l| {
                                    let front = l.data_history.front().map(|p| p.0);
                                    let back = l.data_history.back().map(|p| p.0);
                                    front.zip(back).map(|(f, b)| (b - f).max(6.0))
                                })
                                .fold(6.0f64, f64::max);
                            state.x_mode = XAxisMode::Fixed(xr.max(6.0));
                        }
                    });
                if let XAxisMode::Fixed(w) = &mut state.x_mode {
                    let mut w_str = format!("{:.3}", *w);
                    if ui
                        .add_sized([65.0, 20.0], egui::TextEdit::singleline(&mut w_str).hint_text("s"))
                        .changed()
                    {
                        if let Ok(v) = w_str.parse::<f64>() {
                            *w = v.max(0.001);
                        }
                    }
                    ui.label("s");
                }
                egui::ComboBox::from_label("Y轴")
                    .selected_text(y_mode_label(&state.y_mode))
                    .width(80.0)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(state.y_mode == YAxisMode::Auto, "自动").clicked() {
                            state.y_mode = YAxisMode::Auto;
                        }
                        if ui
                            .selectable_label(
                                matches!(state.y_mode, YAxisMode::Fixed { .. }),
                                "固定",
                            )
                            .clicked()
                        {
                            let (lo, hi) = state
                                .legends
                                .iter()
                                .flat_map(|l| l.data_history.iter().map(|p| p.1))
                                .fold((0.0f64, 0.0f64), |(lo, hi), y| (lo.min(y), hi.max(y)));
                            let range = (hi - lo).max(10.0);
                            state.y_mode = YAxisMode::Fixed {
                                min: lo - range * 0.1,
                                max: hi + range * 0.1,
                            };
                        }
                        if ui.selectable_label(state.y_mode == YAxisMode::None, "无").clicked() {
                            state.y_mode = YAxisMode::None;
                        }
                    });
                if let YAxisMode::Fixed { min, max } = &mut state.y_mode {
                    ui.label("min:");
                    let mut min_str = format!("{:.2}", *min);
                    if ui
                        .add_sized([55.0, 20.0], egui::TextEdit::singleline(&mut min_str))
                        .changed()
                    {
                        if let Ok(v) = min_str.parse::<f64>() {
                            *min = v;
                        }
                    }
                    ui.label("max:");
                    let mut max_str = format!("{:.2}", *max);
                    if ui
                        .add_sized([55.0, 20.0], egui::TextEdit::singleline(&mut max_str))
                        .changed()
                    {
                        if let Ok(v) = max_str.parse::<f64>() {
                            *max = v;
                        }
                    }
                }
                if !state.auto_scroll {
                    ui.colored_label(Color32::LIGHT_BLUE, "手动查看中");
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Log:");
                    ui.add_enabled_ui(!state.logging_active, |ui| {
                        if ui.button("选择文件").clicked() {
                            if let Some(p) = rfd::FileDialog::new()
                                .add_filter("CSV", &["csv"])
                                .set_file_name("data.csv")
                                .save_file()
                            {
                                state.log_file = Some(p);
                            }
                        }
                        if state.log_file.is_some() {
                            if ui.button("清除").clicked() {
                                state.log_file = None;
                            }
                        }
                    });
                    if let Some(ref p) = state.log_file {
                        ui.label(p.file_name().unwrap_or_default().to_string_lossy().to_string());
                    } else {
                        ui.label("不 log");
                    }
                });
            });
            ui.add_space(2.0);

            if state.legends.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(
                        RichText::new("暂无监控变量")
                            .size(13.0)
                            .color(Color32::from_rgb(150, 150, 150)),
                    );
                    ui.add_enabled_ui(!state.logging_active, |ui| {
                        if ui.button("📋 打开变量树").clicked() { action = PanelAction::OpenTree; }
                    });
                });
            } else {
                render_chart(ui, state);
            }
        });

        // Dialog (rendered at top layer, always interactive)
        if state.show_line_dialog {
            if let Some(edit_idx) = state.editing_legend {
                let (ext_info, win_title) = {
                    let legend = &state.legends[edit_idx];
                    let ext_info = pool
                        .get(legend.variable_id)
                        .map(|v| (v.name.clone(), v.address, v.ext_type.clone(), v.size));
                    (ext_info, legend.curve_name.clone())
                };
                let mut action: Option<super::line_dialog::DialogAction> = None;
                egui::Window::new(format!("曲线属性 - {}", win_title))
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ui.ctx(), |ui| {
                        let (ext_name, ext_addr, ext_type, ext_size) =
                            ext_info.unwrap_or((String::new(), 0, ExtendType::U32, 0));
                        action = super::line_dialog::line_dialog_ui(
                            ui,
                            &mut state.edit_curve_name,
                            &mut state.edit_color,
                            &mut state.edit_buffer_size,
                            &mut state.edit_visible,
                            &ext_name, ext_addr, &ext_type, ext_size,
                            running,
                            state.logging_active,
                        );
                    });
                if let Some(act) = action {
                    match act {
                        super::line_dialog::DialogAction::Delete => {
                            state.remove_legend(edit_idx);
                        }
                        super::line_dialog::DialogAction::Cancel => {}
                        super::line_dialog::DialogAction::Confirm => {
                            let legend = &mut state.legends[edit_idx];
                            legend.curve_name = std::mem::take(&mut state.edit_curve_name);
                            legend.color = state.edit_color;
                            legend.visible = state.edit_visible;
                            if legend.buffer_size != state.edit_buffer_size {
                                legend.buffer_size = state.edit_buffer_size;
                                legend.data_history = std::collections::VecDeque::with_capacity(state.edit_buffer_size);
                            }
                        }
                    }
                    state.show_line_dialog = false;
                }
            } else {
                state.show_line_dialog = false;
            }
        }
    });
    action
}

fn render_chart(ui: &mut Ui, state: &mut ChartPluginState) {
    let available_h = ui.available_height();
    let plot_height = (available_h - 24.0).max(100.0);

    let has_data = state.legends.iter().any(|l| l.data_history.len() >= 2);
    let show_y_axis = !matches!(state.y_mode, YAxisMode::None);

    let auto_bounds: Option<(f64, f64, f64, f64)> = {
        let t_max = state
            .legends
            .iter()
            .filter_map(|l| l.data_history.back().map(|p| p.0))
            .fold(0.0f64, f64::max);
        let t_min = state
            .legends
            .iter()
            .filter_map(|l| l.data_history.front().map(|p| p.0))
            .fold(f64::MAX, f64::min);
        if has_data {
            let xr = (t_max - t_min).max(6.0);
            let window = state.x_mode.window(xr);
            let x_min = t_max - window;
            let x_max = t_max + window * 0.02;
            let (y_min, y_max) = match &state.y_mode {
                YAxisMode::Auto => {
                    let (g_min, g_max) = state
                        .legends
                        .iter()
                        .flat_map(|l| l.data_history.iter().map(|p| p.1))
                        .fold(
                            (f64::INFINITY, f64::NEG_INFINITY),
                            |(lo, hi), y| (lo.min(y), hi.max(y)),
                        );
                    let y_pad = (g_max - g_min).max(10.0) * 0.1;
                    (g_min - y_pad, g_max + y_pad)
                }
                YAxisMode::Fixed { min, max } => (*min, *max),
                YAxisMode::None => {
                    let (g_min, g_max) = state
                        .legends
                        .iter()
                        .flat_map(|l| l.data_history.iter().map(|p| p.1))
                        .fold(
                            (f64::INFINITY, f64::NEG_INFINITY),
                            |(lo, hi), y| (lo.min(y), hi.max(y)),
                        );
                    let y_pad = (g_max - g_min).max(10.0) * 0.1;
                    (g_min - y_pad, g_max + y_pad)
                }
            };
            Some((x_min, x_max, y_min, y_max))
        } else if matches!(state.x_mode, XAxisMode::Auto) {
            Some((0.0, 6.0, 0.0, 1.0))
        } else {
            None
        }
    };

    let plot_rect = egui::Rect::from_min_size(
        ui.next_widget_position(),
        egui::vec2(ui.available_width(), plot_height),
    );

    let mut cursor_labels: Option<(f32, f32, Vec<(String, f64, f64, Color32)>)> = None;

    Plot::new("chart_plot")
        .height(plot_height)
        .show_axes([true, show_y_axis])
        .show_grid([true, true])
        .allow_zoom([true, true])
        .allow_drag([true, true])
        .allow_scroll(true)
        .allow_boxed_zoom(true)
        .allow_double_click_reset(false)
        .x_axis_formatter(|t, _range| fmt_time(t.value))
        .y_axis_formatter(|v, _range| y_axis_fmt(v.value))
        .set_margin_fraction(egui::vec2(0.02, 0.05))
        .show(ui, |plot_ui| {
            for legend in &state.legends {
                if !legend.visible || legend.data_history.len() < 2 {
                    continue;
                }
                let pts: Vec<[f64; 2]> = legend
                    .data_history
                    .iter()
                    .map(|&(t, val)| [t, val])
                    .collect();
                plot_ui.line(
                    Line::new(legend.curve_name.clone(), PlotPoints::new(pts))
                        .color(legend.color)
                        .width(1.5),
                );
            }

            if let Some(cursor) = plot_ui.pointer_coordinate() {
                let t = cursor.x;
                let screen = plot_ui.screen_from_plot(cursor);
                let mut data: Vec<(String, f64, f64, Color32)> = Vec::new();
                for legend in &state.legends {
                    if !legend.visible || legend.data_history.len() < 2 { continue; }
                    let (dt, dv) = find_point_at(&legend.data_history, t);
                    data.push((legend.curve_name.clone(), dt, dv, legend.color));
                }
                if !data.is_empty() {
                    cursor_labels = Some((screen.x, screen.y, data));
                }
                state.cursor_txt = format!("t:{} v:{:.3}", fmt_time(t), cursor.y);
                plot_ui.vline(
                    egui_plot::VLine::new("cursor", t)
                        .color(Color32::from_rgba_premultiplied(128, 128, 128, 80))
                        .width(1.0),
                );
            } else {
                state.cursor_txt.clear();
            }

            if plot_ui.response().drag_started() {
                state.auto_scroll = false;
            }
            if plot_ui.response().double_clicked() {
                state.auto_scroll = true;
            }

            if state.auto_scroll {
                if let Some((x_min, x_max, y_min, y_max)) = auto_bounds {
                    plot_ui.set_plot_bounds(PlotBounds::from_min_max(
                        [x_min, y_min],
                        [x_max, y_max],
                    ));
                }
            }
        });

    if let Some((sx, sy, cursor_data)) = &cursor_labels {
        let font_id = egui::FontId::proportional(11.0);
        let mut max_w = 0.0f32;
        let mut total_h = 0.0f32;
        for (name, dt, dv, _) in cursor_data {
            let line = format!("{}: {:.3} @ {}", name, dv, fmt_time(*dt));
            let g = ui.painter().layout_no_wrap(line, font_id.clone(), Color32::WHITE);
            max_w = max_w.max(g.size().x);
            total_h += g.size().y + 1.0;
        }
        let w = max_w + 8.0;
        let h = total_h + 4.0;
        let x = (*sx + 16.0).min(plot_rect.right() - w);
        let mut y = *sy + 8.0;
        if y + h > plot_rect.bottom() { y = plot_rect.bottom() - h; }
        let r = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
        ui.painter().rect_filled(r, egui::CornerRadius::same(3), Color32::from_rgba_premultiplied(0, 0, 0, 210));
        let mut ty = r.top() + 2.0;
        for (name, dt, dv, color) in cursor_data {
            let line = format!("{}: {} @ {:.3}", name, fmt_time(*dt), dv);
            let g = ui.painter().layout_no_wrap(line, font_id.clone(), *color);
            let gh = g.size().y;
            ui.painter().galley(egui::pos2(r.left() + 4.0, ty), g, *color);
            ty += gh + 1.0;
        }
    }

    legend_overlay(
        ui,
        state,
        egui::pos2(plot_rect.right() - 5.0, plot_rect.top() + 5.0),
    );
}

fn legend_overlay(ui: &mut Ui, state: &mut ChartPluginState, anchor: egui::Pos2) {
    let mut toggle = None;
    let mut edit = None;
    let mut y = anchor.y + 4.0;
    let x = (anchor.x - 160.0).max(0.0);
    for (i, legend) in state.legends.iter().enumerate() {
        let op = if legend.visible { 1.0 } else { 0.35 };
        let tc = if legend.visible {
            Color32::WHITE
        } else {
            Color32::from_gray(100)
        };
        let text = legend.curve_name.clone();
        let g = ui
            .painter()
            .layout_no_wrap(text, egui::FontId::proportional(11.0), tc);
        let w = g.rect.width() + 24.0;
        let h = g.rect.height() + 6.0;
        let r = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
        let resp = ui.interact(r, egui::Id::new(("chart_legend", i)), egui::Sense::click());
        let bg = if resp.hovered() {
            Color32::from_rgba_premultiplied(40, 40, 50, 200)
        } else {
            Color32::from_rgba_premultiplied(20, 20, 30, 180)
        };
        ui.painter().rect_filled(r, egui::CornerRadius::same(3), bg);
        let bar = egui::Rect::from_min_size(
            egui::pos2(r.left() + 4.0, r.top() + 3.0),
            egui::vec2(8.0, h - 6.0),
        );
        ui.painter().rect_filled(
            bar,
            egui::CornerRadius::same(2),
            legend.color.linear_multiply(op),
        );
        ui.painter()
            .galley(egui::pos2(r.left() + 14.0, r.top() + 3.0), g, tc);
        if resp.clicked() {
            toggle = Some(i);
        }
        if resp.secondary_clicked() {
            edit = Some(i);
        }
        y += h + 3.0;
    }
    if let Some(i) = toggle {
        state.legends[i].visible = !state.legends[i].visible;
    }
    if let Some(i) = edit {
        state.editing_legend = Some(i);
        state.show_line_dialog = true;
        let legend = &state.legends[i];
        state.edit_curve_name = legend.curve_name.clone();
        state.edit_color = legend.color;
        state.edit_buffer_size = legend.buffer_size;
        state.edit_visible = legend.visible;
    }
}

fn decode_value_f64(data: &[u8], ext_type: &crate::types::ExtendType) -> f64 {
    use crate::types::ExtendType::*;
    if data.is_empty() {
        return 0.0;
    }
    match ext_type {
        U8 => *data.first().unwrap_or(&0) as f64,
        I8 => *data.first().unwrap_or(&0) as i8 as f64,
        U16 => {
            if data.len() >= 2 {
                u16::from_le_bytes([data[0], data[1]]) as f64
            } else {
                0.0
            }
        }
        I16 => {
            if data.len() >= 2 {
                i16::from_le_bytes([data[0], data[1]]) as f64
            } else {
                0.0
            }
        }
        U32 => {
            if data.len() >= 4 {
                u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64
            } else {
                0.0
            }
        }
        I32 => {
            if data.len() >= 4 {
                i32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64
            } else {
                0.0
            }
        }
        U64 => {
            if data.len() >= 8 {
                u64::from_le_bytes([
                    data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
                ]) as f64
            } else {
                0.0
            }
        }
        I64 => {
            if data.len() >= 8 {
                i64::from_le_bytes([
                    data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
                ]) as f64
            } else {
                0.0
            }
        }
        Float => {
            if data.len() >= 4 {
                f32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64
            } else {
                0.0
            }
        }
        Double => {
            if data.len() >= 8 {
                f64::from_le_bytes([
                    data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
                ])
            } else {
                0.0
            }
        }
        Other => 0.0,
    }
}

fn find_point_at(data: &std::collections::VecDeque<(f64, f64)>, t: f64) -> (f64, f64) {
    if data.is_empty() { return (t, 0.0); }
    let idx = data.partition_point(|&(x, _)| x < t);
    if idx == 0 { return data[0]; }
    if idx >= data.len() { return data[data.len() - 1]; }
    let p0 = data[idx - 1]; let p1 = data[idx];
    if (t - p0.0).abs() < (p1.0 - t).abs() { p0 } else { p1 }
}

fn y_axis_fmt(v: f64) -> String {
    if v.abs() < 0.01 && v != 0.0 {
        format!("{v:.5}")
    } else {
        format!("{v:.3}")
    }
}

fn fmt_time(t: f64) -> String {
    let s = format!("{:.6}", t);
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        "0s".to_string()
    } else {
        format!("{}s", trimmed)
    }
}

fn y_mode_label(mode: &YAxisMode) -> String {
    match mode {
        YAxisMode::Auto => "自动".to_string(),
        YAxisMode::Fixed { .. } => "固定".to_string(),
        YAxisMode::None => "无".to_string(),
    }
}

fn x_mode_label(mode: &XAxisMode) -> String {
    match mode {
        XAxisMode::Auto => "自动".to_string(),
        XAxisMode::Fixed(w) => format!("{:.3}s", w),
    }
}
