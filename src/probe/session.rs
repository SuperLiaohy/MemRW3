use std::time::{Duration, Instant};
use probe_rs::{MemoryInterface, Session};
use probe_rs::probe::list::Lister;

use crate::model::VariablePool;

pub struct ProbeSession {
    session: Option<Session>,
    pub connected: bool,
    pub running: bool,
    pub chip_name: String,
    pub available_chips: Vec<String>,
    pub protocol: String,
    pub speed_khz: u32,
    pub show_settings: bool,
    last_read: Instant,
    pub last_error: Option<String>,
}

impl Default for ProbeSession {
    fn default() -> Self {
        Self {
            session: None,
            connected: false,
            running: false,
            chip_name: "STM32F407VG".into(),
            available_chips: vec![
                "STM32F407VG".into(), "STM32F429ZI".into(), "STM32H743ZI".into(),
                "nRF52840_xxAA".into(), "STM32F103C8".into(), "RP2040".into(),
                "STM32G474RE".into(), "STM32L476RG".into(), "ATSAMD51P19A".into(),
            ],
            protocol: "SWD".into(),
            speed_khz: 4000,
            show_settings: false,
            last_read: Instant::now(),
            last_error: None,
        }
    }
}

impl ProbeSession {
    pub fn connect(&mut self) -> bool {
        self.last_error = None;
        let protocol = match self.protocol.as_str() {
            "SWD" => Some(probe_rs::probe::WireProtocol::Swd),
            "JTAG" => Some(probe_rs::probe::WireProtocol::Jtag),
            _ => None,
        };
        let config = probe_rs::SessionConfig {
            speed: Some(self.speed_khz),
            protocol,
            ..Default::default()
        };
        match Session::auto_attach(&self.chip_name, config) {
            Ok(session) => { self.session = Some(session); self.connected = true; true }
            Err(e) => { self.last_error = Some(format!("连接失败: {e}")); false }
        }
    }

    pub fn disconnect(&mut self) {
        self.session = None;
        self.connected = false;
        self.running = false;
    }

    pub fn reset_target(&mut self) -> bool {
        self.last_error = None;
        if let Some(ref mut session) = self.session {
            match session.core(0).and_then(|mut core| core.reset()) {
                Ok(_) => true,
                Err(e) => { self.last_error = Some(format!("复位失败: {e}")); false }
            }
        } else { false }
    }

    pub fn list_probes(&mut self) -> Vec<String> {
        Lister::new().list_all().iter().map(|p| p.identifier.clone()).collect()
    }

    pub fn acquire(&mut self, pool: &mut VariablePool, delay_us: f64) {
        if !self.connected || !self.running { return; }
        let delay = Duration::from_micros(delay_us as u64);
        if self.last_read.elapsed() < delay { return; }
        self.last_read = Instant::now();

        if let Some(ref mut session) = self.session {
            let mut core = match session.core(0) {
                Ok(c) => c,
                Err(e) => { self.last_error = Some(format!("获取核心失败: {e}")); return; }
            };
            for var in pool.iter_mut() {
                let addr = Self::parse_addr(&var.tree_node.address_info);
                if addr == 0 { continue; }
                match core.read_word_32(addr) {
                    Ok(val) => { var.current_value = val.to_le_bytes().to_vec(); }
                    Err(e) => { self.last_error = Some(format!("读取 {addr:#010x} 失败: {e}")); }
                }
            }
        }
    }

    pub fn write_u32(&mut self, addr: u64, value: u32) -> bool {
        if let Some(ref mut session) = self.session {
            if let Ok(mut core) = session.core(0) {
                return core.write_word_32(addr, value).is_ok();
            }
        }
        false
    }

    fn parse_addr(s: &str) -> u64 {
        let s = s.trim().trim_start_matches('@').trim();
        if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            u64::from_str_radix(hex, 16).unwrap_or(0)
        } else { s.parse().unwrap_or(0) }
    }
}
