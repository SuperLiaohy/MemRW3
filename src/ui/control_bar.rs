use eframe::egui::{self, Color32, RichText, Ui};
use std::sync::atomic::Ordering;
use crate::app::MemRW3App;

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
                settings_button(ui, app);
                ui.separator();
                delay_slider(ui, app);
                ui.separator();
                reset_button(ui, app);
                ui.separator();
                if ui.button(RichText::new("保存").size(12.0)).clicked() {
                    app.save_config();
                }
                ui.add_enabled_ui(!app.session.is_running(), |ui| {
                    if ui.button(RichText::new("加载").size(12.0)).clicked() {
                        app.load_config();
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    sampling_status(ui, app);
                });
            });
        });

    if app.session.show_probe_settings {
        settings_dialog(ui.ctx(), app);
    }
}

fn settings_dialog(ctx: &egui::Context, app: &mut MemRW3App) {
    let mut show = app.session.show_probe_settings;
    let mut closed = false;
    let mut confirm = false;

    if show && app.session.edit_chip.is_empty() {
        app.session.edit_chip = app.session.probe_chip.clone();
        app.session.edit_protocol = app.session.probe_protocol.clone();
        app.session.edit_speed = app.session.probe_speed_khz;
    }

    let probe_list: Vec<String> = probe_rs::probe::list::Lister::new()
        .list_all()
        .iter()
        .map(|p| p.identifier.clone())
        .collect();
    let probe_text = if probe_list.is_empty() {
        "未检测到 Probe".to_string()
    } else if probe_list.len() == 1 {
        probe_list[0].clone()
    } else {
        format!("{} 个设备", probe_list.len())
    };

    let search_id = egui::Id::new("mcu_search");
    let mut search = ctx.data_mut(|d| d.get_temp::<String>(search_id).unwrap_or_default());

    egui::Window::new("Probe 设置")
        .collapsible(false).resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .default_width(320.0)
        .open(&mut show)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("MCU 型号:");
                ui.label(egui::RichText::new(&app.session.edit_chip).strong());
            });
            ui.add_sized(
                [ui.available_width(), 20.0],
                egui::TextEdit::singleline(&mut search)
                    .hint_text("搜索过滤...")
                    .id(search_id),
            );
            ctx.data_mut(|d| d.insert_temp(search_id, search.clone()));

            let filtered: Vec<&String> = if search.is_empty() {
                app.session.all_chips.iter().collect()
            } else {
                let s = search.to_lowercase();
                app.session.all_chips.iter().filter(|n| n.to_lowercase().contains(&s)).collect()
            };

            let list_h = 200.0;
            let (list_rect, _) = ui.allocate_at_least(
                egui::vec2(ui.available_width(), list_h),
                egui::Sense::hover(),
            );
            let mut list_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(list_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            egui::ScrollArea::vertical().show(&mut list_ui, |ui| {
                ui.set_min_height(list_h);
                for name in filtered {
                    if ui.selectable_label(app.session.edit_chip == *name, name.as_str()).clicked() {
                        app.session.edit_chip = name.clone();
                    }
                }
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);

            egui::Grid::new("probe_settings").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("协议:");
                egui::ComboBox::from_id_salt("protocol_combo")
                    .selected_text(&app.session.edit_protocol)
                    .width(80.0)
                    .show_ui(ui, |ui| {
                        for p in &["SWD".to_string(), "JTAG".to_string()] {
                            ui.selectable_value(&mut app.session.edit_protocol, p.clone(), p.as_str());
                        }
                    });
                ui.end_row();
                ui.label("速度 (kHz):");
                ui.add(egui::Slider::new(&mut app.session.edit_speed, 100..=20000).text("kHz"));
                ui.end_row();
                ui.label("Probe 设备:");
                egui::ComboBox::from_id_salt("probe_combo")
                    .selected_text(&probe_text)
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        if probe_list.is_empty() {
                            ui.label("(无可用设备, 请插入调试器)");
                        }
                        for name in &probe_list {
                            ui.label(name);
                        }
                    });
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("确定").clicked() { closed = true; confirm = true; }
                if ui.button("取消").clicked() { closed = true; }
            });
        });
    if closed {
        app.session.show_probe_settings = false;
        if confirm {
            app.session.probe_chip = std::mem::take(&mut app.session.edit_chip);
            app.session.probe_protocol = std::mem::take(&mut app.session.edit_protocol);
            app.session.probe_speed_khz = app.session.edit_speed;
        }
        app.session.edit_chip.clear();
        app.session.edit_protocol.clear();
    } else {
        app.session.show_probe_settings = show;
    }
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

fn settings_button(ui: &mut Ui, app: &mut MemRW3App) {
    ui.add_enabled_ui(!app.session.connected, |ui| {
        if ui.add(egui::Button::new(RichText::new("⚙ 设置").size(13.0))).clicked() {
            app.session.show_probe_settings = true;
        }
    });
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
            if !app.session.timer_was_started {
                app.session.timer_was_started = true;
                app.reset_timer();
            }
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
    ui.add_enabled_ui(app.session.connected, |ui| {
        if ui.add(egui::Button::new(RichText::new("Reset").size(13.0))).clicked() {
            app.sync_reset();
        }
    });
}

fn sampling_status(ui: &mut Ui, app: &MemRW3App) {
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 0.0);
    let pool_n = app.session.pool.iter().count();
    let slot_n = app.slot_count.load(Ordering::Relaxed);
    ui.label(
        RichText::new(format!("Vari:{} Slot:{}", pool_n, slot_n))
            .size(12.0)
            .color(Color32::from_rgb(150, 200, 255)),
    );
    ui.separator();
    ui.label(RichText::new(format!("Hz: {:.1}", app.session.sampling_hz)).size(13.0).color(Color32::from_rgb(80, 160, 255)));
    ui.separator();
    let (text, color) = if app.session.is_running() {
        ("● 采集中", Color32::from_rgb(80, 220, 80))
    } else {
        ("○ 已暂停", Color32::from_rgb(255, 180, 60))
    };
    ui.label(RichText::new(text).size(13.0).color(color));
}

fn bar_background(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode { Color32::from_rgb(28, 28, 38) } else { Color32::from_rgb(245, 245, 250) }
}
