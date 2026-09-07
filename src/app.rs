use crate::dwarf;
use crate::model::{AppSession, DebugSnapshot, RegisterData, RegisterReadResult, VariablePool};
use crate::probe::{
    AcqSlot, ProbeCommand, ProbeEvent, ProbeSession, ProbeWorkerHandle, VarSlotMapping, WriteKind,
};
use crate::ui;
use crate::ui::chart_plugin::ChartPluginState;
use crate::ui::debug_plugin::DebugPluginState;
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
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
pub struct MemRW3App {
    dock: DockLayoutState,
    pub session: AppSession,
    variable_tree: VariableTreePanel,
    plugins: Vec<Box<dyn MemRWPlugin>>,
    probe_worker: ProbeWorkerHandle,
    pub toasts: egui_notify::Toasts,
    frame_data: FrameData,
    register_data: RegisterData,
    register_sequence: u64,
    system_theme_monitor: ui::theme::SystemThemeMonitor,
    flashing: bool,
    flashing_file_name: Option<String>,
    connection_pending: bool,
    next_request_id: u64,
    rebuild_after_flash: bool,
    debug_snapshot: DebugSnapshot,
    sent_program_generation: u64,
}

impl MemRW3App {
    fn default_plugins() -> Vec<Box<dyn MemRWPlugin>> {
        vec![
            Box::new(ChartPluginState::default()),
            Box::new(TablePluginState::default()),
            Box::new(DebugPluginState::default()),
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

        let system_theme_monitor = ui::theme::SystemThemeMonitor::new(repaint_ctx.clone());
        let probe_worker = ProbeWorkerHandle::spawn(
            session.acquisition_requested.clone(),
            session.running.clone(),
            session.config.delay_us.clone(),
            session.acq_cycle_count.clone(),
            repaint_ctx,
        );

        Self {
            dock: DockLayoutState::default(),
            session,
            variable_tree: VariableTreePanel::new(dwarf_state),
            plugins: Self::default_plugins(),
            probe_worker,
            toasts: egui_notify::Toasts::default().with_anchor(egui_notify::Anchor::BottomRight),
            frame_data: FrameData::default(),
            register_data: RegisterData::default(),
            register_sequence: 0,
            system_theme_monitor,
            flashing: false,
            flashing_file_name: None,
            connection_pending: false,
            next_request_id: 1,
            rebuild_after_flash: false,
            debug_snapshot: DebugSnapshot::default(),
            sent_program_generation: 0,
        }
    }

    pub fn sync_connect(&mut self) {
        if self.connection_pending || self.is_flashing() {
            return;
        }
        let chip = self.session.config.probe_chip.clone();
        let protocol = self.session.config.probe_protocol.clone();
        let speed = self.session.config.probe_speed_khz;
        let probe_id = self.session.probe_id.clone();
        let connected = self.session.connected;

        if connected {
            self.session
                .acquisition_requested
                .store(false, Ordering::Release);
            self.session.set_running(false);
            self.connection_pending = true;
            if let Err(error) = self.probe_worker.send(ProbeCommand::Disconnect) {
                self.connection_pending = false;
                self.show_plugin_toast(ToastLevel::Error, error);
            }
        } else {
            for var in self.session.config.pool.iter() {
                var.incoming.discard_all();
            }
            self.connection_pending = true;
            if let Err(error) = self.probe_worker.send(ProbeCommand::Connect {
                chip_name: chip,
                protocol,
                speed_khz: speed,
                selected_probe_id: probe_id,
            }) {
                self.connection_pending = false;
                self.show_plugin_toast(ToastLevel::Error, error);
            }
        }
    }

    pub fn sync_reset(&mut self) {
        if let Err(error) = self.probe_worker.send(ProbeCommand::Reset) {
            self.show_plugin_toast(ToastLevel::Error, error);
        }
    }

    pub fn is_flashing(&self) -> bool {
        self.flashing
    }

    pub fn is_connection_pending(&self) -> bool {
        self.connection_pending
    }

    pub fn is_target_halted(&self) -> bool {
        self.debug_snapshot.target_state.is_halted()
    }

    pub fn start_flash_firmware(&mut self, path: std::path::PathBuf) -> Result<(), String> {
        if !self.session.connected {
            return Err("请先连接目标设备".to_owned());
        }
        if self.flashing {
            return Err("已有烧录任务正在执行".to_owned());
        }

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("firmware")
            .to_owned();
        self.session.set_running(false);
        self.session
            .acquisition_requested
            .store(false, Ordering::Release);
        self.session.timer_was_started = false;
        self.reset_plugin_data();
        for variable in self.session.config.pool.iter() {
            variable.incoming.discard_all();
        }

        self.probe_worker.send(ProbeCommand::Flash { path })?;
        self.flashing = true;
        self.flashing_file_name = Some(file_name);
        Ok(())
    }

    pub fn reset_timer(&self) {
        let _ = self.probe_worker.send(ProbeCommand::ResetTimer);
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
            self.session
                .acquisition_requested
                .store(false, Ordering::Release);
            self.session.set_running(false);
            return;
        }
        if !self.session.connected || self.is_flashing() {
            return;
        }

        self.session
            .acquisition_requested
            .store(true, Ordering::Release);
        if self.debug_snapshot.target_state.is_halted() {
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
        self.reset_timer();
        for var in self.session.config.pool.iter() {
            var.incoming.discard_all();
        }
    }

    fn next_request_id(&mut self) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        request_id
    }

    pub fn write_variable(&mut self, var_id: usize, value: u64) -> bool {
        let var = match self.session.config.pool.get(var_id) {
            Some(v) => v,
            None => return false,
        };
        let addr = var.address;
        let size = var.size;
        let request_id = self.next_request_id();
        self.probe_worker
            .send(ProbeCommand::WriteValue {
                request_id,
                address: addr,
                size,
                value,
                kind: WriteKind::Variable,
            })
            .is_ok()
    }

    pub fn rebuild_slots(&self) {
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
                    slots.push(AcqSlot {
                        address: addr,
                        needed_for_latest: var.latest_readers > 0,
                    });
                    slot_map.insert(addr, index);
                    index
                };
                if var.latest_readers > 0 {
                    slots[slot_index].needed_for_latest = true;
                }
                slot_indices.push(slot_index);
            }
            mappings.push(VarSlotMapping {
                slot_indices,
                size: var.size,
                byte_offset,
                incoming: var.incoming.clone(),
                latest: var.latest.clone(),
                stream_enabled: var.stream_readers > 0,
                latest_enabled: var.latest_readers > 0,
            });
        }

        let slot_n = slots.len() as u64;
        if self
            .probe_worker
            .send(ProbeCommand::ConfigureSlots { slots, mappings })
            .is_ok()
        {
            self.session.slot_count.store(slot_n, Ordering::Relaxed);
        }
    }

    pub fn unbind_variable(
        &mut self,
        var_id: usize,
        was_enabled: bool,
        read_class: crate::model::VariableReadClass,
    ) {
        self.session
            .config
            .pool
            .unbind(var_id, was_enabled, read_class);
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
                PluginAction::LoadProgram { path } => {
                    match self.variable_tree.load_program_path(path) {
                        Ok(()) => {
                            self.sync_program_to_worker();
                            self.toasts
                                .success("ELF 与调试信息已加载")
                                .duration(Some(Duration::from_secs(3)));
                        }
                        Err(error) => {
                            self.toasts
                                .error(error)
                                .duration(Some(Duration::from_secs(8)))
                                .closable(true);
                        }
                    }
                }
                PluginAction::RemoveVariable {
                    var_id,
                    was_enabled,
                    read_class,
                } => {
                    self.unbind_variable(var_id, was_enabled, read_class);
                    rebuild_slots = true;
                }
                PluginAction::SetVariableEnabled {
                    var_id,
                    enabled,
                    read_class,
                } => {
                    if self
                        .session
                        .config
                        .pool
                        .set_binding_enabled(var_id, enabled, read_class)
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
                    if !self.write_variable(var_id, value) {
                        self.toasts
                            .error("无法提交写入请求")
                            .duration(Some(Duration::from_secs(3)));
                    }
                }
                PluginAction::ReadRegisters { requests } => {
                    if requests.is_empty() || !self.session.connected || self.is_flashing() {
                        continue;
                    }
                    let request_id = self.next_request_id();
                    if let Err(error) = self.probe_worker.send(ProbeCommand::ReadRegisters {
                        request_id,
                        requests,
                    }) {
                        self.show_plugin_toast(ToastLevel::Error, error);
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
                    let request_id = self.next_request_id();
                    if let Err(error) = self.probe_worker.send(ProbeCommand::WriteValue {
                        request_id,
                        address: request.address,
                        size: u32::from(request.size_bytes),
                        value: request.value,
                        kind: WriteKind::Register,
                    }) {
                        self.show_plugin_toast(ToastLevel::Error, error);
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
                PluginAction::Debug(command) => {
                    if self.is_flashing() {
                        self.toasts
                            .error("固件烧录期间不能执行调试命令")
                            .duration(Some(Duration::from_secs(3)));
                        continue;
                    }
                    let request_id = self.next_request_id();
                    if let Err(error) = self.probe_worker.send(ProbeCommand::Debug {
                        request_id,
                        command,
                    }) {
                        self.show_plugin_toast(ToastLevel::Error, error);
                    }
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

    fn poll_probe_events(&mut self) {
        while let Ok(event) = self.probe_worker.event_receiver.try_recv() {
            match event {
                ProbeEvent::ConnectFinished {
                    connected,
                    chip_name,
                    speed_khz,
                    protocol,
                    selected_probe_id,
                    error,
                } => {
                    self.connection_pending = false;
                    self.session.connected = connected;
                    self.toasts
                        .info(format!(
                            "连接配置: chip:{chip_name},freq:{speed_khz},protocol:{protocol},id:{}",
                            selected_probe_id.as_deref().unwrap_or("auto")
                        ))
                        .duration(Some(Duration::from_secs(5)))
                        .closable(true);
                    if connected {
                        self.toasts
                            .success("连接成功")
                            .duration(Some(Duration::from_secs(5)));
                        self.rebuild_slots();
                    } else {
                        self.session.set_running(false);
                        self.toasts
                            .error(error.unwrap_or_else(|| "连接失败".to_owned()))
                            .duration(Some(Duration::from_secs(5)))
                            .closable(true);
                    }
                }
                ProbeEvent::Disconnected => {
                    self.connection_pending = false;
                    self.session.connected = false;
                    self.session.timer_was_started = false;
                    self.toasts
                        .info("已断开连接")
                        .duration(Some(Duration::from_secs(5)));
                }
                ProbeEvent::ResetFinished(result) => match result {
                    Ok(()) => {
                        self.toasts
                            .success("目标已复位")
                            .duration(Some(Duration::from_secs(2)));
                    }
                    Err(error) => {
                        self.toasts
                            .error(error)
                            .duration(Some(Duration::from_secs(5)));
                    }
                },
                ProbeEvent::FlashFinished(result) => {
                    self.flashing = false;
                    let file_name = self
                        .flashing_file_name
                        .take()
                        .unwrap_or_else(|| "firmware".to_owned());
                    for variable in self.session.config.pool.iter() {
                        variable.incoming.discard_all();
                    }
                    match result {
                        Ok(()) => {
                            self.toasts
                                .success(format!("固件 {file_name} 烧录并校验成功"))
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
                ProbeEvent::WriteFinished {
                    request_id,
                    kind,
                    result,
                } => {
                    let _ = request_id;
                    match (kind, result) {
                        (WriteKind::Variable, Ok(())) => {
                            self.toasts
                                .success("写入成功")
                                .duration(Some(Duration::from_secs(2)));
                        }
                        (WriteKind::Register, Ok(())) => {
                            self.toasts
                                .success("寄存器写入成功")
                                .duration(Some(Duration::from_secs(2)));
                        }
                        (_, Err(error)) => {
                            self.toasts
                                .error(error)
                                .duration(Some(Duration::from_secs(3)));
                        }
                    }
                }
                ProbeEvent::RegistersRead {
                    request_id,
                    results,
                } => {
                    let _ = request_id;
                    self.register_sequence = self.register_sequence.wrapping_add(1);
                    let sequence = self.register_sequence;
                    for (id, value) in results {
                        self.register_data
                            .insert(id, RegisterReadResult { sequence, value });
                    }
                }
                ProbeEvent::LinkLost(error) => {
                    self.session.set_running(false);
                    self.session.connected = false;
                    self.session.timer_was_started = false;
                    self.connection_pending = false;
                    self.toasts
                        .error(format!("Probe 物理链路已断开：{error}"))
                        .duration(Some(Duration::from_secs(8)))
                        .closable(true);
                }
                ProbeEvent::ProgramLoaded { generation, result } => {
                    if generation != self.variable_tree.program_generation() {
                        continue;
                    }
                    if let Err(error) = result {
                        self.toasts
                            .error(error)
                            .duration(Some(Duration::from_secs(8)))
                            .closable(true);
                    }
                }
                ProbeEvent::DebugUpdated {
                    request_id,
                    snapshot,
                } => {
                    let _ = request_id;
                    let generation_matches = self.sent_program_generation == 0
                        || snapshot.program_generation == self.sent_program_generation;
                    if generation_matches && snapshot.revision >= self.debug_snapshot.revision {
                        self.debug_snapshot = *snapshot;
                    }
                }
            }
        }
    }

    fn sync_program_to_worker(&mut self) {
        let generation = self.variable_tree.program_generation();
        if generation == 0 || generation == self.sent_program_generation {
            return;
        }
        let Some(path) = self.variable_tree.loaded_elf_path() else {
            return;
        };
        if self
            .probe_worker
            .send(ProbeCommand::LoadProgram {
                path: path.into(),
                generation,
            })
            .is_ok()
        {
            self.sent_program_generation = generation;
        }
    }
}

impl eframe::App for MemRW3App {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        ui::set_native_dialog_parent(frame);
        self.system_theme_monitor.apply(ui.ctx());
        self.poll_probe_events();
        self.sync_program_to_worker();
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
        let acquisition_requested = self.session.acquisition_requested.load(Ordering::Acquire);
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
                acquisition_requested,
                connected,
                hardware_busy,
                debug_snapshot: &self.debug_snapshot,
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

            let mut variable_tree_actions = Vec::new();
            let pop_in_plugin = self.variable_tree.show_popout(
                ui,
                &mut self.plugins,
                &mut self.session.config.pool,
                &mut variable_tree_actions,
            );
            self.handle_plugin_actions(variable_tree_actions);
            if let Some(plugin_id) = pop_in_plugin {
                self.dock.focus_plugin(&plugin_id);
            }
        });
        self.frame_data = frame_data;
        if let Some(file_name) = self.flashing_file_name.as_deref() {
            firmware_flash_modal(ui.ctx(), file_name);
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
        let path = crate::ui::file_dialog()
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
        let path = crate::ui::file_dialog()
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
        let request_id = self.next_request_id();
        let _ = self.probe_worker.send(ProbeCommand::Debug {
            request_id,
            command: crate::model::DebugCommand::ReplaceBreakpoints(Vec::new()),
        });
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

    if let Some((name, data, family)) = load_chinese_font() {
        println!("✅ 使用系统字体: {family}");
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
        println!(
            "⚠️ 未找到支持简体中文的系统字体，中文可能无法显示\n\
             💡 请安装 Noto Sans CJK SC、思源黑体或平台自带的中文字体"
        );
    }

    ctx.set_fonts(fonts);
}

fn load_chinese_font() -> Option<(String, Arc<egui::FontData>, String)> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();

    let face = database
        .faces()
        .filter(|face| face_supports_chinese(&database, face.id))
        .min_by_key(|face| face_preference_key(face))?;
    let face_id = face.id;
    let family = selected_family_name(face);
    let (bytes, index) = database.with_face_data(face_id, |data, index| (data.to_vec(), index))?;

    let mut font_data = egui::FontData::from_owned(bytes);
    // A TTC/OTC can contain region-specific faces. Preserve the exact face
    // selected by fontdb instead of silently using collection index zero.
    font_data.index = index;

    Some((
        "system_chinese_font".to_owned(),
        Arc::new(font_data),
        family,
    ))
}

const CHINESE_GLYPH_PROBES: &[char] = &['中', '文', '采', '集', '调', '试', '变', '量', '烧', '录'];

#[cfg(target_os = "windows")]
const PREFERRED_CHINESE_FAMILIES: &[&str] = &[
    "Microsoft YaHei UI",
    "Microsoft YaHei",
    "DengXian",
    "SimHei",
    "SimSun",
    "Noto Sans CJK SC",
    "Noto Sans SC",
    "Source Han Sans SC",
];

#[cfg(target_os = "macos")]
const PREFERRED_CHINESE_FAMILIES: &[&str] = &[
    "PingFang SC",
    "Hiragino Sans GB",
    "Heiti SC",
    "Songti SC",
    "Noto Sans CJK SC",
    "Noto Sans SC",
    "Source Han Sans SC",
];

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const PREFERRED_CHINESE_FAMILIES: &[&str] = &[
    "Noto Sans CJK SC",
    "Noto Sans SC",
    "Source Han Sans SC",
    "WenQuanYi Micro Hei",
    "Droid Sans Fallback",
    "AR PL UMing CN",
    "Microsoft YaHei",
    "PingFang SC",
];

fn face_supports_chinese(database: &fontdb::Database, id: fontdb::ID) -> bool {
    database
        .with_face_data(id, |data, index| {
            ttf_parser::Face::parse(data, index).is_ok_and(|face| {
                CHINESE_GLYPH_PROBES
                    .iter()
                    .all(|character| face.glyph_index(*character).is_some())
            })
        })
        .unwrap_or(false)
}

fn family_preference_rank(face: &fontdb::FaceInfo) -> usize {
    PREFERRED_CHINESE_FAMILIES
        .iter()
        .position(|preferred| {
            face.families
                .iter()
                .any(|(family, _)| family.eq_ignore_ascii_case(preferred))
        })
        .unwrap_or(PREFERRED_CHINESE_FAMILIES.len())
}

fn face_preference_key(face: &fontdb::FaceInfo) -> (usize, u8, u16, bool) {
    let normal_style = u8::from(face.style != fontdb::Style::Normal);
    (
        family_preference_rank(face),
        normal_style,
        face.weight.0.abs_diff(fontdb::Weight::NORMAL.0),
        face.monospaced,
    )
}

fn selected_family_name(face: &fontdb::FaceInfo) -> String {
    PREFERRED_CHINESE_FAMILIES
        .iter()
        .find_map(|preferred| {
            face.families
                .iter()
                .find(|(family, _)| family.eq_ignore_ascii_case(preferred))
                .map(|(family, _)| family.clone())
        })
        .or_else(|| face.families.first().map(|(family, _)| family.clone()))
        .unwrap_or_else(|| face.post_script_name.clone())
}

#[cfg(test)]
mod font_tests {
    use super::{PREFERRED_CHINESE_FAMILIES, family_preference_rank};

    fn face_with_families(names: &[&str]) -> fontdb::FaceInfo {
        fontdb::FaceInfo {
            id: fontdb::ID::dummy(),
            source: fontdb::Source::Binary(std::sync::Arc::new(Vec::<u8>::new())),
            index: 0,
            families: names
                .iter()
                .map(|name| ((*name).to_owned(), fontdb::Language::English_UnitedStates))
                .collect(),
            post_script_name: String::new(),
            style: fontdb::Style::Normal,
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            monospaced: false,
        }
    }

    #[test]
    fn prefers_platform_chinese_families_case_insensitively() {
        let preferred = PREFERRED_CHINESE_FAMILIES[0];
        let face = face_with_families(&[&preferred.to_ascii_lowercase()]);
        assert_eq!(family_preference_rank(&face), 0);

        let fallback = face_with_families(&["Unlisted CJK Font"]);
        assert_eq!(
            family_preference_rank(&fallback),
            PREFERRED_CHINESE_FAMILIES.len()
        );
    }
}
