use std::fmt::Write as _;
use std::path::PathBuf;

use eframe::egui::{self, RichText, Ui};
use serde::{Deserialize, Serialize};

use super::svd_panel::{SvdPanelState, render_svd_panel};
use super::tree::{CheckState, SavedTableLeaf, SavedTableNode, TableNode};
use crate::dwarf::types::ExtendType;
use crate::model::VariablePool;
use crate::ui::plugin::{
    MemRWPlugin, PluginAction, PluginRenderContext, ToastLevel, VariableCandidate, temp_text_value,
};
use crate::ui::theme;

pub struct TablePluginState {
    roots: Vec<TableNode>,
    next_node_id: u64,
    pending_removals: Vec<(usize, bool)>,
    pending_enabled: Vec<(usize, bool)>,
    pending_writes: Vec<(usize, u64)>,
    errors: Vec<String>,
    svd: SvdPanelState,
}

impl Default for TablePluginState {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            next_node_id: 1,
            pending_removals: Vec::new(),
            pending_enabled: Vec::new(),
            pending_writes: Vec::new(),
            errors: Vec::new(),
            svd: SvdPanelState::default(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct SavedTableConfig {
    roots: Vec<SavedTableNode>,
    #[serde(default)]
    svd_path: Option<String>,
}

#[derive(Deserialize)]
struct LegacyTableEntry {
    variable_name: String,
    variable_address: u64,
    display_name: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SavedTablePayload {
    Current(SavedTableConfig),
    Legacy(Vec<LegacyTableEntry>),
}

impl MemRWPlugin for TablePluginState {
    fn id(&self) -> &'static str {
        "table"
    }

    fn title(&self) -> &'static str {
        "Table 变量与寄存器"
    }

    fn viewport_size(&self) -> egui::Vec2 {
        egui::vec2(1080.0, 620.0)
    }

    fn min_viewport_size(&self) -> egui::Vec2 {
        egui::vec2(720.0, 420.0)
    }

    fn supports_composite_variables(&self) -> bool {
        true
    }

    fn render(&mut self, ui: &mut Ui, ctx: PluginRenderContext<'_>) -> Vec<PluginAction> {
        for root in &mut self.roots {
            root.update_values(ctx.pool, ctx.frame_data, format_value_into);
        }

        let mut actions = Vec::new();
        if table_panel(ui, self, ctx.pool) {
            actions.push(PluginAction::OpenVariableTree {
                plugin_id: self.id().to_owned(),
                viewport_id: ctx.viewport_id,
            });
        }
        actions.extend(
            self.pending_enabled
                .drain(..)
                .map(|(var_id, enabled)| PluginAction::SetVariableEnabled { var_id, enabled }),
        );
        actions.extend(
            self.pending_removals
                .drain(..)
                .map(|(var_id, was_enabled)| PluginAction::RemoveVariable {
                    var_id,
                    was_enabled,
                }),
        );
        actions.extend(
            self.pending_writes
                .drain(..)
                .map(|(var_id, value)| PluginAction::WriteVariable { var_id, value }),
        );
        actions.extend(self.errors.drain(..).map(|message| PluginAction::Toast {
            level: ToastLevel::Error,
            message,
        }));
        actions
    }

    fn add_variable_ui(
        &mut self,
        ui: &mut Ui,
        node_id: usize,
        default_name: &str,
        candidate: &VariableCandidate,
        pool: &mut VariablePool,
    ) -> bool {
        let name_id = ui.make_persistent_id(format!("table_add_name_{node_id}"));
        let name_default_id = ui.make_persistent_id(format!("table_add_name_default_{node_id}"));
        let mut display_name = temp_text_value(ui, name_id, name_default_id, default_name);

        ui.horizontal(|ui| {
            ui.label("显示名:");
            ui.text_edit_singleline(&mut display_name);
        });
        let leaf_count = candidate.readable_leaf_count();
        let label = if candidate.children.is_empty() {
            "添加到 Table".to_owned()
        } else {
            format!("添加到 Table（{leaf_count} 个可读字段）")
        };
        let added = ui
            .add_enabled(leaf_count > 0, egui::Button::new(label))
            .clicked();

        ui.data_mut(|data| data.insert_temp(name_id, display_name.clone()));
        if !added {
            return false;
        }
        if self.roots.iter().any(|root| root.matches_root(candidate)) {
            self.errors
                .push(format!("{} 已添加到 Table", candidate.name));
            return false;
        }

        match TableNode::from_candidate(candidate, display_name, pool, &mut self.next_node_id) {
            Ok(root) => {
                self.roots.push(root);
                true
            }
            Err(error) => {
                self.errors.push(error);
                false
            }
        }
    }

    fn save_config(&self, pool: &VariablePool) -> serde_json::Value {
        let config = SavedTableConfig {
            roots: self.roots.iter().map(|root| root.to_saved(pool)).collect(),
            svd_path: self.svd.path().map(|path| path.display().to_string()),
        };
        serde_json::to_value(config).unwrap_or(serde_json::Value::Null)
    }

    fn load_config(
        &mut self,
        payload: &serde_json::Value,
        pool: &mut VariablePool,
    ) -> Result<(), String> {
        let payload: SavedTablePayload =
            serde_json::from_value(payload.clone()).map_err(|error| error.to_string())?;
        let (saved_roots, svd_path) = match payload {
            SavedTablePayload::Current(config) => (config.roots, config.svd_path),
            SavedTablePayload::Legacy(entries) => (
                entries
                    .into_iter()
                    .map(|entry| SavedTableNode {
                        label: entry.display_name,
                        source_name: entry.variable_name.clone(),
                        source_address: entry.variable_address,
                        expanded: true,
                        variable: Some(SavedTableLeaf {
                            variable_name: entry.variable_name,
                            variable_address: entry.variable_address,
                            variable_type: None,
                            variable_size: None,
                            enabled: true,
                        }),
                        children: Vec::new(),
                    })
                    .collect(),
                None,
            ),
        };

        self.roots.clear();
        self.next_node_id = 1;
        for saved in saved_roots {
            self.roots
                .push(TableNode::from_saved(saved, pool, &mut self.next_node_id)?);
        }
        if let Some(path) = svd_path.filter(|path| !path.is_empty()) {
            self.svd.load_path(PathBuf::from(path));
        }
        Ok(())
    }
}

fn table_panel(ui: &mut Ui, state: &mut TablePluginState, pool: &VariablePool) -> bool {
    let mut open_tree = false;
    egui::Panel::left("table_variable_panel")
        .resizable(true)
        .default_size((ui.available_width() * 0.56).max(380.0))
        .size_range(340.0..=760.0)
        .show_inside(ui, |ui| {
            open_tree = render_variable_panel(ui, state, pool);
        });
    egui::CentralPanel::default().show_inside(ui, |ui| {
        render_svd_panel(ui, &mut state.svd);
    });
    open_tree
}

fn render_variable_panel(ui: &mut Ui, state: &mut TablePluginState, pool: &VariablePool) -> bool {
    let mut open_tree = false;
    ui.horizontal(|ui| {
        ui.heading(RichText::new("变量监控").size(15.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("打开变量树").clicked() {
                open_tree = true;
            }
            let leaf_count = state.roots.iter().map(TableNode::leaf_count).sum::<usize>();
            ui.label(
                RichText::new(format!("{} 组 / {} 个变量", state.roots.len(), leaf_count))
                    .size(10.5)
                    .color(theme::muted_text(ui)),
            );
        });
    });
    ui.horizontal(|ui| {
        ui.add_space(38.0);
        ui.add_sized(
            [180.0, 18.0],
            egui::Label::new(RichText::new("名称").strong()),
        );
        ui.add_sized(
            [135.0, 18.0],
            egui::Label::new(RichText::new("当前值").strong()),
        );
        ui.label(RichText::new("写入").strong());
    });
    ui.separator();

    if state.roots.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(45.0);
            ui.label(RichText::new("暂无变量、结构体或数组").color(theme::muted_text(ui)));
            if ui.button("打开变量树").clicked() {
                open_tree = true;
            }
        });
        return open_tree;
    }

    let mut remove_root = None;
    egui::ScrollArea::both()
        .id_salt("table_variable_tree_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, root) in state.roots.iter_mut().enumerate() {
                if render_node(
                    ui,
                    root,
                    0,
                    true,
                    pool,
                    &mut state.pending_enabled,
                    &mut state.pending_writes,
                    &mut state.errors,
                ) {
                    remove_root = Some(index);
                }
            }
        });
    if let Some(index) = remove_root {
        let root = state.roots.remove(index);
        root.collect_removals(&mut state.pending_removals);
    }
    open_tree
}

#[allow(clippy::too_many_arguments)]
fn render_node(
    ui: &mut Ui,
    node: &mut TableNode,
    depth: usize,
    is_root: bool,
    pool: &VariablePool,
    enabled_changes: &mut Vec<(usize, bool)>,
    pending_writes: &mut Vec<(usize, u64)>,
    errors: &mut Vec<String>,
) -> bool {
    let mut remove = false;
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 16.0);
        if node.children.is_empty() {
            ui.add_space(20.0);
        } else if ui
            .small_button(if node.expanded { "▼" } else { "▶" })
            .clicked()
        {
            node.expanded = !node.expanded;
        }

        if let Some(leaf) = &mut node.leaf {
            let response = ui.checkbox(&mut leaf.enabled, "");
            if response.changed() {
                enabled_changes.push((leaf.variable_id, leaf.enabled));
            }
        } else {
            let (symbol, enabled) = match node.check_state() {
                CheckState::All => ("☑", true),
                CheckState::Partial => ("▣", true),
                CheckState::None => ("☐", true),
                CheckState::Unavailable => ("—", false),
            };
            if ui
                .add_enabled(enabled, egui::Button::new(symbol).frame(false))
                .clicked()
            {
                let target = node.check_state() != CheckState::All;
                node.set_enabled_recursive(target, enabled_changes);
            }
        }

        let name_response = ui.add_sized(
            [180.0 - (depth as f32 * 8.0).min(80.0), 20.0],
            egui::Label::new(&node.label).truncate(),
        );
        name_response.on_hover_text(format!(
            "{} @ 0x{:X}",
            node.source_name, node.source_address
        ));

        if let Some(leaf) = &mut node.leaf {
            ui.add_sized(
                [135.0, 20.0],
                egui::Label::new(RichText::new(&leaf.current_value).monospace().size(11.0))
                    .truncate(),
            );
            ui.add(
                egui::TextEdit::singleline(&mut leaf.edit_buffer)
                    .id(egui::Id::new(("table_write", node.id)))
                    .desired_width(72.0)
                    .font(egui::TextStyle::Monospace),
            );
            if ui.small_button("写").clicked() {
                if let Some(variable) = pool.get(leaf.variable_id) {
                    match validate_write(&leaf.edit_buffer, &variable.ext_type) {
                        Ok(value) => pending_writes.push((leaf.variable_id, value)),
                        Err(error) => errors.push(error),
                    }
                }
            }
        } else {
            ui.add_sized(
                [135.0, 20.0],
                egui::Label::new(
                    RichText::new(format!("{} 个变量", node.leaf_count()))
                        .size(10.5)
                        .color(theme::muted_text(ui)),
                ),
            );
        }

        if is_root
            && ui
                .small_button(RichText::new("删除").color(theme::danger_text(ui)))
                .clicked()
        {
            remove = true;
        }
    });

    if node.expanded {
        for child in &mut node.children {
            render_node(
                ui,
                child,
                depth + 1,
                false,
                pool,
                enabled_changes,
                pending_writes,
                errors,
            );
        }
    }
    remove
}

fn validate_write(input: &str, ext_type: &ExtendType) -> Result<u64, String> {
    let value = input.trim();
    if value.is_empty() {
        return Err("请输入值".to_owned());
    }
    match ext_type {
        ExtendType::U8 => value
            .parse::<u8>()
            .map(u64::from)
            .map_err(|_| "超出 u8 范围 (0-255)".to_owned()),
        ExtendType::I8 => value
            .parse::<i8>()
            .map(|value| value as u64)
            .map_err(|_| "超出 i8 范围 (-128~127)".to_owned()),
        ExtendType::U16 => value
            .parse::<u16>()
            .map(u64::from)
            .map_err(|_| "超出 u16 范围".to_owned()),
        ExtendType::I16 => value
            .parse::<i16>()
            .map(|value| value as u64)
            .map_err(|_| "超出 i16 范围".to_owned()),
        ExtendType::U32 => value
            .parse::<u32>()
            .map(u64::from)
            .map_err(|_| "超出 u32 范围".to_owned()),
        ExtendType::I32 => value
            .parse::<i32>()
            .map(|value| value as u64)
            .map_err(|_| "超出 i32 范围".to_owned()),
        ExtendType::U64 => value.parse::<u64>().map_err(|_| "超出 u64 范围".to_owned()),
        ExtendType::I64 => value
            .parse::<i64>()
            .map(|value| value as u64)
            .map_err(|_| "超出 i64 范围".to_owned()),
        ExtendType::Float => value
            .parse::<f32>()
            .map(|value| u64::from(value.to_bits()))
            .map_err(|_| "无效的 float".to_owned()),
        ExtendType::Double => value
            .parse::<f64>()
            .map(f64::to_bits)
            .map_err(|_| "无效的 double".to_owned()),
        ExtendType::Other => Err("Other 类型不支持写入".to_owned()),
    }
}

fn format_value_into(data: &[u8], ext_type: &ExtendType, output: &mut String) {
    use ExtendType::*;
    output.clear();
    if data.is_empty() {
        output.push_str("--");
        return;
    }
    match ext_type {
        U8 => write!(output, "0x{:02X} ({})", data[0], data[0]),
        I8 => {
            let value = i8::from_le_bytes([data[0]]);
            write!(output, "0x{:02X} ({value})", data[0])
        }
        U16 if data.len() >= 2 => {
            let value = u16::from_le_bytes([data[0], data[1]]);
            write!(output, "0x{value:04X} ({value})")
        }
        I16 if data.len() >= 2 => {
            let value = i16::from_le_bytes([data[0], data[1]]);
            write!(output, "0x{value:04X} ({value})")
        }
        U32 if data.len() >= 4 => {
            let value = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            write!(output, "0x{value:08X} ({value})")
        }
        I32 if data.len() >= 4 => {
            let value = i32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            write!(output, "0x{value:08X} ({value})")
        }
        U64 if data.len() >= 8 => {
            let value = u64::from_le_bytes(data[..8].try_into().unwrap());
            write!(output, "0x{value:016X} ({value})")
        }
        I64 if data.len() >= 8 => {
            let value = i64::from_le_bytes(data[..8].try_into().unwrap());
            write!(output, "0x{value:016X} ({value})")
        }
        Float if data.len() >= 4 => {
            let value = f32::from_le_bytes(data[..4].try_into().unwrap());
            write!(output, "{value:.4}")
        }
        Double if data.len() >= 8 => {
            let value = f64::from_le_bytes(data[..8].try_into().unwrap());
            write!(output, "{value:.6}")
        }
        _ => write!(output, "{data:02X?}"),
    }
    .expect("writing to a String cannot fail");
}

#[cfg(test)]
mod tests {
    use super::{SavedTablePayload, validate_write};
    use crate::dwarf::types::ExtendType;

    #[test]
    fn validates_integer_write_ranges() {
        assert_eq!(validate_write("255", &ExtendType::U8), Ok(255));
        assert!(validate_write("256", &ExtendType::U8).is_err());
        assert_eq!(validate_write("-1", &ExtendType::I8), Ok(u64::MAX));
    }

    #[test]
    fn accepts_legacy_flat_table_configuration() {
        let payload = serde_json::json!([{
            "variable_name": "counter",
            "variable_address": 536870912,
            "display_name": "Counter"
        }]);

        assert!(matches!(
            serde_json::from_value::<SavedTablePayload>(payload).unwrap(),
            SavedTablePayload::Legacy(entries) if entries.len() == 1
        ));
    }
}
