use std::sync::Arc;
use std::time::Instant;
use probe_rs::{MemoryInterface, Session};
use probe_rs::probe::list::Lister;

use crate::model::DoubleBuffer;

pub struct AcqSlot {
    pub address: u64,
    pub size: u32,
    pub incoming: Arc<DoubleBuffer<(f64, [u8; 8])>>,
}

pub struct ProbeSession {
    /// Declared before `session` so it's dropped first.
    /// Cached core obtained via `session.core(0)`, reused across acquisitions.
    /// Invalidated on read errors, re-obtained on next acquire.
    cached_core: Option<probe_rs::Core<'static>>,
    session: Option<Session>,
    pub connected: bool,
    pub chip_name: String,
    pub available_chips: Vec<String>,
    pub protocol: String,
    pub speed_khz: u32,
    pub last_error: Option<String>,
    pub slots: Vec<AcqSlot>,
    pub timer: Instant,
}

impl Default for ProbeSession {
    fn default() -> Self {
        Self {
            cached_core: None,
            session: None,
            connected: false,
            chip_name: "STM32F407VG".into(),
            available_chips: vec![
                "STM32F407VG".into(), "STM32F429ZI".into(), "STM32H743ZI".into(),
                "nRF52840_xxAA".into(), "STM32F103C8".into(), "RP2040".into(),
                "STM32G474RE".into(), "STM32L476RG".into(), "ATSAMD51P19A".into(),
            ],
            protocol: "SWD".into(),
            speed_khz: 4000,
            last_error: None,
            slots: Vec::new(),
            timer: Instant::now(),
        }
    }
}

impl ProbeSession {
    pub fn connect(&mut self) -> bool {
        self.cached_core = None;
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
            Ok(session) => {
                self.session = Some(session);
                self.connected = true;
                self.timer = Instant::now();
                true
            }
            Err(e) => {
                self.last_error = Some(format!("连接失败: {e}"));
                false
            }
        }
    }

    pub fn disconnect(&mut self) {
        self.cached_core = None;
        self.session = None;
        self.connected = false;
    }

    pub fn reset_target(&mut self) -> bool {
        self.cached_core = None;
        self.last_error = None;
        if let Some(ref mut session) = self.session {
            match session.core(0).and_then(|mut core| core.reset()) {
                Ok(_) => true,
                Err(e) => {
                    self.last_error = Some(format!("复位失败: {e}"));
                    false
                }
            }
        } else {
            false
        }
    }

    pub fn list_probes(&mut self) -> Vec<String> {
        Lister::new()
            .list_all()
            .iter()
            .map(|p| p.identifier.clone())
            .collect()
    }

    fn ensure_core(&mut self) -> bool {
        if self.cached_core.is_some() {
            return true;
        }
        let session = match self.session.as_mut() {
            Some(s) => s,
            None => return false,
        };
        match session.core(0) {
            Ok(core) => {
                self.cached_core = Some(unsafe {
                    // SAFETY: core borrows from self.session, both belong to self.
                    // cached_core is declared before session, so it's dropped first.
                    std::mem::transmute::<probe_rs::Core<'_>, probe_rs::Core<'static>>(core)
                });
                true
            }
            Err(e) => {
                self.last_error = Some(format!("获取核心失败: {e}"));
                false
            }
        }
    }

    pub fn acquire_from_slots(&mut self) {
        if !self.connected {
            return;
        }
        if !self.ensure_core() {
            return;
        }
        let ts = self.timer.elapsed().as_secs_f64();
        let core = unsafe { &mut *(self.cached_core.as_mut().unwrap() as *mut probe_rs::Core<'static>) };
        for slot in &self.slots {
            let addr = slot.address;
            if addr == 0 {
                continue;
            }
            let mut val = [0u8; 8];
            match slot.size {
                1 => match core.read_word_8(addr) {
                    Ok(v) => val[..1].copy_from_slice(&v.to_le_bytes()),
                    Err(e) => {
                        self.last_error = Some(format!("读取 {addr:#010x} 失败: {e}"));
                        self.cached_core = None;
                        continue;
                    }
                },
                2 => match core.read_word_16(addr) {
                    Ok(v) => val[..2].copy_from_slice(&v.to_le_bytes()),
                    Err(e) => {
                        self.last_error = Some(format!("读取 {addr:#010x} 失败: {e}"));
                        self.cached_core = None;
                        continue;
                    }
                },
                4 => match core.read_word_32(addr) {
                    Ok(v) => val[..4].copy_from_slice(&v.to_le_bytes()),
                    Err(e) => {
                        self.last_error = Some(format!("读取 {addr:#010x} 失败: {e}"));
                        self.cached_core = None;
                        continue;
                    }
                },
                8 => match core.read_word_64(addr) {
                    Ok(v) => val.copy_from_slice(&v.to_le_bytes()),
                    Err(e) => {
                        self.last_error = Some(format!("读取 {addr:#010x} 失败: {e}"));
                        self.cached_core = None;
                        continue;
                    }
                },
                n => {
                    let mut buf = vec![0u8; n as usize];
                    if let Err(e) = core.read(addr, &mut buf) {
                        self.last_error = Some(format!("读取 {addr:#010x} 失败: {e}"));
                        self.cached_core = None;
                        continue;
                    }
                    let len = buf.len().min(8);
                    val[..len].copy_from_slice(&buf[..len]);
                }
            }
            slot.incoming.push((ts, val));
        }
    }

    pub fn write_u32(&mut self, addr: u64, value: u32) -> bool {
        self.cached_core = None;
        if let Some(ref mut session) = self.session {
            if let Ok(mut core) = session.core(0) {
                return core.write_word_32(addr, value).is_ok();
            }
        }
        false
    }
}
