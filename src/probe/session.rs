use probe_rs::{MemoryInterface, Session};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use crate::model::{RegisterReadRequest, RingBuffer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FirmwareImageKind {
    Elf,
    Hex,
    Bin,
    Uf2,
}

impl FirmwareImageKind {
    fn from_path(path: &Path) -> Result<Self, String> {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| "无法识别固件格式：文件没有扩展名".to_owned())?;

        match extension.as_str() {
            "elf" | "axf" | "out" => Ok(Self::Elf),
            "hex" | "ihex" => Ok(Self::Hex),
            "bin" => Ok(Self::Bin),
            "uf2" => Ok(Self::Uf2),
            _ => Err(format!("不支持的固件格式: .{extension}")),
        }
    }
}

/// A single 32-bit aligned probe read slot.
/// Deduplicated: multiple variables may share the same address.
pub struct AcqSlot {
    pub address: u64,
}

/// Maps one PooledVariable to its set of AcqSlots.
pub struct VarSlotMapping {
    pub slot_indices: Vec<usize>,
    pub size: u32,
    /// Byte offset of the variable's address within the first 32-bit slot.
    pub byte_offset: usize,
    pub incoming: Arc<RingBuffer<(f64, [u8; 8])>>,
}

pub struct ProbeSession {
    session: Option<Session>,
    pub connected: bool,
    pub chip_name: String,
    pub protocol: String,
    pub speed_khz: u32,
    pub selected_probe_id: Option<String>,
    pub last_error: Option<String>,
    /// Deduplicated 32-bit aligned read slots.
    pub slots: Vec<AcqSlot>,
    /// Reused on every acquisition cycle; indexed by `VarSlotMapping::slot_indices`.
    pub slot_values: Vec<[u8; 4]>,
    /// Per-variable mapping: slots → lock-free ring buffer.
    pub var_mappings: Vec<VarSlotMapping>,
    pub timer: Instant,
}

impl Default for ProbeSession {
    fn default() -> Self {
        Self {
            session: None,
            connected: false,
            chip_name: "STM32F407VG".into(),
            protocol: "SWD".into(),
            speed_khz: 10000,
            selected_probe_id: None,
            last_error: None,
            slots: Vec::new(),
            slot_values: Vec::new(),
            var_mappings: Vec::new(),
            timer: Instant::now(),
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

        // 1. 枚举当前所有连入的调试器
        let lister = probe_rs::probe::list::Lister::new();
        let probes = lister.list_all();

        if probes.is_empty() {
            self.last_error = Some("连接失败: 未检测到任何调试器".to_string());
            return false;
        }

        let target_probe_info = if let Some(target_id) = &self.selected_probe_id {
            match probes.into_iter().find(|p| {
                format!(
                    "{},SN:{}",
                    p.identifier,
                    p.serial_number.as_deref().unwrap_or("N/A")
                ) == *target_id
            }) {
                Some(probe) => Some(probe),
                None => {
                    self.last_error =
                        Some("连接失败: 找不到指定的调试器（可能已被拔出）".to_string());
                    return false;
                }
            }
        } else {
            None
        };

        if let Some(probe_info) = target_probe_info {
            match probe_info.open() {
                Ok(mut probe) => {
                    // 4. 先在 probe 层级设置协议和速率，然后再 Attach
                    if let Some(proto) = protocol {
                        if let Err(e) = probe.select_protocol(proto) {
                            self.last_error = Some(format!("协议设置失败: {e}"));
                            return false;
                        }
                    }

                    if let Err(e) = probe.set_speed(self.speed_khz) {
                        self.last_error = Some(format!("速率设置失败: {e}"));
                        return false;
                    }

                    // 5. 将配置好的 Probe Attach 到指定芯片
                    match probe.attach(self.chip_name.clone(), Default::default()) {
                        Ok(session) => {
                            self.session = Some(session);
                            self.connected = true;
                            true
                        }
                        Err(e) => {
                            self.last_error = Some(format!("连接芯片失败: {e}"));
                            false
                        }
                    }
                }
                Err(e) => {
                    self.last_error = Some(format!("打开调试器失败: {e}, 请多次尝试或检查连接"));
                    false
                }
            }
        } else {
            let config = probe_rs::SessionConfig {
                speed: Some(self.speed_khz),
                protocol,
                ..Default::default()
            };
            match Session::auto_attach(&self.chip_name, config) {
                Ok(session) => {
                    self.session = Some(session);
                    self.connected = true;
                    true
                }
                Err(e) => {
                    self.last_error = Some(format!("连接失败: {e}, 请多次尝试或检查连接"));
                    false
                }
            }
        }
    }

    pub fn disconnect(&mut self) {
        self.session = None;
        self.connected = false;
    }

    pub fn reset_target(&mut self) -> bool {
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

    pub fn flash_firmware(&mut self, path: &Path) -> Result<(), String> {
        if !self.connected {
            return Err("请先连接目标设备".to_owned());
        }

        let image_kind = FirmwareImageKind::from_path(path)?;
        self.last_error = None;

        let result: Result<(), String> = (|| {
            let session = self
                .session
                .as_mut()
                .ok_or_else(|| "Probe 会话不可用，请重新连接".to_owned())?;
            let format = match image_kind {
                FirmwareImageKind::Elf => probe_rs::flashing::Format::Elf(Default::default()),
                FirmwareImageKind::Hex => probe_rs::flashing::Format::Hex,
                FirmwareImageKind::Uf2 => probe_rs::flashing::Format::Uf2,
                FirmwareImageKind::Bin => {
                    let base_address = session
                        .target()
                        .memory_map
                        .iter()
                        .filter_map(|region| region.as_nvm_region())
                        .find(|region| region.is_boot_memory() && !region.is_alias)
                        .or_else(|| {
                            session
                                .target()
                                .memory_map
                                .iter()
                                .filter_map(|region| region.as_nvm_region())
                                .find(|region| !region.is_alias)
                        })
                        .map(|region| region.range.start)
                        .ok_or_else(|| {
                            "目标芯片没有可用的 NVM 区域，无法确定 BIN 基址".to_owned()
                        })?;
                    probe_rs::flashing::Format::Bin(probe_rs::flashing::BinOptions {
                        base_address: Some(base_address),
                        skip: 0,
                    })
                }
            };

            let mut options = probe_rs::flashing::DownloadOptions::default();
            options.verify = true;
            probe_rs::flashing::download_file_with_options(session, path, format, options)
                .map_err(|error| format!("烧录失败: {error}"))?;

            session
                .core(0)
                .and_then(|mut core| core.reset())
                .map_err(|error| format!("固件已写入，但复位目标失败: {error}"))?;
            Ok(())
        })();

        if let Err(error) = &result {
            self.last_error = Some(error.clone());
        }
        result
    }

    /// Calculate the set of 32-bit aligned addresses covering [address, address+size).
    /// Size is capped at 8 (max ProbeSession::val capacity).
    pub fn slot_addresses(address: u64, size: u32) -> Vec<u64> {
        let end = address.saturating_add((size as u64).min(8));
        let start = address & !3;
        let mut addrs = Vec::new();
        let mut a = start;
        while a < end {
            addrs.push(a);
            a = a.wrapping_add(4);
        }
        addrs
    }

    /// Two-phase acquisition:
    /// 1. Read all 32-bit slots into the reusable `slot_values` array
    /// 2. Assemble per-variable values from slots → push to the ring buffer
    pub fn acquire_from_slots(&mut self) -> Result<(), String> {
        if !self.connected || self.slots.is_empty() {
            return Ok(());
        }
        let ts = self.timer.elapsed().as_secs_f64();
        let Some(session) = self.session.as_mut() else {
            return Err("Probe 会话不可用，请重新连接".to_owned());
        };
        let mut core = match session.core(0) {
            Ok(core) => core,
            Err(error) => {
                let message = format!("获取核心失败: {error}");
                self.last_error = Some(message.clone());
                return Err(message);
            }
        };

        self.slot_values.resize(self.slots.len(), [0; 4]);
        for (index, slot) in self.slots.iter().enumerate() {
            match core.read_word_32(slot.address) {
                Ok(v) => {
                    self.slot_values[index] = v.to_le_bytes();
                }
                Err(e) => {
                    let message = format!("读取 {:#010x} 失败: {e}", slot.address);
                    self.last_error = Some(message.clone());
                    return Err(message);
                }
            }
        }

        for mapping in &self.var_mappings {
            let mut val = [0u8; 8];
            let mut pos: usize = 0;
            let size = (mapping.size as usize).min(8);
            for (i, &slot_index) in mapping.slot_indices.iter().enumerate() {
                let sv = &self.slot_values[slot_index];
                if i == 0 {
                    let start = mapping.byte_offset.min(3);
                    let copy_len = (4 - start).min(size - pos);
                    val[pos..pos + copy_len].copy_from_slice(&sv[start..start + copy_len]);
                    pos += copy_len;
                } else {
                    let copy_len = 4.min(size - pos);
                    val[pos..pos + copy_len].copy_from_slice(&sv[..copy_len]);
                    pos += copy_len;
                }
                if pos >= size {
                    break;
                }
            }
            mapping.incoming.push((ts, val));
        }
        Ok(())
    }

    /// Perform a read-only core status request to verify that the physical link is alive.
    pub fn check_link(&mut self) -> Result<(), String> {
        if !self.connected {
            return Err("Probe 未连接".to_owned());
        }
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| "Probe 会话不可用，请重新连接".to_owned())?;
        let mut core = session
            .core(0)
            .map_err(|error| format!("获取核心失败: {error}"))?;
        core.status()
            .map(|_| ())
            .map_err(|error| format!("Probe 链路检测失败: {error}"))
    }

    pub fn write_value(&mut self, addr: u64, size: u32, value: u64) -> bool {
        if let Some(ref mut session) = self.session {
            if let Ok(mut core) = session.core(0) {
                return match size {
                    1 => core.write_word_8(addr, value as u8).is_ok(),
                    2 => core.write_word_16(addr, value as u16).is_ok(),
                    4 => core.write_word_32(addr, value as u32).is_ok(),
                    8 => core.write_word_64(addr, value).is_ok(),
                    _ => false,
                };
            }
        }
        false
    }

    /// Read a group of SVD registers while holding one probe core handle.
    /// Individual failures are returned without cancelling the remaining reads.
    pub fn read_registers(
        &mut self,
        requests: &[RegisterReadRequest],
    ) -> Vec<(u64, Result<u64, String>)> {
        let Some(session) = self.session.as_mut() else {
            return requests
                .iter()
                .map(|request| (request.id, Err("Probe 会话不可用，请重新连接".to_owned())))
                .collect();
        };
        let mut core = match session.core(0) {
            Ok(core) => core,
            Err(error) => {
                let error = format!("获取核心失败: {error}");
                return requests
                    .iter()
                    .map(|request| (request.id, Err(error.clone())))
                    .collect();
            }
        };

        requests
            .iter()
            .map(|request| {
                let result = match request.size_bytes {
                    1 => core.read_word_8(request.address).map(u64::from),
                    2 => core.read_word_16(request.address).map(u64::from),
                    4 => core.read_word_32(request.address).map(u64::from),
                    8 => core.read_word_64(request.address),
                    size => {
                        return (request.id, Err(format!("不支持 {size} 字节寄存器读取")));
                    }
                };
                (
                    request.id,
                    result.map_err(|error| format!("读取 0x{:08X} 失败: {error}", request.address)),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::FirmwareImageKind;

    #[test]
    fn detects_supported_firmware_formats_case_insensitively() {
        assert_eq!(
            FirmwareImageKind::from_path(Path::new("firmware.ELF")),
            Ok(FirmwareImageKind::Elf)
        );
        assert_eq!(
            FirmwareImageKind::from_path(Path::new("firmware.axf")),
            Ok(FirmwareImageKind::Elf)
        );
        assert_eq!(
            FirmwareImageKind::from_path(Path::new("firmware.hex")),
            Ok(FirmwareImageKind::Hex)
        );
        assert_eq!(
            FirmwareImageKind::from_path(Path::new("firmware.bin")),
            Ok(FirmwareImageKind::Bin)
        );
        assert_eq!(
            FirmwareImageKind::from_path(Path::new("firmware.uf2")),
            Ok(FirmwareImageKind::Uf2)
        );
        assert!(FirmwareImageKind::from_path(Path::new("firmware.txt")).is_err());
    }
}
