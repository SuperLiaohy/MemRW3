use std::collections::{HashMap, HashSet};
use super::VariablePool;
use crate::types::ExtendConfig;

#[derive(Default)]
pub struct AppSession {
    pub connected: bool,
    pub running: bool,
    pub delay_us: f64,
    pub sampling_hz: f64,
    pub active_bottom_sheet: Option<DockTab>,
    pub bottom_sheet_height: f32,
    /// 拖拽状态：(初始指针Y坐标, 初始高度)，None 表示未拖拽
    pub bottom_sheet_drag: Option<(f32, f32)>,
    pub pool: VariablePool,
    pub selected_variables: HashSet<usize>,
    pub load_error: Option<String>,
    pub extend_configs: HashMap<usize, ExtendConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DockTab {
    #[default]
    Chart,
    Table,
}
