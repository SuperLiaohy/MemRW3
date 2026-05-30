use eframe::egui;
use egui::{Color32, Ui};
use egui_dock::{tab_viewer, DockArea, DockState, NodeIndex, TabViewer};
use object::Object;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::model::{AppSession, DockTab, VariablePool};
use crate::probe::{AcqSlot, ProbeCell, ProbeSession, VarSlotMapping};
use crate::sync::Sync;
use crate::types::DwarfApp;
use crate::ui;
use crate::ui::chart_plugin::ChartPluginState;
use crate::ui::table_plugin::TablePluginState;

use std::collections::HashMap;

type FrameData = HashMap<usize, Vec<(f64, [u8; 8])>>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TabKind {
    Chart,
    Table,
}

pub struct MemRW3App {
    tree: DockState<TabKind>,
    pub dwarf_app: DwarfApp,
    pub elf_path: String,
    pub session: AppSession,
    pub chart_state: ChartPluginState,
    pub table_state: TablePluginState,
    probe: Arc<ProbeCell>,
    sync: Arc<Sync>,
    pub delay_us: Arc<AtomicU64>,
    acq_cycle_count: Arc<AtomicU64>,
    pub slot_count: Arc<AtomicU64>,
    pub toasts: egui_notify::Toasts,
    hz_last_cycles: u64,
    hz_last_time: Instant,
    acq_stop: Arc<AtomicBool>,
    _acq_handle: Option<JoinHandle<()>>,
}

fn acq_thread(
    probe: Arc<ProbeCell>,
    running: Arc<AtomicBool>,
    delay_us: Arc<AtomicU64>,
    cycle_count: Arc<AtomicU64>,
    sync: Arc<Sync>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Relaxed) {
        sync.try_acquire();

        if !running.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        while running.load(Ordering::Acquire) {
            sync.try_acquire();

            if stop.load(Ordering::Relaxed) {
                return;
            }

            let probe_ref = unsafe { probe.get_mut() };
            if !probe_ref.connected {
                break;
            }

            probe_ref.acquire_from_slots();
            cycle_count.fetch_add(1, Ordering::Relaxed);

            if probe_ref.slots.is_empty() {
                thread::sleep(Duration::from_millis(100));
                continue;
            }

            let d = delay_us.load(Ordering::Acquire);
            if d > 0 {
                thread::sleep(Duration::from_micros(d));
            }
        }
    }
}

impl MemRW3App {
    pub fn new(dwarf_app: DwarfApp) -> Self {
        let mut tree = DockState::new(vec![TabKind::Chart]);
        tree.main_surface_mut()
            .split_right(NodeIndex::root(), 0.5, vec![TabKind::Table]);

        let mut session = AppSession {
            bottom_sheet_height: 250.0,
            ..Default::default()
        };
        let mut chips: Vec<String> = probe_rs::config::Registry::from_builtin_families()
            .families()
            .iter()
            .flat_map(|f| f.variants.iter().map(|v| v.name.clone()))
            .collect();
        chips.sort();
        session.all_chips = chips;

        let probe = Arc::new(ProbeCell::new(ProbeSession::default()));
        let sync = Arc::new(Sync::new());
        let acq_stop = Arc::new(AtomicBool::new(false));
        let delay_us = Arc::new(AtomicU64::new(0));
        let acq_cycle_count = Arc::new(AtomicU64::new(0));
        let slot_count = Arc::new(AtomicU64::new(0));

        let acq_probe = probe.clone();
        let acq_sync = sync.clone();
        let acq_running = session.running.clone();
        let acq_delay = delay_us.clone();
        let acq_cycles = acq_cycle_count.clone();
        let acq_stop_th = acq_stop.clone();
        let _acq_handle = Some(thread::spawn(move || {
            acq_thread(
                acq_probe,
                acq_running,
                acq_delay,
                acq_cycles,
                acq_sync,
                acq_stop_th,
            );
        }));

        Self {
            tree,
            dwarf_app,
            elf_path: String::new(),
            session,
            chart_state: ChartPluginState::default(),
            table_state: TablePluginState::default(),
            probe,
            sync,
            delay_us,
            acq_cycle_count,
            slot_count,
            toasts: egui_notify::Toasts::default().with_anchor(egui_notify::Anchor::BottomRight),
            hz_last_cycles: 0,
            hz_last_time: Instant::now(),
            acq_stop,
            _acq_handle,
        }
    }

    fn load_elf(&mut self) {
        self.session.load_error = None;
        let path = self.elf_path.trim().to_string();
        if path.is_empty() {
            self.session.load_error = Some("请输入 ELF 文件路径".into());
            return;
        }
        let data = match fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                self.session.load_error = Some(format!("读取文件失败: {e}"));
                return;
            }
        };
        let object = match object::read::File::parse(&*data) {
            Ok(o) => o,
            Err(e) => {
                self.session.load_error = Some(format!("解析 ELF 失败: {e}"));
                return;
            }
        };
        if object.format() != object::BinaryFormat::Elf {
            self.session.load_error = Some("不是有效的 ELF 文件".into());
            return;
        }
        let endian = match object.endianness() {
            object::Endianness::Little => gimli::RunTimeEndian::Little,
            object::Endianness::Big => gimli::RunTimeEndian::Big,
        };
        let dwarf = match crate::dwarf::load_dwarf(&object, endian) {
            Ok(d) => d,
            Err(e) => {
                self.session.load_error = Some(format!("加载 DWARF 失败: {e}"));
                return;
            }
        };
        let cus = match crate::dwarf::collect_cus(&dwarf) {
            Ok(c) => c,
            Err(e) => {
                self.session.load_error = Some(format!("解析 DWARF 数据失败: {e}"));
                return;
            }
        };
        self.dwarf_app = DwarfApp::new(cus);
        self.session.load_error = None;
    }

    fn trace_variables(&mut self) {
        self.load_elf();
        if self.session.load_error.is_some() {
            return;
        }

        let mut errors: Vec<String> = Vec::new();
        let pool = &mut self.session.pool;

        for var in pool.iter_mut() {
            let name = var.name.clone();
            let path = crate::types::expand_bracket_path(&name);
            let node_ids = self.dwarf_app.trace_exact(&path);
            for &node_id in &node_ids {
                self.dwarf_app.apply_array_path(node_id, &path);
            }

            match node_ids.len() {
                1 => {
                    let node_id = node_ids[0];
                    let node = self.dwarf_app.find_node_by_id(node_id);
                    if let Some(node) = node {
                        let new_type = crate::types::basic_type_to_extend(&node.basic_type);
                        let new_size = match new_type {
                            crate::types::ExtendType::U8 | crate::types::ExtendType::I8 => 1,
                            crate::types::ExtendType::U16 | crate::types::ExtendType::I16 => 2,
                            crate::types::ExtendType::U32
                            | crate::types::ExtendType::I32
                            | crate::types::ExtendType::Float => 4,
                            crate::types::ExtendType::U64
                            | crate::types::ExtendType::I64
                            | crate::types::ExtendType::Double => 8,
                            _ => node.size,
                        };
                        let new_addr = self
                            .dwarf_app
                            .compute_extend_address(node_id)
                            .unwrap_or(node.address);
                        var.address = new_addr;
                        var.ext_type = new_type;
                        var.size = new_size;
                    }
                }
                0 => {
                    errors.push(format!("\"{name}\": 未找到匹配"));
                }
                _ => {
                    errors.push(format!("\"{name}\": 匹配到多个 ({}) 节点", node_ids.len()));
                }
            }
        }

        for err in &errors {
            self.toasts
                .error(err.clone())
                .duration(Some(Duration::from_secs(15)))
                .closable(true);
        }
        if errors.is_empty() {
            self.toasts
                .success("追踪完成, 所有变量已更新")
                .duration(Some(Duration::from_secs(3)));
        }

        self.rebuild_slots();
    }

    pub fn sync_connect(&mut self) {
        let chip = self.session.probe_chip.clone();
        let protocol = self.session.probe_protocol.clone();
        let speed = self.session.probe_speed_khz;
        let probe_id = self.session.probe_id.clone();
        let probe = self.probe.clone();
        let connected = self.session.connected;

        if connected {
            self.session.set_running(false);
            self.sync.send_request(move || {
                unsafe { probe.get_mut() }.disconnect();
            });
            self.session.connected = false;
            self.session.connect_error = None;
            self.toasts
                .info("已断开连接")
                .duration(Some(Duration::from_secs(5)))
                .closable(true);
        } else {
            for var in self.session.pool.iter() {
                var.incoming.drain();
            }
            let sync = self.sync.clone();
            let running = self.session.running.clone();
            sync.send_request(move || {
                let p = unsafe { probe.get_mut() };
                p.chip_name = chip;
                p.protocol = protocol;
                p.speed_khz = speed;
                p.selected_probe_id = probe_id;
                if !p.connect() {
                    running.store(false, Ordering::Release);
                }
            });
            let p = self.probe.get();
            self.session.connected = p.connected;
            self.toasts
                .info(format!(
                    "连接配置: chip:{},freq:{},protocol:{},id:{}",
                    p.chip_name,
                    p.speed_khz,
                    p.protocol,
                    p.selected_probe_id.as_ref().unwrap_or(&"auto".into())
                ))
                .duration(Some(Duration::from_secs(5)))
                .closable(true);
            if !self.session.connected {
                let err = p.last_error.clone().unwrap_or_default();
                self.toasts
                    .error(err)
                    .duration(Some(Duration::from_secs(5)))
                    .closable(true);
                self.session.set_running(false);
            } else {
                self.toasts
                    .success("连接成功")
                    .duration(Some(Duration::from_secs(5)))
                    .closable(true);
                self.session.connect_error = None;
            }
        }
    }

    pub fn sync_reset(&mut self) {
        let probe = self.probe.clone();
        self.sync.send_request(move || {
            unsafe { probe.get_mut() }.reset_target();
        });
    }

    pub fn reset_timer(&self) {
        let probe = self.probe.clone();
        self.sync.send_request(move || {
            unsafe { probe.get_mut() }.timer = Instant::now();
        });
    }

    pub fn clear_all_buffers(&mut self) {
        self.session.timer_was_started = false;
        let pool = &self.session.pool;
        let probe = self.probe.clone();
        self.sync.send_request(move || {
            unsafe { probe.get_mut() }.timer = Instant::now();
            for var in pool.iter() {
                var.incoming.drain();
            }
        });
    }

    pub fn write_variable(&self, var_id: usize, value: u64) -> bool {
        let var = match self.session.pool.get(var_id) {
            Some(v) => v,
            None => return false,
        };
        let addr = var.address;
        let size = var.size;
        let probe = self.probe.clone();
        let mut ok = false;
        self.sync.send_request(|| {
            ok = unsafe { probe.get_mut() }.write_value(addr, size, value);
        });
        ok
    }

    pub fn rebuild_slots(&self) {
        let probe = self.probe.clone();
        let pool = &self.session.pool;
        let mut slot_map: std::collections::HashMap<u64, Arc<AcqSlot>> =
            std::collections::HashMap::new();
        let mut mappings: Vec<VarSlotMapping> = Vec::new();

        for var in pool.iter() {
            let addrs = ProbeSession::slot_addresses(var.address, var.size);
            let byte_offset = (var.address & 3) as usize;
            let mut var_slots: Vec<Arc<AcqSlot>> = Vec::with_capacity(addrs.len());
            for addr in addrs {
                var_slots.push(
                    slot_map
                        .entry(addr)
                        .or_insert_with(|| Arc::new(AcqSlot { address: addr }))
                        .clone(),
                );
            }
            mappings.push(VarSlotMapping {
                slots: var_slots,
                size: var.size,
                byte_offset,
                incoming: var.incoming.clone(),
            });
        }

        let slots: Vec<Arc<AcqSlot>> = slot_map.into_values().collect();
        let slot_n = slots.len() as u64;
        let sc = self.slot_count.clone();
        self.sync.send_request(move || {
            let p = unsafe { probe.get_mut() };
            p.slots = slots;
            p.var_mappings = mappings;
            sc.store(slot_n, Ordering::Relaxed);
        });
    }

    fn push_slot_for_new_var(&self, _var_id: usize) {
        self.rebuild_slots();
    }

    pub fn unbind_variable(&mut self, var_id: usize) {
        let should_remove = {
            if let Some(var) = self.session.pool.get_mut(var_id) {
                var.plugins_cnt = var.plugins_cnt.saturating_sub(1);
                var.plugins_cnt == 0
            } else {
                false
            }
        };
        if should_remove {
            self.session.pool.remove(var_id);
            self.session.selected_variables.remove(&var_id);
            self.rebuild_slots();
        }
    }
}

impl Drop for MemRW3App {
    fn drop(&mut self) {
        self.acq_stop.store(true, Ordering::Relaxed);
    }
}

struct TabViewerCtx<'a> {
    chart_state: &'a mut ChartPluginState,
    table_state: &'a mut TablePluginState,
    pool: &'a VariablePool,
    frame_data: &'a FrameData,
    running: bool,
    open_tree: &'a mut Option<DockTab>,
}

impl<'a> TabViewer for TabViewerCtx<'a> {
    type Tab = TabKind;
    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            TabKind::Chart => "Chart 实时数据".into(),
            TabKind::Table => "Table 读写数据".into(),
        }
    }
    fn on_close(&mut self, _tab: &mut Self::Tab) -> tab_viewer::OnCloseResponse {
        tab_viewer::OnCloseResponse::Ignore
    }
    fn ui(&mut self, ui: &mut Ui, tab: &mut Self::Tab) {
        match tab {
            TabKind::Chart => {
                let a = ui::chart_plugin::chart_panel(
                    ui,
                    self.chart_state,
                    self.pool,
                    self.frame_data,
                    self.running,
                );
                if a == ui::chart_plugin::PanelAction::OpenTree {
                    *self.open_tree = Some(DockTab::Chart);
                }
            }
            TabKind::Table => {
                let a =
                    ui::table_plugin::table_panel(ui, self.table_state, self.pool, self.frame_data);
                if a == ui::table_plugin::PanelAction::OpenTree {
                    *self.open_tree = Some(DockTab::Table);
                }
            }
        }
    }
}

fn bottom_sheet_handle(
    ui: &mut egui::Ui,
    drag_state: &mut Option<(f32, f32)>,
    current_h: f32,
) -> f32 {
    let mut w = ui.available_width();
    if !w.is_finite() || w <= 0.0 {
        w = ui.ctx().screen_rect().width(); // 如果不正常，回退到屏幕宽度
    }

    let (rect, response) = ui.allocate_exact_size(egui::vec2(w, 20.0), egui::Sense::drag());

    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }

    let handle_color = if response.dragged() {
        ui.visuals().widgets.active.bg_fill
    } else if response.hovered() {
        ui.visuals().widgets.hovered.bg_fill
    } else if ui.visuals().dark_mode {
        egui::Color32::from_gray(120)
    } else {
        egui::Color32::from_gray(200)
    };

    let capsule = egui::Rect::from_center_size(rect.center(), egui::vec2(40.0, 4.0));
    ui.painter()
        .rect_filled(capsule, egui::CornerRadius::same(2), handle_color);

    if response.dragged() {
        if let Some(pointer) = response.interact_pointer_pos() {
            if drag_state.is_none() {
                *drag_state = Some((pointer.y, current_h));
            }
            let (origin_y, initial_h) = drag_state.unwrap();
            let displacement = origin_y - pointer.y;
            return initial_h + displacement;
        }
        current_h
    } else {
        *drag_state = None;
        current_h
    }
}

impl eframe::App for MemRW3App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let running = self.session.is_running();
        if running {
            ui.ctx().request_repaint();
        }

        let cycles = self.acq_cycle_count.load(Ordering::Relaxed);
        let elapsed = self.hz_last_time.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            self.session.sampling_hz = (cycles - self.hz_last_cycles) as f64 / elapsed;
            self.hz_last_cycles = cycles;
            self.hz_last_time = Instant::now();
        }

        let mut frame_data: FrameData = HashMap::new();
        if running {
            for var in self.session.pool.iter() {
                let drained = var.incoming.drain();
                if !drained.is_empty() {
                    frame_data.insert(var.id, drained);
                }
            }
        }

        let total_h = ui.available_height();
        let bs_open = self.session.active_bottom_sheet.is_some();
        let dialog_open = self.table_state.show_entry_dialog;
        let running = self.session.is_running();

        egui::Frame::NONE
            .fill(if ui.visuals().dark_mode { Color32::from_rgb(35, 35, 38) } else { Color32::from_rgb(230, 230, 230) })
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(180, 180, 200)))
            .corner_radius(2)
            .show(ui, |ui| {
        ui.vertical(|ui| {
            ui.add_enabled_ui(!bs_open && !dialog_open, |ui| {
                ui::control_bar(ui, self);
            });

            let remaining = ui.available_height();
            let dock_h = remaining;

            if bs_open {}
            if dock_h > 0.0 {

                let mut open_tree = self.session.active_bottom_sheet;
                let mut viewer = TabViewerCtx {
                    chart_state: &mut self.chart_state,
                    table_state: &mut self.table_state,
                    pool: &self.session.pool,
                    frame_data: &frame_data,
                    running,
                    open_tree: &mut open_tree,
                };
                DockArea::new(&mut self.tree)
                    .style(egui_dock::Style::from_egui(ui.style()))
                    .show_close_buttons(false)
                    .show_leaf_collapse_buttons(false)
                    .show_inside(ui, &mut viewer);

                self.session.active_bottom_sheet = open_tree;

                let removed_chart: Vec<usize> = self.chart_state.removed_var_ids.drain(..).collect();
                for var_id in removed_chart {
                    self.unbind_variable(var_id);
                }
                let removed_table: Vec<usize> = self.table_state.removed_var_ids.drain(..).collect();
                for var_id in removed_table {
                    self.unbind_variable(var_id);
                }
                let writes: Vec<(usize, u64)> = self.table_state.pending_writes.drain(..).collect();
                for (var_id, value) in writes {
                    let ok = self.write_variable(var_id, value);
                    if ok {
                        self.toasts.success("写入成功").duration(Some(Duration::from_secs(2)));
                    } else {
                        self.toasts.error("写入失败").duration(Some(Duration::from_secs(3)));
                    }
                }
                if let Some(ref msg) = self.table_state.status_message {
                    if self.table_state.status_error {
                        self.toasts.error(msg.clone()).duration(Some(Duration::from_secs(3)));
                    }
                    self.table_state.status_message = None;
                }
                if self.chart_state.reset_timer {
                    self.chart_state.reset_timer = false;
                    self.clear_all_buffers();
                }
                if self.chart_state.log_started {
                    self.chart_state.log_started = false;
                    self.toasts.success("Log 开始").duration(Some(Duration::from_secs(2)));
                }
                if self.chart_state.log_stopped {
                    self.chart_state.log_stopped = false;
                    self.toasts.success("Log 停止").duration(Some(Duration::from_secs(2)));
                }
            }

            if bs_open {
                let bs_id = egui::Id::new("bottom_sheet");
                let window_w = ui.ctx().viewport_rect().width();
                let window_h = ui.ctx().viewport_rect().height();

                egui::Area::new("modal_overlay".into())
                    .fixed_pos(ui.ctx().viewport_rect().min)
                    .show(ui.ctx(), |ui| {
                        ui.painter().rect_filled(
                            ui.ctx().viewport_rect(),
                            0.0,
                            egui::Color32::from_black_alpha(100),
                        );
                        if ui.interact(ui.ctx().viewport_rect(), ui.next_auto_id(), egui::Sense::click()).clicked() {
                            self.session.active_bottom_sheet = None;
                        }
                    });
             
                egui::Area::new(bs_id)
                    .anchor(egui::Align2::LEFT_BOTTOM, egui::Vec2::ZERO)
                    .fixed_pos(egui::pos2(0.0, ui.viewport_rect().bottom()))
                    .order(egui::Order::Foreground)
                    .constrain(true)
                    .show(ui.ctx(), |ui| {
                        ui.set_width(window_w);

                        let card_bg = ui.visuals().window_fill();
                        let card_stroke = ui.visuals().window_stroke();

                        let target_tab = self.session.active_bottom_sheet;
                        egui::Frame::NONE
                            .fill(card_bg)
                            .stroke(card_stroke)
                            .corner_radius(egui::CornerRadius { nw: 16, ne: 16, sw: 0, se: 0 })
                            .show(ui, |ui| {
                            let target = bottom_sheet_handle(
                                ui,
                                &mut self.session.bottom_sheet_drag,
                                self.session.bottom_sheet_height,
                            );
                            self.session.bottom_sheet_height = target.clamp(window_h * 0.3, window_h * 0.8);
                            ui.set_height(self.session.bottom_sheet_height);
                            egui::Frame::NONE
                                .inner_margin(egui::Margin {
                                    left: 14,   // 左右给大一点边距，更美观
                                    right: 14,
                                    top: 26,     // 上面边距稍微收紧
                                    bottom: 10,
                                })
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label("ELF 文件:");
                                        ui.add_sized(
                                            [ui.available_width() - 200.0, 20.0],
                                            egui::TextEdit::singleline(&mut self.elf_path)
                                                .hint_text("输入 firmware.elf 路径..."),
                                        );
                                        if ui.button("浏览").clicked() {
                                            if let Some(path) = rfd::FileDialog::new()
                                                .add_filter("ELF/AXF", &["elf", "axf"])
                                                .add_filter("全部", &["*"])
                                                .pick_file()
                                            {
                                                self.elf_path = path.display().to_string();
                                            }
                                        }
                                        if ui.button("加载").clicked() { self.load_elf(); }
                                        if ui.button("追踪").clicked() {
                                            self.trace_variables();
                                        }
                                        if let Some(ref err) = self.session.load_error {
                                            self.toasts.error(err.clone()).duration(Some(Duration::from_secs(8))).closable(true);
                                            self.session.load_error = None;
                                        }
                                    });
                                    ui.add_space(4.0);
                                    ui.separator();
                                    ui.add_space(2.0);
                                    ui.horizontal(|ui| {
                                        ui.heading("变量列表 (DWARF Tree)");
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.button("关闭").clicked() { self.session.active_bottom_sheet = None; }
                                        });
                                    });
                                    ui.add_space(4.0);
                                    ui.separator();
                                    ui.add_space(4.0);
                                    let rem_h = ui.available_height().max(0.0);
                                    let total_w = ui.available_width().max(0.0);
                                    let right_w = (total_w * 0.32).clamp(220.0, 350.0);
                                    let left_w = (total_w - right_w - 8.0).max(200.0);
                                    ui.horizontal(|ui| {
                                        let (left_rect, _) = ui.allocate_exact_size(egui::vec2(left_w, rem_h), egui::Sense::hover());
                                        let mut left_ui = ui.new_child(egui::UiBuilder::new().max_rect(left_rect).layout(egui::Layout::top_down(egui::Align::Min)));
                                        egui::ScrollArea::both()
                                            .id_salt("left_tree_scroll")
                                            .auto_shrink([false, false]) 
                                            .show(&mut left_ui, |ui| {
                                                ui::vari_tree_ui(ui, &mut self.dwarf_app);
                                            });
                                        ui.separator();
                                        let (right_rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), rem_h), egui::Sense::hover());
                                        let mut right_ui = ui.new_child(egui::UiBuilder::new().max_rect(right_rect).layout(egui::Layout::top_down(egui::Align::Min)));
                                        egui::ScrollArea::both()
                                            .id_salt("right_props_scroll")
                                            .auto_shrink([false, false])
                                            .show(&mut right_ui, |ui| {
                                                
                                                let selected = self.dwarf_app.selected_node.clone();
                                                if let Some(ref node) = selected {
                                                    let node_id = node.id;
                                                    let node_size = node.size;
                                                    let node_basic_type = node.basic_type.clone();
                                                    let default_type = crate::types::basic_type_to_extend(&node_basic_type);
                                                    let config = self.session.extend_configs.entry(node_id).or_insert_with(|| crate::types::ExtendConfig {
                                                        name: String::new(), address: 0, ext_type: default_type, size: node_size, array_index: None, array_count: None,
                                                    });
                                                    if let Some((count, elem_size)) = self.dwarf_app.parent_array_info(node_id) {
                                                        config.array_count = Some(count);
                                                        if node.name.starts_with('[') {
                                                            if let Ok(parsed) = node.name[1..node.name.len()-1].parse::<u64>() {
                                                                if parsed < count && config.array_index != Some(parsed) {
                                                                    config.array_index = Some(parsed);
                                                                }
                                                            }
                                                        }
                                                        if config.array_index.is_none() { config.array_index = Some(0); }
                                                        let idx = config.array_index.unwrap_or(0);
                                                        let new_name = format!("[{}]", idx);
                                                        let new_addr = elem_size * idx;
                                                        if let Some(tree_node) = self.dwarf_app.find_node_mut(node_id) {
                                                            tree_node.name = new_name.clone();
                                                            tree_node.address = new_addr;
                                                        }
                                                        self.dwarf_app.selected_node.as_mut().map(|sel| { sel.name = new_name; sel.address = new_addr; });
                                                        config.name = self.dwarf_app.compute_extend_name(node_id);
                                                        config.address = self.dwarf_app.compute_extend_address(node_id).unwrap_or(0);
                                                    } else {
                                                        if config.name.is_empty() {
                                                            config.name = self.dwarf_app.compute_extend_name(node_id);
                                                            config.address = self.dwarf_app.compute_extend_address(node_id).unwrap_or(0);
                                                        }
                                                    }
                                                    let already_exists = self
                                                        .session
                                                        .pool
                                                        .find_by_name_addr(&config.name, config.address);
                                                    let (var_id, is_new_var) = if let Some(var) = already_exists {
                                                        (var.id, false)
                                                    } else {
                                                        let id = self.session.pool.add(config);
                                                        self.session.selected_variables.insert(id);
                                                        (id, true)
                                                    };
                                                    let color_id = ui.make_persistent_id(format!("chart_add_color_{}", node.id));
                                                    let name_id = ui.make_persistent_id(format!("chart_add_name_{}", node.id));
                                                    let name_default_id = ui.make_persistent_id(format!("chart_add_name_default_{}", node.id));
                                                    let table_name_id = ui.make_persistent_id(format!("table_add_name_{}", node.id));
                                                    let table_name_default_id = ui.make_persistent_id(format!("table_add_name_default_{}", node.id));
                                                    let mut chart_color = ui.data_mut(|d| *d.get_temp_mut_or(color_id, Color32::from_rgb(66,133,244)));
                                                    let default_add_name = format!("{} @ 0x{:X}", config.name, config.address);
                                                    let mut chart_curve_name = add_name_value(
                                                        ui,
                                                        name_id,
                                                        name_default_id,
                                                        &default_add_name,
                                                    );
                                                    let mut table_display_name = add_name_value(
                                                        ui,
                                                        table_name_id,
                                                        table_name_default_id,
                                                        &default_add_name,
                                                    );
                                                    let added = match target_tab {
                                                        Some(DockTab::Chart) => {
                                                            let result = ui::vari_properties_ui(ui, node, config, |ui, node_name| {
                                                                ui::chart_plugin::chart_add_config_ui(ui, node_name, &mut chart_curve_name, &mut chart_color);
                                                                ui.button("添加到 Chart").clicked()
                                                            });
                                                            ui.data_mut(|d| {
                                                                d.insert_temp(color_id, chart_color);
                                                                d.insert_temp(name_id, chart_curve_name.clone());
                                                            });
                                                            if result {
                                                                self.chart_state.add_legend(
                                                                    var_id,
                                                                    &self.session.pool,
                                                                    std::mem::take(&mut chart_curve_name),
                                                                    chart_color,
                                                                );
                                                            }
                                                            result
                                                        }
                                                        Some(DockTab::Table) => {
                                                            let result = ui::vari_properties_ui(ui, node, config, |ui, node_name| {
                                                                ui::table_plugin::table_add_config_ui(ui, node_name, &mut table_display_name);
                                                                ui.button("添加到 Table").clicked()
                                                            });
                                                            ui.data_mut(|d| { d.insert_temp(table_name_id, table_display_name.clone()); });
                                                            if result {
                                                                self.table_state.add_entry(
                                                                    var_id,
                                                                    &self.session.pool,
                                                                    std::mem::take(&mut table_display_name),
                                                                );
                                                            }
                                                            result
                                                        }
                                                        None => false,
                                                    };
                                                    if added {
                                                        if let Some(var) = self.session.pool.get_mut(var_id) {
                                                            var.plugins_cnt += 1;
                                                        }
                                                    } else if is_new_var {
                                                        self.session.pool.remove(var_id);
                                                        self.session.selected_variables.remove(&var_id);
                                                    }
                                                    if added && is_new_var {
                                                        self.push_slot_for_new_var(var_id);
                                                    }
                                                    // Re-sync tree/selected_node after vari_properties_ui
                                                    // (DragValue may have changed array_index)
                                                    {
                                                        let par = self.dwarf_app.parent_array_info(node_id);
                                                        if let Some((_count, elem_size)) = par {
                                                            let cfg = self.session.extend_configs.get(&node_id);
                                                            if let Some(cfg) = cfg {
                                                                let idx = cfg.array_index.unwrap_or(0);
                                                                let new_name = format!("[{}]", idx);
                                                                let new_addr = elem_size * idx;
                                                                if let Some(tree_node) = self.dwarf_app.find_node_mut(node_id) {
                                                                    tree_node.name = new_name.clone();
                                                                    tree_node.address = new_addr;
                                                                }
                                                                self.dwarf_app.selected_node.as_mut().map(|sel| { sel.name = new_name; sel.address = new_addr; });
                                                            }
                                                        }
                                                    }
                                                } else { ui.label("选择节点以查看属性"); }
                                            });
                                    });
                                });
                        });
                    });
            }
        });
        });
        self.toasts.show(ui.ctx());
    }
}

fn add_name_value(
    ui: &mut Ui,
    value_id: egui::Id,
    default_id: egui::Id,
    default_name: &str,
) -> String {
    ui.data_mut(|data| {
        let previous_default = data.get_temp::<String>(default_id);
        let mut value = data
            .get_temp::<String>(value_id)
            .unwrap_or_else(|| default_name.to_owned());
        if value.is_empty() || previous_default.as_deref() != Some(default_name) {
            value = default_name.to_owned();
        }
        data.insert_temp(default_id, default_name.to_owned());
        value
    })
}

#[derive(Serialize, Deserialize)]
struct SaveConfig {
    elf_path: String,
    probe_chip: String,
    probe_protocol: String,
    probe_speed_khz: u32,
    variables: Vec<SavedVariable>,
    chart_legends: Vec<SavedChartLegend>,
    table_entries: Vec<SavedTableEntry>,
}

#[derive(Serialize, Deserialize)]
struct SavedVariable {
    name: String,
    address: u64,
    ext_type: String,
    size: u32,
}

#[derive(Serialize, Deserialize)]
struct SavedChartLegend {
    variable_name: String,
    variable_address: u64,
    curve_name: String,
    color: [u8; 4],
    visible: bool,
    buffer_size: usize,
}

#[derive(Serialize, Deserialize)]
struct SavedTableEntry {
    variable_name: String,
    variable_address: u64,
    display_name: String,
}

impl MemRW3App {
    pub fn save_config(&mut self) {
        let path = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .set_file_name("memrw3_config.json")
            .save_file();
        let Some(path) = path else { return };

        let config = SaveConfig {
            elf_path: self.elf_path.clone(),
            probe_chip: self.session.probe_chip.clone(),
            probe_protocol: self.session.probe_protocol.clone(),
            probe_speed_khz: self.session.probe_speed_khz,
            variables: self
                .session
                .pool
                .iter()
                .map(|v| SavedVariable {
                    name: v.name.clone(),
                    address: v.address,
                    ext_type: format!("{:?}", v.ext_type),
                    size: v.size,
                })
                .collect(),
            chart_legends: self
                .chart_state
                .legends
                .iter()
                .map(|l| {
                    let v = self.session.pool.get(l.variable_id);
                    SavedChartLegend {
                        variable_name: v.map(|v| v.name.clone()).unwrap_or_default(),
                        variable_address: v.map(|v| v.address).unwrap_or(0),
                        curve_name: l.curve_name.clone(),
                        color: [l.color.r(), l.color.g(), l.color.b(), l.color.a()],
                        visible: l.visible,
                        buffer_size: l.buffer_size,
                    }
                })
                .collect(),
            table_entries: self
                .table_state
                .entries
                .iter()
                .map(|e| {
                    let v = self.session.pool.get(e.variable_id);
                    SavedTableEntry {
                        variable_name: v.map(|v| v.name.clone()).unwrap_or_default(),
                        variable_address: v.map(|v| v.address).unwrap_or(0),
                        display_name: e.display_name.clone(),
                    }
                })
                .collect(),
        };

        if let Ok(json) = serde_json::to_string_pretty(&config) {
            std::fs::write(&path, json).ok();
            self.toasts.success("配置已保存").duration(Some(Duration::from_secs(2)));
        }
    }

    pub fn load_config(&mut self) {
        let path = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .pick_file();
        let Some(path) = path else { return };

        let Ok(json) = std::fs::read_to_string(&path) else {
            self.toasts.error("读取配置文件失败").duration(Some(Duration::from_secs(3)));
            return;
        };
        let config: SaveConfig = match serde_json::from_str(&json) {
            Ok(c) => c,
            Err(e) => {
                self.toasts.error(format!("解析 JSON 失败: {e}")).duration(Some(Duration::from_secs(5)));
                return;
            }
        };

        self.session.probe_chip = config.probe_chip;
        self.session.probe_protocol = config.probe_protocol;
        self.session.probe_speed_khz = config.probe_speed_khz;
        self.elf_path = config.elf_path;

        self.session.pool = VariablePool::default();
        self.chart_state.legends.clear();
        self.table_state.entries.clear();

        for sv in &config.variables {
            let ext_type = match sv.ext_type.as_str() {
                "U8" => crate::types::ExtendType::U8,
                "U16" => crate::types::ExtendType::U16,
                "U32" => crate::types::ExtendType::U32,
                "U64" => crate::types::ExtendType::U64,
                "I8" => crate::types::ExtendType::I8,
                "I16" => crate::types::ExtendType::I16,
                "I32" => crate::types::ExtendType::I32,
                "I64" => crate::types::ExtendType::I64,
                "Float" => crate::types::ExtendType::Float,
                "Double" => crate::types::ExtendType::Double,
                _ => crate::types::ExtendType::Other,
            };
            let c = crate::types::ExtendConfig {
                name: sv.name.clone(),
                address: sv.address,
                ext_type,
                size: sv.size,
                array_index: None,
                array_count: None,
            };
            self.session.pool.add(&c);
        }

        for sl in &config.chart_legends {
            let var_id = self
                .session
                .pool
                .find_by_name_addr(&sl.variable_name, sl.variable_address)
                .map(|v| v.id);
            match var_id {
                Some(id) => {
                    let mut legend = crate::ui::chart_plugin::ChartLegend::new(id, sl.curve_name.clone());
                    legend.color = Color32::from_rgba_premultiplied(sl.color[0], sl.color[1], sl.color[2], sl.color[3]);
                    legend.visible = sl.visible;
                    legend.buffer_size = sl.buffer_size;
                    self.chart_state.legends.push(legend);
                    if let Some(var) = self.session.pool.get_mut(id) {
                        var.plugins_cnt += 1;
                    }
                }
                None => {
                    self.session.pool = VariablePool::default();
                    self.chart_state.legends.clear();
                    self.table_state.entries.clear();
                    self.toasts
                        .error(format!("图表变量 \"{}\" 匹配失败", sl.variable_name))
                        .duration(Some(Duration::from_secs(10)))
                        .closable(true);
                    return;
                }
            }
        }

        for se in &config.table_entries {
            let var_id = self
                .session
                .pool
                .find_by_name_addr(&se.variable_name, se.variable_address)
                .map(|v| v.id);
            match var_id {
                Some(id) => {
                    let mut entry = crate::ui::table_plugin::TableEntry::new(id, se.display_name.clone());
                    entry.display_name = se.display_name.clone();
                    self.table_state.entries.push(entry);
                    if let Some(var) = self.session.pool.get_mut(id) {
                        var.plugins_cnt += 1;
                    }
                }
                None => {
                    self.session.pool = VariablePool::default();
                    self.chart_state.legends.clear();
                    self.table_state.entries.clear();
                    self.toasts
                        .error(format!("表格变量 \"{}\" 匹配失败", se.variable_name))
                        .duration(Some(Duration::from_secs(10)))
                        .closable(true);
                    return;
                }
            }
        }

        self.toasts.success("配置已加载").duration(Some(Duration::from_secs(2)));
        self.trace_variables();
    }
}

pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    
    if let Some((name, data, path)) = load_chinese_font() {
        println!("✅ 使用字体: {}", path);
        fonts.font_data.insert(name.clone(), data);
        
        // 添加为备选字体，不覆盖默认英文字体
        fonts.families.entry(egui::FontFamily::Proportional).or_default().push(name.clone());
        fonts.families.entry(egui::FontFamily::Monospace).or_default().push(name);
    } else {
        println!("⚠️ 未找到中文字体，中文可能无法显示\n💡 Linux: sudo apt install fonts-noto-cjk");
    }
    
    ctx.set_fonts(fonts);
}

fn load_chinese_font() -> Option<(String, Arc<egui::FontData>, String)> {
    get_font_paths()
        .into_iter()
        .find_map(|path| {
            let path = std::path::PathBuf::from(path);
            path.exists().then(|| {
                std::fs::read(&path).ok().map(|bytes| {
                    ("chinese_font".to_owned(), Arc::new(egui::FontData::from_owned(bytes)), path.display().to_string())
                })
            }).flatten()
        })
        .or_else(scan_font_directories)
}

fn get_font_paths() -> Vec<String> {
    let mut paths = Vec::new();
    
    #[cfg(target_os = "windows")]
    paths.extend([
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyh.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
    ].map(String::from));
    
    #[cfg(target_os = "macos")]
    paths.extend([
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/Library/Fonts/NotoSansCJK.ttc",
    ].map(String::from));
    
    #[cfg(target_os = "linux")]
    {
        paths.extend([
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
            "/usr/share/fonts/truetype/arphic/uming.ttc",
        ].map(String::from));
        
        if let Ok(home) = std::env::var("HOME") {
            paths.push(format!("{home}/.local/share/fonts/NotoSansCJK-Regular.ttc"));
            paths.push(format!("{home}/.fonts/NotoSansCJK-Regular.ttc"));
        }
    }
    
    paths
}

#[cfg(target_os = "linux")]
fn scan_font_directories() -> Option<(String, Arc<egui::FontData>, String)> {
    const KEYWORDS: &[&str] = &["noto", "cjk", "wqy", "droid", "arphic", "uming", "microhei", "song", "hei"];
    const VALID_EXTS: &[&str] = &["ttf", "ttc", "otf"];
    
    for dir in ["/usr/share/fonts", "/usr/local/share/fonts"] {
        if let Some(font) = find_font(dir, KEYWORDS, VALID_EXTS) {
            return Some(font);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn find_font(dir: &str, keywords: &[&str], valid_exts: &[&str]) -> Option<(String, Arc<egui::FontData>, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return None };
    
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        
        if path.is_dir() {
            if let Some(found) = find_font(path.to_str()?, keywords, valid_exts) {
                return Some(found);
            }
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if valid_exts.contains(&ext) {
                let name = path.file_name()?.to_str()?.to_lowercase();
                if keywords.iter().any(|kw| name.contains(kw)) {
                    if let Ok(bytes) = std::fs::read(&path) {
                        return Some((
                            "scanned_font".to_owned(),
                            Arc::new(egui::FontData::from_owned(bytes)),
                            path.display().to_string(),
                        ));
                    }
                }
            }
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn scan_font_directories() -> Option<(String, Arc<egui::FontData>, String)> {
    None
}