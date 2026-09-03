use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText, Ui};
use serde::{Deserialize, Serialize};

use crate::model::{RegisterData, RegisterReadRequest, RegisterWriteRequest};
use crate::svd::{SvdField, SvdPeripheral, SvdRegister, SvdTree, load_svd};
use crate::ui::plugin::{PluginAction, ToastLevel};
use crate::ui::theme;

const DEFAULT_REGISTER_HZ: u32 = 5;
const MIN_REGISTER_HZ: u32 = 1;
const MAX_REGISTER_HZ: u32 = 30;
static NEXT_REGISTER_READ_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct SavedSvdRegister {
    peripheral_name: String,
    register_name: String,
    address: u64,
    enabled: bool,
    read_hz: u32,
}

struct SvdLoadTask {
    receiver: Receiver<Result<(PathBuf, SvdTree), String>>,
}

struct RegisterRuntime {
    read_id: u64,
    enabled: bool,
    read_hz: u32,
    last_request: Option<Instant>,
    last_sequence: u64,
    value: Option<Result<u64, String>>,
    write_text: String,
}

impl Default for RegisterRuntime {
    fn default() -> Self {
        Self {
            read_id: NEXT_REGISTER_READ_ID.fetch_add(1, Ordering::Relaxed),
            enabled: false,
            read_hz: DEFAULT_REGISTER_HZ,
            last_request: None,
            last_sequence: 0,
            value: None,
            write_text: String::new(),
        }
    }
}

#[derive(Default)]
pub struct SvdPanelState {
    path: Option<PathBuf>,
    tree: Option<SvdTree>,
    task: Option<SvdLoadTask>,
    error: Option<String>,
    search_input: String,
    active_search: String,
    generation: u64,
    collapse_generation: u64,
    registers: HashMap<(usize, usize), RegisterRuntime>,
    pending_settings: Vec<SavedSvdRegister>,
}

impl SvdPanelState {
    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    pub fn load_path(&mut self, path: PathBuf) {
        if self.task.is_some() {
            return;
        }
        self.error = None;
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = load_svd(&path)
                .map(|tree| (path, tree))
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
        self.task = Some(SvdLoadTask { receiver });
    }

    fn poll(&mut self) {
        let Some(task) = self.task.as_ref() else {
            return;
        };
        let result = match task.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("SVD 解析线程意外结束".to_owned()),
        };
        self.task = None;
        match result {
            Ok((path, tree)) => {
                self.registers = register_runtimes(&tree);
                let pending_settings = std::mem::take(&mut self.pending_settings);
                apply_saved_settings(&tree, &mut self.registers, &pending_settings);
                self.path = Some(path);
                self.tree = Some(tree);
                self.error = None;
                self.generation = self.generation.wrapping_add(1);
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn commit_search(&mut self) {
        self.active_search = self.search_input.trim().to_ascii_lowercase();
    }

    fn collapse_all(&mut self) {
        self.collapse_generation = self.collapse_generation.wrapping_add(1);
    }

    pub fn update(
        &mut self,
        register_data: &RegisterData,
        connected: bool,
        hardware_busy: bool,
        egui_ctx: &egui::Context,
    ) -> Vec<PluginAction> {
        self.poll();
        if self.task.is_some() {
            egui_ctx.request_repaint_after(Duration::from_millis(50));
        }
        self.apply_results(register_data);
        if !connected || hardware_busy {
            return Vec::new();
        }

        let now = Instant::now();
        let mut requests = Vec::new();
        let mut next_repaint = None;
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        for (peripheral_index, peripheral) in tree.peripherals.iter().enumerate() {
            for (register_index, register) in peripheral.registers.iter().enumerate() {
                let Some(runtime) = self.registers.get_mut(&(peripheral_index, register_index))
                else {
                    continue;
                };
                if !runtime.enabled || !register_is_readable(register) {
                    continue;
                }
                let interval = register_interval(runtime.read_hz);
                let remaining = runtime
                    .last_request
                    .and_then(|last| interval.checked_sub(now.saturating_duration_since(last)));
                if remaining.is_none() {
                    let Ok(size_bytes) = register_size_bytes(register.size_bits) else {
                        runtime.value = Some(Err("仅支持 1–64 bit 寄存器".to_owned()));
                        continue;
                    };
                    requests.push(RegisterReadRequest {
                        id: runtime.read_id,
                        address: register.address,
                        size_bytes,
                    });
                    runtime.last_request = Some(now);
                    next_repaint = min_duration(next_repaint, interval);
                } else if let Some(remaining) = remaining {
                    next_repaint = min_duration(next_repaint, remaining);
                }
            }
        }

        if let Some(delay) = next_repaint {
            egui_ctx.request_repaint_after(delay.max(Duration::from_millis(1)));
        }
        if requests.is_empty() {
            Vec::new()
        } else {
            // The follow-up repaint applies results written by the app after this update pass.
            egui_ctx.request_repaint();
            vec![PluginAction::ReadRegisters { requests }]
        }
    }

    pub fn reset_values(&mut self) {
        for runtime in self.registers.values_mut() {
            runtime.last_request = None;
            runtime.last_sequence = 0;
            runtime.value = None;
        }
    }

    pub fn saved_registers(&self) -> Vec<SavedSvdRegister> {
        let Some(tree) = &self.tree else {
            return self.pending_settings.clone();
        };
        tree.peripherals
            .iter()
            .enumerate()
            .flat_map(|(peripheral_index, peripheral)| {
                peripheral.registers.iter().enumerate().filter_map(
                    move |(register_index, register)| {
                        self.registers
                            .get(&(peripheral_index, register_index))
                            .map(|runtime| SavedSvdRegister {
                                peripheral_name: peripheral.name.clone(),
                                register_name: register.name.clone(),
                                address: register.address,
                                enabled: runtime.enabled,
                                read_hz: runtime.read_hz,
                            })
                    },
                )
            })
            .collect()
    }

    pub fn set_saved_registers(&mut self, settings: Vec<SavedSvdRegister>) {
        self.pending_settings = settings;
        if let Some(tree) = &self.tree {
            apply_saved_settings(tree, &mut self.registers, &self.pending_settings);
        }
    }

    fn apply_results(&mut self, register_data: &RegisterData) {
        for runtime in self.registers.values_mut() {
            let Some(result) = register_data.get(&runtime.read_id) else {
                continue;
            };
            if result.sequence > runtime.last_sequence {
                runtime.last_sequence = result.sequence;
                runtime.value = Some(result.value.clone());
            }
        }
    }
}

pub fn render_svd_panel(ui: &mut Ui, state: &mut SvdPanelState) -> Vec<PluginAction> {
    let mut actions = Vec::new();
    state.poll();
    if state.task.is_some() {
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }

    ui.horizontal(|ui| {
        ui.heading(RichText::new("SVD 寄存器").size(15.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(state.task.is_none(), egui::Button::new("加载 SVD"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("CMSIS-SVD", &["svd", "xml"])
                    .pick_file()
            {
                state.load_path(path);
            }
            if ui
                .add_enabled(state.tree.is_some(), egui::Button::new("全部折叠"))
                .clicked()
            {
                state.collapse_all();
            }
        });
    });

    if let Some(path) = state.path() {
        ui.label(
            RichText::new(path.display().to_string())
                .size(10.0)
                .color(theme::muted_text(ui)),
        );
    }
    let search_response = ui.add(
        egui::TextEdit::singleline(&mut state.search_input)
            .hint_text("输入顶层外设名称，按 Enter 搜索...")
            .desired_width(f32::INFINITY),
    );
    if search_response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
        state.commit_search();
    }
    if state.task.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("正在解析 SVD...");
        });
    }
    if let Some(error) = &state.error {
        ui.label(RichText::new(error).color(theme::danger_text(ui)));
    }
    ui.separator();

    let Some(tree) = &state.tree else {
        if state.task.is_none() {
            ui.vertical_centered(|ui| {
                ui.add_space(35.0);
                ui.label(
                    RichText::new("加载 SVD 文件以浏览外设寄存器").color(theme::muted_text(ui)),
                );
            });
        }
        return actions;
    };

    ui.label(
        RichText::new(format!(
            "{} · {} 个外设",
            tree.device_name,
            tree.peripherals.len()
        ))
        .size(11.0)
        .color(theme::muted_text(ui)),
    );
    let query = state.active_search.as_str();
    let view_generation = (state.generation, state.collapse_generation);
    let registers = &mut state.registers;
    egui::ScrollArea::vertical()
        .id_salt(("svd_scroll", state.generation))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (peripheral_index, peripheral) in tree.peripherals.iter().enumerate() {
                if !peripheral_matches(peripheral, query) {
                    continue;
                }
                let label = format!("{}  @ 0x{:08X}", peripheral.name, peripheral.base_address);
                let response = egui::CollapsingHeader::new(label)
                    .id_salt((view_generation, "peripheral", peripheral_index))
                    .default_open(false)
                    .show(ui, |ui| {
                        for (register_index, register) in peripheral.registers.iter().enumerate() {
                            render_register(
                                ui,
                                view_generation,
                                peripheral_index,
                                register_index,
                                register,
                                registers,
                                &mut actions,
                            );
                        }
                    });
                if let Some(description) = &peripheral.description {
                    response.header_response.on_hover_text(description);
                }
            }
        });
    actions
}

fn render_register(
    ui: &mut Ui,
    generation: (u64, u64),
    peripheral_index: usize,
    register_index: usize,
    register: &SvdRegister,
    registers: &mut HashMap<(usize, usize), RegisterRuntime>,
    actions: &mut Vec<PluginAction>,
) {
    let runtime = registers
        .entry((peripheral_index, register_index))
        .or_default();
    let metadata = [
        register.size_bits.map(|size| format!("{size}-bit")),
        register.access.clone(),
        register
            .reset_value
            .map(|value| format!("reset=0x{value:X}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let readable = register_is_readable(register);
    let writable = register_is_writable(register);
    if !readable {
        runtime.enabled = false;
    }
    let id = ui.make_persistent_id((generation, "register", peripheral_index, register_index));
    let (_, header_response, _) =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
            .show_header(ui, |ui| {
                ui.add_enabled(readable, egui::Checkbox::without_text(&mut runtime.enabled))
                    .on_hover_text(if readable {
                        "勾选后按设定频率读取"
                    } else {
                        "该寄存器不可读"
                    });
                ui.label(
                    RichText::new(format!("{}  0x{:08X}", register.name, register.address))
                        .monospace()
                        .size(11.0),
                );
                if readable {
                    ui.add(
                        egui::DragValue::new(&mut runtime.read_hz)
                            .range(MIN_REGISTER_HZ..=MAX_REGISTER_HZ)
                            .suffix(" Hz")
                            .speed(0.25),
                    )
                    .on_hover_text("独立寄存器读取频率（1–30 Hz）");
                }
                let value = register_value_text(runtime);
                let response = ui.label(RichText::new(value).monospace().size(11.0));
                if let Some(Err(error)) = &runtime.value {
                    response.on_hover_text(error);
                }
            })
            .body(|ui| {
                if !metadata.is_empty() {
                    ui.label(
                        RichText::new(&metadata)
                            .size(10.5)
                            .color(theme::muted_text(ui)),
                    );
                }
                if writable {
                    render_register_write(ui, register, runtime, actions);
                }
                if register.fields.is_empty() {
                    ui.label(
                        RichText::new("无字段定义")
                            .size(10.5)
                            .color(theme::muted_text(ui)),
                    );
                }
                for field in &register.fields {
                    render_field(ui, field);
                }
            });
    if let Some(description) = &register.description {
        header_response.response.on_hover_text(description);
    }
}

fn render_register_write(
    ui: &mut Ui,
    register: &SvdRegister,
    runtime: &mut RegisterRuntime,
    actions: &mut Vec<PluginAction>,
) {
    ui.horizontal(|ui| {
        ui.label("写入:");
        let response = ui.add(
            egui::TextEdit::singleline(&mut runtime.write_text)
                .hint_text("0x... 或十进制")
                .desired_width(130.0),
        );
        let submit = ui.button("写入").clicked()
            || (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
        if !submit {
            return;
        }
        let size_bytes = match register_size_bytes(register.size_bits) {
            Ok(size) => size,
            Err(error) => {
                actions.push(PluginAction::Toast {
                    level: ToastLevel::Error,
                    message: error,
                });
                return;
            }
        };
        match parse_register_value(&runtime.write_text, size_bytes) {
            Ok(value) => actions.push(PluginAction::WriteRegister {
                request: RegisterWriteRequest {
                    address: register.address,
                    size_bytes,
                    value,
                },
            }),
            Err(error) => actions.push(PluginAction::Toast {
                level: ToastLevel::Error,
                message: error,
            }),
        }
    });
}

fn register_runtimes(tree: &SvdTree) -> HashMap<(usize, usize), RegisterRuntime> {
    tree.peripherals
        .iter()
        .enumerate()
        .flat_map(|(peripheral_index, peripheral)| {
            peripheral
                .registers
                .iter()
                .enumerate()
                .map(move |(register_index, _)| {
                    (
                        (peripheral_index, register_index),
                        RegisterRuntime::default(),
                    )
                })
        })
        .collect()
}

fn apply_saved_settings(
    tree: &SvdTree,
    runtimes: &mut HashMap<(usize, usize), RegisterRuntime>,
    settings: &[SavedSvdRegister],
) {
    for setting in settings {
        let Some((peripheral_index, peripheral)) = tree
            .peripherals
            .iter()
            .enumerate()
            .find(|(_, peripheral)| peripheral.name == setting.peripheral_name)
        else {
            continue;
        };
        let Some((register_index, register)) =
            peripheral
                .registers
                .iter()
                .enumerate()
                .find(|(_, register)| {
                    register.name == setting.register_name && register.address == setting.address
                })
        else {
            continue;
        };
        if let Some(runtime) = runtimes.get_mut(&(peripheral_index, register_index)) {
            runtime.enabled = setting.enabled && register_is_readable(register);
            runtime.read_hz = setting.read_hz.clamp(MIN_REGISTER_HZ, MAX_REGISTER_HZ);
            runtime.last_request = None;
        }
    }
}

fn register_access(register: &SvdRegister) -> String {
    register
        .access
        .as_deref()
        .unwrap_or("read-write")
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn register_is_readable(register: &SvdRegister) -> bool {
    !matches!(
        register_access(register).as_str(),
        "writeonly" | "writeonce"
    )
}

fn register_is_writable(register: &SvdRegister) -> bool {
    register_access(register) != "readonly"
}

fn register_size_bytes(size_bits: Option<u32>) -> Result<u8, String> {
    match size_bits.unwrap_or(32) {
        1..=8 => Ok(1),
        9..=16 => Ok(2),
        17..=32 => Ok(4),
        33..=64 => Ok(8),
        bits => Err(format!("不支持 {bits} bit 寄存器")),
    }
}

fn register_interval(read_hz: u32) -> Duration {
    Duration::from_secs_f64(1.0 / f64::from(read_hz.clamp(MIN_REGISTER_HZ, MAX_REGISTER_HZ)))
}

fn min_duration(current: Option<Duration>, candidate: Duration) -> Option<Duration> {
    Some(current.map_or(candidate, |current| current.min(candidate)))
}

fn register_value_text(runtime: &RegisterRuntime) -> String {
    match &runtime.value {
        Some(Ok(value)) => format!("= 0x{value:X}"),
        Some(Err(_)) => "= 读取失败".to_owned(),
        None => "= —".to_owned(),
    }
}

fn parse_register_value(text: &str, size_bytes: u8) -> Result<u64, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("请输入寄存器值".to_owned());
    }
    let value = if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16)
    } else {
        text.parse::<u64>()
    }
    .map_err(|_| format!("无法解析寄存器值: {text}"))?;
    let max = match size_bytes {
        1 => u64::from(u8::MAX),
        2 => u64::from(u16::MAX),
        4 => u64::from(u32::MAX),
        8 => u64::MAX,
        _ => return Err(format!("不支持 {size_bytes} 字节寄存器")),
    };
    if value > max {
        return Err(format!("值 0x{value:X} 超出 {size_bytes} 字节范围"));
    }
    Ok(value)
}

fn render_field(ui: &mut Ui, field: &SvdField) {
    let high = field
        .bit_offset
        .saturating_add(field.bit_width.saturating_sub(1));
    let bits = if field.bit_width == 1 {
        format!("[{}]", field.bit_offset)
    } else {
        format!("[{high}:{}]", field.bit_offset)
    };
    let access = field.access.as_deref().unwrap_or("inherited");
    let response = ui.label(
        RichText::new(format!("  {bits:<9} {:<24} {access}", field.name))
            .monospace()
            .size(10.5),
    );
    if let Some(description) = &field.description {
        response.on_hover_text(description);
    }
}

fn peripheral_matches(peripheral: &SvdPeripheral, query: &str) -> bool {
    query.is_empty() || peripheral.name.to_ascii_lowercase().contains(query)
}

#[cfg(test)]
mod tests {
    use eframe::egui;

    use crate::model::RegisterData;
    use crate::svd::{SvdPeripheral, SvdRegister, SvdTree};
    use crate::ui::plugin::PluginAction;

    use super::{
        SvdPanelState, parse_register_value, peripheral_matches, register_is_readable,
        register_is_writable, register_runtimes, register_size_bytes,
    };

    fn register(name: &str, address: u64, access: Option<&str>) -> SvdRegister {
        SvdRegister {
            name: name.to_owned(),
            description: None,
            address,
            size_bits: Some(32),
            access: access.map(str::to_owned),
            reset_value: None,
            fields: Vec::new(),
        }
    }

    #[test]
    fn search_matches_only_top_level_peripheral_names() {
        let peripheral = SvdPeripheral {
            name: "GPIOA".to_owned(),
            description: Some("General purpose IO".to_owned()),
            base_address: 0,
            registers: vec![register("TIMER_MATCH", 0, None)],
        };

        assert!(peripheral_matches(&peripheral, "gpio"));
        assert!(!peripheral_matches(&peripheral, "timer"));
        assert!(!peripheral_matches(&peripheral, "general"));
    }

    #[test]
    fn enter_commit_and_collapse_generation_are_explicit() {
        let mut state = SvdPanelState {
            search_input: "  GpioA  ".to_owned(),
            ..Default::default()
        };
        state.commit_search();
        assert_eq!(state.active_search, "gpioa");

        state.collapse_all();
        assert_eq!(state.collapse_generation, 1);
    }

    #[test]
    fn register_access_controls_read_and_write_capabilities() {
        let read_only = register("RO", 0, Some("read-only"));
        assert!(register_is_readable(&read_only));
        assert!(!register_is_writable(&read_only));

        let write_only = register("WO", 4, Some("write-only"));
        assert!(!register_is_readable(&write_only));
        assert!(register_is_writable(&write_only));

        let inherited_default = register("RW", 8, None);
        assert!(register_is_readable(&inherited_default));
        assert!(register_is_writable(&inherited_default));
    }

    #[test]
    fn register_sizes_and_write_values_are_range_checked() {
        assert_eq!(register_size_bytes(Some(1)), Ok(1));
        assert_eq!(register_size_bytes(Some(16)), Ok(2));
        assert_eq!(register_size_bytes(Some(24)), Ok(4));
        assert_eq!(register_size_bytes(Some(64)), Ok(8));
        assert!(register_size_bytes(Some(65)).is_err());

        assert_eq!(parse_register_value("0xFE", 1), Ok(0xFE));
        assert_eq!(parse_register_value("254", 1), Ok(254));
        assert!(parse_register_value("0x100", 1).is_err());
        assert!(parse_register_value("not-a-value", 4).is_err());
    }

    #[test]
    fn checked_registers_are_batched_and_throttled_outside_variable_slots() {
        let tree = SvdTree {
            device_name: "TEST".to_owned(),
            peripherals: vec![SvdPeripheral {
                name: "GPIO".to_owned(),
                description: None,
                base_address: 0x4000_0000,
                registers: vec![
                    register("A", 0x4000_0000, Some("read-write")),
                    register("B", 0x4000_0004, Some("read-only")),
                ],
            }],
        };
        let mut state = SvdPanelState {
            registers: register_runtimes(&tree),
            tree: Some(tree),
            ..Default::default()
        };
        assert!(state.registers.values().all(|runtime| !runtime.enabled));
        for runtime in state.registers.values_mut() {
            runtime.enabled = true;
        }

        let actions = state.update(
            &RegisterData::default(),
            true,
            false,
            &egui::Context::default(),
        );
        let [PluginAction::ReadRegisters { requests }] = actions.as_slice() else {
            panic!("expected one batched register read action");
        };
        assert_eq!(requests.len(), 2);

        let immediate = state.update(
            &RegisterData::default(),
            true,
            false,
            &egui::Context::default(),
        );
        assert!(immediate.is_empty());
    }
}
