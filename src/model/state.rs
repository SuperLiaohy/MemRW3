use std::collections::HashSet;
use super::VariablePool;

#[derive(Default)]
pub struct AppSession {
    pub connected: bool,
    pub running: bool,
    pub delay_us: f64,
    pub sampling_hz: f64,
    pub active_bottom_sheet: Option<DockTab>,
    pub bottom_sheet_height: f32,
    pub pool: VariablePool,
    pub selected_variables: HashSet<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DockTab {
    #[default]
    Chart,
    Table,
}
