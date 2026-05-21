use eframe::egui::{self, Color32, RichText, Ui};
use crate::model::VariablePool;
use super::table_dialog::{TableEntry, table_entry_dialog_ui};

#[derive(PartialEq)]
pub enum PanelAction {
    None,
    OpenTree,
}

pub struct TablePluginState {
    pub entries: Vec<TableEntry>,
    pub editing_entry: Option<usize>,
}

impl Default for TablePluginState {
    fn default() -> Self {
        Self { entries: Vec::new(), editing_entry: None }
    }
}

impl TablePluginState {
    pub fn add_from_pool(&mut self, pool: &VariablePool, variable_id: usize) {
        if let Some(var) = pool.get(variable_id) {
            self.entries.push(TableEntry::new(variable_id, var.tree_node.name.clone()));
        }
    }

    pub fn remove_entry(&mut self, index: usize) {
        if index < self.entries.len() {
            self.entries.remove(index);
            if self.editing_entry == Some(index) { self.editing_entry = None; }
        }
    }

    pub fn entry_ids(&self) -> Vec<usize> {
        self.entries.iter().map(|e| e.variable_id).collect()
    }
}

pub fn table_panel(ui: &mut Ui, state: &mut TablePluginState, pool: &VariablePool) -> PanelAction {
    let mut action = PanelAction::None;

    ui.vertical(|ui| {
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

        if let Some(edit_idx) = state.editing_entry {
            let entry = &mut state.entries[edit_idx];
            ui.separator();
            ui.label(RichText::new(format!("编辑: {}", entry.display_name)).size(13.0));
            if table_entry_dialog_ui(ui, entry) { state.remove_entry(edit_idx); }
            if ui.button("完成").clicked() { state.editing_entry = None; }
            ui.separator();
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            if state.entries.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(RichText::new("暂无监控变量").size(13.0).color(Color32::from_rgb(150, 150, 150)));
                    ui.label(RichText::new("点击右上角「打开变量树」添加变量").size(12.0).color(Color32::from_rgb(130, 130, 130)));
                    if ui.button("📋 打开变量树").clicked() { action = PanelAction::OpenTree; }
                });
            } else {
                render_table(ui, state, pool);
            }
        });
    });

    action
}

fn render_table(ui: &mut Ui, state: &mut TablePluginState, pool: &VariablePool) {
    let mut to_remove = None;
    let mut to_edit = None;

    egui::Grid::new("var_table")
        .striped(true)
        .min_col_width(60.0)
        .show(ui, |ui| {
            ui.strong("Name");
            ui.strong("Address");
            ui.strong("Size");
            ui.strong("Value");
            ui.strong("Write");
            ui.strong("");
            ui.end_row();

            for (i, entry) in state.entries.iter_mut().enumerate() {
                let addr_str = pool.get(entry.variable_id)
                    .map(|v| v.tree_node.address_info.clone())
                    .unwrap_or_else(|| "--".into());
                let size_str = pool.get(entry.variable_id)
                    .map(|v| v.tree_node.size_info.clone())
                    .unwrap_or_else(|| "--".into());

                ui.label(RichText::new(&entry.display_name).size(12.0));
                ui.label(RichText::new(&addr_str).size(11.0).color(Color32::from_rgb(120, 180, 220)));
                ui.label(RichText::new(&size_str).size(11.0));

                let current_val = pool.get(entry.variable_id)
                    .map(|v| format_value(&v.current_value))
                    .unwrap_or_else(|| "--".into());
                ui.label(RichText::new(&current_val).size(12.0));

                let mut edit_buf = entry.edit_buffer.clone();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut edit_buf)
                        .desired_width(80.0)
                        .font(egui::TextStyle::Monospace),
                );
                if resp.changed() { entry.edit_buffer = edit_buf; }

                if ui.small_button("写").clicked() {
                    entry.current_value = std::mem::take(&mut entry.edit_buffer);
                }

                ui.end_row();

                let row_resp = ui.interact(ui.min_rect(), ui.next_auto_id(), egui::Sense::click());
                if row_resp.double_clicked() { to_edit = Some(i); }
                let del_btn = ui.small_button("✕");
                if del_btn.clicked() { to_remove = Some(i); }
            }
        });

    if let Some(i) = to_remove { state.remove_entry(i); }
    if let Some(i) = to_edit { state.editing_entry = Some(i); }
}

fn format_value(data: &[u8]) -> String {
    if data.len() >= 4 {
        let val = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        format!("0x{val:08X} ({val})")
    } else if data.is_empty() {
        "--".into()
    } else {
        format!("{data:02X?}")
    }
}
