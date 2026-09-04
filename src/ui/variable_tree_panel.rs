use std::collections::HashMap;

use eframe::egui::{self, Ui};

use crate::dwarf;
use crate::model::VariablePool;
use crate::ui::plugin::{MemRWPlugin, PluginAction, ToastLevel, VariableCandidate};

// Keep the sheet above normal panels while reserving Foreground for Toasts and popups.
const VARIABLE_TREE_LAYER_ORDER: egui::Order = egui::Order::Middle;

#[derive(Debug, Clone)]
struct VariableTreeTarget {
    plugin_id: String,
    viewport_id: egui::ViewportId,
    popped: bool,
}

pub struct VariableTreePanel {
    pub dwarf_state: dwarf::types::DwarfState,
    pub elf_path: String,
    target: Option<VariableTreeTarget>,
    drag_state: Option<(f32, f32)>,
    height: f32,
    extend_configs: HashMap<usize, dwarf::types::ExtendConfig>,
    loaded_elf_path: Option<String>,
    program_generation: u64,
}

impl VariableTreePanel {
    pub fn new(dwarf_state: dwarf::types::DwarfState) -> Self {
        Self {
            dwarf_state,
            elf_path: String::new(),
            target: None,
            drag_state: None,
            height: 250.0,
            extend_configs: HashMap::new(),
            loaded_elf_path: None,
            program_generation: 0,
        }
    }

    pub fn open(&mut self, plugin_id: impl Into<String>, viewport_id: egui::ViewportId) {
        let popped = self.target.as_ref().is_some_and(|target| target.popped);
        self.target = Some(VariableTreeTarget {
            plugin_id: plugin_id.into(),
            viewport_id,
            popped,
        });
        if !popped {
            self.drag_state = None;
        }
    }

    pub fn is_open_in(&self, viewport_id: egui::ViewportId) -> bool {
        self.target
            .as_ref()
            .is_some_and(|target| !target.popped && target.viewport_id == viewport_id)
    }

    pub fn close_in(&mut self, viewport_id: egui::ViewportId) {
        if self.is_open_in(viewport_id) {
            self.target = None;
            self.drag_state = None;
        }
    }

    pub fn show(
        &mut self,
        host_ui: &mut Ui,
        plugin: &mut dyn MemRWPlugin,
        pool: &mut VariablePool,
        actions: &mut Vec<PluginAction>,
    ) {
        let viewport_id = host_ui.ctx().viewport_id();
        let Some(target) = self.target.as_ref() else {
            return;
        };
        if target.popped || target.viewport_id != viewport_id || target.plugin_id != plugin.id() {
            return;
        }

        let window_width = host_ui.ctx().viewport_rect().width();
        let window_height = host_ui.ctx().viewport_rect().height();
        let colors = crate::ui::theme::palette(host_ui);
        let overlay_id = egui::Id::new(("variable_tree_overlay", viewport_id));
        let sheet_id = egui::Id::new(("variable_tree_sheet", viewport_id));

        egui::Area::new(overlay_id)
            .fixed_pos(host_ui.ctx().viewport_rect().min)
            .order(VARIABLE_TREE_LAYER_ORDER)
            .show(host_ui.ctx(), |ui| {
                ui.painter()
                    .rect_filled(ui.ctx().viewport_rect(), 0.0, colors.modal_overlay);
                if ui
                    .interact(
                        ui.ctx().viewport_rect(),
                        ui.next_auto_id(),
                        egui::Sense::click(),
                    )
                    .clicked()
                {
                    self.target = None;
                }
            });

        egui::Area::new(sheet_id)
            .anchor(egui::Align2::LEFT_BOTTOM, egui::Vec2::ZERO)
            .fixed_pos(egui::pos2(0.0, host_ui.ctx().viewport_rect().bottom()))
            .order(VARIABLE_TREE_LAYER_ORDER)
            .constrain(true)
            .show(host_ui.ctx(), |ui| {
                ui.set_width(window_width);
                egui::Frame::NONE
                    .fill(colors.elevated_bg)
                    .stroke(crate::ui::theme::panel_stroke(ui))
                    .corner_radius(egui::CornerRadius {
                        nw: 16,
                        ne: 16,
                        sw: 0,
                        se: 0,
                    })
                    .show(ui, |ui| {
                        let target_height =
                            bottom_sheet_handle(ui, &mut self.drag_state, self.height);
                        self.height = target_height.clamp(window_height * 0.3, window_height * 0.8);
                        ui.set_height(self.height);
                        egui::Frame::NONE
                            .inner_margin(egui::Margin {
                                left: 14,
                                right: 14,
                                top: 26,
                                bottom: 10,
                            })
                            .show(ui, |ui| {
                                self.show_contents(ui, plugin, pool, actions);
                            });
                    });
            });
    }

    pub fn show_popout(
        &mut self,
        host_ui: &mut Ui,
        plugins: &mut [Box<dyn MemRWPlugin>],
        pool: &mut VariablePool,
        actions: &mut Vec<PluginAction>,
    ) -> Option<String> {
        let target = self.target.clone().filter(|target| target.popped)?;
        let Some(plugin) = plugins
            .iter_mut()
            .find(|plugin| plugin.id() == target.plugin_id)
        else {
            self.target = None;
            return None;
        };
        let viewport_id = egui::ViewportId::from_hash_of("variable_tree_popout");
        let host_viewport_id = host_ui.ctx().viewport_id();
        let title = variable_tree_window_title(plugin.id());
        let mut close_requested = false;
        host_ui.ctx().show_viewport_immediate(
            viewport_id,
            egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size([1000.0, 620.0])
                .with_min_inner_size([700.0, 450.0])
                .with_resizable(true),
            |viewport_ui, _class| {
                if viewport_ui
                    .ctx()
                    .input(|input| input.viewport().close_requested())
                {
                    close_requested = true;
                    return;
                }
                egui::CentralPanel::default().show_inside(viewport_ui, |ui| {
                    self.show_contents(ui, plugin.as_mut(), pool, actions);
                });
            },
        );
        if close_requested {
            self.target = None;
            self.drag_state = None;
            return None;
        }

        let popped_in = self
            .target
            .as_ref()
            .is_some_and(|current| current.plugin_id == target.plugin_id && !current.popped);
        if popped_in {
            if let Some(current) = self.target.as_mut() {
                current.viewport_id = host_viewport_id;
            }
            Some(target.plugin_id)
        } else {
            None
        }
    }

    fn show_contents(
        &mut self,
        ui: &mut Ui,
        plugin: &mut dyn MemRWPlugin,
        pool: &mut VariablePool,
        actions: &mut Vec<PluginAction>,
    ) {
        ui.horizontal(|ui| {
            ui.label("ELF 文件:");
            ui.add_sized(
                [(ui.available_width() - 200.0).max(80.0), 20.0],
                egui::TextEdit::singleline(&mut self.elf_path)
                    .hint_text("输入 firmware.elf 路径..."),
            );
            if ui.button("浏览").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("ELF/AXF", &["elf", "axf"])
                    .add_filter("全部", &["*"])
                    .pick_file()
                {
                    self.elf_path = path.display().to_string();
                }
            }
            if ui.button("加载").clicked() {
                if let Err(message) = self.load_elf() {
                    actions.push(PluginAction::Toast {
                        level: ToastLevel::Error,
                        message,
                    });
                }
            }
            if ui.button("追踪").clicked() {
                self.trace_variables(pool, actions);
            }
        });
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(2.0);
        let is_popped = self.target.as_ref().is_some_and(|target| target.popped);
        let mut close_requested = false;
        let mut pop_requested = None;
        ui.horizontal(|ui| {
            ui.heading("变量列表 (DWARF Tree)");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("关闭").clicked() {
                    close_requested = true;
                }
                let pop_label = if is_popped { "Pop in" } else { "Pop out" };
                if ui.button(pop_label).clicked() {
                    pop_requested = Some(!is_popped);
                }
            });
        });
        if close_requested {
            self.target = None;
        } else if let Some(popped) = pop_requested
            && let Some(target) = self.target.as_mut()
        {
            target.popped = popped;
            self.drag_state = None;
        }
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);

        let remaining_height = ui.available_height().max(0.0);
        let total_width = ui.available_width().max(0.0);
        let right_width = (total_width * 0.32).clamp(220.0, 350.0);
        let left_width = (total_width - right_width - 8.0).max(200.0);
        ui.horizontal(|ui| {
            let (left_rect, _) = ui.allocate_exact_size(
                egui::vec2(left_width, remaining_height),
                egui::Sense::hover(),
            );
            let mut left_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(left_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            egui::ScrollArea::both()
                .id_salt(("left_tree_scroll", ui.ctx().viewport_id()))
                .auto_shrink([false, false])
                .show(&mut left_ui, |ui| {
                    crate::ui::vari_tree_ui(ui, &mut self.dwarf_state);
                });
            ui.separator();
            let (right_rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), remaining_height),
                egui::Sense::hover(),
            );
            let mut right_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(right_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            egui::ScrollArea::both()
                .id_salt(("right_props_scroll", ui.ctx().viewport_id()))
                .auto_shrink([false, false])
                .show(&mut right_ui, |ui| {
                    self.show_selected_properties(ui, plugin, pool, actions);
                });
        });
    }

    fn show_selected_properties(
        &mut self,
        ui: &mut Ui,
        plugin: &mut dyn MemRWPlugin,
        pool: &mut VariablePool,
        actions: &mut Vec<PluginAction>,
    ) {
        let Some(node_id) = self.dwarf_state.selected_node else {
            ui.label("选择节点以查看属性");
            return;
        };
        let Some(node) = self.dwarf_state.find_node_ref(node_id) else {
            self.dwarf_state.selected_node = None;
            return;
        };
        let default_type = dwarf::types::basic_type_to_extend(&node.basic_type);
        let config =
            self.extend_configs
                .entry(node_id)
                .or_insert_with(|| dwarf::types::ExtendConfig {
                    name: String::new(),
                    address: 0,
                    ext_type: default_type,
                    size: node.size,
                    array_index: None,
                    array_count: None,
                });
        if let Some((count, _)) = self.dwarf_state.parent_array_info(node_id) {
            config.array_count = Some(count);
            if node.name.starts_with('[') {
                if let Ok(index) = node.name[1..node.name.len() - 1].parse::<u64>() {
                    if index < count && config.array_index != Some(index) {
                        config.array_index = Some(index);
                    }
                }
            }
            if config.array_index.is_none() {
                config.array_index = Some(0);
            }
            config.name = self.dwarf_state.compute_extend_name(node_id);
            config.address = self
                .dwarf_state
                .compute_extend_address(node_id)
                .unwrap_or(0);
        } else if config.name.is_empty() || config.name.contains('[') {
            config.name = self.dwarf_state.compute_extend_name(node_id);
            config.address = self
                .dwarf_state
                .compute_extend_address(node_id)
                .unwrap_or(0);
        }

        let added = crate::ui::vari_properties_ui(
            ui,
            node,
            config,
            plugin.supports_composite_variables(),
            |ui, default_name, current_config| {
                let mut candidate = || variable_candidate(node, current_config);
                match plugin.add_variable_ui(ui, node_id, default_name, &mut candidate, pool) {
                    Ok(added) => added,
                    Err(error) => {
                        ui.label(
                            egui::RichText::new(error).color(crate::ui::theme::danger_text(ui)),
                        );
                        false
                    }
                }
            },
        );
        if added {
            actions.push(PluginAction::RebuildSlots);
        }

        if let Some((_count, element_size)) = self.dwarf_state.parent_array_info(node_id) {
            if let Some(config) = self.extend_configs.get(&node_id) {
                let index = config.array_index.unwrap_or(0);
                let new_name = format!("[{index}]");
                let new_address = element_size.saturating_mul(index);
                if let Some(tree_node) = self.dwarf_state.find_node_mut(node_id) {
                    tree_node.name = new_name.clone();
                    tree_node.address = new_address;
                }
            }
        }
    }

    fn load_elf(&mut self) -> Result<(), String> {
        self.dwarf_state = load_dwarf_state(&self.elf_path)?;
        self.loaded_elf_path = Some(self.elf_path.trim().to_owned());
        self.program_generation = self.program_generation.wrapping_add(1).max(1);
        self.extend_configs.clear();
        Ok(())
    }

    pub fn load_program_path(&mut self, path: String) -> Result<(), String> {
        self.elf_path = path;
        self.load_elf()
    }

    pub fn loaded_elf_path(&self) -> Option<&str> {
        self.loaded_elf_path.as_deref()
    }

    pub fn program_generation(&self) -> u64 {
        self.program_generation
    }

    pub fn prepare_config(
        elf_path: &str,
        pool: &mut VariablePool,
    ) -> Result<dwarf::types::DwarfState, String> {
        if elf_path.trim().is_empty() && pool.iter().next().is_none() {
            return Ok(dwarf::types::DwarfState::new(Vec::new()));
        }
        let mut dwarf_state = load_dwarf_state(elf_path)?;
        let errors = trace_pool(&mut dwarf_state, pool);
        if errors.is_empty() {
            Ok(dwarf_state)
        } else {
            Err(format!("配置中的变量追踪失败:\n{}", errors.join("\n")))
        }
    }

    pub fn apply_config_source(&mut self, elf_path: String, dwarf_state: dwarf::types::DwarfState) {
        self.elf_path = elf_path;
        self.loaded_elf_path = (!self.elf_path.trim().is_empty()).then(|| self.elf_path.clone());
        self.program_generation = self.program_generation.wrapping_add(1).max(1);
        self.dwarf_state = dwarf_state;
        self.extend_configs.clear();
        self.target = None;
        self.drag_state = None;
    }

    pub fn trace_variables(&mut self, pool: &mut VariablePool, actions: &mut Vec<PluginAction>) {
        if let Err(message) = self.load_elf() {
            actions.push(PluginAction::Toast {
                level: ToastLevel::Error,
                message,
            });
            return;
        }

        let errors = trace_pool(&mut self.dwarf_state, pool);

        if errors.is_empty() {
            actions.push(PluginAction::Toast {
                level: ToastLevel::Success,
                message: "追踪完成，所有变量已更新".to_owned(),
            });
        } else {
            actions.extend(errors.into_iter().map(|message| PluginAction::Toast {
                level: ToastLevel::Error,
                message,
            }));
        }
        actions.push(PluginAction::RebuildSlots);
    }
}

fn variable_tree_window_title(plugin_id: &str) -> String {
    let owner = match plugin_id {
        "chart" => "Chart".to_owned(),
        "table" => "Table".to_owned(),
        _ => {
            let owner = plugin_id
                .chars()
                .filter(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
                })
                .collect::<String>();
            if owner.is_empty() {
                "Plugin".to_owned()
            } else {
                owner
            }
        }
    };
    format!("Variable Tree - {owner}")
}

fn load_dwarf_state(path: &str) -> Result<dwarf::types::DwarfState, String> {
    let path = path.trim().to_owned();
    let cus = dwarf::extract::load_elf(&path).map_err(|error| error.to_string())?;
    Ok(dwarf::types::DwarfState::new(cus))
}

fn trace_pool(dwarf_state: &mut dwarf::types::DwarfState, pool: &mut VariablePool) -> Vec<String> {
    let mut errors = Vec::new();
    for variable in pool.iter_mut() {
        let name = variable.name.clone();
        let path = dwarf::types::expand_bracket_path(&name);
        let node_ids = dwarf_state.trace_exact(&path);
        for &node_id in &node_ids {
            dwarf_state.apply_array_path(node_id, &path);
        }
        match node_ids.as_slice() {
            [node_id] => {
                if let Some(node) = dwarf_state.find_node_by_id(*node_id) {
                    let ext_type = dwarf::types::basic_type_to_extend(&node.basic_type);
                    variable.size = extend_type_size(&ext_type).unwrap_or(node.size);
                    variable.address = dwarf_state
                        .compute_extend_address(*node_id)
                        .unwrap_or(node.address);
                    variable.ext_type = ext_type;
                }
            }
            [] => errors.push(format!("\"{name}\": 未找到匹配")),
            matches => errors.push(format!("\"{name}\": 匹配到多个 ({}) 节点", matches.len())),
        }
    }
    errors
}

fn extend_type_size(ext_type: &dwarf::types::ExtendType) -> Option<u32> {
    use dwarf::types::ExtendType::*;
    match ext_type {
        U8 | I8 => Some(1),
        U16 | I16 => Some(2),
        U32 | I32 | Float => Some(4),
        U64 | I64 | Double => Some(8),
        Other => None,
    }
}

fn bottom_sheet_handle(ui: &mut Ui, drag_state: &mut Option<(f32, f32)>, height: f32) -> f32 {
    let width = ui.available_width().max(1.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 20.0), egui::Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    let colors = crate::ui::theme::palette(ui);
    let color = if response.dragged() {
        ui.visuals().widgets.active.bg_fill
    } else if response.hovered() {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        colors.border_strong
    };
    ui.painter().rect_filled(
        egui::Rect::from_center_size(rect.center(), egui::vec2(40.0, 4.0)),
        egui::CornerRadius::same(2),
        color,
    );

    if response.dragged() {
        if let Some(pointer) = response.interact_pointer_pos() {
            let (origin, initial_height) = *drag_state.get_or_insert((pointer.y, height));
            return initial_height + origin - pointer.y;
        }
    } else {
        *drag_state = None;
    }
    height
}

fn variable_candidate(
    node: &dwarf::types::TreeNode,
    config: &dwarf::types::ExtendConfig,
) -> Result<VariableCandidate, String> {
    if config.ext_type != dwarf::types::ExtendType::Other {
        return Ok(VariableCandidate {
            label: node.name.clone(),
            name: config.name.clone(),
            address: config.address,
            ext_type: config.ext_type.clone(),
            size: config.size,
            children: Vec::new(),
        });
    }
    materialize_candidate_node(node, node.name.clone(), config.name.clone(), config.address)
}

fn materialize_candidate_node(
    node: &dwarf::types::TreeNode,
    label: String,
    name: String,
    address: u64,
) -> Result<VariableCandidate, String> {
    let mut children = Vec::new();
    match &node.basic_type {
        dwarf::types::BasicType::ArrayElem(_, count) => {
            let prototype = node
                .children
                .first()
                .ok_or_else(|| format!("数组 {name} 缺少元素类型信息"))?;
            let stride = u64::from(prototype.size);
            if *count > 1 && stride == 0 {
                return Err(format!("数组 {name} 的元素大小为 0，无法展开"));
            }
            for index in 0..*count {
                let offset = index
                    .checked_mul(stride)
                    .ok_or_else(|| format!("数组 {name} 的元素偏移溢出"))?;
                let child_address = address
                    .checked_add(offset)
                    .ok_or_else(|| format!("数组 {name} 的元素地址溢出"))?;
                children.push(materialize_candidate_node(
                    prototype,
                    format!("[{index}]"),
                    format!("{name}[{index}]"),
                    child_address,
                )?);
            }
        }
        _ if !node.children.is_empty() => {
            for child in &node.children {
                let child_address = address
                    .checked_add(child.address)
                    .ok_or_else(|| format!("字段 {name}.{} 的地址溢出", child.name))?;
                let child_name = if child.name.starts_with('[') {
                    format!("{name}{}", child.name)
                } else {
                    format!("{name}.{}", child.name)
                };
                children.push(materialize_candidate_node(
                    child,
                    child.name.clone(),
                    child_name,
                    child_address,
                )?);
            }
        }
        _ => {}
    }
    let ext_type = if children.is_empty() {
        dwarf::types::basic_type_to_extend(&node.basic_type)
    } else {
        dwarf::types::ExtendType::Other
    };
    Ok(VariableCandidate {
        label,
        name,
        address,
        ext_type,
        size: node.size,
        children,
    })
}

#[cfg(test)]
mod tests {
    use eframe::egui;

    use crate::dwarf::types::{BasicType, ExtendConfig, ExtendType, TreeNode};

    use super::{
        VARIABLE_TREE_LAYER_ORDER, VariableTreePanel, variable_candidate,
        variable_tree_window_title,
    };

    #[test]
    fn native_variable_tree_titles_are_ascii_only() {
        assert_eq!(variable_tree_window_title("chart"), "Variable Tree - Chart");
        assert_eq!(variable_tree_window_title("table"), "Variable Tree - Table");
        assert_eq!(variable_tree_window_title("插件-x"), "Variable Tree - -x");
        assert!(variable_tree_window_title("插件").is_ascii());
    }

    #[test]
    fn variable_tree_keeps_foreground_available_for_toasts() {
        assert!(VARIABLE_TREE_LAYER_ORDER < egui::Order::Foreground);
    }

    #[test]
    fn popped_tree_no_longer_blocks_or_closes_with_its_host_viewport() {
        let mut panel = VariableTreePanel::new(crate::dwarf::types::DwarfState::new(Vec::new()));
        let viewport_id = egui::ViewportId::ROOT;
        panel.open("chart", viewport_id);
        assert!(panel.is_open_in(viewport_id));

        panel.target.as_mut().unwrap().popped = true;
        assert!(!panel.is_open_in(viewport_id));

        panel.close_in(viewport_id);
        assert!(panel.target.is_some());
    }

    #[test]
    fn opening_tree_preserves_popout_and_retargets_the_requesting_plugin() {
        let mut panel = VariableTreePanel::new(crate::dwarf::types::DwarfState::new(Vec::new()));
        panel.open("chart", egui::ViewportId::ROOT);
        panel.target.as_mut().unwrap().popped = true;
        let table_viewport = egui::ViewportId::from_hash_of("table_viewport");

        panel.open("table", table_viewport);

        let target = panel.target.as_ref().unwrap();
        assert!(target.popped);
        assert_eq!(target.plugin_id, "table");
        assert_eq!(target.viewport_id, table_viewport);
    }

    fn node(
        id: usize,
        name: &str,
        basic_type: BasicType,
        address: u64,
        size: u32,
        children: Vec<TreeNode>,
    ) -> TreeNode {
        TreeNode {
            id,
            parent_id: None,
            name: name.to_owned(),
            type_name: String::new(),
            basic_type,
            address,
            size,
            children,
        }
    }

    #[test]
    fn materializes_every_array_element_from_the_prototype() {
        let prototype = node(4, "[7]", BasicType::U16, 99, 2, Vec::new());
        let array = node(
            3,
            "samples",
            BasicType::ArrayElem(Box::new(BasicType::U16), 3),
            4,
            6,
            vec![prototype],
        );
        let root = node(
            1,
            "state",
            BasicType::Struct("State".to_owned()),
            0x2000_0000,
            10,
            vec![node(2, "value", BasicType::U32, 0, 4, Vec::new()), array],
        );
        let config = ExtendConfig {
            name: "state".to_owned(),
            address: 0x2000_0000,
            ext_type: ExtendType::Other,
            size: 10,
            array_index: None,
            array_count: None,
        };
        let candidate = variable_candidate(&root, &config).unwrap();
        let samples = &candidate.children[1];
        assert_eq!(samples.children.len(), 3);
        assert_eq!(samples.children[0].name, "state.samples[0]");
        assert_eq!(samples.children[0].address, 0x2000_0004);
        assert_eq!(samples.children[1].address, 0x2000_0006);
        assert_eq!(samples.children[2].address, 0x2000_0008);
    }

    #[test]
    fn rejects_array_address_overflow() {
        let root = node(
            1,
            "samples",
            BasicType::ArrayElem(Box::new(BasicType::U32), 2),
            u64::MAX - 1,
            8,
            vec![node(2, "[0]", BasicType::U32, 0, 4, Vec::new())],
        );
        let config = ExtendConfig {
            name: "samples".to_owned(),
            address: u64::MAX - 1,
            ext_type: ExtendType::Other,
            size: 8,
            array_index: None,
            array_count: None,
        };
        assert!(variable_candidate(&root, &config).is_err());
    }

    #[test]
    fn routes_the_panel_to_the_requesting_viewport() {
        let mut panel = VariableTreePanel::new(crate::dwarf::types::DwarfState::new(Vec::new()));
        let popout = egui::ViewportId::from_hash_of("chart_popout");

        panel.open("chart", popout);

        assert!(panel.is_open_in(popout));
        assert!(!panel.is_open_in(egui::ViewportId::ROOT));
        panel.close_in(popout);
        assert!(!panel.is_open_in(popout));
    }
}
