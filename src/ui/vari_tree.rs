use std::collections::HashSet;

use eframe::egui::{self, Ui};
use egui_ltreeview::{Action, RowLayout, TreeView, TreeViewState, NodeBuilder};

use egui_ltreeview::TreeViewBuilder;
use crate::types::{DwarfApp, TreeNode};

pub fn vari_tree_ui(ui: &mut Ui, app: &mut DwarfApp) {
    ui.horizontal(|ui| {
        let text_response = ui.text_edit_singleline(&mut app.search_text);
        if ui.button("Search").clicked()
            || (text_response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
        {
            app.perform_search();
            app.search_mode = true;
        }
    });

    let prev_mode = app.search_mode;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut app.search_mode, false, "All");
        ui.selectable_value(&mut app.search_mode, true, "Search");
    });
    if prev_mode && !app.search_mode {
        app.needs_all_reset = true;
    }

    if app.search_mode && app.search_results.is_empty() && !app.search_text.trim().is_empty() {
        ui.label("No matching results");
    }

    let scroll_target = app.scroll_target_id.take();
    let viewport_h = ui.available_height();

    let final_offset = scroll_target.map(|target_id| {
        let row_h = 24.0;
        let count_before = app.count_nodes_before(target_id);
        ((count_before as f32 * row_h) - viewport_h / 2.0 + row_h / 2.0).max(0.0)
    });

    let mut scroll_area = egui::ScrollArea::vertical().id_salt("vari_tree_scroll");
    if let Some(offset) = final_offset {
        scroll_area = scroll_area.vertical_scroll_offset(offset);
    }
    scroll_area.show(ui, |ui| {
        show_tree(ui, app);
    });
}

fn show_tree(ui: &mut Ui, app: &mut DwarfApp) {
    if app.needs_all_reset {
        *app.tree_state.borrow_mut() = TreeViewState::default();
        app.needs_all_reset = false;
    }

    let highlight: Option<&HashSet<usize>> = if app.search_mode {
        Some(&app.search_results)
    } else {
        None
    };

    let (_response, actions) = TreeView::new(ui.make_persistent_id("dwarf_tree"))
        .row_layout(RowLayout::Compact)
        .show_state(ui, &mut *app.tree_state.borrow_mut(), |builder| {
            for cu in &app.cus {
                if cu.variables.is_empty() { continue; }
                if app.search_mode && !app.cu_has_result(cu) { continue; }
                let cu_name = cu.cu_name.clone();
                builder.node(NodeBuilder::dir(cu.dir_id)
                    .label_ui(move |ui| { ui.add(egui::Label::new(cu_name.clone()).selectable(false)); })
                    .default_open(false));
                for var in &cu.variables { build_node_recursive(builder, var, highlight); }
                builder.close_dir();
            }
        });

    for action in actions {
        if let Action::SetSelected(ids) = action {
            app.selected_node = ids.first().and_then(|id| app.find_node_by_id(*id));
        }
    }
}

fn build_node_recursive(
    builder: &mut TreeViewBuilder<usize>,
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
        builder.leaf(node.id, label);
    } else {
        builder.node(NodeBuilder::dir(node.id)
            .label_ui(move |ui| { ui.add(egui::Label::new(label.clone()).selectable(false)); })
            .default_open(false));
        for child in &node.children {
            build_node_recursive(builder, child, highlight_ids);
        }
        builder.close_dir();
    }
}
