use eframe::egui::{self, Color32, RichText, Ui};
use std::sync::atomic::Ordering;
use crate::app::MemRW3App;
use crate::model::AppSession;

pub fn control_bar(ui: &mut Ui, app: &mut MemRW3App) {
    egui::Frame::NONE
        .fill(bar_background(ui))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(180, 180, 200)))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(12, 0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 0.0);
                connect_button(ui, app);
                ui.separator();
                run_control(ui, app);
                ui.separator();
                settings_button(ui, &mut app.session);
                ui.separator();
                delay_slider(ui, app);
                ui.separator();
                reset_button(ui, app);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    sampling_status(ui, &app.session);
                });
            });
        });

    if app.session.show_probe_settings {
        settings_dialog(ui.ctx(), app);
    }
}

fn settings_dialog(ctx: &egui::Context, app: &mut MemRW3App) {
    let session = &mut app.session;
    let mut show = session.show_probe_settings;
    let mut closed = false;
    egui::Window::new("Probe 设置")
        .collapsible(false).resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .open(&mut show)
        .show(ctx, |ui| {
            egui::Grid::new("probe_settings").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("MCU 型号:");
                egui::ComboBox::from_id_salt("chip_combo")
                    .selected_text(&session.probe_chip)
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        for name in &session.probe_chips {
                            ui.selectable_value(&mut session.probe_chip, name.clone(), name.as_str());
                        }
                    });
                ui.end_row();
                ui.label("协议:");
                egui::ComboBox::from_id_salt("protocol_combo")
                    .selected_text(&session.probe_protocol)
                    .width(80.0)
                    .show_ui(ui, |ui| {
                        for p in &["SWD".to_string(), "JTAG".to_string()] {
                            ui.selectable_value(&mut session.probe_protocol, p.clone(), p.as_str());
                        }
                    });
                ui.end_row();
                ui.label("速度 (kHz):");
                ui.add(egui::Slider::new(&mut session.probe_speed_khz, 100..=20000).text("kHz"));
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            if ui.button("确定").clicked() { closed = true; }
            if ui.button("取消").clicked() { closed = true; }
        });
    if closed { session.show_probe_settings = false; }
    else { session.show_probe_settings = show; }
}

fn connect_button(ui: &mut Ui, app: &mut MemRW3App) {
    let label = if app.session.connected { "断开" } else { "连接" };
    if ui.add(egui::Button::new(RichText::new(label).size(13.0))).clicked() {
        app.sync_connect();
    }
    if let Some(ref err) = app.session.connect_error {
        ui.colored_label(Color32::from_rgb(255, 80, 80), err);
    }
}

fn settings_button(ui: &mut Ui, session: &mut AppSession) {
    if ui.add(egui::Button::new(RichText::new("⚙ 设置").size(13.0))).clicked() {
        session.show_probe_settings = true;
    }
}

fn run_control(ui: &mut Ui, app: &mut MemRW3App) {
    let enabled = app.session.connected;
    let label = if !enabled { "开始" } else if app.session.is_running() { "暂停" } else { "开始" };
    let resp = if enabled {
        ui.add(egui::Button::new(RichText::new(label).size(13.0)))
    } else {
        ui.add_enabled(false, egui::Button::new(RichText::new(label).size(13.0)))
    };
    if resp.clicked() {
        let new_running = !app.session.is_running();
        app.session.set_running(new_running);
        if new_running {
            app.rebuild_slots();
        }
    }
}

fn delay_slider(ui: &mut Ui, app: &mut MemRW3App) {
    ui.add_enabled_ui(!app.session.is_running(), |ui| {
        ui.label(RichText::new("延迟:").size(12.0));
        let mut val = app.delay_us.load(Ordering::Acquire) as f64;
        if ui.add(egui::Slider::new(&mut val, 0.0..=10000.0).step_by(50.0).text("μs")).changed() {
            app.delay_us.store(val as u64, Ordering::Release);
        }
    });
}

fn reset_button(ui: &mut Ui, app: &mut MemRW3App) {
    if ui.add(egui::Button::new(RichText::new("Reset").size(13.0))).clicked() {
        app.sync_reset();
    }
}

fn sampling_status(ui: &mut Ui, session: &AppSession) {
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 0.0);
    ui.label(RichText::new(format!("Hz: {:.1}", session.sampling_hz)).size(13.0).color(Color32::from_rgb(80, 160, 255)));
    ui.separator();
    let (text, color) = if session.is_running() {
        ("● 采集中", Color32::from_rgb(80, 220, 80))
    } else {
        ("○ 已暂停", Color32::from_rgb(255, 180, 60))
    };
    ui.label(RichText::new(text).size(13.0).color(color));
}

fn bar_background(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode { Color32::from_rgb(28, 28, 38) } else { Color32::from_rgb(245, 245, 250) }
}
