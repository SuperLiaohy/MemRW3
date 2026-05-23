use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use super::VariablePool;
use crate::types::ExtendConfig;

pub struct AppSession {
    pub connected: bool,
    pub running: Arc<AtomicBool>,
    pub sampling_hz: f64,
    pub active_bottom_sheet: Option<DockTab>,
    pub bottom_sheet_height: f32,
    pub bottom_sheet_drag: Option<(f32, f32)>,
    pub pool: VariablePool,
    pub selected_variables: HashSet<usize>,
    pub load_error: Option<String>,
    pub connect_error: Option<String>,
    pub extend_configs: HashMap<usize, ExtendConfig>,
    pub probe_chip: String,
    pub probe_chips: Vec<String>,
    pub probe_protocol: String,
    pub probe_speed_khz: u32,
    pub show_probe_settings: bool,
    pub timer_was_started: bool,
}

impl AppSession {
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub fn set_running(&self, r: bool) {
        self.running.store(r, Ordering::Release);
    }
}

impl Default for AppSession {
    fn default() -> Self {
        Self {
            connected: false,
            running: Arc::new(AtomicBool::new(false)),
            sampling_hz: 0.0,
            active_bottom_sheet: None,
            bottom_sheet_height: 250.0,
            bottom_sheet_drag: None,
            pool: VariablePool::default(),
            selected_variables: HashSet::new(),
            load_error: None,
            connect_error: None,
            extend_configs: HashMap::new(),
            probe_chip: "STM32F407VG".into(),
            probe_chips: vec![
                "STM32F407VG".into(), "STM32F429ZI".into(), "STM32H723VGTX".into(),
                "nRF52840_xxAA".into(), "STM32F103C8".into(), "RP2040".into(),
                "STM32G474RE".into(), "STM32L476RG".into(), "ATSAMD51P19A".into(),
            ],
            probe_protocol: "SWD".into(),
            probe_speed_khz: 4000,
            show_probe_settings: false,
            timer_was_started: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DockTab {
    #[default]
    Chart,
    Table,
}
