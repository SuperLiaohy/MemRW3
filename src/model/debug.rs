use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TargetState {
    Disconnected,
    #[default]
    Unknown,
    Running,
    Sleeping,
    Halted {
        reason: String,
    },
    LockedUp,
}

impl TargetState {
    pub fn is_halted(&self) -> bool {
        matches!(self, Self::Halted { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DebugStartMode {
    #[default]
    Attach,
    Reset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepKind {
    Instruction,
    Into,
    Over,
    Out,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakpointSpec {
    Source {
        path: String,
        line: u64,
        column: Option<u64>,
    },
    Instruction {
        address: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalBreakpoint {
    pub id: u64,
    pub spec: BreakpointSpec,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakpointView {
    pub id: u64,
    pub spec: BreakpointSpec,
    pub address: Option<u64>,
    pub enabled: bool,
    pub verified: bool,
    pub message: Option<String>,
    pub resolved_source: Option<SourceLocationView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugCommand {
    Start(DebugStartMode),
    Stop,
    Halt,
    Continue,
    RunTo(BreakpointSpec),
    Disassemble(BreakpointSpec),
    Step(StepKind),
    SetBreakpoint(LogicalBreakpoint),
    RemoveBreakpoint(u64),
    SetBreakpointEnabled {
        id: u64,
        enabled: bool,
    },
    ReplaceBreakpoints(Vec<LogicalBreakpoint>),
    SelectFrame(usize),
    ExpandVariable {
        stop_id: u64,
        frame_index: usize,
        variable_ref: i64,
    },
    WriteVariable {
        stop_id: u64,
        frame_index: usize,
        variable_ref: i64,
        value: String,
    },
    Refresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterView {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocationView {
    pub path: String,
    pub line: Option<u64>,
    pub column: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableLineView {
    pub path: String,
    pub line: u64,
    pub address: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackFrameView {
    pub index: usize,
    pub function: String,
    pub pc: u64,
    pub source: Option<SourceLocationView>,
    pub is_inline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableView {
    pub reference: i64,
    pub parent_reference: i64,
    pub name: String,
    pub type_name: String,
    pub value: String,
    pub has_children: bool,
    pub writable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstructionView {
    pub address: u64,
    pub bytes: String,
    pub instruction: String,
    pub source: Option<SourceLocationView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackMemoryWord {
    pub address: u64,
    pub value: u32,
}

#[derive(Debug, Clone, Default)]
pub struct DebugSnapshot {
    pub revision: u64,
    pub program_generation: u64,
    pub stop_id: u64,
    pub target_state: TargetState,
    pub active: bool,
    pub pc: Option<u64>,
    pub registers: Vec<RegisterView>,
    pub frames: Vec<StackFrameView>,
    pub selected_frame: usize,
    pub variables: Vec<VariableView>,
    pub instructions: Vec<InstructionView>,
    pub stack_memory: Vec<StackMemoryWord>,
    pub breakpoints: Vec<BreakpointView>,
    pub last_error: Option<String>,
    pub warnings: Vec<String>,
    pub breakpoint_capacity: Option<u32>,
    pub program_path: Option<String>,
    pub source_files: Vec<String>,
    pub executable_lines: std::sync::Arc<[ExecutableLineView]>,
}
