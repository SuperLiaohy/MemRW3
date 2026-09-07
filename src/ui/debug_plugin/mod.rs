use std::collections::{BTreeMap, HashMap, HashSet};

use eframe::egui::{self, RichText, Ui};
use serde::{Deserialize, Serialize};

use crate::model::{
    BreakpointSpec, DebugCommand, DebugSnapshot, DebugStartMode, LogicalBreakpoint,
    SourceLocationView, StepExecutionMethod, StepKind, TargetState, VariablePool, VariableView,
};
use crate::ui::plugin::{
    MemRWPlugin, PluginAction, PluginRenderContext, PluginUpdateContext, VariableCandidate,
};

mod workspace;

use workspace::{WorkspaceLayoutState, show_pane};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodeView {
    Assembly,
    Source,
    Split,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavigatorTab {
    Project,
    Breakpoints,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InspectorTab {
    CallStack,
    StackMemory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceNavigation {
    path: String,
    line: Option<u64>,
}

pub struct DebugPluginState {
    snapshot: DebugSnapshot,
    connected: bool,
    acquisition_requested: bool,
    pending: Vec<DebugCommand>,
    next_breakpoint_id: u64,
    code_view: CodeView,
    expanded_variables: HashSet<i64>,
    variable_edits: HashMap<i64, String>,
    queued_after_load: Vec<LogicalBreakpoint>,
    source_cache: HashMap<String, Result<std::sync::Arc<[String]>, String>>,
    executable_line_index: HashMap<String, BTreeMap<u64, u64>>,
    command_pending: bool,
    project_tree: SourceTreeNode,
    project_generation: u64,
    debug_source_root: String,
    source_root_override: Option<String>,
    selected_source_path: Option<String>,
    source_tabs: Vec<String>,
    navigation_history: Vec<SourceNavigation>,
    navigation_index: Option<usize>,
    source_scroll_target: Option<usize>,
    navigator_tab: NavigatorTab,
    inspector_tab: InspectorTab,
    source_cursor: Option<(String, u64)>,
    assembly_cursor: Option<u64>,
    assembly_scroll_target: Option<u64>,
    pending_assembly_source: Option<(String, u64)>,
    source_filter: String,
    expanded_source_assembly: HashSet<(String, u64)>,
    inline_assembly_cache: HashMap<(String, u64), Vec<crate::model::InstructionView>>,
    workspace_layout: WorkspaceLayoutState,
    start_mode: DebugStartMode,
}

impl Default for DebugPluginState {
    fn default() -> Self {
        Self {
            snapshot: DebugSnapshot::default(),
            connected: false,
            acquisition_requested: false,
            pending: Vec::new(),
            next_breakpoint_id: 1,
            code_view: CodeView::Assembly,
            expanded_variables: HashSet::new(),
            variable_edits: HashMap::new(),
            queued_after_load: Vec::new(),
            source_cache: HashMap::new(),
            executable_line_index: HashMap::new(),
            command_pending: false,
            project_tree: SourceTreeNode::root(),
            project_generation: 0,
            debug_source_root: String::new(),
            source_root_override: None,
            selected_source_path: None,
            source_tabs: Vec::new(),
            navigation_history: Vec::new(),
            navigation_index: None,
            source_scroll_target: None,
            navigator_tab: NavigatorTab::Project,
            inspector_tab: InspectorTab::CallStack,
            source_cursor: None,
            assembly_cursor: None,
            assembly_scroll_target: None,
            pending_assembly_source: None,
            source_filter: String::new(),
            expanded_source_assembly: HashSet::new(),
            inline_assembly_cache: HashMap::new(),
            workspace_layout: WorkspaceLayoutState::default(),
            start_mode: DebugStartMode::Attach,
        }
    }
}

#[derive(Clone, Default)]
struct SourceTreeNode {
    name: String,
    debug_path: Option<String>,
    children: BTreeMap<String, SourceTreeNode>,
}

impl SourceTreeNode {
    fn root() -> Self {
        Self {
            name: "Sources".to_owned(),
            ..Default::default()
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct SavedDebugConfig {
    breakpoints: Vec<LogicalBreakpoint>,
    #[serde(default)]
    source_root_override: Option<String>,
    #[serde(default)]
    workspace_layout: WorkspaceLayoutState,
    #[serde(default)]
    start_mode: DebugStartMode,
}

impl MemRWPlugin for DebugPluginState {
    fn id(&self) -> &'static str {
        "debug"
    }

    fn title(&self) -> &'static str {
        "Debug 调试器"
    }

    fn viewport_size(&self) -> egui::Vec2 {
        egui::vec2(1160.0, 720.0)
    }

    fn min_viewport_size(&self) -> egui::Vec2 {
        egui::vec2(800.0, 500.0)
    }

    fn update(&mut self, ctx: PluginUpdateContext<'_>) -> Vec<PluginAction> {
        let snapshot_changed = self.snapshot.revision != ctx.debug_snapshot.revision
            || self.snapshot.program_generation != ctx.debug_snapshot.program_generation;
        let old_stop_id = self.snapshot.stop_id;
        let old_selected_frame = self.snapshot.selected_frame;
        if snapshot_changed {
            self.command_pending = false;
            self.variable_edits.clear();
            self.snapshot = ctx.debug_snapshot.clone();
            self.capture_inline_assembly();
            self.resolve_pending_assembly_scroll();
        }
        if old_stop_id != self.snapshot.stop_id {
            self.expanded_variables.clear();
            if let Some(pc) = self.snapshot.pc {
                self.assembly_cursor = Some(pc);
                self.assembly_scroll_target = Some(pc);
            }
        }
        self.connected = ctx.connected;
        self.acquisition_requested = ctx.acquisition_requested;
        if self.snapshot.active
            && !self.acquisition_requested
            && !self.command_pending
            && !self
                .pending
                .iter()
                .any(|command| matches!(command, DebugCommand::Stop))
        {
            self.pending.push(DebugCommand::Stop);
        }
        if self.project_generation != self.snapshot.program_generation {
            let (tree, common_root) = build_source_tree(&self.snapshot.source_files);
            self.executable_line_index.clear();
            for line in self.snapshot.executable_lines.iter() {
                self.executable_line_index
                    .entry(line.path.clone())
                    .or_default()
                    .entry(line.line)
                    .or_insert(line.address);
            }
            self.project_tree = tree;
            self.debug_source_root = common_root;
            self.project_generation = self.snapshot.program_generation;
            self.source_cache.clear();
            self.source_tabs.clear();
            self.selected_source_path = None;
            self.navigation_history.clear();
            self.navigation_index = None;
            self.expanded_source_assembly.clear();
            self.inline_assembly_cache.clear();
        }
        if old_stop_id != self.snapshot.stop_id
            || old_selected_frame != self.snapshot.selected_frame
        {
            if let Some(source) = self
                .snapshot
                .frames
                .get(self.snapshot.selected_frame)
                .and_then(|frame| frame.source.clone())
            {
                let current_view = self.code_view;
                self.navigate_to_source(source.path.clone(), source.line, false);
                self.code_view = current_view;
            }
        }
        if !self.queued_after_load.is_empty() {
            self.pending.push(DebugCommand::ReplaceBreakpoints(
                self.queued_after_load.drain(..).collect(),
            ));
        }
        if !self.pending.is_empty() {
            self.command_pending = true;
        }
        self.pending.drain(..).map(PluginAction::Debug).collect()
    }

    fn reset_data(&mut self) {
        self.expanded_variables.clear();
    }

    fn on_enabled_changed(&mut self, enabled: bool) -> Vec<PluginAction> {
        if enabled {
            Vec::new()
        } else {
            self.command_pending = true;
            vec![PluginAction::Debug(DebugCommand::Stop)]
        }
    }

    fn render(&mut self, ui: &mut Ui, ctx: PluginRenderContext<'_>) -> Vec<PluginAction> {
        let mut actions = Vec::new();
        let original_widgets = ui.visuals().widgets.clone();
        let original_scroll = ui.spacing().scroll;
        stabilize_debug_style(ui.style_mut());
        handle_debug_shortcuts(ui, self);
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| render_toolbar(ui, self));
        let program_bar_width = ui.available_width();
        ui.allocate_ui_with_layout(
            egui::vec2(program_bar_width, 26.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                if ui.button("加载调试 ELF").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("ELF/AXF", &["elf", "axf", "out"])
                        .pick_file()
                {
                    actions.push(PluginAction::LoadProgram {
                        path: path.display().to_string(),
                    });
                }
                if ui.button("静态变量树").clicked() {
                    actions.push(PluginAction::OpenVariableTree {
                        plugin_id: self.id().to_owned(),
                        viewport_id: ctx.viewport_id,
                    });
                }
                ui.separator();
                if debug_icon_button(
                    ui,
                    DebugIcon::Split,
                    self.selected_source_path.is_some(),
                    self.code_view == CodeView::Split,
                    "源码与汇编并排显示",
                )
                .clicked()
                {
                    if self.code_view == CodeView::Split {
                        self.code_view = CodeView::Source;
                    } else {
                        self.code_view = CodeView::Split;
                        self.jump_selected_line_to_assembly();
                    }
                }
                if debug_icon_button(
                    ui,
                    DebugIcon::Assembly,
                    self.source_cursor.is_some(),
                    self.code_view == CodeView::Assembly,
                    "将选中源码行跳转到汇编视图",
                )
                .clicked()
                {
                    self.jump_selected_line_to_assembly();
                }
                if debug_icon_button(
                    ui,
                    DebugIcon::Source,
                    self.selected_assembly_source().is_some(),
                    false,
                    "将选中汇编指令跳转到对应源码",
                )
                .clicked()
                {
                    self.jump_selected_instruction_to_source();
                }
                if debug_icon_button(
                    ui,
                    DebugIcon::Forward,
                    self.can_navigate_forward(),
                    false,
                    "前进",
                )
                .clicked()
                {
                    self.navigate_forward();
                }
                if debug_icon_button(ui, DebugIcon::Back, self.can_navigate_back(), false, "后退")
                    .clicked()
                {
                    self.navigate_back();
                }
                let program = if self.snapshot.program_generation == 0 {
                    "尚未加载 ELF 调试信息".to_owned()
                } else {
                    let name = self
                        .snapshot
                        .program_path
                        .as_deref()
                        .map(short_path)
                        .unwrap_or("ELF");
                    format!("{name} · {} 个源码文件", self.snapshot.source_files.len())
                };
                let color = if self.snapshot.program_generation == 0 {
                    ui.visuals().warn_fg_color
                } else {
                    ui.visuals().text_color()
                };
                ui.add_sized(
                    [ui.available_width(), 22.0],
                    egui::Label::new(RichText::new(program).color(color)).truncate(),
                );
            },
        );
        ui.separator();

        let workspace_size = ui.available_size().max(egui::vec2(1.0, 1.0));
        let (workspace_rect, _) = ui.allocate_exact_size(workspace_size, egui::Sense::hover());
        let rects = self.workspace_layout.layout(workspace_rect);
        show_pane(
            ui,
            ("debug_navigator", ctx.viewport_id),
            rects.navigator,
            |ui| render_navigator(ui, self),
        );
        show_pane(ui, ("debug_editor", ctx.viewport_id), rects.editor, |ui| {
            render_code(ui, self)
        });
        show_pane(
            ui,
            ("debug_inspector", ctx.viewport_id),
            rects.inspector,
            |ui| render_inspector(ui, self),
        );
        show_pane(
            ui,
            ("debug_bottom_locals", ctx.viewport_id),
            rects.bottom_left,
            |ui| render_locals(ui, self),
        );
        show_pane(
            ui,
            ("debug_bottom_registers", ctx.viewport_id),
            rects.bottom_right,
            |ui| render_registers_panel(ui, self),
        );
        self.workspace_layout
            .show_splitters(ui, ctx.viewport_id, rects);

        if !self.pending.is_empty() {
            self.command_pending = true;
        }
        actions.extend(self.pending.drain(..).map(PluginAction::Debug));
        ui.style_mut().visuals.widgets = original_widgets;
        ui.style_mut().spacing.scroll = original_scroll;
        actions
    }

    fn add_variable_ui(
        &mut self,
        _ui: &mut Ui,
        _node_id: usize,
        _default_name: &str,
        _candidate: &mut dyn FnMut() -> Result<VariableCandidate, String>,
        _pool: &mut VariablePool,
    ) -> Result<bool, String> {
        Ok(false)
    }

    fn save_config(&self, _pool: &VariablePool) -> serde_json::Value {
        let breakpoints = self
            .snapshot
            .breakpoints
            .iter()
            .map(|breakpoint| LogicalBreakpoint {
                id: breakpoint.id,
                spec: breakpoint.spec.clone(),
                enabled: breakpoint.enabled,
            })
            .collect();
        serde_json::to_value(SavedDebugConfig {
            breakpoints,
            source_root_override: self.source_root_override.clone(),
            workspace_layout: self.workspace_layout,
            start_mode: self.start_mode,
        })
        .unwrap_or_default()
    }

    fn load_config(
        &mut self,
        payload: &serde_json::Value,
        _pool: &mut VariablePool,
    ) -> Result<(), String> {
        if payload.is_null() {
            return Ok(());
        }
        let config: SavedDebugConfig =
            serde_json::from_value(payload.clone()).map_err(|error| error.to_string())?;
        self.next_breakpoint_id = config
            .breakpoints
            .iter()
            .map(|breakpoint| breakpoint.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.queued_after_load = config.breakpoints;
        self.source_root_override = config.source_root_override;
        self.workspace_layout = config.workspace_layout;
        self.start_mode = config.start_mode;
        Ok(())
    }
}

fn stabilize_debug_style(style: &mut egui::Style) {
    let widgets = &mut style.visuals.widgets;
    widgets.noninteractive.expansion = 0.0;
    widgets.inactive.expansion = 0.0;
    widgets.hovered.expansion = 0.0;
    widgets.active.expansion = 0.0;
    widgets.open.expansion = 0.0;
    widgets.hovered.bg_stroke.width = widgets.inactive.bg_stroke.width;
    widgets.active.bg_stroke.width = widgets.inactive.bg_stroke.width;
    widgets.open.bg_stroke.width = widgets.inactive.bg_stroke.width;
    style.spacing.scroll = egui::style::ScrollStyle::solid();
}

fn render_toolbar(ui: &mut Ui, state: &mut DebugPluginState) {
    let active = state.snapshot.active;
    let halted = active && state.snapshot.target_state.is_halted();
    let (status, color, halt_detail) = if !active {
        ("未启动", ui.visuals().weak_text_color(), None)
    } else {
        match &state.snapshot.target_state {
            TargetState::Disconnected => ("未连接", ui.visuals().weak_text_color(), None),
            TargetState::Unknown => ("未知", ui.visuals().warn_fg_color, None),
            TargetState::Running => ("运行中", egui::Color32::from_rgb(35, 160, 95), None),
            TargetState::Sleeping => ("休眠", ui.visuals().warn_fg_color, None),
            TargetState::Halted { reason } => ("已暂停", ui.visuals().warn_fg_color, Some(reason)),
            TargetState::LockedUp => ("Locked up", ui.visuals().error_fg_color, None),
        }
    };
    let row_width = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(row_width, 22.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(RichText::new("●").color(color));
            ui.add_sized(
                [76.0, 20.0],
                egui::Label::new(RichText::new(status).strong()).truncate(),
            );
            if state.command_pending {
                ui.add_sized([18.0, 18.0], egui::Spinner::new().size(14.0));
            } else {
                ui.allocate_space(egui::vec2(18.0, 18.0));
            }
            let pc = state
                .snapshot
                .pc
                .map(|pc| format!("PC 0x{pc:08X}"))
                .unwrap_or_default();
            ui.add_sized(
                [142.0, 20.0],
                egui::Label::new(RichText::new(pc).monospace()).truncate(),
            );
            let step_method = step_method_label(state.snapshot.last_step_method);
            ui.add_sized(
                [156.0, 20.0],
                egui::Label::new(RichText::new(step_method).monospace().size(11.0)).truncate(),
            )
            .on_hover_text("显示最近一次实际采用的步进方式；硬件断点不可用时自动回退 SingleStep");

            let (diagnostic, diagnostic_color) = if let Some(error) = &state.snapshot.last_error {
                (format!("错误: {error}"), ui.visuals().error_fg_color)
            } else if !state.snapshot.warnings.is_empty() {
                (
                    format!("警告: {}", state.snapshot.warnings.join("；")),
                    ui.visuals().warn_fg_color,
                )
            } else if let Some(reason) = halt_detail {
                (
                    format!("停止原因: {reason}"),
                    ui.visuals().weak_text_color(),
                )
            } else if !active {
                (
                    if state.connected && !state.acquisition_requested {
                        "请先点击全局“开始”，再选择 Attach/Reset 启动调试".to_owned()
                    } else {
                        "选择 Attach 或 Reset，然后点击启动".to_owned()
                    },
                    ui.visuals().weak_text_color(),
                )
            } else {
                (String::new(), ui.visuals().weak_text_color())
            };
            let diagnostic_response = ui.add_sized(
                [ui.available_width(), 20.0],
                egui::Label::new(RichText::new(&diagnostic).color(diagnostic_color)).truncate(),
            );
            if !diagnostic.is_empty() {
                diagnostic_response.on_hover_text(diagnostic);
            }
        },
    );

    ui.allocate_ui_with_layout(
        egui::vec2(row_width, 28.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            if debug_icon_button(
                ui,
                DebugIcon::Attach,
                !active && !state.command_pending,
                state.start_mode == DebugStartMode::Attach,
                "启动模式：Attach（保持目标当前状态）",
            )
            .clicked()
            {
                state.start_mode = DebugStartMode::Attach;
            }
            if debug_icon_button(
                ui,
                DebugIcon::Reset,
                !active && !state.command_pending,
                state.start_mode == DebugStartMode::Reset,
                "启动模式：Reset（复位并暂停目标）",
            )
            .clicked()
            {
                state.start_mode = DebugStartMode::Reset;
            }
            ui.separator();
            if debug_icon_button(
                ui,
                DebugIcon::Start,
                state.connected && state.acquisition_requested && !active && !state.command_pending,
                false,
                "启动 DebugPlugin",
            )
            .clicked()
            {
                state.pending.push(DebugCommand::Start(state.start_mode));
            }
            if debug_icon_button(
                ui,
                DebugIcon::Stop,
                active && !state.command_pending,
                false,
                "停止调试并卸载硬件断点",
            )
            .clicked()
            {
                state.pending.push(DebugCommand::Stop);
            }
            if debug_icon_button(
                ui,
                DebugIcon::Pause,
                active && state.connected && !halted && !state.command_pending,
                false,
                "暂停目标 (F6)",
            )
            .clicked()
            {
                state.pending.push(DebugCommand::Halt);
            }
            if debug_icon_button(
                ui,
                DebugIcon::Continue,
                halted && !state.command_pending,
                false,
                "继续运行 (F5)",
            )
            .clicked()
            {
                state.pending.push(DebugCommand::Continue);
            }
            let cursor = current_cursor_spec(state);
            if debug_icon_button(
                ui,
                DebugIcon::RunToCursor,
                halted && !state.command_pending && cursor.is_some(),
                false,
                "运行到光标",
            )
            .clicked()
                && let Some(cursor) = cursor
            {
                state.pending.push(DebugCommand::RunTo(cursor));
            }
            ui.separator();
            let top_frame_selected = state.snapshot.selected_frame == 0;
            for (icon, kind, tooltip) in [
                (
                    DebugIcon::InstructionStep,
                    StepKind::Instruction,
                    "指令步进",
                ),
                (DebugIcon::StepInto, StepKind::Into, "源码步入 (F11)"),
                (
                    DebugIcon::StepOver,
                    StepKind::Over,
                    "源码步过 (F10，函数调用自动加速)",
                ),
                (
                    DebugIcon::StepOut,
                    StepKind::Out,
                    "源码步出 (Shift+F11，优先运行到调用者)",
                ),
            ] {
                let response = debug_icon_button(
                    ui,
                    icon,
                    active && halted && !state.command_pending && top_frame_selected,
                    false,
                    tooltip,
                );
                let response = if top_frame_selected {
                    response
                } else {
                    response.on_disabled_hover_text("调试命令只作用于当前执行帧，请先选择 #0")
                };
                if response.clicked() {
                    state.pending.push(DebugCommand::Step(kind));
                }
            }
            if debug_icon_button(
                ui,
                DebugIcon::Interrupt,
                active && state.command_pending,
                false,
                "手动打断当前步进",
            )
            .clicked()
            {
                state.pending.push(DebugCommand::Interrupt);
            }
            if debug_icon_button(
                ui,
                DebugIcon::Refresh,
                halted && !state.command_pending,
                false,
                "重新读取暂停快照",
            )
            .clicked()
            {
                state.pending.push(DebugCommand::Refresh);
            }
        },
    );
}

fn step_method_label(method: Option<StepExecutionMethod>) -> &'static str {
    match method {
        Some(StepExecutionMethod::HardwareBreakpoint) => "步进: 硬件断点加速",
        Some(StepExecutionMethod::SingleStep) => "步进: SingleStep",
        None => "步进: —",
    }
}

#[derive(Debug, Clone, Copy)]
enum DebugIcon {
    Attach,
    Reset,
    Start,
    Stop,
    Pause,
    Continue,
    RunToCursor,
    InstructionStep,
    StepInto,
    StepOver,
    StepOut,
    Interrupt,
    Refresh,
    Back,
    Forward,
    Assembly,
    Source,
    Split,
}

fn debug_icon_button(
    ui: &mut Ui,
    icon: DebugIcon,
    enabled: bool,
    selected: bool,
    tooltip: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(28.0, 24.0),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let fill = if !enabled {
        ui.visuals()
            .widgets
            .inactive
            .weak_bg_fill
            .gamma_multiply(0.55)
    } else if selected {
        ui.visuals().selection.bg_fill
    } else if enabled && response.hovered() {
        ui.visuals().selection.bg_fill.gamma_multiply(0.72)
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 3.0, fill);
    let border = if !enabled {
        egui::Stroke::new(
            1.0,
            ui.visuals()
                .widgets
                .inactive
                .bg_stroke
                .color
                .gamma_multiply(0.45),
        )
    } else if response.hovered() || selected {
        egui::Stroke::new(1.5, ui.visuals().selection.stroke.color)
    } else {
        ui.visuals().widgets.inactive.bg_stroke
    };
    ui.painter()
        .rect_stroke(rect, 3.0, border, egui::StrokeKind::Inside);
    let color = if !enabled {
        ui.visuals().weak_text_color().gamma_multiply(0.45)
    } else if response.hovered() || selected {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().text_color()
    };
    paint_debug_icon(
        ui.painter(),
        rect.shrink(if response.hovered() && enabled {
            4.0
        } else {
            5.0
        }),
        icon,
        color,
    );
    response.on_hover_text(tooltip)
}

fn paint_debug_icon(
    painter: &egui::Painter,
    rect: egui::Rect,
    icon: DebugIcon,
    color: egui::Color32,
) {
    let stroke = egui::Stroke::new(1.5, color);
    let center = rect.center();
    let left = rect.left();
    let right = rect.right();
    let top = rect.top();
    let bottom = rect.bottom();
    let triangle = |x: f32, direction: f32| {
        let tip = egui::pos2(x + 5.0 * direction, center.y);
        let base = egui::pos2(x - 3.0 * direction, center.y);
        painter.add(egui::Shape::convex_polygon(
            vec![
                tip,
                base + egui::vec2(0.0, -5.0),
                base + egui::vec2(0.0, 5.0),
            ],
            color,
            egui::Stroke::NONE,
        ));
    };
    match icon {
        DebugIcon::Attach => {
            painter.circle_stroke(egui::pos2(center.x - 4.0, center.y), 3.0, stroke);
            painter.circle_stroke(egui::pos2(center.x + 4.0, center.y), 3.0, stroke);
            painter.line_segment(
                [
                    egui::pos2(center.x - 1.0, center.y),
                    egui::pos2(center.x + 1.0, center.y),
                ],
                stroke,
            );
        }
        DebugIcon::Reset | DebugIcon::Refresh => {
            painter.circle_stroke(center, 5.0, stroke);
            triangle(center.x + 3.0, 1.0);
        }
        DebugIcon::Start | DebugIcon::Continue => triangle(center.x - 2.0, 1.0),
        DebugIcon::Stop => {
            painter.rect_filled(
                egui::Rect::from_center_size(center, egui::vec2(9.0, 9.0)),
                1.0,
                color,
            );
        }
        DebugIcon::Pause => {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(center.x - 5.0, top),
                    egui::pos2(center.x - 1.5, bottom),
                ),
                0.0,
                color,
            );
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(center.x + 1.5, top),
                    egui::pos2(center.x + 5.0, bottom),
                ),
                0.0,
                color,
            );
        }
        DebugIcon::RunToCursor => {
            triangle(left + 3.0, 1.0);
            painter.line_segment(
                [
                    egui::pos2(right - 2.0, top),
                    egui::pos2(right - 2.0, bottom),
                ],
                stroke,
            );
        }
        DebugIcon::InstructionStep => {
            triangle(center.x - 3.0, 1.0);
            painter.line_segment(
                [
                    egui::pos2(right - 1.0, top),
                    egui::pos2(right - 1.0, bottom),
                ],
                stroke,
            );
        }
        DebugIcon::StepInto | DebugIcon::StepOut => {
            let direction = if matches!(icon, DebugIcon::StepInto) {
                1.0
            } else {
                -1.0
            };
            painter.line_segment(
                [egui::pos2(center.x, top), egui::pos2(center.x, bottom)],
                stroke,
            );
            let y = if direction > 0.0 { bottom } else { top };
            painter.line_segment(
                [
                    egui::pos2(center.x, y),
                    egui::pos2(center.x - 4.0, y - 4.0 * direction),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x, y),
                    egui::pos2(center.x + 4.0, y - 4.0 * direction),
                ],
                stroke,
            );
        }
        DebugIcon::StepOver => {
            painter.line_segment(
                [egui::pos2(left, bottom), egui::pos2(left, center.y)],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(left, center.y),
                    egui::pos2(right - 3.0, center.y),
                ],
                stroke,
            );
            triangle(right - 7.0, 1.0);
        }
        DebugIcon::Interrupt => {
            painter.circle_stroke(center, 6.0, stroke);
            painter.line_segment(
                [
                    egui::pos2(center.x - 3.0, center.y - 3.0),
                    egui::pos2(center.x + 3.0, center.y + 3.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x + 3.0, center.y - 3.0),
                    egui::pos2(center.x - 3.0, center.y + 3.0),
                ],
                stroke,
            );
        }
        DebugIcon::Back => triangle(center.x + 2.0, -1.0),
        DebugIcon::Forward => triangle(center.x - 2.0, 1.0),
        DebugIcon::Assembly => {
            for offset in [-4.0, 0.0, 4.0] {
                painter.line_segment(
                    [
                        egui::pos2(left, center.y + offset),
                        egui::pos2(right, center.y + offset),
                    ],
                    stroke,
                );
            }
        }
        DebugIcon::Source => {
            painter.rect_stroke(rect, 1.0, stroke, egui::StrokeKind::Inside);
            for offset in [-3.5, 0.0, 3.5] {
                painter.line_segment(
                    [
                        egui::pos2(left + 3.0, center.y + offset),
                        egui::pos2(right - 3.0, center.y + offset),
                    ],
                    stroke,
                );
            }
        }
        DebugIcon::Split => {
            painter.rect_stroke(rect, 1.0, stroke, egui::StrokeKind::Inside);
            painter.line_segment(
                [egui::pos2(center.x, top), egui::pos2(center.x, bottom)],
                stroke,
            );
        }
    }
}

fn stable_selectable_value<T: Copy + PartialEq>(
    ui: &mut Ui,
    current: &mut T,
    value: T,
    text: &str,
    width: f32,
) -> egui::Response {
    let selected = *current == value;
    let response =
        stable_selectable_label_sized(ui, selected, RichText::new(text), egui::vec2(width, 24.0));
    if response.clicked() && !selected {
        *current = value;
    }
    response
}

fn stable_selectable_label(ui: &mut Ui, selected: bool, text: RichText) -> egui::Response {
    let fill = if selected {
        ui.visuals().selection.bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.add(
        egui::Button::new(text)
            .selected(selected)
            .fill(fill)
            .stroke(egui::Stroke::NONE)
            .frame(true)
            .truncate(),
    )
}

fn stable_selectable_job(
    ui: &mut Ui,
    selected: bool,
    job: egui::text::LayoutJob,
) -> egui::Response {
    let fill = if selected {
        ui.visuals().selection.bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.add(
        egui::Button::new(job)
            .selected(selected)
            .fill(fill)
            .stroke(egui::Stroke::NONE)
            .frame(true)
            .truncate(),
    )
}

fn compact_expander(
    ui: &mut Ui,
    expanded: bool,
    expand_tip: &str,
    collapse_tip: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
    let fill = if response.hovered() {
        ui.visuals().selection.bg_fill.gamma_multiply(0.65)
    } else {
        ui.visuals()
            .widgets
            .inactive
            .weak_bg_fill
            .gamma_multiply(0.45)
    };
    let stroke = if response.hovered() {
        egui::Stroke::new(1.25, ui.visuals().selection.stroke.color)
    } else {
        egui::Stroke::new(1.0, ui.visuals().widgets.inactive.bg_stroke.color)
    };
    ui.painter().rect_filled(rect, 3.0, fill);
    ui.painter()
        .rect_stroke(rect, 3.0, stroke, egui::StrokeKind::Inside);
    let center = rect.center();
    let mark = egui::Stroke::new(1.4, ui.visuals().text_color());
    ui.painter().line_segment(
        [
            egui::pos2(center.x - 3.5, center.y),
            egui::pos2(center.x + 3.5, center.y),
        ],
        mark,
    );
    if !expanded {
        ui.painter().line_segment(
            [
                egui::pos2(center.x, center.y - 3.5),
                egui::pos2(center.x, center.y + 3.5),
            ],
            mark,
        );
    }
    response.on_hover_text(if expanded { collapse_tip } else { expand_tip })
}

fn stable_selectable_label_sized(
    ui: &mut Ui,
    selected: bool,
    text: RichText,
    size: egui::Vec2,
) -> egui::Response {
    let fill = if selected {
        ui.visuals().selection.bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.add_sized(
        size,
        egui::Button::new(text)
            .selected(selected)
            .fill(fill)
            .stroke(egui::Stroke::NONE)
            .frame(true)
            .truncate(),
    )
}

fn stable_left_selectable_label_sized(
    ui: &mut Ui,
    selected: bool,
    text: &str,
    size: egui::Vec2,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if selected {
        ui.painter()
            .rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
    }
    let text_rect = rect.shrink2(egui::vec2(4.0, 0.0));
    ui.painter().with_clip_rect(text_rect).text(
        text_rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        egui::TextStyle::Button.resolve(ui.style()),
        ui.visuals().text_color(),
    );
    response
}

fn render_navigator(ui: &mut Ui, state: &mut DebugPluginState) {
    ui.horizontal(|ui| {
        stable_selectable_value(
            ui,
            &mut state.navigator_tab,
            NavigatorTab::Project,
            "工程",
            72.0,
        );
        stable_selectable_value(
            ui,
            &mut state.navigator_tab,
            NavigatorTab::Breakpoints,
            &format!("断点 ({})", state.snapshot.breakpoints.len()),
            96.0,
        );
    });
    ui.separator();
    match state.navigator_tab {
        NavigatorTab::Project => render_project(ui, state),
        NavigatorTab::Breakpoints => render_breakpoints(ui, state),
    }
}

fn render_breakpoints(ui: &mut Ui, state: &mut DebugPluginState) {
    ui.heading("断点");
    let capacity_label = if let Some(capacity) = state.snapshot.breakpoint_capacity {
        let used = state
            .snapshot
            .breakpoints
            .iter()
            .filter(|breakpoint| breakpoint.verified)
            .count();
        format!("硬件断点 {used}/{capacity}")
    } else {
        "硬件断点 --/--".to_owned()
    };
    ui.add_sized(
        [ui.available_width(), 18.0],
        egui::Label::new(RichText::new(capacity_label).small()).truncate(),
    );
    egui::ScrollArea::both()
        .id_salt("debug_breakpoints")
        .max_height(ui.available_height())
        .show(ui, |ui| {
            for breakpoint in state.snapshot.breakpoints.clone() {
                let mut navigate = false;
                ui.horizontal(|ui| {
                    let mut enabled = breakpoint.enabled;
                    if ui.checkbox(&mut enabled, "").changed() {
                        state.pending.push(DebugCommand::SetBreakpointEnabled {
                            id: breakpoint.id,
                            enabled,
                        });
                    }
                    ui.colored_label(
                        if breakpoint.verified {
                            egui::Color32::from_rgb(220, 65, 65)
                        } else {
                            ui.visuals().weak_text_color()
                        },
                        if breakpoint.verified { "●" } else { "○" },
                    );
                    navigate = stable_selectable_label(
                        ui,
                        false,
                        RichText::new(breakpoint_label(&breakpoint.spec)),
                    )
                    .on_hover_text("在源码或汇编视图中定位")
                    .clicked();
                    if ui.small_button("×").clicked() {
                        state
                            .pending
                            .push(DebugCommand::RemoveBreakpoint(breakpoint.id));
                    }
                });
                if navigate {
                    state.focus_breakpoint(&breakpoint);
                }
                if let Some(message) = breakpoint.message {
                    let color = if breakpoint.verified {
                        ui.visuals().weak_text_color()
                    } else {
                        ui.visuals().warn_fg_color
                    };
                    ui.small(RichText::new(message).color(color));
                }
                if let Some(address) = breakpoint.address {
                    let resolved = breakpoint
                        .resolved_source
                        .as_ref()
                        .map(|source| {
                            format!(
                                "{}:{}",
                                short_path(&source.path),
                                source.line.unwrap_or_default()
                            )
                        })
                        .unwrap_or_else(|| "指令地址".to_owned());
                    ui.small(format!("→ {resolved} @ 0x{address:08X}"));
                }
            }
        });
}

fn render_call_stack(ui: &mut Ui, state: &mut DebugPluginState) {
    ui.horizontal(|ui| {
        ui.strong("调用栈");
        ui.small(format!("{} frames", state.snapshot.frames.len()));
    });
    egui::ScrollArea::both()
        .id_salt("debug_call_stack")
        .show(ui, |ui| {
            for frame in state.snapshot.frames.clone() {
                let label = if let Some(source) = &frame.source {
                    format!(
                        "#{} {}\n{}:{}",
                        frame.index,
                        frame.function,
                        short_path(&source.path),
                        source.line.unwrap_or(0)
                    )
                } else {
                    format!("#{} {} @ 0x{:08X}", frame.index, frame.function, frame.pc)
                };
                if stable_selectable_label_sized(
                    ui,
                    state.snapshot.selected_frame == frame.index,
                    RichText::new(label),
                    egui::vec2(ui.available_width(), 38.0),
                )
                .clicked()
                {
                    state.pending.push(DebugCommand::SelectFrame(frame.index));
                }
            }
        });
}

fn render_project(ui: &mut Ui, state: &mut DebugPluginState) {
    ui.horizontal(|ui| {
        if ui.button("映射源码根目录").clicked()
            && let Some(path) = rfd::FileDialog::new().pick_folder()
        {
            state.source_root_override = Some(path.display().to_string());
            state.source_cache.clear();
        }
    });
    if let Some(program) = &state.snapshot.program_path {
        ui.small(format!("ELF: {}", short_path(program)));
    }
    if let Some(root) = state.source_root_override.clone() {
        ui.horizontal(|ui| {
            ui.small(format!("Local root: {root}"));
            if ui.small_button("清除").clicked() {
                state.source_root_override = None;
                state.source_cache.clear();
            }
        });
    }
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.source_filter)
                .hint_text("筛选源码文件")
                .desired_width(f32::INFINITY),
        );
        if !state.source_filter.is_empty() && ui.small_button("清除").clicked() {
            state.source_filter.clear();
        }
    });
    let source_filter = state.source_filter.trim().to_lowercase();
    let selected = egui::ScrollArea::both()
        .id_salt("debug_project_tree")
        .max_height(ui.available_height())
        .show(ui, |ui| {
            render_source_tree(
                ui,
                &state.project_tree,
                0,
                state.selected_source_path.as_deref(),
                "sources",
                &source_filter,
            )
        })
        .inner;
    if let Some(path) = selected {
        state.open_source(path);
        state.source_scroll_target = None;
        state.source_cursor = None;
        state.code_view = CodeView::Source;
    }
    if state.snapshot.source_files.is_empty() {
        ui.small("ELF 中没有可用源码文件记录");
    }
}

fn render_source_tree(
    ui: &mut Ui,
    node: &SourceTreeNode,
    depth: usize,
    selected_path: Option<&str>,
    parent_key: &str,
    filter: &str,
) -> Option<String> {
    let mut selected = None;
    for child in node.children.values() {
        let (display_name, display_node) = compressed_directory(child);
        if !source_node_matches(display_node, &display_name, filter) {
            continue;
        }
        let child_key = format!("{parent_key}/{display_name}");
        if let Some(path) = &display_node.debug_path {
            if stable_left_selectable_label_sized(
                ui,
                selected_path == Some(path.as_str()),
                &format!("▧ {display_name}"),
                egui::vec2(ui.available_width(), 22.0),
            )
            .on_hover_text(path)
            .clicked()
            {
                selected = Some(path.clone());
            }
        } else {
            let response = egui::CollapsingHeader::new(format!("▸ {display_name}"))
                .id_salt(("debug_source_dir", &child_key))
                .default_open(depth < 1 || !filter.is_empty())
                .show(ui, |ui| {
                    render_source_tree(
                        ui,
                        display_node,
                        depth + 1,
                        selected_path,
                        &child_key,
                        filter,
                    )
                });
            if let Some(path) = response.body_returned.flatten() {
                selected = Some(path);
            }
        }
    }
    selected
}

fn source_node_matches(node: &SourceTreeNode, display_name: &str, filter: &str) -> bool {
    filter.is_empty()
        || display_name.to_lowercase().contains(filter)
        || node
            .debug_path
            .as_deref()
            .is_some_and(|path| path.to_lowercase().contains(filter))
        || node
            .children
            .values()
            .any(|child| source_node_matches(child, &child.name, filter))
}

fn compressed_directory(node: &SourceTreeNode) -> (String, &SourceTreeNode) {
    let mut label = node.name.clone();
    let mut current = node;
    while current.debug_path.is_none() && current.children.len() == 1 {
        let Some(only_child) = current.children.values().next() else {
            break;
        };
        if only_child.debug_path.is_some() {
            break;
        }
        if !label.ends_with('/') {
            label.push('/');
        }
        label.push_str(only_child.name.trim_start_matches('/'));
        current = only_child;
    }
    (label, current)
}

fn render_code(ui: &mut Ui, state: &mut DebugPluginState) {
    let mut activate_source = None;
    let mut close_source = None;
    ui.horizontal(|ui| {
        let assembly_clicked =
            stable_selectable_value(ui, &mut state.code_view, CodeView::Assembly, "汇编", 64.0)
                .clicked();
        ui.separator();
        egui::ScrollArea::horizontal()
            .id_salt("debug_source_tabs")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (index, path) in state.source_tabs.iter().enumerate() {
                        let selected =
                            matches!(state.code_view, CodeView::Source | CodeView::Split)
                                && state.selected_source_path.as_deref() == Some(path.as_str());
                        let label_width = (short_path(path).chars().count() as f32 * 8.0 + 18.0)
                            .clamp(76.0, 180.0);
                        if stable_selectable_label_sized(
                            ui,
                            selected,
                            RichText::new(short_path(path)),
                            egui::vec2(label_width, 24.0),
                        )
                        .on_hover_text(path)
                        .clicked()
                        {
                            activate_source = Some(path.clone());
                        }
                        if ui
                            .add_sized([22.0, 22.0], egui::Button::new("×"))
                            .on_hover_text("关闭源码缓冲区")
                            .clicked()
                        {
                            close_source = Some(index);
                        }
                    }
                });
            });
        if assembly_clicked
            && let Some(spec) =
                state
                    .source_cursor
                    .as_ref()
                    .map(|(path, line)| BreakpointSpec::Source {
                        path: path.clone(),
                        line: *line,
                        column: None,
                    })
        {
            state.pending.push(DebugCommand::Disassemble(spec));
        }
    });
    if let Some(path) = activate_source {
        state.open_source(path);
    }
    if let Some(index) = close_source {
        state.close_source(index);
    }
    if state.code_view == CodeView::Assembly {
        ui.small(format!(
            "{} instructions",
            state.snapshot.instructions.len()
        ));
    } else if state.source_tabs.is_empty() {
        ui.small("从工程树、调用栈或汇编视图打开源码文件");
    }
    match state.code_view {
        CodeView::Assembly => render_disassembly(ui, state, "debug_disassembly"),
        CodeView::Source => render_source_code(ui, state),
        CodeView::Split => {
            let available = ui.available_rect_before_wrap();
            let (source_rect, splitter_rect, assembly_rect, ratio) =
                split_code_rects(available, state.workspace_layout.code_split_ratio);
            state.workspace_layout.code_split_ratio = ratio;
            let mut source_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("debug_split_source")
                    .max_rect(source_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            source_ui.set_clip_rect(source_rect);
            render_source_code(&mut source_ui, state);
            let mut assembly_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("debug_split_assembly")
                    .max_rect(assembly_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            assembly_ui.set_clip_rect(assembly_rect);
            render_disassembly(&mut assembly_ui, state, "debug_split_disassembly");
            let splitter = ui
                .interact(
                    splitter_rect.expand2(egui::vec2(2.0, 0.0)),
                    ui.id().with("debug_code_splitter"),
                    egui::Sense::drag(),
                )
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            if splitter.dragged() && available.width() > CODE_SPLITTER_SIZE {
                state.workspace_layout.code_split_ratio +=
                    splitter.drag_delta().x / (available.width() - CODE_SPLITTER_SIZE);
            }
            let splitter_color = if splitter.hovered() || splitter.dragged() {
                ui.visuals().selection.stroke.color
            } else {
                ui.visuals().widgets.noninteractive.bg_stroke.color
            };
            ui.painter().rect_filled(splitter_rect, 0.0, splitter_color);
            ui.advance_cursor_after_rect(available);
        }
    }
}

fn render_disassembly(ui: &mut Ui, state: &mut DebugPluginState, id: &'static str) {
    let scroll_target = state.assembly_scroll_target;
    let mut target_found = false;
    egui::ScrollArea::both().id_salt(id).show(ui, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        for instruction in state.snapshot.instructions.clone() {
            if let Some(source) = &instruction.source
                && ui
                    .add(
                        egui::Label::new(
                            RichText::new(format!(
                                "{}:{}",
                                short_path(&source.path),
                                source.line.unwrap_or(0)
                            ))
                            .color(ui.visuals().weak_text_color()),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .on_hover_text("打开对应源码")
                    .clicked()
            {
                state.navigate_to_source(source.path.clone(), source.line, true);
            }
            let current = state.snapshot.pc.map(|pc| pc & !1) == Some(instruction.address & !1);
            let breakpoint = state.snapshot.breakpoints.iter().any(|breakpoint| {
                breakpoint.enabled
                    && breakpoint.address.map(|address| address & !1)
                        == Some(instruction.address & !1)
            });
            let prefix = match (current, breakpoint) {
                (true, true) => "▶●",
                (true, false) => "▶ ",
                (false, true) => " ●",
                _ => "  ",
            };
            let text = format!(
                "{prefix} 0x{:08X}  {:<14} {}",
                instruction.address, instruction.bytes, instruction.instruction
            );
            let rich = if current {
                RichText::new(text).monospace().strong()
            } else {
                RichText::new(text).monospace()
            };
            let response = stable_selectable_label(
                ui,
                state.assembly_cursor == Some(instruction.address),
                rich,
            );
            if scroll_target.map(normalize_code_address)
                == Some(normalize_code_address(instruction.address))
            {
                response.scroll_to_me(Some(egui::Align::Center));
                target_found = true;
            }
            if response.clicked() {
                state.assembly_cursor = Some(instruction.address);
            }
            if response.double_clicked() {
                if let Some(existing) = state.snapshot.breakpoints.iter().find(|breakpoint| {
                    breakpoint.address.map(|address| address & !1) == Some(instruction.address & !1)
                }) {
                    state
                        .pending
                        .push(DebugCommand::RemoveBreakpoint(existing.id));
                } else {
                    state.add_breakpoint(BreakpointSpec::Instruction {
                        address: instruction.address,
                    });
                }
            }
        }
        if state.snapshot.instructions.is_empty() {
            ui.label("没有可显示的汇编。请加载带代码段的 ELF，或暂停目标后刷新。");
        }
    });
    if target_found {
        state.assembly_scroll_target = None;
    }
}

fn normalize_code_address(address: u64) -> u64 {
    address & !1
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceToken {
    Plain,
    Keyword,
    Type,
    Number,
    String,
    Comment,
    Preprocessor,
}

fn source_highlight_job(
    ui: &Ui,
    line_number: usize,
    line: &str,
    current: bool,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let line_color = if current {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().weak_text_color()
    };
    job.append(
        &format!("{line_number:>4}  "),
        0.0,
        egui::TextFormat {
            font_id: font_id.clone(),
            color: line_color,
            ..Default::default()
        },
    );
    if line.trim_start().starts_with('#') {
        append_source_token(ui, &mut job, line, SourceToken::Preprocessor, &font_id);
        return job;
    }

    let bytes = line.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"//") {
            append_source_token(ui, &mut job, &line[index..], SourceToken::Comment, &font_id);
            break;
        }
        let byte = bytes[index];
        if byte == b'"' || byte == b'\'' {
            let quote = byte;
            let start = index;
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index = (index + 2).min(bytes.len());
                } else if bytes[index] == quote {
                    index += 1;
                    break;
                } else {
                    index += 1;
                }
            }
            append_source_token(
                ui,
                &mut job,
                &line[start..index],
                SourceToken::String,
                &font_id,
            );
        } else if byte.is_ascii_digit() {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'_' | b'.'))
            {
                index += 1;
            }
            append_source_token(
                ui,
                &mut job,
                &line[start..index],
                SourceToken::Number,
                &font_id,
            );
        } else if byte.is_ascii_alphabetic() || byte == b'_' {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let word = &line[start..index];
            let token = if is_source_keyword(word) {
                SourceToken::Keyword
            } else if is_source_type(word) {
                SourceToken::Type
            } else {
                SourceToken::Plain
            };
            append_source_token(ui, &mut job, word, token, &font_id);
        } else {
            let char_len = line[index..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(1);
            append_source_token(
                ui,
                &mut job,
                &line[index..index + char_len],
                SourceToken::Plain,
                &font_id,
            );
            index += char_len;
        }
    }
    job
}

fn append_source_token(
    ui: &Ui,
    job: &mut egui::text::LayoutJob,
    text: &str,
    token: SourceToken,
    font_id: &egui::FontId,
) {
    let dark = ui.visuals().dark_mode;
    let color = match (token, dark) {
        (SourceToken::Plain, _) => ui.visuals().text_color(),
        (SourceToken::Keyword, true) => egui::Color32::from_rgb(198, 120, 221),
        (SourceToken::Keyword, false) => egui::Color32::from_rgb(125, 52, 145),
        (SourceToken::Type, true) => egui::Color32::from_rgb(86, 182, 194),
        (SourceToken::Type, false) => egui::Color32::from_rgb(20, 116, 125),
        (SourceToken::Number, true) => egui::Color32::from_rgb(209, 154, 102),
        (SourceToken::Number, false) => egui::Color32::from_rgb(156, 88, 34),
        (SourceToken::String, true) => egui::Color32::from_rgb(152, 195, 121),
        (SourceToken::String, false) => egui::Color32::from_rgb(54, 124, 46),
        (SourceToken::Comment, true) => egui::Color32::from_rgb(106, 153, 85),
        (SourceToken::Comment, false) => egui::Color32::from_rgb(70, 128, 55),
        (SourceToken::Preprocessor, true) => egui::Color32::from_rgb(229, 192, 123),
        (SourceToken::Preprocessor, false) => egui::Color32::from_rgb(150, 91, 22),
    };
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: font_id.clone(),
            color,
            italics: token == SourceToken::Comment,
            ..Default::default()
        },
    );
}

fn is_source_keyword(word: &str) -> bool {
    matches!(
        word,
        "alignas"
            | "alignof"
            | "asm"
            | "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "constexpr"
            | "continue"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "extern"
            | "for"
            | "if"
            | "inline"
            | "let"
            | "loop"
            | "match"
            | "mut"
            | "namespace"
            | "new"
            | "operator"
            | "override"
            | "pub"
            | "return"
            | "sizeof"
            | "static"
            | "struct"
            | "switch"
            | "template"
            | "this"
            | "throw"
            | "trait"
            | "try"
            | "typedef"
            | "typename"
            | "union"
            | "unsafe"
            | "using"
            | "virtual"
            | "volatile"
            | "where"
            | "while"
    )
}

fn is_source_type(word: &str) -> bool {
    matches!(
        word,
        "bool"
            | "char"
            | "double"
            | "f32"
            | "f64"
            | "float"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "int"
            | "isize"
            | "long"
            | "short"
            | "signed"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "uint8_t"
            | "uint16_t"
            | "uint32_t"
            | "uint64_t"
            | "unsigned"
            | "usize"
            | "void"
    )
}

const SOURCE_ROW_HEIGHT: f32 = 20.0;

#[cfg(test)]
std::thread_local! {
    static SOURCE_ROWS_RENDERED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static SOURCE_ROW_WIDTHS: std::cell::RefCell<Vec<f32>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceVisualRow {
    Source(usize),
    Inline {
        line_index: usize,
        instruction_index: usize,
    },
}

const CODE_SPLITTER_SIZE: f32 = 5.0;
const CODE_PANE_MIN_WIDTH: f32 = 120.0;

fn split_code_rects(
    available: egui::Rect,
    requested_ratio: f32,
) -> (egui::Rect, egui::Rect, egui::Rect, f32) {
    let content_width = (available.width() - CODE_SPLITTER_SIZE).max(0.0);
    let ratio = if content_width >= CODE_PANE_MIN_WIDTH * 2.0 {
        let minimum = CODE_PANE_MIN_WIDTH / content_width;
        requested_ratio.clamp(minimum, 1.0 - minimum)
    } else {
        0.5
    };
    let split_x = available.left() + content_width * ratio;
    let source = egui::Rect::from_min_max(available.min, egui::pos2(split_x, available.bottom()));
    let splitter = egui::Rect::from_min_max(
        egui::pos2(split_x, available.top()),
        egui::pos2(split_x + CODE_SPLITTER_SIZE, available.bottom()),
    );
    let assembly = egui::Rect::from_min_max(
        egui::pos2(split_x + CODE_SPLITTER_SIZE, available.top()),
        available.max,
    );
    (source, splitter, assembly, ratio)
}

/// Store only expanded lines. Mapping a visible visual row is proportional to
/// the number of expanded lines, rather than the source file length.
fn source_expanded_rows(
    state: &DebugPluginState,
    path: &str,
    line_count: usize,
) -> Vec<(usize, usize)> {
    let mut rows = state
        .expanded_source_assembly
        .iter()
        .filter_map(|(expanded_path, line)| {
            let line_index = usize::try_from(line.saturating_sub(1)).ok()?;
            if expanded_path != path || line_index >= line_count {
                return None;
            }
            let instruction_count = state
                .inline_assembly_cache
                .get(&(expanded_path.clone(), *line))
                .map_or(1, |instructions| instructions.len().max(1));
            Some((line_index, instruction_count))
        })
        .collect::<Vec<_>>();
    rows.sort_unstable_by_key(|row| row.0);
    rows
}

fn source_visual_row_count(line_count: usize, expanded: &[(usize, usize)]) -> usize {
    expanded
        .iter()
        .fold(line_count, |count, (_, extra)| count.saturating_add(*extra))
}

fn source_visual_row_for_line(line_index: usize, expanded: &[(usize, usize)]) -> usize {
    expanded
        .iter()
        .filter(|(expanded_line, _)| *expanded_line < line_index)
        .fold(line_index, |row, (_, extra)| row.saturating_add(*extra))
}

fn source_visual_row_at(
    visual_row: usize,
    line_count: usize,
    expanded: &[(usize, usize)],
) -> Option<SourceVisualRow> {
    let mut source_index = 0usize;
    let mut display_index = 0usize;
    for &(expanded_line, extra_rows) in expanded {
        let source_rows_before = expanded_line.saturating_sub(source_index);
        if visual_row < display_index.saturating_add(source_rows_before) {
            return Some(SourceVisualRow::Source(
                source_index + visual_row.saturating_sub(display_index),
            ));
        }
        display_index = display_index.saturating_add(source_rows_before);
        if visual_row == display_index {
            return Some(SourceVisualRow::Source(expanded_line));
        }
        display_index = display_index.saturating_add(1);
        if visual_row < display_index.saturating_add(extra_rows) {
            return Some(SourceVisualRow::Inline {
                line_index: expanded_line,
                instruction_index: visual_row.saturating_sub(display_index),
            });
        }
        display_index = display_index.saturating_add(extra_rows);
        source_index = expanded_line.saturating_add(1);
    }
    let remaining_index = source_index.saturating_add(visual_row.saturating_sub(display_index));
    (remaining_index < line_count).then_some(SourceVisualRow::Source(remaining_index))
}

fn render_inline_source_row(
    ui: &mut Ui,
    state: &DebugPluginState,
    path: &str,
    row: SourceVisualRow,
) {
    let SourceVisualRow::Inline {
        line_index,
        instruction_index,
    } = row
    else {
        return;
    };
    let key = (path.to_owned(), line_index as u64 + 1);
    let text = state
        .inline_assembly_cache
        .get(&key)
        .and_then(|instructions| instructions.get(instruction_index))
        .map(|instruction| {
            format!(
                "    0x{:08X}  {:<14} {}",
                instruction.address, instruction.bytes, instruction.instruction
            )
        })
        .unwrap_or_else(|| "    正在加载该行对应汇编…".to_owned());
    let width = ui.available_width();
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(width, SOURCE_ROW_HEIGHT), egui::Sense::hover());
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        egui::TextStyle::Monospace.resolve(ui.style()),
        ui.visuals().text_color(),
    );
}

fn render_source_code(ui: &mut Ui, state: &mut DebugPluginState) {
    let frame_location = state
        .snapshot
        .frames
        .get(state.snapshot.selected_frame)
        .and_then(|frame| frame.source.clone());
    let debug_path = state
        .selected_source_path
        .clone()
        .or_else(|| frame_location.as_ref().map(|source| source.path.clone()));
    let Some(debug_path) = debug_path else {
        ui.centered_and_justified(|ui| ui.label("请从工程树选择源码文件"));
        return;
    };
    let current_line = frame_location
        .as_ref()
        .filter(|source| source.path == debug_path)
        .and_then(|source| source.line)
        .map(|line| line.max(1) as usize);
    let local_path = resolve_local_source_path(
        &debug_path,
        &state.debug_source_root,
        state.source_root_override.as_deref(),
    );
    ui.horizontal(|ui| {
        ui.monospace(short_path(&debug_path));
        ui.small("· 可执行行  ● 断点  ▶ PC");
        if local_path != debug_path {
            ui.small(format!("→ {local_path}"));
        }
    });
    let source = state
        .source_cache
        .entry(local_path.clone())
        .or_insert_with(|| {
            std::fs::read_to_string(&local_path)
                .map(|text| text.lines().map(str::to_owned).collect::<Vec<_>>().into())
                .map_err(|error| format!("无法读取源码文件 {local_path}: {error}"))
        });
    let Ok(lines) = source else {
        ui.colored_label(
            ui.visuals().error_fg_color,
            source.as_ref().unwrap_err().as_str(),
        );
        return;
    };

    // Keep the immutable file shared while navigation mutates the UI state;
    // cloning the handle avoids copying every source line on every repaint.
    let lines = std::sync::Arc::clone(lines);
    let expanded_rows = source_expanded_rows(state, &debug_path, lines.len());
    let visual_row_count = source_visual_row_count(lines.len(), &expanded_rows);
    let target = state
        .source_scroll_target
        .filter(|line| (1..=lines.len()).contains(line));
    // Source rows fill the editor viewport. Long text is already clipped by
    // the fixed row control, so horizontal content sizing only made the layout
    // change while scrolling between short and long lines.
    let mut scroll = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .id_salt(("debug_source", &debug_path));
    if let Some(line) = target {
        let target_row = source_visual_row_for_line(line - 1, &expanded_rows);
        let row_stride = SOURCE_ROW_HEIGHT + ui.spacing().item_spacing.y;
        let offset = (target_row as f32 * row_stride - ui.available_height() * 0.5).max(0.0);
        scroll = scroll.vertical_scroll_offset(offset);
    }
    scroll.show_rows(
        ui,
        SOURCE_ROW_HEIGHT,
        visual_row_count,
        |ui, visible_rows| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            for visual_row in visible_rows {
                #[cfg(test)]
                SOURCE_ROWS_RENDERED.with(|count| count.set(count.get() + 1));
                let Some(row) = source_visual_row_at(visual_row, lines.len(), &expanded_rows)
                else {
                    continue;
                };
                let SourceVisualRow::Source(index) = row else {
                    render_inline_source_row(ui, state, &debug_path, row);
                    continue;
                };
                let line = &lines[index];
                let line_number = index + 1;
                let has_breakpoint =
                    source_breakpoint_at(state, &debug_path, line_number as u64).is_some();
                let executable_address =
                    executable_line_address(state, &debug_path, line_number as u64);
                let current = current_line == Some(line_number);
                let marker = match (current, has_breakpoint, executable_address.is_some()) {
                    (true, true, _) => "▶●",
                    (true, false, true) => "▶·",
                    (true, false, false) => "▶ ",
                    (false, true, _) => " ●",
                    (false, false, true) => " ·",
                    _ => "  ",
                };
                let mut toggle_breakpoint = false;
                let mut toggle_inline_assembly = false;
                let mut select_line = false;
                let response = ui
                    .allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), SOURCE_ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            toggle_breakpoint = ui
                                .add_sized(
                                    [22.0, 18.0],
                                    egui::Label::new(RichText::new(marker).monospace())
                                        .sense(egui::Sense::click()),
                                )
                                .on_hover_text("单击设置或移除源码断点 (F9)")
                                .clicked();
                            if executable_address.is_some() {
                                let expanded = state
                                    .expanded_source_assembly
                                    .contains(&(debug_path.clone(), line_number as u64));
                                toggle_inline_assembly = compact_expander(
                                    ui,
                                    expanded,
                                    "展开该行对应汇编",
                                    "收起该行汇编",
                                )
                                .clicked();
                            } else {
                                ui.allocate_space(egui::vec2(16.0, 16.0));
                            }
                            let job = source_highlight_job(ui, line_number, line, current);
                            let code = stable_selectable_job(
                                ui,
                                state.source_cursor.as_ref().is_some_and(|(path, line)| {
                                    path == &debug_path && *line == line_number as u64
                                }),
                                job,
                            );
                            select_line = code.clicked();
                            toggle_breakpoint |= code.double_clicked();
                        },
                    )
                    .response;
                #[cfg(test)]
                SOURCE_ROW_WIDTHS.with(|widths| widths.borrow_mut().push(response.rect.width()));
                if target == Some(line_number) {
                    response.scroll_to_me(Some(egui::Align::Center));
                    state.source_scroll_target = None;
                }
                if select_line {
                    state.source_cursor = Some((debug_path.clone(), line_number as u64));
                    state.push_navigation(debug_path.clone(), Some(line_number as u64));
                    if let Some(address) = executable_address {
                        state.assembly_cursor = Some(address);
                    }
                }
                if toggle_breakpoint {
                    if let Some(id) = source_breakpoint_at(state, &debug_path, line_number as u64) {
                        state.pending.push(DebugCommand::RemoveBreakpoint(id));
                    } else {
                        state.add_breakpoint(BreakpointSpec::Source {
                            path: debug_path.clone(),
                            line: line_number as u64,
                            column: None,
                        });
                    }
                }
                let inline_key = (debug_path.clone(), line_number as u64);
                if toggle_inline_assembly {
                    if state.expanded_source_assembly.remove(&inline_key) {
                        state.inline_assembly_cache.remove(&inline_key);
                    } else {
                        state.expanded_source_assembly.insert(inline_key.clone());
                        state.cache_inline_assembly(&inline_key);
                        state
                            .pending
                            .push(DebugCommand::Disassemble(BreakpointSpec::Source {
                                path: debug_path.clone(),
                                line: line_number as u64,
                                column: None,
                            }));
                    }
                }
            }
        },
    );
}

fn source_breakpoint_at(state: &DebugPluginState, path: &str, line: u64) -> Option<u64> {
    state.snapshot.breakpoints.iter().find_map(|breakpoint| {
        if let Some(resolved) = &breakpoint.resolved_source {
            return (resolved.path == path && resolved.line == Some(line)).then_some(breakpoint.id);
        }
        match &breakpoint.spec {
            BreakpointSpec::Source {
                path: breakpoint_path,
                line: breakpoint_line,
                ..
            } if breakpoint_path == path && *breakpoint_line == line => Some(breakpoint.id),
            _ => None,
        }
    })
}

fn executable_line_address(state: &DebugPluginState, path: &str, line: u64) -> Option<u64> {
    state
        .executable_line_index
        .get(path)
        .and_then(|lines| lines.get(&line))
        .copied()
}

fn current_cursor_spec(state: &DebugPluginState) -> Option<BreakpointSpec> {
    match state.code_view {
        CodeView::Assembly => state
            .assembly_cursor
            .map(|address| BreakpointSpec::Instruction { address }),
        CodeView::Source | CodeView::Split => {
            state
                .source_cursor
                .as_ref()
                .map(|(path, line)| BreakpointSpec::Source {
                    path: path.clone(),
                    line: *line,
                    column: None,
                })
        }
    }
}

fn render_inspector(ui: &mut Ui, state: &mut DebugPluginState) {
    ui.horizontal(|ui| {
        stable_selectable_value(
            ui,
            &mut state.inspector_tab,
            InspectorTab::CallStack,
            "调用栈",
            72.0,
        );
        stable_selectable_value(
            ui,
            &mut state.inspector_tab,
            InspectorTab::StackMemory,
            "栈内存",
            72.0,
        );
    });
    ui.separator();
    match state.inspector_tab {
        InspectorTab::CallStack => render_call_stack(ui, state),
        InspectorTab::StackMemory => render_stack_memory(ui, state),
    }
}

fn render_registers_panel(ui: &mut Ui, state: &DebugPluginState) {
    ui.horizontal(|ui| {
        ui.strong("寄存器");
        ui.small(format!("{} registers", state.snapshot.registers.len()));
    });
    ui.separator();
    render_registers(ui, state);
}

fn render_registers(ui: &mut Ui, state: &DebugPluginState) {
    egui::ScrollArea::vertical()
        .id_salt("debug_registers")
        .show(ui, |ui| {
            let width = ui.available_width().max(1.0);
            let name_width = (width * 0.34).clamp(72.0, 116.0).min(width);
            for (index, register) in state.snapshot.registers.iter().enumerate() {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, 22.0), egui::Sense::hover());
                if index % 2 == 0 {
                    ui.painter()
                        .rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
                }
                let name_rect = egui::Rect::from_min_max(
                    rect.min,
                    egui::pos2(rect.left() + name_width, rect.bottom()),
                )
                .shrink2(egui::vec2(4.0, 0.0));
                let value_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.left() + name_width, rect.top()),
                    rect.max,
                )
                .shrink2(egui::vec2(4.0, 0.0));
                let font = egui::TextStyle::Monospace.resolve(ui.style());
                let painter = ui.painter().with_clip_rect(rect);
                painter.text(
                    name_rect.left_center(),
                    egui::Align2::LEFT_CENTER,
                    &register.name,
                    font.clone(),
                    ui.visuals().text_color(),
                );
                painter.text(
                    value_rect.left_center(),
                    egui::Align2::LEFT_CENTER,
                    register.value.as_deref().unwrap_or("不可用"),
                    font,
                    ui.visuals().text_color(),
                );
            }
        });
}

fn render_stack_memory(ui: &mut Ui, state: &DebugPluginState) {
    egui::ScrollArea::both()
        .id_salt("debug_stack_memory")
        .show(ui, |ui| {
            for word in &state.snapshot.stack_memory {
                ui.monospace(format!("0x{:08X}  0x{:08X}", word.address, word.value));
            }
            if state.snapshot.stack_memory.is_empty() {
                ui.label("目标暂停后显示 SP 附近的原始栈内存");
            }
        });
}

fn render_locals(ui: &mut Ui, state: &mut DebugPluginState) {
    ui.horizontal(|ui| {
        ui.strong("局部变量");
        ui.small(format!("frame #{}", state.snapshot.selected_frame));
    });
    ui.separator();
    egui::ScrollArea::both()
        .id_salt("debug_bottom_locals")
        .show(ui, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            render_variable_children(ui, state, 0, 0);
            if state.snapshot.variables.is_empty() {
                ui.label("当前栈帧没有可用的局部变量");
            }
        });
}

fn render_variable_children(
    ui: &mut Ui,
    state: &mut DebugPluginState,
    parent_reference: i64,
    depth: usize,
) {
    let children: Vec<VariableView> = state
        .snapshot
        .variables
        .iter()
        .filter(|variable| variable.parent_reference == parent_reference)
        .cloned()
        .collect();
    for variable in children {
        let mut commit_value = None;
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 12.0);
            if variable.has_children {
                let expanded = state.expanded_variables.contains(&variable.reference);
                if compact_expander(ui, expanded, "展开变量", "收起变量").clicked() {
                    if expanded {
                        state.expanded_variables.remove(&variable.reference);
                    } else {
                        state.expanded_variables.insert(variable.reference);
                        state.pending.push(DebugCommand::ExpandVariable {
                            stop_id: state.snapshot.stop_id,
                            frame_index: state.snapshot.selected_frame,
                            variable_ref: variable.reference,
                        });
                    }
                }
            } else {
                ui.allocate_space(egui::vec2(16.0, 16.0));
            }
            ui.monospace(&variable.name);
            ui.label(RichText::new(&variable.type_name).color(ui.visuals().weak_text_color()));
            if variable.writable {
                let edit = state
                    .variable_edits
                    .entry(variable.reference)
                    .or_insert_with(|| editable_variable_value(&variable.value));
                let edit_width = (ui.available_width() - 38.0).max(1.0);
                let response = ui.add(
                    egui::TextEdit::singleline(edit)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(edit_width),
                );
                let enter =
                    response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                let submit = ui
                    .add_enabled(
                        !state.command_pending,
                        egui::Button::new("写").min_size(egui::vec2(28.0, 20.0)),
                    )
                    .on_hover_text("写入局部变量；也可按 Enter")
                    .clicked();
                if enter || submit {
                    commit_value = Some(edit.clone());
                }
            } else {
                let value = single_line_variable_value(&variable.value);
                ui.add_sized(
                    [ui.available_width().max(1.0), 20.0],
                    egui::Label::new(RichText::new(value).monospace()).truncate(),
                );
            }
        });
        if let Some(value) = commit_value {
            state.pending.push(DebugCommand::WriteVariable {
                stop_id: state.snapshot.stop_id,
                frame_index: state.snapshot.selected_frame,
                variable_ref: variable.reference,
                value,
            });
        }
        if state.expanded_variables.contains(&variable.reference) {
            render_variable_children(ui, state, variable.reference, depth + 1);
        }
    }
}

fn editable_variable_value(value: &str) -> String {
    let single = single_line_variable_value(value);
    single
        .split_once(" (")
        .map(|(plain, _)| plain)
        .unwrap_or(&single)
        .trim()
        .to_owned()
}

fn single_line_variable_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl DebugPluginState {
    fn open_source(&mut self, path: String) {
        self.navigate_to_source(path, None, true);
    }

    fn navigate_to_source(&mut self, path: String, line: Option<u64>, record: bool) {
        let changed = self.selected_source_path.as_deref() != Some(path.as_str());
        if !self.source_tabs.iter().any(|open| open == &path) {
            self.source_tabs.push(path.clone());
        }
        self.selected_source_path = Some(path.clone());
        if self.code_view != CodeView::Split {
            self.code_view = CodeView::Source;
        }
        if changed {
            self.source_scroll_target = None;
            self.source_cursor = None;
        }
        if let Some(line) = line {
            self.source_scroll_target = Some(line as usize);
            self.source_cursor = Some((path.clone(), line));
        }
        if record {
            self.push_navigation(path, line);
        }
    }

    fn push_navigation(&mut self, path: String, line: Option<u64>) {
        let point = SourceNavigation { path, line };
        if self
            .navigation_index
            .and_then(|index| self.navigation_history.get(index))
            == Some(&point)
        {
            return;
        }
        let keep = self.navigation_index.map_or(0, |index| index + 1);
        self.navigation_history.truncate(keep);
        self.navigation_history.push(point);
        self.navigation_index = Some(self.navigation_history.len() - 1);
    }

    fn can_navigate_back(&self) -> bool {
        self.navigation_index.is_some_and(|index| index > 0)
    }

    fn can_navigate_forward(&self) -> bool {
        self.navigation_index
            .is_some_and(|index| index + 1 < self.navigation_history.len())
    }

    fn navigate_back(&mut self) {
        let Some(index) = self.navigation_index.filter(|index| *index > 0) else {
            return;
        };
        let index = index - 1;
        self.navigation_index = Some(index);
        if let Some(point) = self.navigation_history.get(index).cloned() {
            self.navigate_to_source(point.path, point.line, false);
        }
    }

    fn navigate_forward(&mut self) {
        let Some(index) = self
            .navigation_index
            .filter(|index| *index + 1 < self.navigation_history.len())
        else {
            return;
        };
        let index = index + 1;
        self.navigation_index = Some(index);
        if let Some(point) = self.navigation_history.get(index).cloned() {
            self.navigate_to_source(point.path, point.line, false);
        }
    }

    fn jump_selected_line_to_assembly(&mut self) {
        let Some((path, line)) = self.source_cursor.clone() else {
            return;
        };
        self.pending_assembly_source = Some((path.clone(), line));
        if let Some(address) = self
            .snapshot
            .executable_lines
            .iter()
            .find(|record| record.path == path && record.line == line)
            .map(|record| record.address)
        {
            self.assembly_cursor = Some(address);
            self.assembly_scroll_target = Some(address);
        }
        self.pending
            .push(DebugCommand::Disassemble(BreakpointSpec::Source {
                path,
                line,
                column: None,
            }));
        if self.code_view != CodeView::Split {
            self.code_view = CodeView::Assembly;
        }
    }

    fn selected_assembly_source(&self) -> Option<SourceLocationView> {
        let address = self.assembly_cursor.or(self.snapshot.pc)?;
        self.snapshot
            .instructions
            .iter()
            .find(|instruction| {
                normalize_code_address(instruction.address) == normalize_code_address(address)
            })
            .and_then(|instruction| instruction.source.clone())
    }

    fn jump_selected_instruction_to_source(&mut self) {
        let Some(source) = self.selected_assembly_source() else {
            return;
        };
        self.navigate_to_source(source.path, source.line, true);
    }

    fn resolve_pending_assembly_scroll(&mut self) {
        let Some((path, line)) = self.pending_assembly_source.clone() else {
            return;
        };
        let address = self
            .snapshot
            .instructions
            .iter()
            .find(|instruction| {
                instruction
                    .source
                    .as_ref()
                    .is_some_and(|source| source.path == path && source.line == Some(line))
            })
            .map(|instruction| instruction.address);
        if let Some(address) = address {
            self.assembly_cursor = Some(address);
            self.assembly_scroll_target = Some(address);
            self.pending_assembly_source = None;
        }
    }

    fn capture_inline_assembly(&mut self) {
        for key in self.expanded_source_assembly.clone() {
            self.cache_inline_assembly(&key);
        }
    }

    fn cache_inline_assembly(&mut self, key: &(String, u64)) {
        let instructions = self
            .snapshot
            .instructions
            .iter()
            .filter(|instruction| {
                instruction
                    .source
                    .as_ref()
                    .is_some_and(|source| source.path == key.0 && source.line == Some(key.1))
            })
            .cloned()
            .collect::<Vec<_>>();
        if !instructions.is_empty() {
            self.inline_assembly_cache.insert(key.clone(), instructions);
        }
    }

    fn close_source(&mut self, index: usize) {
        if index >= self.source_tabs.len() {
            return;
        }
        let was_selected =
            self.selected_source_path.as_deref() == self.source_tabs.get(index).map(String::as_str);
        self.source_tabs.remove(index);
        if was_selected {
            self.source_scroll_target = None;
            self.source_cursor = None;
            self.selected_source_path = self
                .source_tabs
                .get(index.min(self.source_tabs.len().saturating_sub(1)))
                .cloned();
            if self.selected_source_path.is_none() {
                self.code_view = CodeView::Assembly;
                self.source_cursor = None;
            }
        }
    }

    fn add_breakpoint(&mut self, spec: BreakpointSpec) {
        let id = self.next_breakpoint_id;
        self.next_breakpoint_id = self.next_breakpoint_id.saturating_add(1);
        self.pending
            .push(DebugCommand::SetBreakpoint(LogicalBreakpoint {
                id,
                spec,
                enabled: true,
            }));
    }

    fn focus_breakpoint(&mut self, breakpoint: &crate::model::BreakpointView) {
        self.assembly_cursor = breakpoint.address;
        if let Some(source) = breakpoint.resolved_source.as_ref()
            && let Some(line) = source.line
        {
            self.navigate_to_source(source.path.clone(), Some(line), true);
        } else if let Some(address) = breakpoint.address.or(match breakpoint.spec {
            BreakpointSpec::Instruction { address } => Some(address),
            BreakpointSpec::Source { .. } => None,
        }) {
            self.code_view = CodeView::Assembly;
            self.pending
                .push(DebugCommand::Disassemble(BreakpointSpec::Instruction {
                    address,
                }));
        }
    }
}

fn handle_debug_shortcuts(ui: &Ui, state: &mut DebugPluginState) {
    if ui.ctx().egui_wants_keyboard_input() || state.command_pending || !state.snapshot.active {
        return;
    }

    let halted = state.snapshot.target_state.is_halted();
    let top_frame_selected = state.snapshot.selected_frame == 0;
    let command = ui.ctx().input_mut(|input| {
        if input.consume_key(egui::Modifiers::NONE, egui::Key::F5) {
            halted.then_some(DebugCommand::Continue)
        } else if input.consume_key(egui::Modifiers::NONE, egui::Key::F6) {
            (state.connected && !halted).then_some(DebugCommand::Halt)
        } else if input.consume_key(egui::Modifiers::NONE, egui::Key::F10) {
            (halted && top_frame_selected).then_some(DebugCommand::Step(StepKind::Over))
        } else if input.consume_key(egui::Modifiers::SHIFT, egui::Key::F11) {
            (halted && top_frame_selected).then_some(DebugCommand::Step(StepKind::Out))
        } else if input.consume_key(egui::Modifiers::NONE, egui::Key::F11) {
            (halted && top_frame_selected).then_some(DebugCommand::Step(StepKind::Into))
        } else if input.consume_key(egui::Modifiers::NONE, egui::Key::F9) {
            current_cursor_spec(state).map(|spec| {
                if let Some(id) = breakpoint_id_for_spec(state, &spec) {
                    DebugCommand::RemoveBreakpoint(id)
                } else {
                    let id = state.next_breakpoint_id;
                    state.next_breakpoint_id = state.next_breakpoint_id.saturating_add(1);
                    DebugCommand::SetBreakpoint(LogicalBreakpoint {
                        id,
                        spec,
                        enabled: true,
                    })
                }
            })
        } else {
            None
        }
    });
    if let Some(command) = command {
        state.pending.push(command);
    }
}

fn breakpoint_id_for_spec(state: &DebugPluginState, spec: &BreakpointSpec) -> Option<u64> {
    state.snapshot.breakpoints.iter().find_map(|breakpoint| {
        if &breakpoint.spec == spec {
            Some(breakpoint.id)
        } else {
            None
        }
    })
}

fn breakpoint_label(spec: &BreakpointSpec) -> String {
    match spec {
        BreakpointSpec::Instruction { address } => format!("0x{address:08X}"),
        BreakpointSpec::Source { path, line, .. } => format!("{}:{line}", short_path(path)),
    }
}

fn short_path(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn normalized_components(path: &str) -> Vec<String> {
    path.replace('\\', "/")
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .map(str::to_owned)
        .collect()
}

fn build_source_tree(files: &[String]) -> (SourceTreeNode, String) {
    let components: Vec<Vec<String>> = files
        .iter()
        .map(|path| normalized_components(path))
        .collect();
    let common_len = components
        .first()
        .map(|first| {
            first
                .iter()
                .enumerate()
                .take_while(|(index, component)| {
                    components
                        .iter()
                        .all(|path| path.get(*index) == Some(*component))
                })
                .count()
        })
        .unwrap_or(0);
    let mut common_root = components
        .first()
        .map(|path| path[..common_len.min(path.len().saturating_sub(1))].join("/"))
        .unwrap_or_default();
    if files.first().is_some_and(|path| path.starts_with('/')) && !common_root.is_empty() {
        common_root.insert(0, '/');
    }
    let trim_len = common_len.min(
        components
            .iter()
            .map(Vec::len)
            .min()
            .unwrap_or_default()
            .saturating_sub(1),
    );
    let mut root = SourceTreeNode::root();
    if trim_len > 0 {
        root.children.insert(
            common_root.clone(),
            SourceTreeNode {
                name: common_root.clone(),
                ..Default::default()
            },
        );
    }
    for (original, path) in files.iter().zip(components) {
        let mut current = if trim_len > 0 {
            root.children
                .get_mut(&common_root)
                .expect("common source root was inserted")
        } else {
            &mut root
        };
        for (index, component) in path.iter().enumerate().skip(trim_len) {
            current = current
                .children
                .entry(component.clone())
                .or_insert_with(|| SourceTreeNode {
                    name: component.clone(),
                    ..Default::default()
                });
            if index + 1 == path.len() {
                current.debug_path = Some(original.clone());
            }
        }
    }
    (root, common_root)
}

fn resolve_local_source_path(
    debug_path: &str,
    debug_root: &str,
    local_root: Option<&str>,
) -> String {
    if std::path::Path::new(debug_path).is_file() {
        return debug_path.to_owned();
    }
    let Some(local_root) = local_root else {
        return debug_path.to_owned();
    };
    let normalized = debug_path.replace('\\', "/");
    let relative = normalized
        .strip_prefix(debug_root)
        .unwrap_or(&normalized)
        .trim_start_matches('/');
    // Keep mapped source paths in the same separator form as DWARF paths.
    // `Path::join` uses the host separator, which produced mixed paths such
    // as `/workspace/src\\driver/gpio.c` on Windows.  Forward slashes are
    // accepted by Windows filesystem APIs and make path keys/comparisons
    // deterministic across platforms.
    std::path::Path::new(local_root)
        .join(relative)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::{
        CODE_PANE_MIN_WIDTH, CODE_SPLITTER_SIZE, CodeView, SOURCE_ROW_WIDTHS, SOURCE_ROWS_RENDERED,
        SourceVisualRow, breakpoint_label, build_source_tree, compact_expander,
        compressed_directory, current_cursor_spec, editable_variable_value, render_source_code,
        render_toolbar, resolve_local_source_path, single_line_variable_value,
        source_highlight_job, source_node_matches, source_visual_row_at, source_visual_row_count,
        source_visual_row_for_line, split_code_rects, stabilize_debug_style, step_method_label,
    };
    use crate::model::VariablePool;
    use crate::model::{
        BreakpointSpec, BreakpointView, DebugCommand, DebugStartMode, InstructionView,
        SourceLocationView,
    };
    use crate::ui::plugin::{
        FrameData, MemRWPlugin, PluginAction, PluginRenderContext, PluginUpdateContext,
    };

    #[test]
    fn formats_source_and_instruction_breakpoints() {
        assert_eq!(
            breakpoint_label(&BreakpointSpec::Instruction { address: 0x100 }),
            "0x00000100"
        );
        assert_eq!(
            breakpoint_label(&BreakpointSpec::Source {
                path: "/tmp/src/main.rs".to_owned(),
                line: 12,
                column: None,
            }),
            "main.rs:12"
        );
    }

    #[test]
    fn source_buffers_open_once_switch_and_close_to_an_adjacent_tab() {
        let mut plugin = super::DebugPluginState::default();
        plugin.open_source("/src/main.cpp".to_owned());
        plugin.open_source("/src/driver.cpp".to_owned());
        plugin.open_source("/src/main.cpp".to_owned());
        assert_eq!(plugin.source_tabs.len(), 2);
        assert_eq!(
            plugin.selected_source_path.as_deref(),
            Some("/src/main.cpp")
        );

        plugin.close_source(0);
        assert_eq!(plugin.source_tabs, vec!["/src/driver.cpp"]);
        assert_eq!(
            plugin.selected_source_path.as_deref(),
            Some("/src/driver.cpp")
        );
        plugin.close_source(0);
        assert!(plugin.source_tabs.is_empty());
        assert_eq!(plugin.code_view, CodeView::Assembly);
    }

    #[test]
    fn local_variable_editor_removes_the_display_only_hex_suffix() {
        assert_eq!(editable_variable_value("-1 (0xFF)"), "-1");
        assert_eq!(editable_variable_value("3.5"), "3.5");
        assert_eq!(
            single_line_variable_value("uint32_t[3] = [\n  64,\n  1024,\n]"),
            "uint32_t[3] = [ 64, 1024, ]"
        );
    }

    #[test]
    fn compact_expander_keeps_identical_geometry_for_collapsed_and_expanded_states() {
        eframe::egui::__run_test_ui(|ui| {
            let collapsed = compact_expander(ui, false, "expand", "collapse");
            let expanded = compact_expander(ui, true, "expand", "collapse");
            assert_eq!(collapsed.rect.size(), eframe::egui::vec2(16.0, 16.0));
            assert_eq!(expanded.rect.size(), collapsed.rect.size());
        });
    }

    #[test]
    fn source_highlighter_preserves_text_and_styles_common_tokens() {
        eframe::egui::__run_test_ui(|ui| {
            let job =
                source_highlight_job(ui, 42, "if (value >= 10) return \"ok\"; // done", false);
            assert_eq!(job.text, "   42  if (value >= 10) return \"ok\"; // done");
            assert!(job.sections.len() > 6);
            assert!(job.sections.iter().any(|section| section.format.italics));
            let distinct_colors = job
                .sections
                .iter()
                .map(|section| section.format.color)
                .collect::<std::collections::HashSet<_>>();
            assert!(distinct_colors.len() >= 4);
        });
    }

    #[test]
    fn source_navigation_supports_back_forward_and_split_mode() {
        let mut plugin = super::DebugPluginState::default();
        plugin.navigate_to_source("/src/a.cpp".to_owned(), Some(10), true);
        plugin.navigate_to_source("/src/b.cpp".to_owned(), Some(20), true);
        assert!(plugin.can_navigate_back());
        plugin.navigate_back();
        assert_eq!(plugin.selected_source_path.as_deref(), Some("/src/a.cpp"));
        assert_eq!(plugin.source_cursor, Some(("/src/a.cpp".to_owned(), 10)));
        assert!(plugin.can_navigate_forward());
        plugin.code_view = CodeView::Split;
        plugin.navigate_forward();
        assert_eq!(plugin.selected_source_path.as_deref(), Some("/src/b.cpp"));
        assert_eq!(plugin.code_view, CodeView::Split);
    }

    #[test]
    fn source_and_assembly_jumps_preserve_split_mode_and_set_scroll_targets() {
        let path = "/src/main.cpp".to_owned();
        let mut plugin = super::DebugPluginState {
            code_view: CodeView::Split,
            source_cursor: Some((path.clone(), 42)),
            assembly_cursor: Some(0x100),
            snapshot: crate::model::DebugSnapshot {
                executable_lines: vec![crate::model::ExecutableLineView {
                    path: path.clone(),
                    line: 42,
                    address: 0x100,
                }]
                .into(),
                instructions: vec![InstructionView {
                    address: 0x100,
                    bytes: "00 BF".to_owned(),
                    instruction: "nop".to_owned(),
                    source: Some(SourceLocationView {
                        path: path.clone(),
                        line: Some(42),
                        column: None,
                    }),
                }],
                ..Default::default()
            },
            ..Default::default()
        };

        plugin.jump_selected_line_to_assembly();
        assert_eq!(plugin.code_view, CodeView::Split);
        assert_eq!(plugin.assembly_scroll_target, Some(0x100));
        assert!(matches!(
            plugin.pending.last(),
            Some(DebugCommand::Disassemble(BreakpointSpec::Source {
                line: 42,
                ..
            }))
        ));

        plugin.jump_selected_instruction_to_source();
        assert_eq!(plugin.code_view, CodeView::Split);
        assert_eq!(plugin.selected_source_path.as_deref(), Some(path.as_str()));
        assert_eq!(plugin.source_scroll_target, Some(42));
    }

    #[test]
    fn a_new_stop_centers_the_program_counter_in_assembly() {
        let mut plugin = super::DebugPluginState {
            code_view: CodeView::Assembly,
            ..Default::default()
        };
        let pool = VariablePool::default();
        let frame_data = FrameData::default();
        let register_data = crate::model::RegisterData::default();
        let snapshot = crate::model::DebugSnapshot {
            revision: 1,
            stop_id: 1,
            pc: Some(0x0800_1235),
            frames: vec![crate::model::StackFrameView {
                index: 0,
                function: "main".to_owned(),
                pc: 0x0800_1235,
                source: Some(SourceLocationView {
                    path: "/src/main.cpp".to_owned(),
                    line: Some(42),
                    column: None,
                }),
                is_inline: false,
            }],
            ..Default::default()
        };
        let context = eframe::egui::Context::default();
        plugin.update(PluginUpdateContext {
            pool: &pool,
            frame_data: &frame_data,
            register_data: &register_data,
            running: false,
            acquisition_requested: true,
            connected: true,
            hardware_busy: false,
            debug_snapshot: &snapshot,
            egui_ctx: &context,
        });
        assert_eq!(plugin.assembly_cursor, Some(0x0800_1235));
        assert_eq!(plugin.assembly_scroll_target, Some(0x0800_1235));
        assert_eq!(plugin.code_view, CodeView::Assembly);
    }

    #[test]
    fn expanded_source_line_caches_matching_disassembly() {
        let key = ("/src/main.cpp".to_owned(), 42);
        let mut plugin = super::DebugPluginState::default();
        plugin.expanded_source_assembly.insert(key.clone());
        plugin.snapshot.instructions = vec![
            InstructionView {
                address: 0x100,
                bytes: "00 BF".to_owned(),
                instruction: "nop".to_owned(),
                source: Some(SourceLocationView {
                    path: key.0.clone(),
                    line: Some(42),
                    column: None,
                }),
            },
            InstructionView {
                address: 0x102,
                bytes: "00 BF".to_owned(),
                instruction: "nop".to_owned(),
                source: Some(SourceLocationView {
                    path: key.0.clone(),
                    line: Some(43),
                    column: None,
                }),
            },
        ];
        plugin.capture_inline_assembly();
        assert_eq!(plugin.inline_assembly_cache[&key].len(), 1);
        assert_eq!(plugin.inline_assembly_cache[&key][0].address, 0x100);
    }

    #[test]
    fn builds_project_tree_and_maps_the_debug_source_root() {
        let files = vec![
            "/build/fw/src/main.c".to_owned(),
            "/build/fw/src/driver/gpio.c".to_owned(),
        ];
        let (tree, root) = build_source_tree(&files);
        assert_eq!(root, "/build/fw/src");
        let common = tree.children.get("/build/fw/src").unwrap();
        assert!(common.children.contains_key("main.c"));
        assert!(common.children.contains_key("driver"));
        assert_eq!(
            resolve_local_source_path(
                "/build/fw/src/driver/gpio.c",
                "/build/fw/src",
                Some("/workspace/src"),
            ),
            "/workspace/src/driver/gpio.c"
        );
    }

    #[test]
    fn collapses_single_child_directory_chains() {
        let files = vec![
            "/home/liaohy/User/project/src/main.c".to_owned(),
            "/home/liaohy/User/project/src/lib/util.c".to_owned(),
        ];
        let (tree, _) = build_source_tree(&files);
        let node = tree.children.values().next().unwrap();
        let (label, compressed) = compressed_directory(node);
        assert_eq!(label, "/home/liaohy/User/project/src");
        assert!(compressed.children.contains_key("main.c"));
        assert!(compressed.children.contains_key("lib"));
    }

    #[test]
    fn empty_debug_plugin_renders_without_hardware() {
        eframe::egui::__run_test_ui(|ui| {
            let widgets = ui.visuals().widgets.clone();
            let scroll = ui.spacing().scroll;
            let mut plugin = super::DebugPluginState::default();
            let pool = VariablePool::default();
            let actions = plugin.render(
                ui,
                PluginRenderContext {
                    pool: &pool,
                    running: false,
                    viewport_id: ui.ctx().viewport_id(),
                },
            );
            assert!(actions.is_empty());
            assert_eq!(ui.visuals().widgets, widgets);
            assert_eq!(ui.spacing().scroll, scroll);
        });
    }

    #[test]
    fn debug_style_has_no_hover_growth_or_floating_scrollbar_growth() {
        let mut style = eframe::egui::Style::default();
        style.visuals.widgets.hovered.expansion = 4.0;
        style.visuals.widgets.active.expansion = 3.0;
        style.visuals.widgets.open.expansion = 2.0;
        stabilize_debug_style(&mut style);

        assert_eq!(style.visuals.widgets.noninteractive.expansion, 0.0);
        assert_eq!(style.visuals.widgets.inactive.expansion, 0.0);
        assert_eq!(style.visuals.widgets.hovered.expansion, 0.0);
        assert_eq!(style.visuals.widgets.active.expansion, 0.0);
        assert_eq!(style.visuals.widgets.open.expansion, 0.0);
        assert!(!style.spacing.scroll.floating);
    }

    #[test]
    fn disabling_debug_plugin_requests_debug_stop_without_auto_restart() {
        let mut plugin = super::DebugPluginState {
            start_mode: DebugStartMode::Reset,
            ..Default::default()
        };
        let actions = plugin.on_enabled_changed(false);
        assert!(matches!(
            actions.as_slice(),
            [PluginAction::Debug(DebugCommand::Stop)]
        ));
        assert!(plugin.on_enabled_changed(true).is_empty());
        assert_eq!(plugin.start_mode, DebugStartMode::Reset);
    }

    #[test]
    fn stopping_global_acquisition_stops_an_active_debug_engine_once() {
        let mut plugin = super::DebugPluginState::default();
        let pool = VariablePool::default();
        let frame_data = FrameData::default();
        let register_data = crate::model::RegisterData::default();
        let snapshot = crate::model::DebugSnapshot {
            revision: 1,
            active: true,
            ..Default::default()
        };
        let context = eframe::egui::Context::default();
        let actions = plugin.update(PluginUpdateContext {
            pool: &pool,
            frame_data: &frame_data,
            register_data: &register_data,
            running: false,
            acquisition_requested: false,
            connected: true,
            hardware_busy: false,
            debug_snapshot: &snapshot,
            egui_ctx: &context,
        });
        assert!(matches!(
            actions.as_slice(),
            [PluginAction::Debug(DebugCommand::Stop)]
        ));

        let repeated = plugin.update(PluginUpdateContext {
            pool: &pool,
            frame_data: &frame_data,
            register_data: &register_data,
            running: false,
            acquisition_requested: false,
            connected: true,
            hardware_busy: false,
            debug_snapshot: &snapshot,
            egui_ctx: &context,
        });
        assert!(repeated.is_empty());
    }

    #[test]
    fn legacy_debug_config_defaults_to_attach_start_mode() {
        let mut plugin = super::DebugPluginState {
            start_mode: DebugStartMode::Reset,
            ..Default::default()
        };
        let mut pool = VariablePool::default();
        plugin
            .load_config(&serde_json::json!({ "breakpoints": [] }), &mut pool)
            .unwrap();
        assert_eq!(plugin.start_mode, DebugStartMode::Attach);
    }

    #[test]
    fn toolbar_height_is_stable_across_status_pending_and_error_changes() {
        eframe::egui::__run_test_ui(|ui| {
            ui.set_width(760.0);
            let mut plugin = super::DebugPluginState::default();
            let idle_height = ui
                .scope(|ui| render_toolbar(ui, &mut plugin))
                .response
                .rect
                .height();

            plugin.connected = true;
            plugin.command_pending = true;
            plugin.snapshot.active = true;
            plugin.snapshot.target_state = crate::model::TargetState::Halted {
                reason: "Breakpoint(Hardware)".to_owned(),
            };
            plugin.snapshot.pc = Some(0x0800_1234);
            plugin.snapshot.last_error =
                Some("一条很长的错误消息不应改变工具栏高度或把按钮挤到下一行".to_owned());
            let busy_height = ui
                .scope(|ui| render_toolbar(ui, &mut plugin))
                .response
                .rect
                .height();

            assert_eq!(idle_height, busy_height);
        });
    }

    #[test]
    fn status_labels_the_actual_step_execution_method() {
        assert_eq!(step_method_label(None), "步进: —");
        assert_eq!(
            step_method_label(Some(crate::model::StepExecutionMethod::SingleStep)),
            "步进: SingleStep"
        );
        assert_eq!(
            step_method_label(Some(crate::model::StepExecutionMethod::HardwareBreakpoint)),
            "步进: 硬件断点加速"
        );
    }

    #[test]
    fn virtual_source_rows_keep_inline_assembly_at_the_correct_lines() {
        let expanded = vec![(1, 2), (4, 1)];
        assert_eq!(source_visual_row_count(6, &expanded), 9);
        assert_eq!(source_visual_row_for_line(0, &expanded), 0);
        assert_eq!(source_visual_row_for_line(1, &expanded), 1);
        assert_eq!(source_visual_row_for_line(2, &expanded), 4);
        assert_eq!(source_visual_row_for_line(5, &expanded), 8);
        assert_eq!(
            source_visual_row_at(0, 6, &expanded),
            Some(SourceVisualRow::Source(0))
        );
        assert_eq!(
            source_visual_row_at(1, 6, &expanded),
            Some(SourceVisualRow::Source(1))
        );
        assert_eq!(
            source_visual_row_at(2, 6, &expanded),
            Some(SourceVisualRow::Inline {
                line_index: 1,
                instruction_index: 0
            })
        );
        assert_eq!(
            source_visual_row_at(3, 6, &expanded),
            Some(SourceVisualRow::Inline {
                line_index: 1,
                instruction_index: 1
            })
        );
        assert_eq!(
            source_visual_row_at(8, 6, &expanded),
            Some(SourceVisualRow::Source(5))
        );
        assert_eq!(source_visual_row_at(9, 6, &expanded), None);
    }

    #[test]
    fn split_code_panes_follow_and_clamp_the_saved_drag_ratio() {
        let available = eframe::egui::Rect::from_min_max(
            eframe::egui::pos2(10.0, 20.0),
            eframe::egui::pos2(810.0, 520.0),
        );
        let (source, splitter, assembly, ratio) = split_code_rects(available, 0.7);
        assert!((ratio - 0.7).abs() < f32::EPSILON);
        assert!((splitter.width() - CODE_SPLITTER_SIZE).abs() < f32::EPSILON);
        assert_eq!(source.right(), splitter.left());
        assert_eq!(splitter.right(), assembly.left());
        assert_eq!(source.left(), available.left());
        assert_eq!(assembly.right(), available.right());

        let (source, _, assembly, ratio) = split_code_rects(available, 1.0);
        assert!(ratio < 1.0);
        assert!(source.width() >= CODE_PANE_MIN_WIDTH);
        assert!(assembly.width() >= CODE_PANE_MIN_WIDTH);
    }

    #[test]
    fn large_source_view_only_renders_visible_rows() {
        eframe::egui::__run_test_ui(|ui| {
            ui.set_width(900.0);
            ui.set_height(520.0);
            let mut plugin = super::DebugPluginState {
                selected_source_path: Some("/virtual/large.c".to_owned()),
                ..Default::default()
            };
            let lines = (0..100_000)
                .map(|line| format!("int value_{line} = {line};"))
                .collect::<Vec<_>>()
                .into();
            plugin
                .source_cache
                .insert("/virtual/large.c".to_owned(), Ok(lines));

            SOURCE_ROWS_RENDERED.with(|count| count.set(0));
            SOURCE_ROW_WIDTHS.with(|widths| widths.borrow_mut().clear());
            render_source_code(ui, &mut plugin);
            let rendered = SOURCE_ROWS_RENDERED.with(std::cell::Cell::get);
            assert!(rendered > 0);
            assert!(
                rendered < 100,
                "rendered {rendered} rows for a 100k-line file"
            );
            let widths = SOURCE_ROW_WIDTHS.with(|widths| widths.borrow().clone());
            assert!(!widths.is_empty());
            assert!(
                widths
                    .iter()
                    .all(|width| (*width - widths[0]).abs() <= f32::EPSILON),
                "source rows changed width while scrolling: {widths:?}"
            );
        });
    }

    #[test]
    fn run_to_cursor_uses_the_active_code_view() {
        let mut plugin = super::DebugPluginState {
            source_cursor: Some(("/src/main.c".to_owned(), 42)),
            assembly_cursor: Some(0x0800_0100),
            code_view: CodeView::Source,
            ..Default::default()
        };
        assert_eq!(
            current_cursor_spec(&plugin),
            Some(BreakpointSpec::Source {
                path: "/src/main.c".to_owned(),
                line: 42,
                column: None,
            })
        );

        plugin.code_view = CodeView::Assembly;
        assert_eq!(
            current_cursor_spec(&plugin),
            Some(BreakpointSpec::Instruction {
                address: 0x0800_0100,
            })
        );

        plugin.code_view = CodeView::Split;
        assert_eq!(
            current_cursor_spec(&plugin),
            Some(BreakpointSpec::Source {
                path: "/src/main.c".to_owned(),
                line: 42,
                column: None,
            })
        );
    }

    #[test]
    fn project_filter_matches_names_and_full_paths() {
        let files = vec![
            "/build/fw/src/main.c".to_owned(),
            "/build/fw/drivers/gpio.c".to_owned(),
        ];
        let (tree, _) = build_source_tree(&files);
        assert!(source_node_matches(&tree, "Sources", "gpio"));
        assert!(source_node_matches(&tree, "Sources", "drivers"));
        assert!(!source_node_matches(&tree, "Sources", "uart"));
    }

    #[test]
    fn focusing_a_breakpoint_opens_its_resolved_source() {
        let mut plugin = super::DebugPluginState::default();
        plugin.focus_breakpoint(&BreakpointView {
            id: 7,
            spec: BreakpointSpec::Source {
                path: "/build/fw/src/main.c".to_owned(),
                line: 19,
                column: None,
            },
            address: Some(0x0800_0120),
            enabled: true,
            verified: true,
            message: None,
            resolved_source: Some(SourceLocationView {
                path: "/build/fw/src/main.c".to_owned(),
                line: Some(21),
                column: None,
            }),
        });

        assert_eq!(plugin.code_view, CodeView::Source);
        assert_eq!(
            plugin.source_cursor,
            Some(("/build/fw/src/main.c".to_owned(), 21))
        );
        assert_eq!(plugin.source_scroll_target, Some(21));
        assert_eq!(plugin.assembly_cursor, Some(0x0800_0120));
        assert!(
            !plugin
                .pending
                .iter()
                .any(|command| matches!(command, DebugCommand::Disassemble(_)))
        );
    }
}
