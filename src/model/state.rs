use super::VariablePool;
use crate::dwarf::types::ExtendConfig;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

pub struct AppSession {
    pub connected: bool,
    pub running: Arc<AtomicBool>,
    pub sampling_hz: f64,
    pub acq_cycle_count: Arc<AtomicU64>,
    pub slot_count: Arc<AtomicU64>,
    pub hz_last_cycles: u64,
    pub hz_last_time: Instant,
    pub acq_stop: Arc<AtomicBool>,
    pub active_bottom_sheet: Option<String>,
    pub bottom_sheet_drag: Option<(f32, f32)>,
    pub load_error: Option<String>,
    pub connect_error: Option<String>,
    pub extend_configs: HashMap<usize, ExtendConfig>,
    pub all_chips: Vec<String>,
    pub probe_id: Option<String>,
    pub cached_probe_list: Option<Vec<String>>,
    pub show_probe_settings: bool,
    pub edit_chip: String,
    pub edit_protocol: String,
    pub edit_speed: u32,
    pub edit_id: Option<String>,
    pub timer_was_started: bool,
    pub config: Config,
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
            acq_cycle_count: Arc::new(AtomicU64::new(0)),
            slot_count: Arc::new(AtomicU64::new(0)),
            hz_last_cycles: 0,
            hz_last_time: Instant::now(),
            acq_stop: Arc::new(AtomicBool::new(false)),
            active_bottom_sheet: None,
            bottom_sheet_drag: None,
            load_error: None,
            connect_error: None,
            extend_configs: HashMap::new(),
            all_chips: Vec::new(),
            probe_id: None,
            cached_probe_list: None,
            show_probe_settings: false,
            edit_chip: String::new(),
            edit_protocol: String::new(),
            edit_speed: 10000,
            edit_id: None,
            timer_was_started: false,
            config: Config::default(),
        }
    }
}

pub struct Config {
    pub elf_path: String,
    pub delay_us: Arc<AtomicU64>,
    pub bottom_sheet_height: f32,
    pub pool: VariablePool,
    pub probe_chip: String,
    pub probe_protocol: String,
    pub probe_speed_khz: u32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            delay_us: Arc::new(AtomicU64::new(0)),
            elf_path: String::new(),
            bottom_sheet_height: 250.0,
            pool: VariablePool::default(),
            probe_chip: "STM32F407VG".into(),
            probe_protocol: "SWD".into(),
            probe_speed_khz: 10000,
        }
    }
}
