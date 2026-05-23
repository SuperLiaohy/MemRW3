use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use probe_rs::{MemoryInterface, Session};
use probe_rs::probe::list::Lister;

use crate::model::DoubleBuffer;

/// A single 32-bit aligned probe read slot.
/// Deduplicated: multiple variables may share the same address.
pub struct AcqSlot {
    pub address: u64,
}

/// Maps one PooledVariable to its set of AcqSlots.
pub struct VarSlotMapping {
    pub slots: Vec<Arc<AcqSlot>>,
    pub size: u32,
    /// Byte offset of the variable's address within the first 32-bit slot.
    pub byte_offset: usize,
    pub incoming: Arc<DoubleBuffer<(f64, [u8; 8])>>,
}

pub struct ProbeSession {
    /// Declared before `session` — dropped first (while session alive).
    cached_core: Option<probe_rs::Core<'static>>,
    session: Option<Session>,
    pub connected: bool,
    pub chip_name: String,
    pub available_chips: Vec<String>,
    pub protocol: String,
    pub speed_khz: u32,
    pub last_error: Option<String>,
    /// Deduplicated 32-bit aligned read slots.
    pub slots: Vec<Arc<AcqSlot>>,
    /// Per-variable mapping: slots → DoubleBuffer.
    pub var_mappings: Vec<VarSlotMapping>,
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
            var_mappings: Vec::new(),
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

    /// Calculate the set of 32-bit aligned addresses covering [address, address+size).
    pub fn slot_addresses(address: u64, size: u32) -> Vec<u64> {
        let end = address.saturating_add(size as u64);
        let start = address & !3;
        let mut addrs = Vec::new();
        let mut a = start;
        while a < end {
            addrs.push(a);
            a = a.wrapping_add(4);
        }
        addrs
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

    /// Two-phase acquisition:
    /// 1. Read all 32-bit slots → slot_values
    /// 2. Assemble per-variable values from slots → push to DoubleBuffer
    pub fn acquire_from_slots(&mut self) {
        if !self.connected || self.slots.is_empty() {
            return;
        }
        if !self.ensure_core() {
            return;
        }
        let ts = self.timer.elapsed().as_secs_f64();
        let core = unsafe {
            &mut *(self.cached_core.as_mut().unwrap() as *mut probe_rs::Core<'static>)
        };

        let mut slot_values: HashMap<u64, [u8; 4]> =
            HashMap::with_capacity(self.slots.len());
        for slot in &self.slots {
            match core.read_word_32(slot.address) {
                Ok(v) => {
                    slot_values.insert(slot.address, v.to_le_bytes());
                }
                Err(e) => {
                    self.last_error = Some(format!(
                        "读取 {:#010x} 失败: {e}",
                        slot.address
                    ));
                    self.cached_core = None;
                    return;
                }
            }
        }

        for mapping in &self.var_mappings {
            let mut val = [0u8; 8];
            let mut pos: usize = 0;
            for (i, slot) in mapping.slots.iter().enumerate() {
                let sv = match slot_values.get(&slot.address) {
                    Some(v) => v,
                    None => continue,
                };
                if i == 0 {
                    let start = mapping.byte_offset;
                    let copy_len = (4 - start).min(mapping.size as usize - pos);
                    val[pos..pos + copy_len]
                        .copy_from_slice(&sv[start..start + copy_len]);
                    pos += copy_len;
                } else {
                    let copy_len = 4.min(mapping.size as usize - pos);
                    val[pos..pos + copy_len].copy_from_slice(&sv[..copy_len]);
                    pos += copy_len;
                }
                if pos >= mapping.size as usize {
                    break;
                }
            }
            mapping.incoming.push((ts, val));
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
