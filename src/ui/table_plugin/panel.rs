use super::table_dialog::{TableEntry, table_add_config_ui, table_entry_dialog_ui};
use crate::dwarf::types::ExtendType;
use crate::model::VariablePool;
use crate::ui::plugin::{
    MemRWPlugin, PluginAction, PluginRenderContext, ToastLevel, temp_text_value,
};
use crate::ui::theme;
use eframe::egui::{self, RichText, Ui};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub struct TablePluginState {
    pub entries: Vec<TableEntry>,
    pub editing_entry: Option<usize>,
    pub show_entry_dialog: bool,
    pub removed_var_ids: Vec<usize>,
    pub pending_writes: Vec<(usize, u64)>,
    pub status_message: Option<String>,
    pub status_error: bool,
}

impl Default for TablePluginState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            editing_entry: None,
            show_entry_dialog: false,
            removed_var_ids: Vec::new(),
            pending_writes: Vec::new(),
            status_message: None,
            status_error: false,
        }
    }
}

impl TablePluginState {
    pub fn add_entry(&mut self, variable_id: usize, pool: &VariablePool, display_name: String) {
        if let Some(var) = pool.get(variable_id) {
            let mut entry = TableEntry::new(variable_id, var.name.clone());
            if !display_name.is_empty() {
                entry.display_name = display_name;
            }
            self.entries.push(entry);
        }
    }
    pub fn remove_entry(&mut self, index: usize) {
        if index < self.entries.len() {
            let var_id = self.entries[index].variable_id;
            self.removed_var_ids.push(var_id);
            self.entries.remove(index);
            if self.editing_entry == Some(index) {
                self.editing_entry = None;
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
struct SavedTableEntry {
    variable_name: String,
    variable_address: u64,
    display_name: String,
}

impl MemRWPlugin for TablePluginState {
    fn id(&self) -> &'static str {
        "table"
    }

    fn title(&self) -> &'static str {
        "Table 读写数据"
    }

    fn viewport_size(&self) -> egui::Vec2 {
        egui::vec2(520.0, 360.0)
    }

    fn min_viewport_size(&self) -> egui::Vec2 {
        egui::vec2(320.0, 220.0)
    }

    fn render(&mut self, ui: &mut Ui, ctx: PluginRenderContext<'_>) -> Vec<PluginAction> {
        let mut actions = Vec::new();
        if table_panel(ui, self, ctx.pool, ctx.frame_data) {
            actions.push(PluginAction::OpenVariableTree {
                plugin_id: self.id().to_owned(),
            });
        }

        for var_id in self.removed_var_ids.drain(..) {
            actions.push(PluginAction::RemoveVariable { var_id });
        }
        for (var_id, value) in self.pending_writes.drain(..) {
            actions.push(PluginAction::WriteVariable { var_id, value });
        }
        if let Some(message) = self.status_message.take() {
            if self.status_error {
                actions.push(PluginAction::Toast {
                    level: ToastLevel::Error,
                    message,
                });
            }
            self.status_error = false;
        }

        actions
    }

    fn add_variable_ui(
        &mut self,
        ui: &mut Ui,
        node_id: usize,
        default_name: &str,
        variable_id: usize,
        pool: &VariablePool,
    ) -> bool {
        let name_id = ui.make_persistent_id(format!("table_add_name_{node_id}"));
        let name_default_id = ui.make_persistent_id(format!("table_add_name_default_{node_id}"));
        let mut display_name = temp_text_value(ui, name_id, name_default_id, default_name);

        table_add_config_ui(ui, default_name, &mut display_name);
        let added = ui.button("添加到 Table").clicked();

        ui.data_mut(|d| {
            d.insert_temp(name_id, display_name.clone());
        });

        if added {
            self.add_entry(variable_id, pool, display_name);
        }
        added
    }

    fn is_dialog_open(&self) -> bool {
        self.show_entry_dialog
    }

    fn save_config(&self, pool: &VariablePool) -> serde_json::Value {
        let entries: Vec<SavedTableEntry> = self
            .entries
            .iter()
            .map(|entry| {
                let var = pool.get(entry.variable_id);
                SavedTableEntry {
                    variable_name: var.map(|v| v.name.clone()).unwrap_or_default(),
                    variable_address: var.map(|v| v.address).unwrap_or(0),
                    display_name: entry.display_name.clone(),
                }
            })
            .collect();
        serde_json::to_value(entries).unwrap_or(serde_json::Value::Null)
    }

    fn load_config(
        &mut self,
        payload: &serde_json::Value,
        pool: &mut VariablePool,
    ) -> Result<(), String> {
        let entries: Vec<SavedTableEntry> =
            serde_json::from_value(payload.clone()).map_err(|e| e.to_string())?;

        self.entries.clear();
        for saved in entries {
            let var_id = pool
                .find_by_name_addr(&saved.variable_name, saved.variable_address)
                .map(|v| v.id)
                .ok_or_else(|| format!("表格变量 \"{}\" 匹配失败", saved.variable_name))?;

            self.entries
                .push(TableEntry::new(var_id, saved.display_name));
            if let Some(var) = pool.get_mut(var_id) {
                var.plugins_cnt += 1;
            }
        }
        Ok(())
    }
}

pub fn table_panel(
    ui: &mut Ui,
    state: &mut TablePluginState,
    pool: &VariablePool,
    frame_data: &HashMap<usize, Vec<(f64, [u8; 8])>>,
) -> bool {
    let mut open_tree = false;

    ui.vertical(|ui| {
        let dialog_is_open = state.show_entry_dialog;

        ui.add_enabled_ui(!dialog_is_open, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new("📋 变量读写表格").size(16.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(RichText::new("📋 打开变量树").size(12.0))
                        .clicked()
                    {
                        open_tree = true;
                    }
                    ui.label(format!("{} 个变量", state.entries.len()));
                });
            });
            ui.add_space(4.0);

            egui::ScrollArea::vertical().show(ui, |ui| {
                if state.entries.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.label(
                            RichText::new("暂无监控变量")
                                .size(13.0)
                                .color(theme::muted_text(ui)),
                        );
                        if ui.button("📋 打开变量树").clicked() {
                            open_tree = true;
                        }
                    });
                } else {
                    render_table(ui, state, pool, frame_data);
                }
            });
        });

        if state.show_entry_dialog {
            if let Some(edit_idx) = state.editing_entry {
                let mut dialog_remove = false;
                let mut should_close = false;
                let entry = &mut state.entries[edit_idx];
                let ext_info = pool
                    .get(entry.variable_id)
                    .map(|v| (v.name.clone(), v.address, v.ext_type.clone(), v.size));
                egui::Modal::new(egui::Id::new("table_entry_modal")).show(ui.ctx(), |ui| {
                    ui.set_width(320.0);
                    egui::Frame::NONE
                        .inner_margin(egui::Margin {
                            left: 20,
                            right: 20,
                            top: 16,
                            bottom: 16,
                        })
                        .show(ui, |ui| {
                            ui.heading(format!("变量属性 - {}", entry.display_name));
                            ui.separator();
                            let (ext_name, ext_addr, ext_type, ext_size) =
                                ext_info.unwrap_or((String::new(), 0, ExtendType::U32, 0));
                            if let Some(remove) = table_entry_dialog_ui(
                                ui, entry, &ext_name, ext_addr, &ext_type, ext_size,
                            ) {
                                dialog_remove = remove;
                                should_close = true;
                            }
                        });
                });
                if should_close {
                    state.show_entry_dialog = false;
                    if dialog_remove {
                        state.remove_entry(edit_idx);
                    }
                }
            } else {
                state.show_entry_dialog = false;
            }
        }
    });

    open_tree
}

fn render_table(
    ui: &mut Ui,
    state: &mut TablePluginState,
    pool: &VariablePool,
    frame_data: &HashMap<usize, Vec<(f64, [u8; 8])>>,
) {
    let mut to_edit = None;

    egui::Grid::new("var_table")
        .striped(true)
        .min_col_width(70.0)
        .show(ui, |ui| {
            ui.strong("Name");
            ui.strong("Read");
            ui.strong("Write");
            ui.end_row();

            for (i, entry) in state.entries.iter_mut().enumerate() {
                let row_id = egui::Id::new(("table_row", i));
                let var = pool.get(entry.variable_id);

                if ui
                    .add_sized(
                        [120.0, 20.0],
                        egui::Button::new(RichText::new(&entry.display_name).size(12.0)),
                    )
                    .double_clicked()
                {
                    to_edit = Some(i);
                }

                let current_val = var
                    .and_then(|v| {
                        frame_data
                            .get(&entry.variable_id)
                            .and_then(|d| d.last())
                            .map(|(_, data)| format_value(data, &v.ext_type))
                    })
                    .unwrap_or_else(|| "--".into());
                ui.label(RichText::new(&current_val).size(12.0).monospace());

                let var_info = var.map(|v| (v.ext_type.clone(), v.size));
                let mut edit_buf = entry.edit_buffer.clone();
                ui.horizontal(|ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut edit_buf)
                            .id(row_id.with("write_edit"))
                            .desired_width(70.0)
                            .font(egui::TextStyle::Monospace),
                    );
                    if resp.changed() {
                        entry.edit_buffer = edit_buf;
                    }
                    if ui.add(egui::Button::new("写").small()).clicked() {
                        if let Some((ref ext_type, _)) = var_info {
                            match validate_write(&entry.edit_buffer, ext_type) {
                                Ok(value) => {
                                    state.pending_writes.push((entry.variable_id, value));
                                    state.status_message = Some("写入请求已发送".into());
                                    state.status_error = false;
                                }
                                Err(e) => {
                                    state.status_message = Some(e);
                                    state.status_error = true;
                                }
                            }
                        }
                    }
                });
                ui.end_row();
            }
        });

    if let Some(i) = to_edit {
        state.editing_entry = Some(i);
        state.show_entry_dialog = true;
    }
}

fn validate_write(input: &str, ext_type: &ExtendType) -> Result<u64, String> {
    let v = input.trim();
    if v.is_empty() {
        return Err("请输入值".into());
    }
    match ext_type {
        ExtendType::U8 => v
            .parse::<u8>()
            .map(|x| x as u64)
            .map_err(|_| "超出 u8 范围 (0-255)".into()),
        ExtendType::I8 => v
            .parse::<i8>()
            .map(|x| x as u64)
            .map_err(|_| "超出 i8 范围 (-128~127)".into()),
        ExtendType::U16 => v
            .parse::<u16>()
            .map(|x| x as u64)
            .map_err(|_| "超出 u16 范围".into()),
        ExtendType::I16 => v
            .parse::<i16>()
            .map(|x| x as u64)
            .map_err(|_| "超出 i16 范围".into()),
        ExtendType::U32 => v
            .parse::<u32>()
            .map(|x| x as u64)
            .map_err(|_| "超出 u32 范围".into()),
        ExtendType::I32 => v
            .parse::<i32>()
            .map(|x| x as u64)
            .map_err(|_| "超出 i32 范围".into()),
        ExtendType::U64 => v.parse::<u64>().map_err(|_| "超出 u64 范围".into()),
        ExtendType::I64 => v
            .parse::<i64>()
            .map(|x| x as u64)
            .map_err(|_| "超出 i64 范围".into()),
        ExtendType::Float => {
            let f: f32 = v.parse().map_err(|_| "无效的 float".to_string())?;
            Ok(f.to_bits() as u64)
        }
        ExtendType::Double => {
            let d: f64 = v.parse().map_err(|_| "无效的 double".to_string())?;
            Ok(d.to_bits())
        }
        ExtendType::Other => Err("Other 类型不支持写入".into()),
    }
}

fn format_value(data: &[u8], ext_type: &ExtendType) -> String {
    use ExtendType::*;
    if data.is_empty() {
        return "--".into();
    }
    match ext_type {
        U8 => format!("0x{:02X} ({})", data[0], data[0]),
        I8 => {
            let val = i8::from_le_bytes([data[0]]);
            format!("0x{:02X} ({})", data[0], val)
        }
        U16 if data.len() >= 2 => {
            let val = u16::from_le_bytes([data[0], data[1]]);
            format!("0x{val:04X} ({val})")
        }
        I16 if data.len() >= 2 => {
            let val = i16::from_le_bytes([data[0], data[1]]);
            format!("0x{val:04X} ({val})")
        }
        U32 if data.len() >= 4 => {
            let val = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            format!("0x{val:08X} ({val})")
        }
        I32 if data.len() >= 4 => {
            let val = i32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            format!("0x{val:08X} ({val})")
        }
        U64 if data.len() >= 8 => {
            let val = u64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]);
            format!("0x{val:016X} ({val})")
        }
        I64 if data.len() >= 8 => {
            let val = i64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]);
            format!("0x{val:016X} ({val})")
        }
        Float if data.len() >= 4 => {
            let val = f32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            format!("{val:.4}")
        }
        Double if data.len() >= 8 => {
            let val = f64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]);
            format!("{val:.6}")
        }
        Other => format!("{data:02X?}"),
        _ => format!("{data:02X?}"),
    }
}
