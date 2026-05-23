use eframe::egui::{self, Color32, RichText, Ui};
use crate::model::VariablePool;
use crate::types::ExtendType;
use super::table_dialog::{TableEntry, table_entry_dialog_ui};
use std::collections::HashMap;

#[derive(PartialEq)]
pub enum PanelAction { None, OpenTree }

pub struct TablePluginState {
    pub entries: Vec<TableEntry>,
    pub editing_entry: Option<usize>,
    pub show_entry_dialog: bool,
    pub removed_var_ids: Vec<usize>,
}

impl Default for TablePluginState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            editing_entry: None,
            show_entry_dialog: false,
            removed_var_ids: Vec::new(),
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
            if self.editing_entry == Some(index) { self.editing_entry = None; }
        }
    }
    pub fn entry_ids(&self) -> Vec<usize> {
        self.entries.iter().map(|e| e.variable_id).collect()
    }
}

pub fn table_panel(
    ui: &mut Ui,
    state: &mut TablePluginState,
    pool: &VariablePool,
    frame_data: &HashMap<usize, Vec<(f64, [u8; 8])>>,
) -> PanelAction {
    let mut action = PanelAction::None;

    ui.vertical(|ui| {
        let dialog_is_open = state.show_entry_dialog;

        ui.add_enabled_ui(!dialog_is_open, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new("📋 变量读写表格").size(16.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new("📋 打开变量树").size(12.0)).clicked() {
                        action = PanelAction::OpenTree;
                    }
                    ui.label(format!("{} 个变量", state.entries.len()));
                });
            });
            ui.add_space(4.0);

            egui::ScrollArea::vertical().show(ui, |ui| {
                if state.entries.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.label(RichText::new("暂无监控变量").size(13.0).color(Color32::from_rgb(150, 150, 150)));
                        if ui.button("📋 打开变量树").clicked() { action = PanelAction::OpenTree; }
                    });
                } else {
                    render_table(ui, state, pool, frame_data);
                }
            });
        });

        // Dialog (rendered at top layer, always interactive)
        if state.show_entry_dialog {
            if let Some(edit_idx) = state.editing_entry {
                let mut dialog_remove = false;
                let mut should_close = false;
                {
                let entry = &mut state.entries[edit_idx];
                let ext_info = pool.get(entry.variable_id).map(|v| {
                    (
                        v.name.clone(),
                        v.address,
                        v.ext_type.clone(),
                        v.size,
                    )
                });
                egui::Window::new(format!("变量属性 - {}", entry.display_name))
                    .collapsible(false).resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ui.ctx(), |ui| {
                        let (ext_name, ext_addr, ext_type, ext_size) = ext_info.unwrap_or((String::new(), 0, ExtendType::U32, 0));
                        if let Some(remove) = table_entry_dialog_ui(ui, entry, &ext_name, ext_addr, &ext_type, ext_size) {
                            dialog_remove = remove;
                            should_close = true;
                        }
                    });
                }
                if should_close {
                    state.show_entry_dialog = false;
                    if dialog_remove { state.remove_entry(edit_idx); }
                }
            } else { state.show_entry_dialog = false; }
        }
    });

    action
}

fn render_table(ui: &mut Ui, state: &mut TablePluginState, pool: &VariablePool, frame_data: &HashMap<usize, Vec<(f64, [u8; 8])>>) {
    let mut to_remove = None;
    let mut to_edit = None;

    egui::Grid::new("var_table")
        .striped(true)
        .min_col_width(70.0)
        .show(ui, |ui| {
            ui.strong("Name");
            ui.strong("Value");
            ui.strong("Write");
            ui.strong("");
            ui.end_row();

            for (i, entry) in state.entries.iter_mut().enumerate() {
                let row_id = egui::Id::new(("table_row", i));

                ui.label(RichText::new(&entry.display_name).size(12.0));

                let var = pool.get(entry.variable_id);
                let current_val = var
                    .and_then(|v| {
                        frame_data
                            .get(&entry.variable_id)
                            .and_then(|d| d.last())
                            .map(|(_, data)| format_value(data, &v.ext_type))
                            .or_else(|| {
                                v.incoming
                                    .latest()
                                    .map(|(_, data)| format_value(&data, &v.ext_type))
                            })
                    })
                    .unwrap_or_else(|| "--".into());
                ui.label(RichText::new(&current_val).size(12.0));

                let mut edit_buf = entry.edit_buffer.clone();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut edit_buf)
                        .id(row_id.with("write_edit"))
                        .desired_width(80.0)
                        .font(egui::TextStyle::Monospace),
                );
                if resp.changed() { entry.edit_buffer = edit_buf; }

                if ui.add(egui::Button::new("写").small()).clicked() {
                    entry.current_value = std::mem::take(&mut entry.edit_buffer);
                }

                ui.end_row();

                let row_rect = ui.min_rect();
                let int_resp = ui.interact(row_rect, row_id.with("click"), egui::Sense::click());
                if int_resp.double_clicked() { to_edit = Some(i); }

                let del_btn = ui.add_sized([20.0, 16.0], egui::Button::new("✕").small());
                if del_btn.clicked() { to_remove = Some(i); }
            }
        });

    if let Some(i) = to_remove { state.remove_entry(i); }
    if let Some(i) = to_edit { state.editing_entry = Some(i); state.show_entry_dialog = true; }
}

fn format_value(data: &[u8], ext_type: &ExtendType) -> String {
    use ExtendType::*;
    if data.is_empty() { return "--".into(); }
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
            let val = u64::from_le_bytes([data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]]);
            format!("0x{val:016X} ({val})")
        }
        I64 if data.len() >= 8 => {
            let val = i64::from_le_bytes([data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]]);
            format!("0x{val:016X} ({val})")
        }
        Float if data.len() >= 4 => {
            let val = f32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            format!("{val:.4}")
        }
        Double if data.len() >= 8 => {
            let val = f64::from_le_bytes([data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]]);
            format!("{val:.6}")
        }
        Other => format!("{data:02X?}"),
        _ => format!("{data:02X?}"),
    }
}
