use crate::types::*;
pub use crate::types::DwarfApp;
use eframe::egui;
use eframe;
use egui_ltreeview::{Action, NodeBuilder, RowLayout, TreeView, TreeViewState};
use std::collections::HashSet;

impl eframe::App for DwarfApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::left("tree_panel")
            .resizable(true)
            .default_size(500.0)
            .show_inside(ui, |ui| {
                // Search bar
                ui.horizontal(|ui| {
                    let text_response = ui.text_edit_singleline(&mut self.search_text);
                    if ui.button("Search").clicked()
                        || (text_response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        self.perform_search();
                        self.search_mode = true;
                    }
                });
                // All / Search toggle
                let prev_mode = self.search_mode;
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.search_mode, false, "All");
                    ui.selectable_value(&mut self.search_mode, true, "Search");
                });
                if prev_mode && !self.search_mode {
                    self.needs_all_reset = true;
                }
                // No results message in search mode
                if self.search_mode && self.search_results.is_empty() && !self.search_text.trim().is_empty() {
                    ui.label("No matching results");
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.show_tree_view(ui);
                });
            });
        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.show_properties(ui);
        });
    }
}

impl DwarfApp {
    pub fn show_tree_view(&mut self, ui: &mut egui::Ui) {
        if self.needs_all_reset {
            *self.tree_state.borrow_mut() = TreeViewState::default();
            self.needs_all_reset = false;
        }

        let highlight: Option<&HashSet<usize>> = if self.search_mode {
            Some(&self.search_results)
        } else {
            None
        };

        let (_response, actions) = TreeView::new(ui.make_persistent_id("dwarf_tree"))
            .row_layout(RowLayout::Compact)
            .show_state(ui, &mut *self.tree_state.borrow_mut(), |builder| {
                for cu in &self.cus {
                    if cu.variables.is_empty() {
                        continue;
                    }
                    if self.search_mode && !self.cu_has_result(cu) {
                        continue;
                    }
                    builder.node(NodeBuilder::dir(cu.dir_id).default_open(false).label(&cu.cu_name));
                    for var in &cu.variables {
                        self.build_tree_recursive(builder, var, highlight);
                    }
                    builder.close_dir();
                }
            });
        for action in actions {
            match action {
                Action::SetSelected(ids) => {
                    self.selected_node = ids.first().and_then(|id| self.find_node_by_id(*id));
                }
                _ => {}
            }
        }
    }

    pub fn build_tree_recursive(
        &self,
        builder: &mut egui_ltreeview::TreeViewBuilder<usize>,
        node: &TreeNode,
        highlight_ids: Option<&HashSet<usize>>,
    ) {
        let label: egui::WidgetText = if highlight_ids.map_or(false, |h| h.contains(&node.id)) {
            egui::RichText::new(&node.name)
                .background_color(egui::Color32::from_rgb(80, 80, 160))
                .into()
        } else {
            egui::RichText::new(&node.name).into()
        };

        if node.children.is_empty() {
            builder.node(NodeBuilder::leaf(node.id).label(label));
        } else {
            builder.node(NodeBuilder::dir(node.id).default_open(false).label(label));
            for child in &node.children {
                self.build_tree_recursive(builder, child, highlight_ids);
            }
            builder.close_dir();
        }
    }

    pub fn find_node_by_id(&self, id: usize) -> Option<TreeNode> {
        for cu in &self.cus {
            for var in &cu.variables {
                if let Some(node) = self.find_in_tree(var, id) {
                    return Some(node);
                }
            }
        }
        None
    }

    fn find_in_tree(&self, node: &TreeNode, id: usize) -> Option<TreeNode> {
        if node.id == id {
            return Some(node.clone());
        }
        for child in &node.children {
            if let Some(found) = self.find_in_tree(child, id) {
                return Some(found);
            }
        }
        None
    }

    pub fn perform_search(&mut self) {
        self.search_results.clear();
        self.search_path_nodes.clear();

        let query = self.search_text.trim();
        if query.is_empty() {
            return;
        }

        let levels: Vec<&str> = query.split('.').collect();

        for cu in &self.cus {
            for var in &cu.variables {
                let level0 = &levels[0];
                    if !node_name_matches(&var.name, level0) {
                        let results = self.search_in_tree(var, &levels, 0, &mut Vec::new());
                        for path in results {
                            self.search_results
                                .insert(path.last().copied().unwrap_or(var.id));
                        for &id in &path {
                            self.search_path_nodes.insert(id);
                        }
                    }
                    continue;
                }

                if levels.len() == 1 {
                    self.search_results.insert(var.id);
                    self.search_path_nodes.insert(var.id);
                } else {
                    let results =
                        self.search_in_tree(var, &levels, 1, &mut vec![var.id]);
                    for path in results {
                        self.search_results
                            .insert(path.last().copied().unwrap_or(var.id));
                        for &id in &path {
                            self.search_path_nodes.insert(id);
                        }
                    }
                }
            }
        }

        for cu in &self.cus {
            if self.cu_has_result(cu) {
                self.search_path_nodes.insert(cu.dir_id);
            }
        }

        for &id in &self.search_path_nodes {
            self.tree_state.borrow_mut().set_openness(id, true);
        }
        if let Some(&first_id) = self.search_results.iter().next() {
            let all: Vec<usize> = self.search_results.iter().copied().collect();
            self.tree_state.borrow_mut().set_selected(all);
            self.selected_node = self.find_node_by_id(first_id);
        }
    }

    fn search_in_tree(
        &self,
        node: &TreeNode,
        levels: &[&str],
        level_idx: usize,
        path: &mut Vec<usize>,
    ) -> Vec<Vec<usize>> {
        path.push(node.id);
        let mut results = Vec::new();

        if level_idx >= levels.len() {
            results.push(path.clone());
            path.pop();
            return results;
        }

        let target = levels[level_idx];
        for child in &node.children {
            if node_name_matches(&child.name, target) {
                if level_idx + 1 == levels.len() {
                    let mut full_path = path.clone();
                    full_path.push(child.id);
                    results.push(full_path);
                } else {
                    let child_results =
                        self.search_in_tree(child, levels, level_idx + 1, &mut path.clone());
                    results.extend(child_results);
                }
            } else {
                let child_results =
                    self.search_in_tree(child, levels, level_idx, &mut path.clone());
                results.extend(child_results);
            }
        }

        path.pop();
        results
    }

    fn cu_has_result(&self, cu: &CuInfo) -> bool {
        cu.variables.iter().any(|v| self.tree_has_result(v))
    }

    fn tree_has_result(&self, node: &TreeNode) -> bool {
        if self.search_results.contains(&node.id) || self.search_path_nodes.contains(&node.id) {
            return true;
        }
        node.children.iter().any(|c| self.tree_has_result(c))
    }

    pub fn show_properties(&self, ui: &mut egui::Ui) {
        ui.heading("Properties");
        ui.separator();
        if let Some(ref node) = self.selected_node {
            egui::Grid::new("props").striped(true).show(ui, |ui| {
                ui.label("Name:");
                ui.label(&node.name);
                ui.end_row();
                ui.label("Type:");
                ui.label(&node.type_name);
                ui.end_row();
                ui.label("Address:");
                ui.label(&node.address_info);
                ui.end_row();
                ui.label("Size:");
                ui.label(&node.size_info);
                ui.end_row();
                ui.label("Children:");
                ui.label(format!("{}", node.children.len()));
                ui.end_row();
            });
        } else {
            ui.label("Select a node in the tree to view properties.");
        }
    }
}

pub fn node_name_matches(name: &str, query: &str) -> bool {
    name.to_lowercase().contains(&query.to_lowercase())
}
