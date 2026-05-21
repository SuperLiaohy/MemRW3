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
                probe_config(ui, probe, session);
                ui.separator();
                delay_slider(ui, session);
                ui.separator();
                reset_button(ui, probe);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    sampling_status(ui, session);
                });
            });
        });
}

fn connect_button(ui: &mut Ui, session: &mut AppSession, probe: &mut ProbeSession) {
    let label = if session.connected { "断开" } else { "连接" };
    let btn = egui::Button::new(RichText::new(label).size(13.0));
    if ui.add(btn).clicked() {
        if session.connected {
            probe.disconnect();
            session.connected = false;
            session.running = false;
        } else {
            session.connected = probe.connect();
        }
    }
}

fn run_control(ui: &mut Ui, session: &mut AppSession) {
    let ui_enabled = session.connected;
    let label = if !ui_enabled {
        "开始"
    } else if session.running {
        "暂停"
    } else {
        "开始"
    };
    let btn = egui::Button::new(RichText::new(label).size(13.0));
    let response = if ui_enabled {
        ui.add(btn)
    } else {
        ui.add_enabled(false, btn)
    };
    if response.clicked() {
        session.running = !session.running;
    }
}

fn probe_config(ui: &mut Ui, probe: &mut ProbeSession, session: &AppSession) {
    let enabled = !session.connected && !session.running;
    ui.add_enabled_ui(enabled, |ui| {
        ui.label(RichText::new("MCU:").size(12.0));
        egui::ComboBox::from_id_salt("mcu_sel")
            .selected_text(&probe.chip_name)
            .width(130.0)
            .show_ui(ui, |ui| {
                for name in &probe.available_chips {
                    ui.selectable_value(&mut probe.chip_name, name.clone(), name.as_str());
                }
            });
    });

    if let Some(ref err) = probe.last_error {
        ui.label(RichText::new(err).size(11.0).color(Color32::from_rgb(255, 80, 80)));
    }
}

fn delay_slider(ui: &mut Ui, session: &mut AppSession) {
    let enabled = !session.running;
    ui.add_enabled_ui(enabled, |ui| {
        ui.label(RichText::new("延迟:").size(12.0));
        let mut val = session.delay_us;
        if ui
            .add(
                egui::Slider::new(&mut val, 0.0..=10000.0)
                    .step_by(50.0)
                    .text("μs"),
            )
            .changed()
        {
            session.delay_us = val;
        }
    });
}

fn reset_button(ui: &mut Ui, probe: &mut ProbeSession) {
    if ui
        .add(egui::Button::new(RichText::new("Reset").size(13.0)))
        .clicked()
    {
        probe.reset_target();
    }
}

fn sampling_status(ui: &mut Ui, session: &AppSession) {
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 0.0);

    let hz = RichText::new(format!("Hz: {:.1}", session.sampling_hz))
        .size(13.0)
        .color(Color32::from_rgb(80, 160, 255));
    ui.label(hz);

    ui.separator();

    let (text, color) = if session.running {
        ("● 采集中", Color32::from_rgb(80, 220, 80))
    } else {
        ("○ 已暂停", Color32::from_rgb(255, 180, 60))
    };
    ui.label(RichText::new(text).size(13.0).color(color));
}

fn bar_background(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(28, 28, 38)
    } else {
        Color32::from_rgb(245, 245, 250)
    }
}
