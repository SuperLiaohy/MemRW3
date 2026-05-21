use eframe::egui::{self, Color32, RichText, Ui};
use crate::model::AppSession;
use crate::probe::ProbeSession;

pub fn control_bar(ui: &mut Ui, session: &mut AppSession, probe: &mut ProbeSession) {
    egui::Frame::NONE
        .fill(bar_background(ui))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(180, 180, 200)))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(12, 0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 0.0);
                connect_button(ui, session, probe);
                ui.separator();
                run_control(ui, session);
                ui.separator();
                settings_button(ui, probe);
                ui.separator();
                delay_slider(ui, session);
                ui.separator();
                reset_button(ui, probe);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    sampling_status(ui, session);
                });
            });
        });

    if probe.show_settings {
        settings_dialog(ui.ctx(), probe);
    }
}

fn settings_dialog(ctx: &egui::Context, probe: &mut ProbeSession) {
    let mut show = probe.show_settings;
    let mut closed = false;
    egui::Window::new("Probe 设置")
        .collapsible(false).resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .open(&mut show)
        .show(ctx, |ui| {
            egui::Grid::new("probe_settings").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("MCU 型号:");
                egui::ComboBox::from_id_salt("chip_combo")
                    .selected_text(&probe.chip_name)
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        for name in &probe.available_chips {
                            ui.selectable_value(&mut probe.chip_name, name.clone(), name.as_str());
                        }
                    });
                ui.end_row();
                ui.label("协议:");
                egui::ComboBox::from_id_salt("protocol_combo")
                    .selected_text(&probe.protocol)
                    .width(80.0)
                    .show_ui(ui, |ui| {
                        for p in &["SWD".to_string(), "JTAG".to_string()] {
                            ui.selectable_value(&mut probe.protocol, p.clone(), p.as_str());
                        }
                    });
                ui.end_row();
                ui.label("速度 (kHz):");
                ui.add(egui::Slider::new(&mut probe.speed_khz, 100..=20000).text("kHz"));
                ui.end_row();
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);

            ui.label(RichText::new("已连接 Probe:").size(12.0).strong());
            if ui.button("刷新").clicked() { probe.list_probes(); }

            ui.add_space(4.0);
            if let Some(ref err) = probe.last_error {
                ui.label(RichText::new(err).size(11.0).color(Color32::from_rgb(255, 80, 80)));
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("确定").clicked() { closed = true; }
                if ui.button("取消").clicked() { closed = true; }
                ui.label("（设置将在下次连接时生效）");
            });
        });
    if closed { probe.show_settings = false; }
    else { probe.show_settings = show; }
}

fn connect_button(ui: &mut Ui, session: &mut AppSession, probe: &mut ProbeSession) {
    let label = if session.connected { "断开" } else { "连接" };
    if ui.add(egui::Button::new(RichText::new(label).size(13.0))).clicked() {
        if session.connected {
            probe.disconnect();
            session.connected = false;
            session.running = false;
        } else {
            session.connected = probe.connect();
        }
    }
}

fn settings_button(ui: &mut Ui, probe: &mut ProbeSession) {
    if ui.add(egui::Button::new(RichText::new("⚙ 设置").size(13.0))).clicked() {
        probe.show_settings = true;
        probe.list_probes();
    }
}

fn run_control(ui: &mut Ui, session: &mut AppSession) {
    let enabled = session.connected;
    let label = if !enabled { "开始" } else if session.running { "暂停" } else { "开始" };
    let resp = if enabled {
        ui.add(egui::Button::new(RichText::new(label).size(13.0)))
    } else {
        ui.add_enabled(false, egui::Button::new(RichText::new(label).size(13.0)))
    };
    if resp.clicked() { session.running = !session.running; }
}

fn delay_slider(ui: &mut Ui, session: &mut AppSession) {
    ui.add_enabled_ui(!session.running, |ui| {
        ui.label(RichText::new("延迟:").size(12.0));
        let mut val = session.delay_us;
        if ui.add(egui::Slider::new(&mut val, 0.0..=10000.0).step_by(50.0).text("μs")).changed() {
            session.delay_us = val;
        }
    });
}

fn reset_button(ui: &mut Ui, probe: &mut ProbeSession) {
    if ui.add(egui::Button::new(RichText::new("Reset").size(13.0))).clicked() {
        probe.reset_target();
    }
}

fn sampling_status(ui: &mut Ui, session: &AppSession) {
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 0.0);
    ui.label(RichText::new(format!("Hz: {:.1}", session.sampling_hz)).size(13.0).color(Color32::from_rgb(80, 160, 255)));
    ui.separator();
    let (text, color) = if session.running {
        ("● 采集中", Color32::from_rgb(80, 220, 80))
    } else {
        ("○ 已暂停", Color32::from_rgb(255, 180, 60))
    };
    ui.label(RichText::new(text).size(13.0).color(color));
}

fn bar_background(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode { Color32::from_rgb(28, 28, 38) } else { Color32::from_rgb(245, 245, 250) }
}
