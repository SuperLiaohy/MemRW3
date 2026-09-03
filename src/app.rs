use crate::dwarf;
use crate::model::{AppSession, RegisterData, RegisterReadResult, VariablePool};
use crate::probe::{AcqSlot, ProbeCell, ProbeSession, VarSlotMapping};
use crate::sync::Sync;
use crate::ui;
use crate::ui::chart_plugin::ChartPluginState;
use crate::ui::dock::DockLayoutState;
use crate::ui::plugin::{
    FrameData, MemRWPlugin, PluginAction, PluginUpdateContext, SavedPluginConfig, ToastLevel,
};
use crate::ui::table_plugin::TablePluginState;
use crate::ui::variable_tree_panel::VariableTreePanel;
use eframe::egui;
use egui::Ui;
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
pub struct MemRW3App {
    dock: DockLayoutState,
    pub session: AppSession,
    variable_tree: VariableTreePanel,
    plugins: Vec<Box<dyn MemRWPlugin>>,
    probe: Arc<ProbeCell>,
    sync: Arc<Sync>,
    pub toasts: egui_notify::Toasts,
    frame_data: FrameData,
    register_data: RegisterData,
    register_sequence: u64,
    link_event_receiver: std::sync::mpsc::Receiver<String>,
    flash_task: Option<FlashTask>,
    rebuild_after_flash: bool,
    _acq_handle: Option<JoinHandle<()>>,
}

struct FlashTask {
    receiver: std::sync::mpsc::Receiver<Result<(), String>>,
    file_name: String,
    handle: Option<JoinHandle<()>>,
}

fn acq_thread(
    probe: Arc<ProbeCell>,
    running: Arc<AtomicBool>,
    delay_us: Arc<AtomicU64>,
    cycle_count: Arc<AtomicU64>,
    sync: Arc<Sync>,
    stop: Arc<AtomicBool>,
    link_event_sender: std::sync::mpsc::SyncSender<String>,
    repaint_ctx: egui::Context,
) {
    const LINK_CHECK_INTERVAL: Duration = Duration::from_millis(500);
    let mut link_monitor = LinkHealthMonitor::default();
    let mut last_link_check = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        sync.try_acquire();

        if stop.load(Ordering::Relaxed) {
            return;
        }

        let acquisition_running = running.load(Ordering::Acquire);
        let health_check_due = last_link_check.elapsed() >= LINK_CHECK_INTERVAL;
        let (connected, slots_empty, acquired, health_result) = probe.with_mut(|probe_ref| {
            if !probe_ref.connected {
                return (false, true, false, None);
            }
            let slots_empty = probe_ref.slots.is_empty();
            let acquisition_result = if acquisition_running && !slots_empty {
                Some(probe_ref.acquire_from_slots())
            } else {
                None
            };
            let acquired = matches!(acquisition_result, Some(Ok(())));
            let acquisition_failed = matches!(acquisition_result, Some(Err(_)));
            let should_check =
                health_check_due && (!acquisition_running || slots_empty || acquisition_failed);
            let health_result = should_check.then(|| probe_ref.check_link());
            (true, slots_empty, acquired, health_result)
        });
        if !connected {
            link_monitor.reset();
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        if let Some(health_result) = health_result {
            last_link_check = Instant::now();
            if let Some(error) = link_monitor.observe(health_result) {
                probe.with_mut(ProbeSession::disconnect);
                running.store(false, Ordering::Release);
                let _ = link_event_sender.try_send(error);
                repaint_ctx.request_repaint();
                continue;
            }
        }

        if acquired {
            cycle_count.fetch_add(1, Ordering::Relaxed);
        }
        if acquisition_running && !slots_empty && acquired {
            let d = delay_us.load(Ordering::Acquire);
            if d > 0 {
                thread::sleep(Duration::from_micros(d));
            }
        } else {
            thread::sleep(Duration::from_millis(50));
        }
    }
}

#[derive(Default)]
struct LinkHealthMonitor {
    consecutive_failures: u8,
}

impl LinkHealthMonitor {
    const FAILURE_LIMIT: u8 = 3;

    fn observe(&mut self, result: Result<(), String>) -> Option<String> {
        match result {
            Ok(()) => {
                self.reset();
                None
            }
            Err(error) => {
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                (self.consecutive_failures >= Self::FAILURE_LIMIT).then_some(error)
            }
        }
    }

    fn reset(&mut self) {
        self.consecutive_failures = 0;
    }
}

impl MemRW3App {
    fn default_plugins() -> Vec<Box<dyn MemRWPlugin>> {
        vec![
            Box::new(ChartPluginState::default()),
            Box::new(TablePluginState::default()),
        ]
    }

    pub fn new(dwarf_state: dwarf::types::DwarfState, repaint_ctx: egui::Context) -> Self {
        let mut session = AppSession::default();
        let mut chips: Vec<String> = probe_rs::config::Registry::from_builtin_families()
            .families()
            .iter()
            .flat_map(|f| f.variants.iter().map(|v| v.name.clone()))
            .collect();
        chips.sort();
        session.all_chips = chips;

        let probe = Arc::new(ProbeCell::new(ProbeSession::default()));
        let sync = Arc::new(Sync::new());

        let acq_probe = probe.clone();
        let acq_sync = sync.clone();
        let acq_running = session.running.clone();
        let acq_cycles = session.acq_cycle_count.clone();
        let acq_stop_th = session.acq_stop.clone();
        let delay_us = session.config.delay_us.clone();
        let (link_event_sender, link_event_receiver) = std::sync::mpsc::sync_channel(1);
        let _acq_handle = Some(thread::spawn(move || {
            acq_thread(
                acq_probe,
                acq_running,
                delay_us,
                acq_cycles,
                acq_sync,
                acq_stop_th,
                link_event_sender,
                repaint_ctx,
            );
        }));

        Self {
            dock: DockLayoutState::default(),
            session,
            variable_tree: VariableTreePanel::new(dwarf_state),
            plugins: Self::default_plugins(),
            probe,
            sync,
            toasts: egui_notify::Toasts::default().with_anchor(egui_notify::Anchor::BottomRight),
            frame_data: FrameData::default(),
            register_data: RegisterData::default(),
            register_sequence: 0,
            link_event_receiver,
            flash_task: None,
            rebuild_after_flash: false,
            _acq_handle,
        }
    }

    pub fn sync_connect(&mut self) {
        let chip = self.session.config.probe_chip.clone();
        let protocol = self.session.config.probe_protocol.clone();
        let speed = self.session.config.probe_speed_khz;
        let probe_id = self.session.probe_id.clone();
        let probe = self.probe.clone();
        let connected = self.session.connected;

        if connected {
            self.session.set_running(false);
            self.sync.send_request(move || {
                probe.with_mut(ProbeSession::disconnect);
            });
            self.session.connected = false;
            self.session.timer_was_started = false;
            self.session.connect_error = None;
            self.toasts
                .info("已断开连接")
                .duration(Some(Duration::from_secs(5)))
                .closable(true);
        } else {
            for var in self.session.config.pool.iter() {
                var.incoming.discard_all();
            }
            let sync = self.sync.clone();
            let running = self.session.running.clone();
            let connection_result = sync.send_request(move || {
                probe.with_mut(|probe| {
                    probe.chip_name = chip;
                    probe.protocol = protocol;
                    probe.speed_khz = speed;
                    probe.selected_probe_id = probe_id;
                    let connected = probe.connect();
                    if !connected {
                        running.store(false, Ordering::Release);
                    }
                    (
                        connected,
                        probe.chip_name.clone(),
                        probe.speed_khz,
                        probe.protocol.clone(),
                        probe.selected_probe_id.clone(),
                        probe.last_error.clone(),
                    )
                })
            });
            let (connected, chip_name, speed_khz, protocol, selected_probe_id, last_error) =
                connection_result;
            self.session.connected = connected;
            self.toasts
                .info(format!(
                    "连接配置: chip:{},freq:{},protocol:{},id:{}",
                    chip_name,
                    speed_khz,
                    protocol,
                    selected_probe_id.as_deref().unwrap_or("auto")
                ))
                .duration(Some(Duration::from_secs(5)))
                .closable(true);
            if !self.session.connected {
                let err = last_error.unwrap_or_default();
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
            probe.with_mut(ProbeSession::reset_target);
        });
    }

    pub fn is_flashing(&self) -> bool {
        self.flash_task.is_some()
    }

    pub fn start_flash_firmware(&mut self, path: std::path::PathBuf) -> Result<(), String> {
        if !self.session.connected {
            return Err("请先连接目标设备".to_owned());
        }
        if self.flash_task.is_some() {
            return Err("已有烧录任务正在执行".to_owned());
        }

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("firmware")
            .to_owned();
        self.session.set_running(false);
        self.session.timer_was_started = false;
        self.reset_plugin_data();
        for variable in self.session.config.pool.iter() {
            variable.incoming.discard_all();
        }

        let probe = self.probe.clone();
        let sync = self.sync.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let handle = thread::spawn(move || {
            let outcome = sync.send_request(|| probe.with_mut(|probe| probe.flash_firmware(&path)));
            let _ = sender.send(outcome);
        });

        self.flash_task = Some(FlashTask {
            receiver,
            file_name,
            handle: Some(handle),
        });
        Ok(())
    }

    fn poll_flash_task(&mut self) {
        let Some(task) = self.flash_task.as_ref() else {
            return;
        };
        let outcome = match task.receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("烧录线程意外结束".to_owned()),
        };
        let mut task = self.flash_task.take().unwrap();
        if let Some(handle) = task.handle.take() {
            let _ = handle.join();
        }

        for variable in self.session.config.pool.iter() {
            variable.incoming.discard_all();
        }
        match outcome {
            Ok(()) => {
                self.toasts
                    .success(format!("固件 {} 烧录并校验成功", task.file_name))
                    .duration(Some(Duration::from_secs(5)));
            }
            Err(error) => {
                self.toasts
                    .error(error)
                    .duration(Some(Duration::from_secs(8)))
                    .closable(true);
            }
        }
        if self.rebuild_after_flash {
            self.rebuild_after_flash = false;
            self.rebuild_slots();
        }
    }

    fn poll_link_events(&mut self) {
        while let Ok(error) = self.link_event_receiver.try_recv() {
            if !self.session.connected {
                continue;
            }
            self.session.set_running(false);
            self.session.connected = false;
            self.session.timer_was_started = false;
            self.session.connect_error = Some(error.clone());
            self.toasts
                .error(format!("Probe 物理链路已断开：{error}"))
                .duration(Some(Duration::from_secs(8)))
                .closable(true);
        }
    }

    pub fn reset_timer(&self) {
        let probe = self.probe.clone();
        self.sync.send_request(move || {
            probe.with_mut(|probe| probe.timer = Instant::now());
        });
    }

    fn reset_plugin_data(&mut self) {
        for plugin in &mut self.plugins {
            plugin.reset_data();
        }
    }

    fn reset_active_plugin_data(&mut self) {
        for plugin in &mut self.plugins {
            if !self.dock.is_plugin_paused(plugin.id()) {
                plugin.reset_data();
            }
        }
    }

    pub fn set_acquisition_running(&mut self, running: bool) {
        if !running {
            self.session.set_running(false);
            return;
        }
        if !self.session.connected || self.is_flashing() {
            return;
        }

        self.rebuild_slots();
        if !self.session.timer_was_started {
            for variable in self.session.config.pool.iter() {
                variable.incoming.discard_all();
            }
            self.reset_active_plugin_data();
            self.reset_timer();
            self.session.timer_was_started = true;
        }
        self.session.set_running(true);
    }

    pub fn clear_all_buffers(&mut self) {
        self.session.timer_was_started = self.session.is_running();
        self.reset_plugin_data();
        let pool = &self.session.config.pool;
        let probe = self.probe.clone();
        self.sync.send_request(move || {
            probe.with_mut(|probe| probe.timer = Instant::now());
            for var in pool.iter() {
                var.incoming.discard_all();
            }
        });
    }

    pub fn write_variable(&self, var_id: usize, value: u64) -> bool {
        let var = match self.session.config.pool.get(var_id) {
            Some(v) => v,
            None => return false,
        };
        let addr = var.address;
        let size = var.size;
        let probe = self.probe.clone();
        self.sync
            .send_request(|| probe.with_mut(|probe| probe.write_value(addr, size, value)))
    }

    pub fn rebuild_slots(&self) {
        let probe = self.probe.clone();
        let pool = &self.session.config.pool;
        let mut slot_map: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
        let mut slots: Vec<AcqSlot> = Vec::new();
        let mut mappings: Vec<VarSlotMapping> = Vec::new();

        for var in pool.iter() {
            if var.active_readers == 0 {
                continue;
            }
            let addrs = ProbeSession::slot_addresses(var.address, var.size);
            let byte_offset = (var.address & 3) as usize;
            let mut slot_indices: Vec<usize> = Vec::with_capacity(addrs.len());
            for addr in addrs {
                let slot_index = if let Some(&index) = slot_map.get(&addr) {
                    index
                } else {
                    let index = slots.len();
                    slots.push(AcqSlot { address: addr });
                    slot_map.insert(addr, index);
                    index
                };
                slot_indices.push(slot_index);
            }
            mappings.push(VarSlotMapping {
                slot_indices,
                size: var.size,
                byte_offset,
                incoming: var.incoming.clone(),
            });
        }

        let slot_n = slots.len() as u64;
        let sc = self.session.slot_count.clone();
        self.sync.send_request(move || {
            probe.with_mut(|probe| {
                probe.slots = slots;
                probe.slot_values.resize(probe.slots.len(), [0; 4]);
                probe.var_mappings = mappings;
            });
            sc.store(slot_n, Ordering::Relaxed);
        });
    }

    pub fn unbind_variable(&mut self, var_id: usize, was_enabled: bool) {
        self.session.config.pool.unbind(var_id, was_enabled);
    }

    fn handle_plugin_actions(&mut self, actions: Vec<PluginAction>) {
        let mut rebuild_slots = false;
        for action in actions {
            match action {
                PluginAction::OpenVariableTree {
                    plugin_id,
                    viewport_id,
                } => {
                    self.variable_tree.open(plugin_id, viewport_id);
                }
                PluginAction::RemoveVariable {
                    var_id,
                    was_enabled,
                } => {
                    self.unbind_variable(var_id, was_enabled);
                    rebuild_slots = true;
                }
                PluginAction::SetVariableEnabled { var_id, enabled } => {
                    if self
                        .session
                        .config
                        .pool
                        .set_binding_enabled(var_id, enabled)
                    {
                        if !enabled {
                            if let Some(variable) = self.session.config.pool.get(var_id) {
                                if variable.active_readers == 0 {
                                    variable.incoming.discard_all();
                                }
                            }
                        }
                        rebuild_slots = true;
                    }
                }
                PluginAction::WriteVariable { var_id, value } => {
                    if self.is_flashing() {
                        self.toasts
                            .error("固件烧录期间不能写变量")
                            .duration(Some(Duration::from_secs(3)));
                        continue;
                    }
                    let ok = self.write_variable(var_id, value);
                    if ok {
                        self.toasts
                            .success("写入成功")
                            .duration(Some(Duration::from_secs(2)));
                    } else {
                        self.toasts
                            .error("写入失败")
                            .duration(Some(Duration::from_secs(3)));
                    }
                }
                PluginAction::ReadRegisters { requests } => {
                    if requests.is_empty() || !self.session.connected || self.is_flashing() {
                        continue;
                    }
                    let probe = self.probe.clone();
                    let results = self.sync.send_request(move || {
                        probe.with_mut(|probe| probe.read_registers(&requests))
                    });
                    self.register_sequence = self.register_sequence.wrapping_add(1);
                    let sequence = self.register_sequence;
                    for (id, value) in results {
                        self.register_data
                            .insert(id, RegisterReadResult { sequence, value });
                    }
                }
                PluginAction::WriteRegister { request } => {
                    if !self.session.connected {
                        self.toasts
                            .error("请先连接目标设备")
                            .duration(Some(Duration::from_secs(3)));
                        continue;
                    }
                    if self.is_flashing() {
                        self.toasts
                            .error("固件烧录期间不能写寄存器")
                            .duration(Some(Duration::from_secs(3)));
                        continue;
                    }
                    let probe = self.probe.clone();
                    let ok = self.sync.send_request(move || {
                        probe.with_mut(|probe| {
                            probe.write_value(
                                request.address,
                                u32::from(request.size_bytes),
                                request.value,
                            )
                        })
                    });
                    if ok {
                        self.toasts
                            .success("寄存器写入成功")
                            .duration(Some(Duration::from_secs(2)));
                    } else {
                        self.toasts
                            .error("寄存器写入失败")
                            .duration(Some(Duration::from_secs(3)));
                    }
                }
                PluginAction::ResetTimer => {
                    if !self.is_flashing() {
                        self.clear_all_buffers();
                    }
                }
                PluginAction::RebuildSlots => {
                    rebuild_slots = true;
                }
                PluginAction::Toast { level, message } => {
                    self.show_plugin_toast(level, message);
                }
            }
        }
        if rebuild_slots {
            if self.is_flashing() {
                self.rebuild_after_flash = true;
            } else {
                self.rebuild_slots();
            }
        }
    }

    fn show_plugin_toast(&mut self, level: ToastLevel, message: String) {
        match level {
            ToastLevel::Success => {
                self.toasts
                    .success(message)
                    .duration(Some(Duration::from_secs(2)));
            }
            ToastLevel::Error => {
                self.toasts
                    .error(message)
                    .duration(Some(Duration::from_secs(3)));
            }
        }
    }
}

impl Drop for MemRW3App {
    fn drop(&mut self) {
        if let Some(mut task) = self.flash_task.take() {
            if let Some(handle) = task.handle.take() {
                let _ = handle.join();
            }
        }
        self.session.acq_stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self._acq_handle.take() {
            let _ = handle.join();
        }
    }
}

impl eframe::App for MemRW3App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.poll_link_events();
        self.poll_flash_task();
        if self.is_flashing() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        let running = self.session.is_running();
        if running {
            ui.ctx().request_repaint();
        }

        let cycles = self.session.acq_cycle_count.load(Ordering::Relaxed);
        let elapsed = self.session.hz_last_time.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            self.session.sampling_hz = (cycles - self.session.hz_last_cycles) as f64 / elapsed;
            self.session.hz_last_cycles = cycles;
            self.session.hz_last_time = Instant::now();
        }

        let mut frame_data = std::mem::take(&mut self.frame_data);
        frame_data.retain(|id, samples| {
            samples.clear();
            self.session.config.pool.contains(*id)
        });
        if running {
            for var in self.session.config.pool.iter() {
                let samples = frame_data.entry(var.id).or_default();
                var.incoming.drain_into(samples);
            }
        }
        let connected = self.session.connected;
        let hardware_busy = self.is_flashing();
        let mut update_actions = Vec::new();
        for plugin in &mut self.plugins {
            if self.dock.is_plugin_paused(plugin.id()) {
                continue;
            }
            update_actions.extend(plugin.update(PluginUpdateContext {
                pool: &self.session.config.pool,
                frame_data: &frame_data,
                register_data: &self.register_data,
                running,
                connected,
                hardware_busy,
                egui_ctx: ui.ctx(),
            }));
        }
        self.handle_plugin_actions(update_actions);

        let bs_open = self.variable_tree.is_open_in(ui.ctx().viewport_id());
        let dialog_open = self.plugins.iter().any(|plugin| plugin.is_dialog_open());
        let running = self.session.is_running();
        let interaction_enabled = !self.is_flashing();

        let colors = ui::theme::palette(ui);
        egui::Frame::NONE.fill(colors.app_bg).show(ui, |ui| {
            let shell_size = ui.available_size();
            let activity_w = 52.0;
            let (shell_rect, _) = ui.allocate_exact_size(shell_size, egui::Sense::hover());
            let activity_rect = egui::Rect::from_min_size(
                shell_rect.min,
                egui::vec2(activity_w, shell_rect.height()),
            );
            let right_rect = egui::Rect::from_min_size(
                egui::pos2(activity_rect.max.x, shell_rect.min.y),
                egui::vec2(
                    (shell_rect.width() - activity_w).max(0.0),
                    shell_rect.height(),
                ),
            );

            let mut activity_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(activity_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            activity_ui.set_clip_rect(activity_rect);
            ui::dock::show_plugin_activity_bar(&mut activity_ui, &mut self.dock, &mut self.plugins);

            let mut right_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(right_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            right_ui.set_clip_rect(right_rect);
            right_ui.vertical(|ui| {
                ui.add_enabled_ui(!bs_open && !dialog_open, |ui| {
                    ui::control_bar(ui, self);
                });

                let dock_h = ui.available_height();
                if dock_h > 0.0 {
                    let pool = &mut self.session.config.pool;
                    let actions = ui::dock::show_active_plugin_content(
                        ui,
                        &mut self.dock,
                        &mut self.plugins,
                        pool,
                        running,
                        interaction_enabled,
                        &mut self.variable_tree,
                    );
                    self.handle_plugin_actions(actions);
                }
            });

            let pool = &mut self.session.config.pool;
            let popout_actions = ui::dock::show_plugin_popouts(
                ui,
                &mut self.dock,
                &mut self.plugins,
                pool,
                running,
                interaction_enabled,
                &mut self.variable_tree,
            );
            self.handle_plugin_actions(popout_actions);
        });
        self.frame_data = frame_data;
        if let Some(task) = self.flash_task.as_ref() {
            firmware_flash_modal(ui.ctx(), &task.file_name);
        }
        self.toasts.show(ui.ctx());
    }
}

fn firmware_flash_modal(ctx: &egui::Context, file_name: &str) {
    egui::Modal::new(egui::Id::new("firmware_flash_modal")).show(ctx, |ui| {
        ui.set_min_width(280.0);
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(18.0));
            ui.vertical(|ui| {
                ui.strong("正在烧录并校验固件");
                ui.label(file_name);
            });
        });
        ui.add_space(6.0);
        ui.label("请保持目标板和调试器连接，完成后目标将自动复位。");
    });
}

#[derive(Serialize, Deserialize)]
struct SaveConfig {
    elf_path: String,
    probe_chip: String,
    probe_protocol: String,
    probe_speed_khz: u32,
    variables: Vec<SavedVariable>,
    plugins: Vec<SavedPluginConfig>,
}

#[derive(Serialize, Deserialize)]
struct SavedVariable {
    name: String,
    address: u64,
    ext_type: dwarf::types::ExtendType,
    size: u32,
}

impl MemRW3App {
    pub fn save_config(&mut self) {
        let path = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .set_file_name("memrw3_config.json")
            .save_file();
        let Some(path) = path else { return };

        let config = SaveConfig {
            elf_path: self.variable_tree.elf_path.clone(),
            probe_chip: self.session.config.probe_chip.clone(),
            probe_protocol: self.session.config.probe_protocol.clone(),
            probe_speed_khz: self.session.config.probe_speed_khz,
            variables: self
                .session
                .config
                .pool
                .iter()
                .map(|v| SavedVariable {
                    name: v.name.clone(),
                    address: v.address,
                    ext_type: v.ext_type.clone(),
                    size: v.size,
                })
                .collect(),
            plugins: self
                .plugins
                .iter()
                .map(|plugin| SavedPluginConfig {
                    plugin_id: plugin.id().to_owned(),
                    payload: plugin.save_config(&self.session.config.pool),
                })
                .collect(),
        };

        match serde_json::to_string_pretty(&config) {
            Ok(json) => match std::fs::write(&path, json) {
                Ok(()) => {
                    self.toasts
                        .success("配置已保存")
                        .duration(Some(Duration::from_secs(2)));
                }
                Err(e) => {
                    self.toasts
                        .error(format!("保存配置失败: {e}"))
                        .duration(Some(Duration::from_secs(5)));
                }
            },
            Err(e) => {
                self.toasts
                    .error(format!("序列化配置失败: {e}"))
                    .duration(Some(Duration::from_secs(5)));
            }
        }
    }

    pub fn load_config(&mut self) {
        if self.session.connected || self.session.is_running() || self.is_flashing() {
            self.toasts
                .error("请先停止采集并断开目标设备，再加载配置")
                .duration(Some(Duration::from_secs(5)));
            return;
        }
        let path = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .pick_file();
        let Some(path) = path else { return };

        let Ok(json) = std::fs::read_to_string(&path) else {
            self.toasts
                .error("读取配置文件失败")
                .duration(Some(Duration::from_secs(3)));
            return;
        };
        let config: SaveConfig = match serde_json::from_str(&json) {
            Ok(c) => c,
            Err(e) => {
                self.toasts
                    .error(format!("解析 JSON 失败: {e}"))
                    .duration(Some(Duration::from_secs(5)));
                return;
            }
        };

        let mut new_pool = VariablePool::default();
        for sv in &config.variables {
            let c = dwarf::types::ExtendConfig {
                name: sv.name.clone(),
                address: sv.address,
                ext_type: sv.ext_type.clone(),
                size: sv.size,
                array_index: None,
                array_count: None,
            };
            new_pool.add(&c);
        }

        let mut new_plugins = Self::default_plugins();
        let mut skipped_plugins = Vec::new();
        let mut seen_plugin_ids = std::collections::HashSet::new();
        for saved_plugin in &config.plugins {
            if !seen_plugin_ids.insert(saved_plugin.plugin_id.as_str()) {
                self.toasts
                    .error(format!("配置包含重复插件项: {}", saved_plugin.plugin_id))
                    .duration(Some(Duration::from_secs(8)));
                return;
            }
            match new_plugins
                .iter_mut()
                .find(|plugin| plugin.id() == saved_plugin.plugin_id)
            {
                Some(plugin) => {
                    if let Err(e) = plugin.load_config(&saved_plugin.payload, &mut new_pool) {
                        self.toasts
                            .error(e)
                            .duration(Some(Duration::from_secs(10)))
                            .closable(true);
                        return;
                    }
                }
                None => skipped_plugins.push(saved_plugin.plugin_id.clone()),
            }
        }

        let new_dwarf_state =
            match VariableTreePanel::prepare_config(&config.elf_path, &mut new_pool) {
                Ok(dwarf_state) => dwarf_state,
                Err(error) => {
                    self.toasts
                        .error(error)
                        .duration(Some(Duration::from_secs(10)))
                        .closable(true);
                    return;
                }
            };

        self.session.config.probe_chip = config.probe_chip;
        self.session.config.probe_protocol = config.probe_protocol;
        self.session.config.probe_speed_khz = config.probe_speed_khz;
        self.session.config.pool = new_pool;
        self.plugins = new_plugins;
        self.variable_tree
            .apply_config_source(config.elf_path, new_dwarf_state);

        for plugin_id in skipped_plugins {
            self.toasts
                .info(format!("跳过未知插件配置: {plugin_id}"))
                .duration(Some(Duration::from_secs(5)));
        }

        self.toasts
            .success("配置已加载")
            .duration(Some(Duration::from_secs(2)));
        self.rebuild_slots();
    }
}

pub fn setup_fonts(ctx: &egui::Context) {
    crate::ui::theme::install(ctx);

    let mut fonts = egui::FontDefinitions::default();

    if let Some((name, data, path)) = load_chinese_font() {
        println!("✅ 使用字体: {}", path);
        fonts.font_data.insert(name.clone(), data);

        // 添加为备选字体，不覆盖默认英文字体
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .push(name.clone());
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .push(name);
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
            path.exists()
                .then(|| {
                    std::fs::read(&path).ok().map(|bytes| {
                        (
                            "chinese_font".to_owned(),
                            Arc::new(egui::FontData::from_owned(bytes)),
                            path.display().to_string(),
                        )
                    })
                })
                .flatten()
        })
        .or_else(scan_font_directories)
}

fn get_font_paths() -> Vec<String> {
    let mut paths = Vec::new();

    #[cfg(target_os = "windows")]
    paths.extend(
        [
            r"C:\Windows\Fonts\msyh.ttc",
            r"C:\Windows\Fonts\msyh.ttf",
            r"C:\Windows\Fonts\simsun.ttc",
            r"C:\Windows\Fonts\simhei.ttf",
        ]
        .map(String::from),
    );

    #[cfg(target_os = "macos")]
    paths.extend(
        [
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
            "/Library/Fonts/NotoSansCJK.ttc",
        ]
        .map(String::from),
    );

    #[cfg(target_os = "linux")]
    {
        paths.extend(
            [
                "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
                "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
                "/usr/share/fonts/truetype/arphic/uming.ttc",
            ]
            .map(String::from),
        );

        if let Ok(home) = std::env::var("HOME") {
            paths.push(format!("{home}/.local/share/fonts/NotoSansCJK-Regular.ttc"));
            paths.push(format!("{home}/.fonts/NotoSansCJK-Regular.ttc"));
        }
    }

    paths
}

#[cfg(target_os = "linux")]
fn scan_font_directories() -> Option<(String, Arc<egui::FontData>, String)> {
    const KEYWORDS: &[&str] = &[
        "noto", "cjk", "wqy", "droid", "arphic", "uming", "microhei", "song", "hei",
    ];
    const VALID_EXTS: &[&str] = &["ttf", "ttc", "otf"];

    for dir in ["/usr/share/fonts", "/usr/local/share/fonts"] {
        if let Some(font) = find_font(dir, KEYWORDS, VALID_EXTS) {
            return Some(font);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn find_font(
    dir: &str,
    keywords: &[&str],
    valid_exts: &[&str],
) -> Option<(String, Arc<egui::FontData>, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return None;
    };

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

#[cfg(test)]
mod tests {
    use super::LinkHealthMonitor;

    #[test]
    fn link_monitor_requires_consecutive_failures_and_recovers_after_success() {
        let mut monitor = LinkHealthMonitor::default();
        assert!(monitor.observe(Err("first".to_owned())).is_none());
        assert!(monitor.observe(Err("second".to_owned())).is_none());
        assert!(monitor.observe(Ok(())).is_none());
        assert!(monitor.observe(Err("first again".to_owned())).is_none());
        assert!(monitor.observe(Err("second again".to_owned())).is_none());
        assert_eq!(
            monitor.observe(Err("link removed".to_owned())),
            Some("link removed".to_owned())
        );
    }
}
