use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use eframe::egui;
use object::{Object, ObjectSection};
use probe_rs::{
    BreakpointCause, CoreStatus, HaltReason, InstructionSet, MemoryInterface, RegisterId,
    RegisterValue,
};
use probe_rs_debug::{
    BitOffset, DebugInfo, DebugRegisters, Modifier, ObjectRef, StackFrame, Variable, VariableCache,
    VariableLocation, VariableType, VariableValue, exception_handler_for_core,
    stack_frame::StackFrameInfo,
};

use crate::dwarf::extract::SourceLineRecord;
use crate::model::{
    BreakpointSpec, BreakpointView, DebugCommand, DebugSnapshot, DebugStartMode, InstructionView,
    RegisterReadRequest, RegisterView, RingBuffer, SourceLocationView, StackFrameView,
    StackMemoryWord, StepKind, TargetState, VariableView,
};

use super::{AcqSlot, ProbeSession, VarSlotMapping};

pub enum ProbeCommand {
    Connect {
        chip_name: String,
        protocol: String,
        speed_khz: u32,
        selected_probe_id: Option<String>,
    },
    Disconnect,
    Reset,
    Flash {
        path: PathBuf,
    },
    ConfigureSlots {
        slots: Vec<AcqSlot>,
        mappings: Vec<VarSlotMapping>,
    },
    ResetTimer,
    WriteValue {
        request_id: u64,
        address: u64,
        size: u32,
        value: u64,
        kind: WriteKind,
    },
    ReadRegisters {
        request_id: u64,
        requests: Vec<RegisterReadRequest>,
    },
    LoadProgram {
        path: PathBuf,
        generation: u64,
    },
    Debug {
        request_id: u64,
        command: DebugCommand,
    },
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteKind {
    Variable,
    Register,
}

pub enum ProbeEvent {
    ConnectFinished {
        connected: bool,
        chip_name: String,
        speed_khz: u32,
        protocol: String,
        selected_probe_id: Option<String>,
        error: Option<String>,
    },
    Disconnected,
    ResetFinished(Result<(), String>),
    FlashFinished(Result<(), String>),
    WriteFinished {
        request_id: u64,
        kind: WriteKind,
        result: Result<(), String>,
    },
    RegistersRead {
        request_id: u64,
        results: Vec<(u64, Result<u64, String>)>,
    },
    LinkLost(String),
    ProgramLoaded {
        generation: u64,
        result: Result<(), String>,
    },
    DebugUpdated {
        request_id: Option<u64>,
        snapshot: Box<DebugSnapshot>,
    },
}

pub struct ProbeWorkerHandle {
    pub command_sender: Sender<ProbeCommand>,
    pub event_receiver: Receiver<ProbeEvent>,
    join_handle: Option<JoinHandle<()>>,
}

impl ProbeWorkerHandle {
    pub fn spawn(
        acquisition_requested: Arc<AtomicBool>,
        running: Arc<AtomicBool>,
        delay_us: Arc<AtomicU64>,
        cycle_count: Arc<AtomicU64>,
        repaint_ctx: egui::Context,
    ) -> Self {
        let (command_sender, command_receiver) = std::sync::mpsc::channel();
        let (event_sender, event_receiver) = std::sync::mpsc::channel();
        let join_handle = thread::spawn(move || {
            ProbeWorker::new(
                running,
                acquisition_requested,
                delay_us,
                cycle_count,
                command_receiver,
                event_sender,
                repaint_ctx,
            )
            .run();
        });
        Self {
            command_sender,
            event_receiver,
            join_handle: Some(join_handle),
        }
    }

    pub fn send(&self, command: ProbeCommand) -> Result<(), String> {
        self.command_sender
            .send(command)
            .map_err(|_| "Probe worker 已停止".to_owned())
    }

    pub fn shutdown(&mut self) {
        let _ = self.command_sender.send(ProbeCommand::Shutdown);
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ProbeWorkerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct ProbeWorker {
    probe: ProbeSession,
    acquisition_requested: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    delay_us: Arc<AtomicU64>,
    cycle_count: Arc<AtomicU64>,
    command_receiver: Receiver<ProbeCommand>,
    event_sender: Sender<ProbeEvent>,
    repaint_ctx: egui::Context,
    last_link_check: Instant,
    link_failures: u8,
    debug_info: Option<DebugInfo>,
    program_generation: u64,
    program_path: Option<String>,
    source_files: Vec<String>,
    source_lines: Arc<[SourceLineRecord]>,
    target_state: TargetState,
    debug_active: bool,
    debug_revision: u64,
    stop_id: u64,
    last_status_poll: Instant,
    breakpoints: BTreeMap<u64, BreakpointView>,
    stack_frames: Vec<StackFrame>,
    selected_frame: usize,
    latest_pc: Option<u64>,
    latest_registers: Vec<RegisterView>,
    latest_instructions: Vec<InstructionView>,
    latest_stack_memory: Vec<StackMemoryWord>,
    latest_debug_warnings: Vec<String>,
    breakpoint_capacity: Option<u32>,
    last_latest_acquisition: Instant,
    temporary_breakpoint: Option<u64>,
    local_value_overrides: std::collections::HashMap<i64, String>,
    program_code: Option<ProgramCode>,
}

impl ProbeWorker {
    const LINK_CHECK_INTERVAL: Duration = Duration::from_millis(500);
    const IDLE_WAIT: Duration = Duration::from_millis(20);
    const FAILURE_LIMIT: u8 = 3;

    fn new(
        running: Arc<AtomicBool>,
        acquisition_requested: Arc<AtomicBool>,
        delay_us: Arc<AtomicU64>,
        cycle_count: Arc<AtomicU64>,
        command_receiver: Receiver<ProbeCommand>,
        event_sender: Sender<ProbeEvent>,
        repaint_ctx: egui::Context,
    ) -> Self {
        Self {
            probe: ProbeSession::default(),
            acquisition_requested,
            running,
            delay_us,
            cycle_count,
            command_receiver,
            event_sender,
            repaint_ctx,
            last_link_check: Instant::now(),
            link_failures: 0,
            debug_info: None,
            program_generation: 0,
            program_path: None,
            source_files: Vec::new(),
            source_lines: Arc::from([]),
            target_state: TargetState::Disconnected,
            debug_active: false,
            debug_revision: 0,
            stop_id: 0,
            last_status_poll: Instant::now(),
            breakpoints: BTreeMap::new(),
            stack_frames: Vec::new(),
            selected_frame: 0,
            latest_pc: None,
            latest_registers: Vec::new(),
            latest_instructions: Vec::new(),
            latest_stack_memory: Vec::new(),
            latest_debug_warnings: Vec::new(),
            breakpoint_capacity: None,
            last_latest_acquisition: Instant::now() - Duration::from_secs(1),
            temporary_breakpoint: None,
            local_value_overrides: std::collections::HashMap::new(),
            program_code: None,
        }
    }

    fn run(mut self) {
        loop {
            match self.drain_commands() {
                WorkerControl::Continue => {}
                WorkerControl::Shutdown => return,
            }

            if !self.probe.connected {
                match self.command_receiver.recv_timeout(Self::IDLE_WAIT) {
                    Ok(command) => {
                        if matches!(self.handle_command(command), WorkerControl::Shutdown) {
                            return;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
                continue;
            }

            if self.debug_active && self.last_status_poll.elapsed() >= Duration::from_millis(50) {
                self.last_status_poll = Instant::now();
                self.poll_target_status();
                if !self.probe.connected {
                    continue;
                }
            }

            let acquisition_running = self.running.load(Ordering::Acquire)
                && matches!(
                    self.target_state,
                    TargetState::Running | TargetState::Sleeping | TargetState::Unknown
                );
            let slots_empty = self.probe.slots.is_empty();
            let acquisition_result = if acquisition_running && !slots_empty {
                Some(self.probe.acquire_from_slots())
            } else if self.target_state.is_halted()
                && !slots_empty
                && self.last_latest_acquisition.elapsed() >= Duration::from_secs(1)
            {
                self.last_latest_acquisition = Instant::now();
                Some(self.probe.acquire_latest_from_slots())
            } else {
                None
            };
            let acquired = matches!(acquisition_result, Some(Ok(())));
            let acquisition_failed = matches!(acquisition_result, Some(Err(_)));

            if acquired {
                self.cycle_count.fetch_add(1, Ordering::Relaxed);
                self.link_failures = 0;
                if !acquisition_running {
                    self.repaint_ctx.request_repaint();
                }
            }

            let health_check_due = self.last_link_check.elapsed() >= Self::LINK_CHECK_INTERVAL;
            if health_check_due && (!acquisition_running || slots_empty || acquisition_failed) {
                self.last_link_check = Instant::now();
                let health = self.probe.check_link();
                self.observe_link_health(health);
            }

            if !self.probe.connected {
                continue;
            }

            if acquisition_running && !slots_empty && acquired {
                let delay = self.delay_us.load(Ordering::Acquire);
                if delay > 0 {
                    thread::sleep(Duration::from_micros(delay));
                }
            } else {
                match self.command_receiver.recv_timeout(Self::IDLE_WAIT) {
                    Ok(command) => {
                        if matches!(self.handle_command(command), WorkerControl::Shutdown) {
                            return;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        }
    }

    fn drain_commands(&mut self) -> WorkerControl {
        // Bound command work per turn so a producer cannot starve acquisition forever.
        for _ in 0..64 {
            match self.command_receiver.try_recv() {
                Ok(command) => {
                    if matches!(self.handle_command(command), WorkerControl::Shutdown) {
                        return WorkerControl::Shutdown;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return WorkerControl::Shutdown,
            }
        }
        WorkerControl::Continue
    }

    fn handle_command(&mut self, command: ProbeCommand) -> WorkerControl {
        match command {
            ProbeCommand::Connect {
                chip_name,
                protocol,
                speed_khz,
                selected_probe_id,
            } => {
                self.debug_active = false;
                self.breakpoint_capacity = None;
                self.invalidate_halted_data();
                self.probe.chip_name = chip_name;
                self.probe.protocol = protocol;
                self.probe.speed_khz = speed_khz;
                self.probe.selected_probe_id = selected_probe_id;
                let connected = self.probe.connect();
                self.link_failures = 0;
                self.last_link_check = Instant::now();
                self.target_state = if connected {
                    TargetState::Unknown
                } else {
                    TargetState::Disconnected
                };
                if connected {
                    self.reconcile_breakpoints();
                }
                self.publish_debug(None);
                self.emit(ProbeEvent::ConnectFinished {
                    connected,
                    chip_name: self.probe.chip_name.clone(),
                    speed_khz: self.probe.speed_khz,
                    protocol: self.probe.protocol.clone(),
                    selected_probe_id: self.probe.selected_probe_id.clone(),
                    error: self.probe.last_error.clone(),
                });
            }
            ProbeCommand::Disconnect => {
                self.running.store(false, Ordering::Release);
                self.acquisition_requested.store(false, Ordering::Release);
                self.temporary_breakpoint = None;
                self.debug_active = false;
                self.probe.disconnect();
                self.target_state = TargetState::Disconnected;
                self.invalidate_halted_data();
                self.breakpoint_capacity = None;
                self.publish_debug(None);
                self.emit(ProbeEvent::Disconnected);
            }
            ProbeCommand::Reset => {
                self.clear_temporary_breakpoint();
                let result = self
                    .probe
                    .reset_target_result()
                    .map_err(|error| format!("复位失败: {error}"));
                self.target_state = TargetState::Unknown;
                self.invalidate_halted_data();
                if self.debug_active {
                    self.reconcile_breakpoints();
                }
                if self.acquisition_requested.load(Ordering::Acquire) {
                    self.running.store(true, Ordering::Release);
                }
                self.publish_debug(None);
                self.emit(ProbeEvent::ResetFinished(result));
            }
            ProbeCommand::Flash { path } => {
                self.clear_temporary_breakpoint();
                let result = self.probe.flash_firmware(&path);
                self.target_state = TargetState::Unknown;
                self.invalidate_halted_data();
                if self.debug_active {
                    self.reconcile_breakpoints();
                }
                self.publish_debug(None);
                self.emit(ProbeEvent::FlashFinished(result));
            }
            ProbeCommand::ConfigureSlots { slots, mappings } => {
                self.probe.slots = slots;
                self.probe
                    .slot_values
                    .resize(self.probe.slots.len(), [0; 4]);
                self.probe.var_mappings = mappings;
            }
            ProbeCommand::ResetTimer => {
                self.probe.timer = Instant::now();
            }
            ProbeCommand::WriteValue {
                request_id,
                address,
                size,
                value,
                kind,
            } => {
                let result = self
                    .probe
                    .write_value_result(address, size, value)
                    .map_err(|error| format!("写入 0x{address:08X} 失败: {error}"));
                self.emit(ProbeEvent::WriteFinished {
                    request_id,
                    kind,
                    result,
                });
            }
            ProbeCommand::ReadRegisters {
                request_id,
                requests,
            } => {
                let results = self.probe.read_registers(&requests);
                self.emit(ProbeEvent::RegistersRead {
                    request_id,
                    results,
                });
            }
            ProbeCommand::LoadProgram { path, generation } => {
                self.clear_temporary_breakpoint();
                self.clear_installed_breakpoints();
                let source_index =
                    crate::dwarf::extract::load_source_index(path.to_string_lossy().as_ref())
                        .unwrap_or_default();
                let program_code = ProgramCode::load(&path).ok();
                let result = match DebugInfo::from_file(&path) {
                    Ok(debug_info) => {
                        self.debug_info = Some(debug_info);
                        self.program_generation = generation;
                        self.program_path = Some(path.display().to_string());
                        self.source_files = source_index.files;
                        self.source_lines = source_index.executable_lines.into();
                        self.program_code = program_code;
                        self.stack_frames.clear();
                        self.selected_frame = 0;
                        self.latest_instructions = self
                            .program_code
                            .as_ref()
                            .and_then(|code| {
                                code.disassemble(
                                    self.source_lines.first().map(|line| line.address),
                                    self.debug_info.as_ref(),
                                )
                                .ok()
                            })
                            .unwrap_or_default();
                        self.reconcile_breakpoints();
                        Ok(())
                    }
                    Err(error) => {
                        self.debug_info = None;
                        self.program_generation = generation;
                        self.program_path = Some(path.display().to_string());
                        self.source_files.clear();
                        self.source_lines = Arc::from([]);
                        self.program_code = None;
                        self.latest_instructions.clear();
                        self.invalidate_halted_data();
                        Err(format!("加载调试信息失败: {error}"))
                    }
                };
                self.publish_debug(None);
                self.emit(ProbeEvent::ProgramLoaded { generation, result });
            }
            ProbeCommand::Debug {
                request_id,
                command,
            } => {
                self.handle_debug_command(request_id, command);
            }
            ProbeCommand::Shutdown => {
                self.running.store(false, Ordering::Release);
                self.probe.disconnect();
                return WorkerControl::Shutdown;
            }
        }
        WorkerControl::Continue
    }

    fn handle_debug_command(&mut self, request_id: u64, command: DebugCommand) {
        let requires_active = matches!(
            &command,
            DebugCommand::Halt
                | DebugCommand::Continue
                | DebugCommand::RunTo(_)
                | DebugCommand::Step(_)
                | DebugCommand::SelectFrame(_)
                | DebugCommand::ExpandVariable { .. }
                | DebugCommand::WriteVariable { .. }
                | DebugCommand::Refresh
        );
        if requires_active && !self.debug_active {
            self.debug_revision = self.debug_revision.wrapping_add(1);
            let mut snapshot = self.build_snapshot();
            snapshot.last_error = Some("请先启动 DebugPlugin".to_owned());
            self.emit(ProbeEvent::DebugUpdated {
                request_id: Some(request_id),
                snapshot: Box::new(snapshot),
            });
            return;
        }
        if requires_active && !self.acquisition_requested.load(Ordering::Acquire) {
            self.debug_revision = self.debug_revision.wrapping_add(1);
            let mut snapshot = self.build_snapshot();
            snapshot.last_error = Some("全局采集已停止，Debug 命令已拒绝".to_owned());
            self.emit(ProbeEvent::DebugUpdated {
                request_id: Some(request_id),
                snapshot: Box::new(snapshot),
            });
            return;
        }
        let result = match command {
            DebugCommand::Start(mode) => self.debug_start(mode),
            DebugCommand::Stop => self.debug_stop(),
            DebugCommand::Halt => self.debug_halt(),
            DebugCommand::Continue => self.debug_continue(),
            DebugCommand::RunTo(spec) => self.debug_run_to(&spec),
            DebugCommand::Disassemble(spec) => self.debug_disassemble(&spec),
            DebugCommand::Step(kind) => self.debug_step(kind),
            DebugCommand::SetBreakpoint(logical) => {
                self.breakpoints.insert(
                    logical.id,
                    BreakpointView {
                        id: logical.id,
                        spec: logical.spec,
                        address: None,
                        enabled: logical.enabled,
                        verified: false,
                        message: None,
                        resolved_source: None,
                    },
                );
                self.install_breakpoint(logical.id)
            }
            DebugCommand::RemoveBreakpoint(id) => self.remove_breakpoint(id),
            DebugCommand::SetBreakpointEnabled { id, enabled } => {
                self.set_breakpoint_enabled(id, enabled)
            }
            DebugCommand::ReplaceBreakpoints(breakpoints) => self.replace_breakpoints(breakpoints),
            DebugCommand::SelectFrame(index) => {
                if index < self.stack_frames.len() {
                    self.selected_frame = index;
                    self.expand_frame_root(index)
                } else {
                    Err("栈帧不存在或已失效".to_owned())
                }
            }
            DebugCommand::ExpandVariable {
                stop_id,
                frame_index,
                variable_ref,
            } => {
                if stop_id != self.stop_id {
                    Err("目标状态已经变化，局部变量请求已丢弃".to_owned())
                } else {
                    self.expand_variable(frame_index, variable_ref)
                }
            }
            DebugCommand::WriteVariable {
                stop_id,
                frame_index,
                variable_ref,
                value,
            } => {
                if stop_id != self.stop_id {
                    Err("目标状态已经变化，局部变量写入请求已丢弃".to_owned())
                } else {
                    self.write_local_variable(frame_index, variable_ref, value)
                }
            }
            DebugCommand::Refresh => self.refresh_halted_data(),
        };

        if let Err(error) = result {
            self.debug_revision = self.debug_revision.wrapping_add(1);
            let mut snapshot = self.build_snapshot();
            snapshot.last_error = Some(error);
            self.emit(ProbeEvent::DebugUpdated {
                request_id: Some(request_id),
                snapshot: Box::new(snapshot),
            });
        } else {
            self.publish_debug(Some(request_id));
        }
    }

    fn debug_start(&mut self, mode: DebugStartMode) -> Result<(), String> {
        if self.debug_active {
            return Ok(());
        }
        if !self.probe.connected {
            return Err("请先连接目标设备".to_owned());
        }
        if !self.acquisition_requested.load(Ordering::Acquire) {
            return Err("请先点击全局“开始”，再启动 DebugPlugin".to_owned());
        }

        self.clear_temporary_breakpoint();
        let (status, known_pc) = {
            let mut core = self
                .probe
                .session_mut()
                .and_then(|session| session.core(0))
                .map_err(|error| format!("启动调试时获取核心失败: {error}"))?;
            match mode {
                DebugStartMode::Attach => {
                    let status = core
                        .status()
                        .map_err(|error| format!("Attach 时读取核心状态失败: {error}"))?;
                    let pc = if status.is_halted() {
                        core.read_core_reg(core.program_counter().id())
                            .ok()
                            .and_then(register_value_u64)
                    } else {
                        None
                    };
                    (status, pc)
                }
                DebugStartMode::Reset => {
                    let information = core
                        .reset_and_halt(Duration::from_millis(500))
                        .map_err(|error| format!("Reset 启动失败: {error}"))?;
                    let status = core
                        .status()
                        .map_err(|error| format!("Reset 后读取核心状态失败: {error}"))?;
                    (status, Some(information.pc))
                }
            }
        };

        self.debug_active = true;
        self.breakpoint_capacity = self.probe.breakpoint_capacity().ok();
        self.target_state = target_state(status);
        self.reconcile_breakpoints();
        if self.target_state.is_halted() {
            self.running.store(false, Ordering::Release);
            self.stop_id = self.stop_id.wrapping_add(1);
            self.refresh_halted_data_at(known_pc)?;
        } else {
            self.invalidate_halted_data();
        }
        Ok(())
    }

    fn debug_stop(&mut self) -> Result<(), String> {
        if !self.debug_active {
            return Ok(());
        }
        self.clear_temporary_breakpoint();
        self.clear_installed_breakpoints();
        self.debug_active = false;
        self.breakpoint_capacity = None;
        self.invalidate_halted_data();
        self.reconcile_breakpoints();
        Ok(())
    }

    fn debug_halt(&mut self) -> Result<(), String> {
        if !self.probe.connected {
            return Err("请先连接目标设备".to_owned());
        }
        self.running.store(false, Ordering::Release);
        let (info, status) = {
            let mut core = self
                .probe
                .session_mut()
                .and_then(|session| session.core(0))
                .map_err(|error| format!("获取核心失败: {error}"))?;
            let info = core
                .halt(Duration::from_millis(200))
                .map_err(|error| format!("暂停目标失败: {error}"))?;
            let status = core
                .status()
                .map_err(|error| format!("读取暂停后状态失败: {error}"))?;
            (info, status)
        };
        self.clear_temporary_breakpoint();
        self.target_state = target_state(status);
        if !self.target_state.is_halted() {
            return Err(format!(
                "暂停命令完成后目标状态异常：{:?}",
                self.target_state
            ));
        }
        self.stop_id = self.stop_id.wrapping_add(1);
        self.refresh_halted_data_at(Some(info.pc))
    }

    fn debug_continue(&mut self) -> Result<(), String> {
        if !self.target_state.is_halted() {
            return Err("目标未处于暂停状态".to_owned());
        }
        let status = {
            let mut core = self
                .probe
                .session_mut()
                .and_then(|session| session.core(0))
                .map_err(|error| format!("获取核心失败: {error}"))?;
            core.run()
                .map_err(|error| format!("继续运行失败: {error}"))?;
            core.status()
                .map_err(|error| format!("读取继续后状态失败: {error}"))?
        };
        self.target_state = target_state(status);
        if self.target_state.is_halted() {
            self.running.store(false, Ordering::Release);
            self.stop_id = self.stop_id.wrapping_add(1);
            self.refresh_halted_data()
        } else {
            self.invalidate_halted_data();
            if self.acquisition_requested.load(Ordering::Acquire) {
                self.running.store(true, Ordering::Release);
            }
            Ok(())
        }
    }

    fn debug_run_to(&mut self, spec: &BreakpointSpec) -> Result<(), String> {
        if !self.target_state.is_halted() {
            return Err("运行到光标前必须先暂停目标".to_owned());
        }
        let resolution = self.resolve_breakpoint(spec)?;
        let address = resolution.address;
        let already_installed = self.breakpoints.values().any(|breakpoint| {
            breakpoint.enabled && breakpoint.verified && breakpoint.address == Some(address)
        });
        if !already_installed {
            self.probe
                .set_hw_breakpoint(address)
                .map_err(|error| format!("设置临时断点失败: {error}"))?;
            self.temporary_breakpoint = Some(address);
        }
        let run_result = (|| {
            let mut core = self.probe.session_mut()?.core(0)?;
            core.run()?;
            core.status()
        })();
        let status = match run_result {
            Ok(status) => status,
            Err(error) => {
                self.clear_temporary_breakpoint();
                return Err(format!("运行到光标失败: {error}"));
            }
        };
        self.target_state = target_state(status);
        if self.target_state.is_halted() {
            self.running.store(false, Ordering::Release);
            self.clear_temporary_breakpoint();
            self.stop_id = self.stop_id.wrapping_add(1);
            self.refresh_halted_data()
        } else {
            self.invalidate_halted_data();
            if self.acquisition_requested.load(Ordering::Acquire) {
                self.running.store(true, Ordering::Release);
            }
            Ok(())
        }
    }

    fn debug_disassemble(&mut self, spec: &BreakpointSpec) -> Result<(), String> {
        let address = self.resolve_breakpoint(spec)?.address;
        let debug_info = self.debug_info.as_ref();
        let from_elf = self
            .program_code
            .as_ref()
            .filter(|code| code.contains_address(address))
            .and_then(|code| code.disassemble(Some(address), debug_info).ok());
        let live = || {
            if !self.target_state.is_halted() {
                return None;
            }
            self.probe
                .session_mut()
                .and_then(|session| session.core(0))
                .ok()
                .and_then(|mut core| disassemble_around_pc(&mut core, debug_info, address).ok())
        };
        self.latest_instructions = from_elf
            .or_else(live)
            .or_else(|| {
                self.program_code
                    .as_ref()
                    .and_then(|code| code.disassemble(Some(address), debug_info).ok())
            })
            .ok_or_else(|| format!("无法读取或反汇编 0x{address:08X} 附近的代码"))?;
        Ok(())
    }

    fn debug_step(&mut self, kind: StepKind) -> Result<(), String> {
        if !self.target_state.is_halted() {
            return Err("单步前必须先暂停目标".to_owned());
        }
        let origin_pc = {
            let mut core = self
                .probe
                .session_mut()
                .and_then(|session| session.core(0))
                .map_err(|error| format!("获取核心失败: {error}"))?;
            core.read_core_reg(core.program_counter().id())
                .and_then(|value: RegisterValue| value.try_into())
                .map_err(|error| format!("读取单步前 PC 失败: {error}"))?
        };

        let (status, actual_pc) = if kind == StepKind::Instruction {
            let mut core = self
                .probe
                .session_mut()
                .and_then(|session| session.core(0))
                .map_err(|error| format!("获取核心失败: {error}"))?;
            let information = core
                .step()
                .map_err(|error| format!("单指令执行失败: {error}"))?;
            let status = core
                .status()
                .map_err(|error| format!("读取单步后状态失败: {error}"))?;
            (status, information.pc)
        } else {
            let installed_breakpoints = self.suspend_user_breakpoints()?;
            let step_result = (|| -> Result<(CoreStatus, u64), String> {
                let debug_info = self
                    .debug_info
                    .as_ref()
                    .ok_or_else(|| "请先加载 ELF 调试信息".to_owned())?;
                let mut core = self
                    .probe
                    .session_mut()
                    .and_then(|session| session.core(0))
                    .map_err(|error| format!("获取核心失败: {error}"))?;
                let interrupt_mask = mask_interrupts_for_source_step(&mut core)?;
                let result = single_step_source(&mut core, debug_info, kind, origin_pc);
                let restore_mask = restore_interrupt_mask(&mut core, interrupt_mask);
                match (result, restore_mask) {
                    (Ok(result), Ok(())) => Ok(result),
                    (Err(error), Ok(())) => Err(error),
                    (_, Err(error)) => Err(error),
                }
            })();
            let restore_breakpoints = self.restore_user_breakpoints(&installed_breakpoints);
            match (step_result, restore_breakpoints) {
                (Ok(result), Ok(())) => result,
                (Err(error), Ok(())) => return Err(error),
                (Ok(_), Err(error)) => return Err(error),
                (Err(step_error), Err(restore_error)) => {
                    return Err(format!("{step_error}；{restore_error}"));
                }
            }
        };
        if kind != StepKind::Instruction
            && status.is_halted()
            && normalize_code_address(actual_pc) == normalize_code_address(origin_pc)
        {
            return Err(format!(
                "源码单步没有找到可靠的下一落点，目标仍停在 0x{actual_pc:08X}；请使用指令步进"
            ));
        }
        self.target_state = target_state(status);
        if self.target_state.is_halted() {
            let hit_user_breakpoint = self.breakpoints.values().any(|breakpoint| {
                breakpoint.enabled
                    && breakpoint.verified
                    && breakpoint.address.map(normalize_code_address)
                        == Some(normalize_code_address(actual_pc))
            });
            if let CoreStatus::Halted(reason) = status
                && matches!(
                    reason,
                    HaltReason::Step
                        | HaltReason::Request
                        | HaltReason::Multiple
                        | HaltReason::Breakpoint(BreakpointCause::Unknown)
                )
            {
                self.target_state = TargetState::Halted {
                    reason: if hit_user_breakpoint {
                        "用户断点".to_owned()
                    } else if kind == StepKind::Instruction {
                        "指令步进".to_owned()
                    } else {
                        "源码步进".to_owned()
                    },
                };
            }
            self.running.store(false, Ordering::Release);
            self.stop_id = self.stop_id.wrapping_add(1);
            self.refresh_halted_data_at(Some(actual_pc))
        } else {
            self.invalidate_halted_data();
            if self.acquisition_requested.load(Ordering::Acquire) {
                self.running.store(true, Ordering::Release);
            }
            Err(format!(
                "单步命令结束后目标没有暂停，当前状态：{:?}",
                self.target_state
            ))
        }
    }

    fn suspend_user_breakpoints(&mut self) -> Result<Vec<u64>, String> {
        let addresses = self
            .breakpoints
            .values()
            .filter(|breakpoint| breakpoint.enabled && breakpoint.verified)
            .filter_map(|breakpoint| breakpoint.address)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut cleared = Vec::new();
        for address in &addresses {
            if let Err(error) = self.probe.clear_hw_breakpoint(*address) {
                for cleared_address in &cleared {
                    let _ = self.probe.set_hw_breakpoint(*cleared_address);
                }
                return Err(format!(
                    "源码步进前暂时卸载用户断点 0x{address:08X} 失败: {error}"
                ));
            }
            cleared.push(*address);
        }
        Ok(addresses)
    }

    fn restore_user_breakpoints(&mut self, addresses: &[u64]) -> Result<(), String> {
        let mut errors = Vec::new();
        for address in addresses {
            if let Err(error) = self.probe.set_hw_breakpoint(*address) {
                errors.push(format!("0x{address:08X}: {error}"));
                for breakpoint in self.breakpoints.values_mut() {
                    if breakpoint.address == Some(*address) {
                        breakpoint.verified = false;
                        breakpoint.message = Some(format!("源码步进后恢复失败: {error}"));
                    }
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!("源码步进后恢复用户断点失败: {}", errors.join("；")))
        }
    }

    fn poll_target_status(&mut self) {
        let status = self
            .probe
            .session_mut()
            .and_then(|session| session.core(0))
            .and_then(|mut core| core.status());
        let Ok(status) = status else {
            return;
        };
        let new_state = target_state(status);
        let became_halted = new_state.is_halted() && !self.target_state.is_halted();
        let changed = new_state != self.target_state;
        self.target_state = new_state;
        if became_halted {
            self.running.store(false, Ordering::Release);
            self.clear_temporary_breakpoint();
            self.stop_id = self.stop_id.wrapping_add(1);
            match self.refresh_halted_data() {
                Ok(()) => self.publish_debug(None),
                Err(error) => {
                    self.debug_revision = self.debug_revision.wrapping_add(1);
                    let mut snapshot = self.build_snapshot();
                    snapshot.last_error = Some(error);
                    self.emit(ProbeEvent::DebugUpdated {
                        request_id: None,
                        snapshot: Box::new(snapshot),
                    });
                }
            }
        } else if changed {
            if !self.target_state.is_halted() {
                self.invalidate_halted_data();
            }
            self.publish_debug(None);
        }
    }

    fn refresh_halted_data(&mut self) -> Result<(), String> {
        self.refresh_halted_data_at(None)
    }

    fn refresh_halted_data_at(&mut self, known_pc: Option<u64>) -> Result<(), String> {
        if !self.target_state.is_halted() {
            return Err("目标运行时不能读取调试快照".to_owned());
        }
        self.latest_debug_warnings.clear();
        let debug_info = self.debug_info.as_ref();
        let mut core = self
            .probe
            .session_mut()
            .and_then(|session| session.core(0))
            .map_err(|error| format!("获取核心失败: {error}"))?;
        core.spill_registers()
            .map_err(|error| format!("保存窗口寄存器失败: {error}"))?;
        let registers = DebugRegisters::from_core(&mut core);
        let mut warnings = Vec::new();
        // The register snapshot is authoritative. Stepping helpers may report an
        // intermediate or Thumb-tagged address while the core has already halted elsewhere.
        let pc = registers
            .get_program_counter()
            .and_then(|register| register.value)
            .and_then(register_value_u64)
            .or(known_pc);
        let register_views = registers
            .0
            .iter()
            .map(|register| RegisterView {
                name: register.get_register_name(),
                value: register.value.map(|value| value.to_string()),
            })
            .collect();
        let stack_memory = match registers
            .get_stack_pointer()
            .and_then(|register| register.value)
            .and_then(register_value_u64)
            .map(|stack_pointer| read_stack_memory(&mut core, stack_pointer))
        {
            Some(Ok(memory)) => memory,
            Some(Err(error)) => {
                warnings.push(error);
                Vec::new()
            }
            None => Vec::new(),
        };
        let instructions = if let Some(pc) = pc {
            if let Some(instructions) = self
                .program_code
                .as_ref()
                .filter(|code| code.contains_address(pc))
                .and_then(|code| code.disassemble(Some(pc), debug_info).ok())
            {
                instructions
            } else {
                match disassemble_around_pc(&mut core, debug_info, pc) {
                    Ok(instructions) => instructions,
                    Err(error) => {
                        warnings.push(error);
                        Vec::new()
                    }
                }
            }
        } else {
            self.program_code
                .as_ref()
                .and_then(|code| code.disassemble(None, debug_info).ok())
                .unwrap_or_default()
        };
        let stack_frames = if let Some(debug_info) = debug_info {
            let exception_handler = exception_handler_for_core(core.core_type());
            let instruction_set = core.instruction_set().ok();
            match debug_info.unwind(
                &mut core,
                registers.clone(),
                exception_handler.as_ref(),
                instruction_set,
                64,
            ) {
                Ok(frames) => frames,
                Err(error) => {
                    warnings.push(format!("调用栈展开失败: {error}"));
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        drop(core);

        self.stack_frames = stack_frames;
        self.selected_frame = 0;
        self.latest_pc = pc;
        self.latest_registers = register_views;
        self.latest_instructions = instructions;
        self.latest_stack_memory = stack_memory;
        if let Err(error) = self.expand_frame_root(0) {
            warnings.push(error);
        }
        self.latest_debug_warnings = warnings;
        Ok(())
    }

    fn expand_frame_root(&mut self, frame_index: usize) -> Result<(), String> {
        if frame_index >= self.stack_frames.len() {
            return Ok(());
        }
        let debug_info = self
            .debug_info
            .as_ref()
            .ok_or_else(|| "请先加载 ELF 调试信息".to_owned())?;
        let frame = &mut self.stack_frames[frame_index];
        let Some(cache) = frame.local_variables.as_mut() else {
            return Ok(());
        };
        let registers = frame.registers.clone();
        let frame_info = StackFrameInfo {
            registers: &registers,
            frame_base: frame.frame_base,
            canonical_frame_address: frame.canonical_frame_address,
        };
        let mut core = self
            .probe
            .session_mut()
            .and_then(|session| session.core(0))
            .map_err(|error| format!("获取核心失败: {error}"))?;
        cache.recurse_deferred_variables(debug_info, &mut core, 2, frame_info);
        drop(core);
        self.refresh_cpp_local_values(frame_index)
    }

    fn expand_variable(&mut self, frame_index: usize, variable_ref: i64) -> Result<(), String> {
        let debug_info = self
            .debug_info
            .as_ref()
            .ok_or_else(|| "请先加载 ELF 调试信息".to_owned())?;
        let frame = self
            .stack_frames
            .get_mut(frame_index)
            .ok_or_else(|| "栈帧不存在或已失效".to_owned())?;
        let cache = frame
            .local_variables
            .as_mut()
            .ok_or_else(|| "该栈帧没有局部变量信息".to_owned())?;
        let key = ObjectRef::from(variable_ref);
        let mut variable = cache
            .get_variable_by_key(key)
            .ok_or_else(|| "局部变量不存在或已失效".to_owned())?;
        let registers = frame.registers.clone();
        let frame_info = StackFrameInfo {
            registers: &registers,
            frame_base: frame.frame_base,
            canonical_frame_address: frame.canonical_frame_address,
        };
        let mut core = self
            .probe
            .session_mut()
            .and_then(|session| session.core(0))
            .map_err(|error| format!("获取核心失败: {error}"))?;
        debug_info
            .cache_deferred_variables(cache, &mut core, &mut variable, frame_info)
            .map_err(|error| format!("展开局部变量失败: {error}"))?;
        drop(core);
        self.refresh_cpp_local_values(frame_index)
    }

    fn write_local_variable(
        &mut self,
        frame_index: usize,
        variable_ref: i64,
        new_value: String,
    ) -> Result<(), String> {
        if !self.target_state.is_halted() {
            return Err("目标运行时不能写入局部变量".to_owned());
        }
        let frame = self
            .stack_frames
            .get_mut(frame_index)
            .ok_or_else(|| "栈帧不存在或已失效".to_owned())?;
        let cache = frame
            .local_variables
            .as_mut()
            .ok_or_else(|| "该栈帧没有局部变量信息".to_owned())?;
        let key = ObjectRef::from(variable_ref);
        let variable = cache
            .get_variable_by_key(key)
            .ok_or_else(|| "局部变量不存在或已失效".to_owned())?;
        if !matches!(variable.memory_location, VariableLocation::Address(_)) {
            return Err(format!(
                "局部变量 {} 不在可写内存中，当前位置为 {}",
                variable.name, variable.memory_location
            ));
        }
        if cache.has_children(&variable) {
            return Err("当前只支持写入标量局部变量".to_owned());
        }

        let original_language = variable.language;
        let original_type = variable.type_name.clone();
        let mut writable_variable = variable;
        if is_cpp_language(writable_variable.language) {
            writable_variable.language = gimli_debug::DW_LANG_C;
            if matches!(&writable_variable.type_name, VariableType::Base(name) if name == "bool") {
                writable_variable.type_name = VariableType::Base("_Bool".to_owned());
            }
            writable_variable.set_value(VariableValue::Valid(new_value.trim().to_owned()));
        }
        let mut core = self
            .probe
            .session_mut()
            .and_then(|session| session.core(0))
            .map_err(|error| format!("写入局部变量时获取核心失败: {error}"))?;
        writable_variable
            .update_value(&mut core, cache, new_value.trim().to_owned())
            .map_err(|error| format!("写入局部变量失败: {error}"))?;
        drop(core);

        if is_cpp_language(original_language)
            && let Some(mut updated) = cache.get_variable_by_key(key)
        {
            updated.language = original_language;
            updated.type_name = original_type;
            cache
                .update_variable(&updated)
                .map_err(|error| format!("恢复 C++ 局部变量元数据失败: {error}"))?;
        }
        self.local_value_overrides.remove(&variable_ref);
        self.refresh_cpp_local_values(frame_index)
    }

    fn refresh_cpp_local_values(&mut self, frame_index: usize) -> Result<(), String> {
        let variables = self
            .stack_frames
            .get(frame_index)
            .and_then(|frame| frame.local_variables.as_ref())
            .map(all_cached_variables)
            .unwrap_or_default();
        let debug_info = self
            .debug_info
            .as_ref()
            .ok_or_else(|| "请先加载 ELF 调试信息".to_owned())?;
        let endian = debug_info.endianness();
        let mut core = self
            .probe
            .session_mut()
            .and_then(|session| session.core(0))
            .map_err(|error| format!("获取核心失败: {error}"))?;
        self.local_value_overrides.clear();
        for variable in variables {
            if !is_cpp_language(variable.language) {
                continue;
            }
            if let Some(value) = read_cpp_variable_value(&mut core, &variable, endian) {
                self.local_value_overrides
                    .insert(i64::from(variable.variable_key()), value);
            }
        }
        Ok(())
    }

    fn install_breakpoint(&mut self, id: u64) -> Result<(), String> {
        let Some(view) = self.breakpoints.get(&id) else {
            return Err("断点不存在".to_owned());
        };
        if !view.enabled {
            return Ok(());
        }
        let spec = view.spec.clone();
        let resolution = match self.resolve_breakpoint(&spec) {
            Ok(resolution) => resolution,
            Err(error) => {
                if let Some(view) = self.breakpoints.get_mut(&id) {
                    view.address = None;
                    view.verified = false;
                    view.message = Some(error.clone());
                    view.resolved_source = None;
                }
                return Err(error);
            }
        };
        let address = resolution.address;
        let resolution_message = breakpoint_resolution_message(&spec, &resolution);
        if !self.debug_active {
            if let Some(view) = self.breakpoints.get_mut(&id) {
                view.address = Some(address);
                view.verified = false;
                view.resolved_source = resolution.source;
                view.message = Some(match resolution_message {
                    Some(message) => format!("{message}；启动调试后安装"),
                    None => "已解析；启动调试后安装".to_owned(),
                });
            }
            return Ok(());
        }
        if !self.probe.connected {
            if let Some(view) = self.breakpoints.get_mut(&id) {
                view.address = Some(address);
                view.verified = false;
                view.resolved_source = resolution.source;
                view.message = Some(match resolution_message {
                    Some(message) => format!("{message}；连接目标后安装"),
                    None => "已解析；连接目标后安装".to_owned(),
                });
            }
            return Ok(());
        }
        let result = self.probe.set_hw_breakpoint(address);
        let view = self.breakpoints.get_mut(&id).expect("breakpoint exists");
        view.address = Some(address);
        view.resolved_source = resolution.source;
        match result {
            Ok(()) => {
                view.verified = true;
                view.message = resolution_message;
                Ok(())
            }
            Err(error) => {
                view.verified = false;
                view.message = Some(error.to_string());
                Err(format!("安装断点失败: {error}"))
            }
        }
    }

    fn remove_breakpoint(&mut self, id: u64) -> Result<(), String> {
        let Some(view) = self.breakpoints.remove(&id) else {
            return Err("断点不存在".to_owned());
        };
        if view.verified
            && let Some(address) = view.address
            && self.probe.connected
        {
            self.probe
                .clear_hw_breakpoint(address)
                .map_err(|error| format!("清除断点失败: {error}"))?;
        }
        Ok(())
    }

    fn set_breakpoint_enabled(&mut self, id: u64, enabled: bool) -> Result<(), String> {
        let (was_verified, address) = {
            let view = self
                .breakpoints
                .get_mut(&id)
                .ok_or_else(|| "断点不存在".to_owned())?;
            let old = (view.verified, view.address);
            view.enabled = enabled;
            view.verified = false;
            old
        };
        if enabled {
            self.install_breakpoint(id)
        } else if was_verified && self.probe.connected {
            if let Some(address) = address {
                self.probe
                    .clear_hw_breakpoint(address)
                    .map_err(|error| format!("禁用断点失败: {error}"))?;
            }
            Ok(())
        } else {
            Ok(())
        }
    }

    fn resolve_breakpoint(&self, spec: &BreakpointSpec) -> Result<BreakpointResolution, String> {
        match spec {
            BreakpointSpec::Instruction { address } => Ok(BreakpointResolution {
                address: normalize_code_address(*address),
                source: self
                    .debug_info
                    .as_ref()
                    .and_then(|debug_info| debug_info.get_source_location(*address))
                    .as_ref()
                    .map(source_location_view),
            }),
            BreakpointSpec::Source { path, line, column } => {
                let debug_info = self
                    .debug_info
                    .as_ref()
                    .ok_or_else(|| "请先加载 ELF 调试信息".to_owned())?;
                let candidates = source_path_candidates(path, &self.source_files)?;
                let mut resolved = Vec::new();
                let mut errors = Vec::new();
                for candidate in candidates {
                    let candidate_result = match debug_info.get_breakpoint_location(
                        typed_path::TypedPath::derive(candidate.as_bytes()),
                        *line,
                        *column,
                    ) {
                        Ok(verified) => Some(BreakpointResolution {
                            address: normalize_code_address(verified.address),
                            source: Some(source_location_view(&verified.source_location)),
                        }),
                        Err(error) => {
                            errors.push(error.to_string());
                            nearest_executable_source_line(
                                &self.source_lines,
                                &candidate,
                                *line,
                                32,
                            )
                            .map(|record| BreakpointResolution {
                                address: normalize_code_address(record.address),
                                source: Some(SourceLocationView {
                                    path: record.path.clone(),
                                    line: Some(record.line),
                                    column: None,
                                }),
                            })
                        }
                    };
                    if let Some(result) = candidate_result
                        && !resolved.iter().any(|existing: &BreakpointResolution| {
                            existing.address == result.address
                        })
                    {
                        resolved.push(result);
                    }
                }
                match resolved.len() {
                    1 => Ok(resolved.remove(0)),
                    0 => Err(format!(
                        "源码断点无法解析：{}:{}。该行及之后 32 行没有可暂停指令。{}",
                        path,
                        line,
                        errors
                            .first()
                            .map(|error| format!(" 首次错误：{error}"))
                            .unwrap_or_default()
                    )),
                    count => Err(format!(
                        "源码路径不唯一：{path}:{line} 可解析到 {count} 个不同地址，请从工程树选择完整路径"
                    )),
                }
            }
        }
    }

    fn reconcile_breakpoints(&mut self) {
        let ids: Vec<u64> = self.breakpoints.keys().copied().collect();
        for id in ids {
            if let Some(view) = self.breakpoints.get_mut(&id) {
                view.verified = false;
                view.address = None;
                view.message = None;
                view.resolved_source = None;
            }
            let _ = self.install_breakpoint(id);
        }
    }

    fn clear_installed_breakpoints(&mut self) {
        let addresses: Vec<u64> = self
            .breakpoints
            .values()
            .filter(|breakpoint| breakpoint.verified)
            .filter_map(|breakpoint| breakpoint.address)
            .collect();
        if self.probe.connected {
            for address in addresses {
                let _ = self.probe.clear_hw_breakpoint(address);
            }
        }
        for breakpoint in self.breakpoints.values_mut() {
            breakpoint.verified = false;
            breakpoint.address = None;
            breakpoint.resolved_source = None;
        }
    }

    fn replace_breakpoints(
        &mut self,
        breakpoints: Vec<crate::model::LogicalBreakpoint>,
    ) -> Result<(), String> {
        let old_ids: Vec<u64> = self.breakpoints.keys().copied().collect();
        for id in old_ids {
            let _ = self.remove_breakpoint(id);
        }
        self.breakpoints = breakpoints
            .into_iter()
            .map(|logical| {
                (
                    logical.id,
                    BreakpointView {
                        id: logical.id,
                        spec: logical.spec,
                        address: None,
                        enabled: logical.enabled,
                        verified: false,
                        message: None,
                        resolved_source: None,
                    },
                )
            })
            .collect();
        self.reconcile_breakpoints();
        Ok(())
    }

    fn invalidate_halted_data(&mut self) {
        self.stack_frames.clear();
        self.selected_frame = 0;
        self.latest_pc = None;
        self.latest_registers.clear();
        self.latest_stack_memory.clear();
        self.latest_debug_warnings.clear();
        self.local_value_overrides.clear();
    }

    fn clear_temporary_breakpoint(&mut self) {
        if let Some(address) = self.temporary_breakpoint.take()
            && self.probe.connected
        {
            let _ = self.probe.clear_hw_breakpoint(address);
        }
    }

    fn publish_debug(&mut self, request_id: Option<u64>) {
        self.debug_revision = self.debug_revision.wrapping_add(1);
        self.emit(ProbeEvent::DebugUpdated {
            request_id,
            snapshot: Box::new(self.build_snapshot()),
        });
    }

    fn build_snapshot(&self) -> DebugSnapshot {
        let frames = self
            .stack_frames
            .iter()
            .enumerate()
            .map(|(index, frame)| StackFrameView {
                index,
                function: frame.function_name.clone(),
                pc: register_value_u64(frame.pc).unwrap_or_default(),
                source: frame.source_location.as_ref().map(source_location_view),
                is_inline: frame.is_inlined,
            })
            .collect();
        let variables = self
            .stack_frames
            .get(self.selected_frame)
            .and_then(|frame| frame.local_variables.as_ref())
            .map(|cache| flatten_variables(cache, &self.local_value_overrides))
            .unwrap_or_default();
        DebugSnapshot {
            revision: self.debug_revision,
            program_generation: self.program_generation,
            stop_id: self.stop_id,
            target_state: self.target_state.clone(),
            active: self.debug_active,
            pc: self.latest_pc,
            registers: self.latest_registers.clone(),
            frames,
            selected_frame: self.selected_frame,
            variables,
            breakpoints: self.breakpoints.values().cloned().collect(),
            instructions: self.latest_instructions.clone(),
            stack_memory: self.latest_stack_memory.clone(),
            warnings: self.latest_debug_warnings.clone(),
            breakpoint_capacity: self.breakpoint_capacity,
            program_path: self.program_path.clone(),
            source_files: self.source_files.clone(),
            executable_lines: self.source_lines.clone(),
            ..Default::default()
        }
    }

    fn observe_link_health(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) => self.link_failures = 0,
            Err(error) => {
                self.link_failures = self.link_failures.saturating_add(1);
                if self.link_failures >= Self::FAILURE_LIMIT {
                    self.running.store(false, Ordering::Release);
                    self.acquisition_requested.store(false, Ordering::Release);
                    self.debug_active = false;
                    self.probe.disconnect();
                    self.target_state = TargetState::Disconnected;
                    self.invalidate_halted_data();
                    self.breakpoint_capacity = None;
                    self.publish_debug(None);
                    self.emit(ProbeEvent::LinkLost(error));
                }
            }
        }
    }

    fn emit(&self, event: ProbeEvent) {
        let _ = self.event_sender.send(event);
        self.repaint_ctx.request_repaint();
    }
}

enum WorkerControl {
    Continue,
    Shutdown,
}

struct BreakpointResolution {
    address: u64,
    source: Option<SourceLocationView>,
}

fn breakpoint_resolution_message(
    spec: &BreakpointSpec,
    resolution: &BreakpointResolution,
) -> Option<String> {
    let BreakpointSpec::Source { path, line, .. } = spec else {
        return None;
    };
    let source = resolution.source.as_ref()?;
    let resolved_line = source.line.unwrap_or_default();
    let path_changed = normalized_source_path(path) != normalized_source_path(&source.path);
    (path_changed || resolved_line != *line).then(|| {
        format!(
            "请求 {}:{}，实际解析到 {}:{} @ 0x{:08X}",
            path, line, source.path, resolved_line, resolution.address
        )
    })
}

fn normalized_source_path(path: &str) -> String {
    let replaced = path.replace('\\', "/");
    let absolute = replaced.starts_with('/');
    let mut components = Vec::new();
    for component in replaced.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            other => components.push(other),
        }
    }
    let normalized = components.join("/");
    if absolute {
        format!("/{normalized}")
    } else {
        normalized
    }
}

fn source_path_candidates(requested: &str, source_files: &[String]) -> Result<Vec<String>, String> {
    let requested_normalized = normalized_source_path(requested);
    let exact: Vec<String> = source_files
        .iter()
        .filter(|candidate| normalized_source_path(candidate) == requested_normalized)
        .cloned()
        .collect();
    if !exact.is_empty() {
        return Ok(exact);
    }

    let requested_components: Vec<&str> = requested_normalized.split('/').collect();
    for suffix_len in (1..=requested_components.len()).rev() {
        let suffix = requested_components[requested_components.len() - suffix_len..].join("/");
        let candidates: Vec<String> = source_files
            .iter()
            .filter(|candidate| {
                let candidate = normalized_source_path(candidate);
                candidate == suffix || candidate.ends_with(&format!("/{suffix}"))
            })
            .cloned()
            .collect();
        if candidates.len() == 1 {
            return Ok(candidates);
        }
        if candidates.len() > 1 {
            if suffix_len > 1 {
                return Ok(candidates);
            }
            return Err(format!(
                "源码文件名不唯一，请从工程树选择完整路径：{}",
                candidates.join(", ")
            ));
        }
    }

    // Still try the exact user input so DebugInfo can report its authoritative error.
    Ok(vec![requested.to_owned()])
}

fn nearest_executable_source_line<'a>(
    source_lines: &'a [SourceLineRecord],
    path: &str,
    requested_line: u64,
    forward_limit: u64,
) -> Option<&'a SourceLineRecord> {
    let normalized_path = normalized_source_path(path);
    let max_line = requested_line.saturating_add(forward_limit);
    source_lines
        .iter()
        .filter(|record| {
            normalized_source_path(&record.path) == normalized_path
                && record.line >= requested_line
                && record.line <= max_line
        })
        .min_by_key(|record| (record.line, record.address))
}

#[allow(dead_code)]
fn _assert_worker_payloads_are_send(_buffer: Arc<RingBuffer<(f64, [u8; 8])>>) {}

fn target_state(status: CoreStatus) -> TargetState {
    match status {
        CoreStatus::Running => TargetState::Running,
        CoreStatus::Sleeping => TargetState::Sleeping,
        CoreStatus::Halted(reason) => TargetState::Halted {
            reason: format!("{reason:?}"),
        },
        CoreStatus::LockedUp => TargetState::LockedUp,
        CoreStatus::Unknown => TargetState::Unknown,
    }
}

fn normalize_code_address(address: u64) -> u64 {
    address & !1
}

fn single_step_source(
    core: &mut probe_rs::Core<'_>,
    debug_info: &DebugInfo,
    kind: StepKind,
    origin_pc: u64,
) -> Result<(CoreStatus, u64), String> {
    let origin = debug_info
        .get_source_location(origin_pc)
        .as_ref()
        .map(source_location_view)
        .ok_or_else(|| format!("PC 0x{origin_pc:08X} 没有 DWARF 源位置"))?;
    if origin.line.is_none() {
        return Err(format!("PC 0x{origin_pc:08X} 的 DWARF 源位置没有行号"));
    }
    let origin_depth = if kind == StepKind::Into {
        None
    } else {
        Some(current_stack_depth(core, debug_info)?)
    };
    let mut previous_pc = normalize_code_address(origin_pc);
    let mut last_location = Some(origin.clone());

    for step_count in 1..=4096 {
        let information = core
            .step()
            .map_err(|error| format!("源码{kind:?}的第 {step_count} 次指令步进失败: {error}"))?;
        let pc = normalize_code_address(information.pc);
        if pc == previous_pc {
            return Err(format!("源码{kind:?}没有前进，目标仍停在 0x{pc:08X}"));
        }
        previous_pc = pc;
        let status = core
            .status()
            .map_err(|error| format!("读取源码步进状态失败: {error}"))?;
        if !status.is_halted() {
            return Err(format!(
                "源码{kind:?}的指令步进后目标未暂停，当前状态: {status:?}"
            ));
        }
        let current = debug_info
            .get_source_location(pc)
            .as_ref()
            .map(source_location_view);
        let location_changed = !same_source_line(current.as_ref(), last_location.as_ref());
        if location_changed {
            last_location = current.clone();
        }
        let left_origin = !same_source_line(current.as_ref(), Some(&origin));
        if current.is_some() && left_origin {
            match kind {
                StepKind::Into => return Ok((status, information.pc)),
                StepKind::Over if location_changed => {
                    if current_stack_depth(core, debug_info)? <= origin_depth.unwrap_or(1) {
                        return Ok((status, information.pc));
                    }
                }
                StepKind::Out if location_changed => {
                    if current_stack_depth(core, debug_info)? < origin_depth.unwrap_or(1) {
                        return Ok((status, information.pc));
                    }
                }
                StepKind::Instruction | StepKind::Over | StepKind::Out => {}
            }
        } else if kind == StepKind::Out
            && step_count % 16 == 0
            && current_stack_depth(core, debug_info)? < origin_depth.unwrap_or(1)
        {
            return Ok((status, information.pc));
        }
    }
    Err(format!(
        "源码{kind:?}执行了 4096 条指令后仍未达到新的有效 DWARF 停止位置"
    ))
}

fn same_source_line(left: Option<&SourceLocationView>, right: Option<&SourceLocationView>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            normalized_source_path(&left.path) == normalized_source_path(&right.path)
                && left.line == right.line
        }
        (None, None) => true,
        _ => false,
    }
}

fn current_stack_depth(
    core: &mut probe_rs::Core<'_>,
    debug_info: &DebugInfo,
) -> Result<usize, String> {
    core.spill_registers()
        .map_err(|error| format!("源码步进读取调用栈前保存寄存器失败: {error}"))?;
    let registers = DebugRegisters::from_core(core);
    let exception_handler = exception_handler_for_core(core.core_type());
    let instruction_set = core.instruction_set().ok();
    debug_info
        .unwind(
            core,
            registers,
            exception_handler.as_ref(),
            instruction_set,
            32,
        )
        .map(|frames| frames.len().max(1))
        .map_err(|error| format!("源码步进判断调用栈深度失败: {error}"))
}

fn mask_interrupts_for_source_step(
    core: &mut probe_rs::Core<'_>,
) -> Result<Option<(RegisterId, RegisterValue)>, String> {
    // Cortex-M exposes PRIMASK as bit 0 of the packed EXTRA/EXTRA_S register.
    // Temporarily mask configurable interrupts so source stepping cannot be
    // diverted into an ISR. Other architectures simply have no such register.
    for name in ["EXTRA", "EXTRA_S"] {
        let Some(register) = core.registers().other_by_name(name) else {
            continue;
        };
        let Ok(original) = core.read_core_reg(register.id()) else {
            continue;
        };
        let Some(masked) = cortex_m_interrupt_masked(original) else {
            continue;
        };
        if core.write_core_reg(register.id(), masked).is_ok() {
            let verified: Option<RegisterValue> = core.read_core_reg(register.id()).ok();
            if verified
                .is_some_and(|value| matches!(value, RegisterValue::U32(value) if value & 1 != 0))
            {
                return Ok(Some((register.id(), original)));
            }
            let _ = core.write_core_reg(register.id(), original);
        }
    }
    if core.core_type().is_cortex_m() {
        Err("无法确认 Cortex-M PRIMASK 已生效，为避免步进进入中断，已取消本次源码步进".to_owned())
    } else {
        Ok(None)
    }
}

fn cortex_m_interrupt_masked(value: RegisterValue) -> Option<RegisterValue> {
    match value {
        RegisterValue::U32(value) => Some(RegisterValue::U32(value | 1)),
        RegisterValue::U64(_) | RegisterValue::U128(_) => None,
    }
}

fn restore_interrupt_mask(
    core: &mut probe_rs::Core<'_>,
    saved: Option<(RegisterId, RegisterValue)>,
) -> Result<(), String> {
    let Some((register, value)) = saved else {
        return Ok(());
    };
    core.write_core_reg(register, value)
        .map_err(|error| format!("恢复单步前中断屏蔽状态失败: {error}"))
}

fn register_value_u64(value: RegisterValue) -> Option<u64> {
    match value {
        RegisterValue::U32(value) => Some(u64::from(value)),
        RegisterValue::U64(value) => Some(value),
        RegisterValue::U128(value) => u64::try_from(value).ok(),
    }
}

fn source_location_view(source: &probe_rs_debug::SourceLocation) -> SourceLocationView {
    SourceLocationView {
        path: source.path.to_path().display().to_string(),
        line: source.line,
        column: source.column.map(|column| match column {
            probe_rs_debug::ColumnType::LeftEdge => 0,
            probe_rs_debug::ColumnType::Column(column) => column,
        }),
    }
}

fn flatten_variables(
    cache: &VariableCache,
    value_overrides: &std::collections::HashMap<i64, String>,
) -> Vec<VariableView> {
    fn visit(
        cache: &VariableCache,
        value_overrides: &std::collections::HashMap<i64, String>,
        parent: ObjectRef,
        display_parent: i64,
        out: &mut Vec<VariableView>,
    ) {
        let children: Vec<_> = cache.get_children(parent).cloned().collect();
        for variable in children {
            let reference = i64::from(variable.variable_key());
            let has_children =
                cache.has_children(&variable) || variable.variable_node_type.is_deferred();
            let mut value = value_overrides
                .get(&reference)
                .cloned()
                .unwrap_or_else(|| variable.to_string(cache));
            if is_cpp_language(variable.language)
                && value.contains("Reading variables for language")
            {
                value = if has_children {
                    format!("<{}>", variable.type_name())
                } else {
                    "<当前值不可用>".to_owned()
                };
            }
            out.push(VariableView {
                reference,
                parent_reference: display_parent,
                name: variable.name.to_string(),
                type_name: if is_cpp_language(variable.language) {
                    cpp_type_name(&variable.type_name)
                } else {
                    variable.type_name()
                },
                value,
                has_children,
                writable: !has_children
                    && matches!(variable.memory_location, VariableLocation::Address(_))
                    && matches!(
                        variable.type_name.inner(),
                        VariableType::Base(_) | VariableType::Bitfield(_, _)
                    ),
            });
            visit(
                cache,
                value_overrides,
                variable.variable_key(),
                reference,
                out,
            );
        }
    }

    let mut variables = Vec::new();
    visit(
        cache,
        value_overrides,
        cache.root_variable().variable_key(),
        0,
        &mut variables,
    );
    variables
}

fn all_cached_variables(cache: &VariableCache) -> Vec<Variable> {
    fn visit(cache: &VariableCache, parent: ObjectRef, out: &mut Vec<Variable>) {
        let children: Vec<_> = cache.get_children(parent).cloned().collect();
        for variable in children {
            visit(cache, variable.variable_key(), out);
            out.push(variable);
        }
    }

    let mut variables = Vec::new();
    visit(cache, cache.root_variable().variable_key(), &mut variables);
    variables
}

fn is_cpp_language(language: gimli_debug::DwLang) -> bool {
    matches!(
        language,
        gimli_debug::DW_LANG_C_plus_plus
            | gimli_debug::DW_LANG_C_plus_plus_03
            | gimli_debug::DW_LANG_C_plus_plus_11
            | gimli_debug::DW_LANG_C_plus_plus_14
    )
}

fn cpp_type_name(variable_type: &VariableType) -> String {
    match variable_type {
        VariableType::Base(name)
        | VariableType::Struct(name)
        | VariableType::Enum(name)
        | VariableType::Other(name) => name.clone(),
        VariableType::Namespace => "namespace".to_owned(),
        VariableType::Pointer(pointee) => format!("{}*", pointee.as_deref().unwrap_or("void")),
        VariableType::Array {
            item_type_name,
            count,
        } => format!("{}[{count}]", cpp_type_name(item_type_name)),
        VariableType::Modified(modifier, inner) => match modifier {
            Modifier::Typedef(name) => name.clone(),
            Modifier::Const => format!("const {}", cpp_type_name(inner)),
            Modifier::Volatile => format!("volatile {}", cpp_type_name(inner)),
            Modifier::Restrict => format!("restrict {}", cpp_type_name(inner)),
            Modifier::Atomic => format!("_Atomic {}", cpp_type_name(inner)),
        },
        VariableType::Bitfield(bitfield, inner) => {
            format!("{} {{{} bits}}", cpp_type_name(inner), bitfield.length)
        }
        VariableType::Unknown => "<unknown>".to_owned(),
    }
}

fn read_cpp_variable_value(
    core: &mut probe_rs::Core<'_>,
    variable: &Variable,
    endian: gimli_debug::RunTimeEndian,
) -> Option<String> {
    let size = usize::try_from(variable.byte_size?).ok()?;
    if size == 0 || size > 16 {
        return None;
    }
    let (mut bytes, from_register) = match variable.memory_location {
        VariableLocation::Address(address) => {
            let mut bytes = vec![0u8; size];
            core.read(address, &mut bytes).ok()?;
            (bytes, false)
        }
        VariableLocation::RegisterValue(value) => {
            let mut bytes = match value {
                RegisterValue::U32(value) => value.to_le_bytes().to_vec(),
                RegisterValue::U64(value) => value.to_le_bytes().to_vec(),
                RegisterValue::U128(value) => value.to_le_bytes().to_vec(),
            };
            bytes.resize(size, 0);
            bytes.truncate(size);
            (bytes, true)
        }
        _ => return None,
    };
    if !from_register && endian == gimli_debug::RunTimeEndian::Big {
        bytes.reverse();
    }
    let raw = bytes
        .iter()
        .enumerate()
        .fold(0u128, |value, (index, byte)| {
            value | (u128::from(*byte) << (index * 8))
        });
    let ty = variable.type_name.inner();
    match ty {
        VariableType::Base(name) if name == "float" && size == 4 => {
            Some(f32::from_bits(raw as u32).to_string())
        }
        VariableType::Base(name) if name == "double" && size == 8 => {
            Some(f64::from_bits(raw as u64).to_string())
        }
        VariableType::Base(name) if name == "bool" || name == "_Bool" => {
            Some((raw != 0).to_string())
        }
        VariableType::Base(name) if name == "char" => {
            let byte = raw as u8;
            Some(if byte.is_ascii_graphic() || byte == b' ' {
                format!("'{}' ({byte})", char::from(byte))
            } else {
                format!("'\\x{byte:02X}' ({byte})")
            })
        }
        VariableType::Base(name) if is_signed_cpp_integer(name) => {
            Some(sign_extend(raw, size).to_string())
        }
        VariableType::Base(_) => Some(format_unsigned(raw, size)),
        VariableType::Enum(_) => Some(format!("{raw} (0x{raw:X})")),
        VariableType::Pointer(_) => Some(format!("0x{raw:0width$X}", width = size * 2)),
        VariableType::Bitfield(bitfield, inner) => {
            let offset = match bitfield.offset {
                BitOffset::FromLsb(offset) => offset,
                BitOffset::FromMsb(offset) => size as u64 * 8 - offset - bitfield.length,
            };
            let mask = if bitfield.length >= 128 {
                u128::MAX
            } else {
                (1u128 << bitfield.length) - 1
            };
            let value = (raw >> offset) & mask;
            if matches!(inner.inner(), VariableType::Base(name) if is_signed_cpp_integer(name)) {
                Some(sign_extend_bits(value, bitfield.length).to_string())
            } else {
                Some(format!("{value} (0x{value:X})"))
            }
        }
        _ => None,
    }
}

fn is_signed_cpp_integer(name: &str) -> bool {
    !name.contains("unsigned")
        && matches!(
            name,
            "signed char"
                | "short"
                | "short int"
                | "short signed int"
                | "int"
                | "signed int"
                | "long"
                | "long int"
                | "long signed int"
                | "long long"
                | "long long int"
                | "long long signed int"
        )
}

fn sign_extend(raw: u128, size: usize) -> i128 {
    sign_extend_bits(raw, (size * 8) as u64)
}

fn sign_extend_bits(raw: u128, bits: u64) -> i128 {
    if bits == 0 {
        return 0;
    }
    if bits >= 128 {
        return raw as i128;
    }
    let shift = 128 - bits;
    ((raw << shift) as i128) >> shift
}

fn format_unsigned(raw: u128, size: usize) -> String {
    format!("{raw} (0x{raw:0width$X})", width = size * 2)
}

fn read_stack_memory(
    core: &mut probe_rs::Core<'_>,
    stack_pointer: u64,
) -> Result<Vec<StackMemoryWord>, String> {
    let start = stack_pointer & !3;
    let ram_ranges = core
        .memory_regions()
        .filter(|region| region.is_ram())
        .map(|region| region.address_range())
        .collect::<Vec<_>>();
    let ram_range = ram_ranges.iter().find(|range| range.contains(&start));
    if !ram_ranges.is_empty() && ram_range.is_none() {
        return Err(format!(
            "栈指针 0x{stack_pointer:08X} 不在目标描述的 RAM 区域内"
        ));
    }
    let word_limit = ram_range
        .map(|range| ((range.end.saturating_sub(start)) / 4).min(32) as usize)
        .unwrap_or(32);
    if word_limit == 0 {
        return Ok(Vec::new());
    }

    let mut words = Vec::with_capacity(word_limit);
    for index in 0..word_limit {
        let address = start + index as u64 * 4;
        match core.read_word_32(address) {
            Ok(value) => words.push(StackMemoryWord { address, value }),
            Err(error) if words.is_empty() => {
                return Err(format!(
                    "栈指针 0x{stack_pointer:08X} 所在内存不可读: {error}"
                ));
            }
            Err(_) => break,
        }
    }
    Ok(words)
}

struct ProgramCode {
    instruction_set: InstructionSet,
    little_endian: bool,
    sections: Vec<CodeSection>,
}

struct CodeSection {
    address: u64,
    bytes: Vec<u8>,
}

impl ProgramCode {
    fn load(path: &std::path::Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|error| format!("读取 ELF 失败: {error}"))?;
        let object = object::File::parse(bytes.as_slice())
            .map_err(|error| format!("解析 ELF 代码段失败: {error}"))?;
        let instruction_set = match object.architecture() {
            object::Architecture::Arm => InstructionSet::Thumb2,
            object::Architecture::Aarch64 | object::Architecture::Aarch64_Ilp32 => {
                InstructionSet::A64
            }
            object::Architecture::Riscv32 => InstructionSet::RV32C,
            other => return Err(format!("暂不支持 {other:?} ELF 的离线反汇编")),
        };
        let sections = object
            .sections()
            .filter(|section| section.kind() == object::SectionKind::Text && section.size() > 0)
            .filter_map(|section| {
                section.data().ok().map(|data| CodeSection {
                    address: section.address(),
                    bytes: data.to_vec(),
                })
            })
            .collect::<Vec<_>>();
        if sections.is_empty() {
            return Err("ELF 中没有可反汇编的代码段".to_owned());
        }
        Ok(Self {
            instruction_set,
            little_endian: object.endianness() == object::Endianness::Little,
            sections,
        })
    }

    fn disassemble(
        &self,
        around: Option<u64>,
        debug_info: Option<&DebugInfo>,
    ) -> Result<Vec<InstructionView>, String> {
        let normalized = around.map(normalize_code_address);
        let section = normalized
            .and_then(|address| {
                self.sections.iter().find(|section| {
                    address >= section.address
                        && address < section.address.saturating_add(section.bytes.len() as u64)
                })
            })
            .or_else(|| self.sections.first())
            .ok_or_else(|| "ELF 中没有可反汇编的代码段".to_owned())?;
        let alignment = u64::from(self.instruction_set.get_minimum_instruction_size());
        let requested_start = normalized
            .unwrap_or(section.address)
            .saturating_sub(64)
            .max(section.address);
        let start = requested_start & !(alignment.saturating_sub(1));
        let offset = usize::try_from(start.saturating_sub(section.address))
            .unwrap_or_default()
            .min(section.bytes.len());
        let end = offset.saturating_add(512).min(section.bytes.len());
        disassemble_bytes(
            self.instruction_set,
            self.little_endian,
            start,
            &section.bytes[offset..end],
            debug_info,
            160,
        )
    }

    fn contains_address(&self, address: u64) -> bool {
        let address = normalize_code_address(address);
        self.sections.iter().any(|section| {
            address >= section.address
                && address < section.address.saturating_add(section.bytes.len() as u64)
        })
    }
}

fn disassemble_around_pc(
    core: &mut probe_rs::Core<'_>,
    debug_info: Option<&DebugInfo>,
    pc: u64,
) -> Result<Vec<InstructionView>, String> {
    let instruction_set = core
        .instruction_set()
        .map_err(|error| format!("读取指令集失败: {error}"))?;
    let normalized_pc = normalize_code_address(pc);
    let alignment = u64::from(instruction_set.get_minimum_instruction_size());
    let region = core
        .memory_regions()
        .find(|region| region.contains(normalized_pc))
        .map(|region| region.address_range());
    let desired_start = normalized_pc.saturating_sub(32) & !(alignment.saturating_sub(1));
    let mut start = region
        .as_ref()
        .map(|range| desired_start.max(range.start))
        .unwrap_or(desired_start);
    if start % alignment != 0 {
        start = start.saturating_add(alignment - start % alignment);
    }
    let available = region
        .as_ref()
        .map(|range| range.end.saturating_sub(start))
        .unwrap_or(128)
        .min(128) as usize;
    let byte_count = available - available % alignment as usize;
    if byte_count < alignment as usize {
        return Err(format!("PC 0x{pc:08X} 不在可读取的代码内存区域"));
    }
    let mut bytes = vec![0u8; byte_count];
    match core.read(start, &mut bytes) {
        Ok(()) => disassemble_bytes(instruction_set, true, start, &bytes, debug_info, 64),
        Err(first_error) => {
            let fallback_start = normalized_pc & !(alignment.saturating_sub(1));
            let fallback_available = region
                .as_ref()
                .map(|range| range.end.saturating_sub(fallback_start))
                .unwrap_or(32)
                .min(32) as usize;
            let fallback_count = fallback_available - fallback_available % alignment as usize;
            if fallback_count < alignment as usize || fallback_start == start {
                return Err(format!("PC 0x{pc:08X} 附近的代码内存不可读: {first_error}"));
            }
            bytes.resize(fallback_count, 0);
            core.read(fallback_start, &mut bytes).map_err(|second_error| {
                format!(
                    "PC 0x{pc:08X} 附近的代码内存不可读: {first_error}; 缩小读取范围后仍失败: {second_error}"
                )
            })?;
            disassemble_bytes(
                instruction_set,
                true,
                fallback_start,
                &bytes,
                debug_info,
                32,
            )
        }
    }
}

fn disassemble_bytes(
    instruction_set: InstructionSet,
    little_endian: bool,
    start: u64,
    bytes: &[u8],
    debug_info: Option<&DebugInfo>,
    instruction_limit: usize,
) -> Result<Vec<InstructionView>, String> {
    use capstone::arch;
    use capstone::prelude::*;

    let endian = if little_endian {
        capstone::Endian::Little
    } else {
        capstone::Endian::Big
    };
    let mut capstone = match instruction_set {
        InstructionSet::Thumb2 => Capstone::new()
            .arm()
            .mode(arch::arm::ArchMode::Thumb)
            .endian(endian)
            .build(),
        InstructionSet::A32 => Capstone::new()
            .arm()
            .mode(arch::arm::ArchMode::Arm)
            .endian(endian)
            .build(),
        InstructionSet::A64 => Capstone::new()
            .arm64()
            .mode(arch::arm64::ArchMode::Arm)
            .endian(endian)
            .build(),
        InstructionSet::RV32 => Capstone::new()
            .riscv()
            .mode(arch::riscv::ArchMode::RiscV32)
            .endian(endian)
            .build(),
        InstructionSet::RV32C => Capstone::new()
            .riscv()
            .mode(arch::riscv::ArchMode::RiscV32)
            .extra_mode(std::iter::once(arch::riscv::ArchExtraMode::RiscVC))
            .endian(endian)
            .build(),
        InstructionSet::Xtensa => return Err("当前版本暂不支持 Xtensa 反汇编".to_owned()),
    }
    .map_err(|error| format!("创建反汇编器失败: {error}"))?;
    capstone
        .set_skipdata(true)
        .map_err(|error| format!("配置反汇编器失败: {error}"))?;
    let instructions = capstone
        .disasm_all(bytes, start)
        .map_err(|error| format!("反汇编失败: {error}"))?;
    Ok(instructions
        .iter()
        .take(instruction_limit)
        .map(|instruction| {
            let mnemonic = instruction.mnemonic().unwrap_or("<unknown>");
            let operands = instruction.op_str().unwrap_or("");
            InstructionView {
                address: instruction.address(),
                bytes: instruction
                    .bytes()
                    .iter()
                    .map(|byte| format!("{byte:02X}"))
                    .collect::<Vec<_>>()
                    .join(" "),
                instruction: format!("{mnemonic:<8} {operands}"),
                source: debug_info
                    .and_then(|info| info.get_source_location(instruction.address()))
                    .as_ref()
                    .map(source_location_view),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::AtomicU64;
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant};

    use probe_rs::{CoreStatus, HaltReason, MemoryInterface, RegisterValue};

    use super::{
        BreakpointView, DebugStartMode, ProbeCommand, ProbeWorker, cortex_m_interrupt_masked,
        cpp_type_name, disassemble_bytes, is_cpp_language, nearest_executable_source_line,
        normalize_code_address, normalized_source_path, register_value_u64, same_source_line,
        sign_extend_bits, source_path_candidates, target_state,
    };
    use crate::dwarf::extract::SourceLineRecord;
    use crate::model::{LogicalBreakpoint, SourceLocationView, TargetState};

    #[test]
    fn maps_probe_status_without_conflating_running_and_halted() {
        assert_eq!(target_state(CoreStatus::Running), TargetState::Running);
        assert_eq!(target_state(CoreStatus::Sleeping), TargetState::Sleeping);
        assert_eq!(
            target_state(CoreStatus::Halted(HaltReason::Breakpoint(
                probe_rs::BreakpointCause::Hardware,
            ))),
            TargetState::Halted {
                reason: "Breakpoint(Hardware)".to_owned(),
            }
        );
    }

    #[test]
    fn normalizes_thumb_addresses_and_register_widths() {
        assert_eq!(normalize_code_address(0x0800_0101), 0x0800_0100);
        assert_eq!(register_value_u64(RegisterValue::U32(42)), Some(42));
        assert_eq!(
            register_value_u64(RegisterValue::U64(u64::MAX)),
            Some(u64::MAX)
        );
        assert_eq!(register_value_u64(RegisterValue::U128(u128::MAX)), None);
        assert!(matches!(
            cortex_m_interrupt_masked(RegisterValue::U32(0x0102_0300)),
            Some(RegisterValue::U32(0x0102_0301))
        ));
        assert!(cortex_m_interrupt_masked(RegisterValue::U64(0)).is_none());
    }

    #[test]
    fn disassembles_thumb_code_without_a_live_target() {
        let instructions = disassemble_bytes(
            probe_rs::InstructionSet::Thumb2,
            true,
            0x0800_0000,
            &[0x00, 0xBF, 0x70, 0x47],
            None,
            8,
        )
        .unwrap();
        assert_eq!(instructions.len(), 2);
        assert_eq!(instructions[0].address, 0x0800_0000);
        assert!(instructions[0].instruction.contains("nop"));
    }

    #[test]
    fn recognizes_cpp_languages_and_sign_extends_values() {
        assert!(is_cpp_language(gimli_debug::DW_LANG_C_plus_plus));
        assert!(is_cpp_language(gimli_debug::DW_LANG_C_plus_plus_14));
        assert!(!is_cpp_language(gimli_debug::DW_LANG_C11));
        assert_eq!(sign_extend_bits(0xFF, 8), -1);
        assert_eq!(sign_extend_bits(0x7F, 8), 127);
        assert_eq!(
            cpp_type_name(&probe_rs_debug::VariableType::Pointer(Some(
                "Widget".to_owned()
            ))),
            "Widget*"
        );
        assert_eq!(
            cpp_type_name(&probe_rs_debug::VariableType::Array {
                item_type_name: Box::new(probe_rs_debug::VariableType::Base("int".to_owned())),
                count: 4,
            }),
            "int[4]"
        );
    }

    #[test]
    fn resolves_mapped_source_paths_by_the_longest_unique_suffix() {
        let sources = vec![
            "/build/firmware/src/main.c".to_owned(),
            "/build/firmware/drivers/gpio.c".to_owned(),
        ];
        assert_eq!(
            source_path_candidates("/workspace/src/main.c", &sources).unwrap(),
            vec!["/build/firmware/src/main.c".to_owned()]
        );
        assert_eq!(
            normalized_source_path(r"C:\firmware\src\..\src\main.c"),
            "C:/firmware/src/main.c"
        );
    }

    #[test]
    fn rejects_ambiguous_source_basenames() {
        let sources = vec![
            "/build/app/main.c".to_owned(),
            "/build/boot/main.c".to_owned(),
        ];
        assert!(source_path_candidates("main.c", &sources).is_err());
    }

    #[test]
    fn moves_source_breakpoints_to_the_next_executable_line() {
        let rows = vec![
            SourceLineRecord {
                path: "/build/src/main.c".to_owned(),
                line: 20,
                address: 0x0800_0200,
            },
            SourceLineRecord {
                path: "/build/src/main.c".to_owned(),
                line: 24,
                address: 0x0800_0240,
            },
        ];
        let resolved = nearest_executable_source_line(&rows, "/build/src/main.c", 18, 32)
            .expect("next executable line");
        assert_eq!(resolved.line, 20);
        assert_eq!(resolved.address, 0x0800_0200);
        assert!(nearest_executable_source_line(&rows, "/build/src/main.c", 21, 2).is_none());
    }

    #[test]
    fn source_line_comparison_normalizes_paths_and_ignores_columns() {
        let left = SourceLocationView {
            path: r"\src\main.c".to_owned(),
            line: Some(10),
            column: Some(4),
        };
        let same = SourceLocationView {
            path: "/src/main.c".to_owned(),
            line: Some(10),
            column: Some(4),
        };
        let next_line = SourceLocationView {
            line: Some(11),
            ..same.clone()
        };
        let other_column = SourceLocationView {
            column: Some(99),
            ..same.clone()
        };
        assert!(same_source_line(Some(&left), Some(&same)));
        assert!(same_source_line(Some(&left), Some(&other_column)));
        assert!(!same_source_line(Some(&left), Some(&next_line)));
        assert!(!same_source_line(Some(&left), None));
    }

    #[test]
    #[ignore = "requires a connected hardware probe and MEMRW3_HARDWARE_CONFIG"]
    fn hardware_step_over_leaves_the_current_dwarf_line() {
        let config_path = std::env::var("MEMRW3_HARDWARE_CONFIG")
            .expect("set MEMRW3_HARDWARE_CONFIG to a MemRW3 JSON config");
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&config_path).expect("read hardware config"))
                .expect("parse hardware config");
        let string = |key: &str| {
            config[key]
                .as_str()
                .unwrap_or_else(|| panic!("missing string config field {key}"))
                .to_owned()
        };
        let speed_khz = config["probe_speed_khz"]
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .expect("valid probe_speed_khz");
        let running = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let acquisition_requested = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let (_command_sender, command_receiver) = mpsc::channel();
        let (event_sender, _event_receiver) = mpsc::channel();
        let mut worker = ProbeWorker::new(
            running,
            acquisition_requested,
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicU64::new(0)),
            command_receiver,
            event_sender,
            eframe::egui::Context::default(),
        );
        worker.handle_command(ProbeCommand::Connect {
            chip_name: string("probe_chip"),
            protocol: string("probe_protocol"),
            speed_khz,
            selected_probe_id: config["selected_probe_id"].as_str().map(str::to_owned),
        });
        assert!(
            worker.probe.connected,
            "hardware connect failed: {:?}",
            worker.probe.last_error
        );

        let was_halted = worker
            .probe
            .session_mut()
            .and_then(|session| session.core(0))
            .and_then(|mut core| core.status())
            .is_ok_and(|status| status.is_halted());
        let test_result = (|| -> Result<(), String> {
            worker.handle_command(ProbeCommand::LoadProgram {
                path: PathBuf::from(string("elf_path")),
                generation: 1,
            });
            if worker.debug_info.is_none() {
                return Err("ELF debug info did not load".to_owned());
            }
            worker.debug_start(DebugStartMode::Attach)?;
            let breakpoint_id = 0xD06;
            let logical = LogicalBreakpoint {
                id: breakpoint_id,
                spec: crate::model::BreakpointSpec::Source {
                    path: "main.cpp".to_owned(),
                    line: 138,
                    column: None,
                },
                enabled: true,
            };
            worker.breakpoints.insert(
                breakpoint_id,
                BreakpointView {
                    id: logical.id,
                    spec: logical.spec,
                    address: None,
                    enabled: true,
                    verified: false,
                    message: None,
                    resolved_source: None,
                },
            );
            worker.install_breakpoint(breakpoint_id)?;
            if worker.target_state.is_halted() {
                worker.debug_continue()?;
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            while !worker.target_state.is_halted() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
                worker.poll_target_status();
            }
            if !worker.target_state.is_halted() {
                return Err("main.cpp:138 breakpoint was not hit within 5 seconds".to_owned());
            }
            let before_pc = worker.latest_pc.ok_or_else(|| "missing PC".to_owned())?;
            let before = worker
                .debug_info
                .as_ref()
                .and_then(|info| info.get_source_location(before_pc))
                .as_ref()
                .map(super::source_location_view)
                .ok_or_else(|| format!("no source location at 0x{before_pc:08X}"))?;
            if before.line != Some(138) {
                return Err(format!(
                    "expected main.cpp:138 before step, got {}:{:?}",
                    before.path, before.line
                ));
            }

            let local = worker
                .stack_frames
                .first()
                .and_then(|frame| frame.local_variables.as_ref())
                .and_then(|cache| {
                    super::all_cached_variables(cache)
                        .into_iter()
                        .find(|variable| variable.name.to_string() == "addr_sign")
                })
                .ok_or_else(|| "addr_sign local variable was not resolved".to_owned())?;
            let local_address = match local.memory_location {
                probe_rs_debug::VariableLocation::Address(address) => address,
                location => return Err(format!("addr_sign is not memory-backed: {location}")),
            };
            let local_reference = i64::from(local.variable_key());
            let original_raw = worker
                .probe
                .session_mut()
                .and_then(|session| session.core(0))
                .and_then(|mut core| core.read_word_8(local_address))
                .map_err(|error| format!("read addr_sign before write failed: {error}"))?;
            let replacement = if original_raw == 1 { -1 } else { 1 };
            let write_result = (|| -> Result<(), String> {
                worker.write_local_variable(0, local_reference, replacement.to_string())?;
                let written = worker
                    .probe
                    .session_mut()
                    .and_then(|session| session.core(0))
                    .and_then(|mut core| core.read_word_8(local_address))
                    .map_err(|error| format!("read addr_sign after write failed: {error}"))?;
                if written != replacement as i8 as u8 {
                    return Err(format!(
                        "addr_sign write verification failed: expected {replacement}, raw value is 0x{written:02X}"
                    ));
                }
                println!(
                    "Local write: addr_sign @ 0x{local_address:08X}: {} -> {replacement}",
                    i8::from_le_bytes([original_raw])
                );
                Ok(())
            })();
            let restore_result = worker.write_local_variable(
                0,
                local_reference,
                i8::from_le_bytes([original_raw]).to_string(),
            );
            write_result?;
            restore_result?;

            worker.debug_step(crate::model::StepKind::Over)?;
            let after_pc = worker
                .latest_pc
                .ok_or_else(|| "missing PC after step".to_owned())?;
            let after = worker
                .debug_info
                .as_ref()
                .and_then(|info| info.get_source_location(after_pc))
                .as_ref()
                .map(super::source_location_view)
                .ok_or_else(|| format!("no source location after step at 0x{after_pc:08X}"))?;
            println!(
                "Step Over: {}:{:?} @ 0x{before_pc:08X} -> {}:{:?} @ 0x{after_pc:08X}",
                before.path, before.line, after.path, after.line
            );
            if after.line != Some(139)
                || normalized_source_path(&after.path) != normalized_source_path(&before.path)
            {
                return Err(format!(
                    "Step Over from line 138 should stop on line 139, got {}:{:?}",
                    after.path, after.line
                ));
            }

            worker.debug_step(crate::model::StepKind::Over)?;
            let branch_pc = worker
                .latest_pc
                .ok_or_else(|| "missing PC after if step".to_owned())?;
            let branch = worker
                .debug_info
                .as_ref()
                .and_then(|info| info.get_source_location(branch_pc))
                .as_ref()
                .map(super::source_location_view)
                .ok_or_else(|| format!("no source location after if step at 0x{branch_pc:08X}"))?;
            println!(
                "Step Over if: {}:{:?} @ 0x{after_pc:08X} -> {}:{:?} @ 0x{branch_pc:08X}",
                after.path, after.line, branch.path, branch.line
            );
            if !matches!(branch.line, Some(140 | 141))
                || normalized_source_path(&branch.path) != normalized_source_path(&after.path)
            {
                return Err(format!(
                    "Step Over on line 139 should follow the active branch to line 140 or 141, got {}:{:?}",
                    branch.path, branch.line
                ));
            }
            Ok(())
        })();

        let _ = worker.remove_breakpoint(0xD06);
        let actual_halted = worker
            .probe
            .session_mut()
            .and_then(|session| session.core(0))
            .and_then(|mut core| core.status())
            .is_ok_and(|status| status.is_halted());
        if was_halted && !actual_halted {
            let _ = worker.debug_halt();
        } else if !was_halted && actual_halted {
            let _ = worker.debug_continue();
        }
        let _ = worker.debug_stop();
        worker.probe.disconnect();
        if let Err(error) = test_result {
            panic!("hardware source step test failed: {error}");
        }
    }

    #[test]
    #[ignore = "parses the complete host test binary and is intentionally slow"]
    fn resolves_a_real_source_breakpoint_from_the_test_elf() {
        if !cfg!(debug_assertions) {
            return;
        }
        let executable = std::env::current_exe().unwrap();
        let debug_info = probe_rs_debug::DebugInfo::from_file(&executable).unwrap();
        let sources =
            crate::dwarf::extract::load_source_index(executable.to_string_lossy().as_ref())
                .unwrap()
                .files;
        let candidates = source_path_candidates(file!(), &sources).unwrap();
        let requested_line = line!() as u64 + 1;
        std::hint::black_box(requested_line);

        let resolved = candidates.iter().any(|candidate| {
            (0..=8).any(|offset| {
                debug_info
                    .get_breakpoint_location(
                        typed_path::TypedPath::derive(candidate.as_bytes()),
                        requested_line + offset,
                        None,
                    )
                    .is_ok()
            })
        });
        assert!(
            resolved,
            "failed to resolve {candidates:?}:{requested_line}"
        );
    }
}
