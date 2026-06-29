use std::collections::HashMap;

use eframe::egui::{self, Ui};

use crate::model::VariablePool;
use crate::ui::plugin::{FrameData, MemRWPlugin, PluginAction, PluginRenderContext};

#[derive(Debug, Clone)]
pub struct DockLayoutState {
    popped: HashMap<String, bool>,
    split_ratio: f32,
    split_drag_start: Option<(f32, f32)>,
}

impl Default for DockLayoutState {
    fn default() -> Self {
        Self {
            popped: HashMap::new(),
            split_ratio: 0.5,
            split_drag_start: None,
        }
    }
}

impl DockLayoutState {
    fn is_popped(&self, plugin_id: &str) -> bool {
        self.popped.get(plugin_id).copied().unwrap_or(false)
    }

    fn set_popped(&mut self, plugin_id: &str, popped: bool) {
        self.popped.insert(plugin_id.to_owned(), popped);
    }
}

pub fn show_plugins_dock(
    ui: &mut Ui,
    dock: &mut DockLayoutState,
    plugins: &mut [Box<dyn MemRWPlugin>],
    pool: &VariablePool,
    frame_data: &FrameData,
    running: bool,
) -> Vec<PluginAction> {
    let mut actions = Vec::new();

    match plugins.len() {
        0 => {
            ui.centered_and_justified(|ui| {
                ui.label("没有已加载的插件。");
            });
        }
        1 => {
            let plugin = plugins[0].as_mut();
            if dock.is_popped(plugin.id()) {
                show_empty_dock(ui);
            } else {
                show_plugin_docked(ui, dock, plugin, pool, frame_data, running, &mut actions);
            }
        }
        _ => {
            let (left_slice, right_slice) = plugins.split_at_mut(1);
            let left = left_slice[0].as_mut();
            let right = right_slice[0].as_mut();
            let left_docked = !dock.is_popped(left.id());
            let right_docked = !dock.is_popped(right.id());

            match (left_docked, right_docked) {
                (true, true) => show_split_dock(
                    ui,
                    dock,
                    left,
                    right,
                    pool,
                    frame_data,
                    running,
                    &mut actions,
                ),
                (true, false) => {
                    show_plugin_docked(ui, dock, left, pool, frame_data, running, &mut actions)
                }
                (false, true) => {
                    show_plugin_docked(ui, dock, right, pool, frame_data, running, &mut actions)
                }
                (false, false) => show_empty_dock(ui),
            }
        }
    }

    show_popout_viewports(ui, dock, plugins, pool, frame_data, running, &mut actions);

    actions
}

fn show_empty_dock(ui: &mut Ui) {
    ui.centered_and_justified(|ui| {
        ui.label("所有插件已弹出为独立窗口，可在窗口内点击 Pop in 返回主区域。");
    });
}

fn show_split_dock(
    ui: &mut Ui,
    dock: &mut DockLayoutState,
    left_plugin: &mut dyn MemRWPlugin,
    right_plugin: &mut dyn MemRWPlugin,
    pool: &VariablePool,
    frame_data: &FrameData,
    running: bool,
    actions: &mut Vec<PluginAction>,
) {
    let available = ui.available_size();
    let splitter_w = 6.0;
    let content_w = (available.x - splitter_w).max(0.0);
    let left_w = if content_w >= 320.0 {
        (content_w * dock.split_ratio).clamp(160.0, content_w - 160.0)
    } else {
        content_w * dock.split_ratio
    };
    let right_w = if content_w >= 320.0 {
        (content_w - left_w).max(160.0)
    } else {
        content_w - left_w
    };

    let (dock_rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());
    let left_rect =
        egui::Rect::from_min_size(dock_rect.min, egui::vec2(left_w, dock_rect.height()));
    let splitter_rect = egui::Rect::from_min_size(
        egui::pos2(left_rect.max.x, dock_rect.min.y),
        egui::vec2(splitter_w, dock_rect.height()),
    );
    let right_rect = egui::Rect::from_min_size(
        egui::pos2(splitter_rect.max.x, dock_rect.min.y),
        egui::vec2(right_w, dock_rect.height()),
    );

    let mut left_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    left_ui.set_clip_rect(left_rect);
    egui::Frame::group(ui.style()).show(&mut left_ui, |ui| {
        ui.set_height(left_rect.height());
        if dock_control_bar(ui, "Pop out") {
            dock.set_popped(left_plugin.id(), true);
        }
        render_plugin_content(ui, left_plugin, pool, frame_data, running, actions);
    });

    show_splitter(ui, dock, splitter_rect, content_w);

    let mut right_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(right_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    right_ui.set_clip_rect(right_rect);
    egui::Frame::group(ui.style()).show(&mut right_ui, |ui| {
        ui.set_height(right_rect.height());
        if dock_control_bar(ui, "Pop out") {
            dock.set_popped(right_plugin.id(), true);
        }
        render_plugin_content(ui, right_plugin, pool, frame_data, running, actions);
    });
}

fn show_plugin_docked(
    ui: &mut Ui,
    dock: &mut DockLayoutState,
    plugin: &mut dyn MemRWPlugin,
    pool: &VariablePool,
    frame_data: &FrameData,
    running: bool,
    actions: &mut Vec<PluginAction>,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_height(ui.available_height());
        if dock_control_bar(ui, "Pop out") {
            dock.set_popped(plugin.id(), true);
        }
        render_plugin_content(ui, plugin, pool, frame_data, running, actions);
    });
}

fn show_splitter(
    ui: &mut Ui,
    dock: &mut DockLayoutState,
    splitter_rect: egui::Rect,
    content_w: f32,
) {
    let splitter_id = ui.make_persistent_id("plugin_splitter");
    let response = ui.interact(splitter_rect, splitter_id, egui::Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if response.dragged() && content_w > 0.0 {
        if let Some(pointer) = response.interact_pointer_pos() {
            let (origin_x, initial_ratio) = dock
                .split_drag_start
                .unwrap_or((pointer.x, dock.split_ratio));
            dock.split_drag_start = Some((origin_x, initial_ratio));
            dock.split_ratio = (initial_ratio + (pointer.x - origin_x) / content_w).clamp(0.2, 0.8);
        }
    } else {
        dock.split_drag_start = None;
    }
    ui.painter().rect_filled(
        splitter_rect.shrink2(egui::vec2(2.0, 0.0)),
        egui::CornerRadius::same(2),
        ui.visuals().widgets.noninteractive.bg_stroke.color,
    );
}

fn show_popout_viewports(
    ui: &mut Ui,
    dock: &mut DockLayoutState,
    plugins: &mut [Box<dyn MemRWPlugin>],
    pool: &VariablePool,
    frame_data: &FrameData,
    running: bool,
    actions: &mut Vec<PluginAction>,
) {
    for plugin in plugins.iter_mut() {
        if !dock.is_popped(plugin.id()) {
            continue;
        }

        let plugin_id = plugin.id().to_owned();
        let title = plugin.title().to_owned();
        let keep_popped = ui.ctx().show_viewport_immediate(
            egui::ViewportId::from_hash_of(format!("{plugin_id}_popout_viewport")),
            egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size(plugin.viewport_size())
                .with_min_inner_size(plugin.min_viewport_size())
                .with_resizable(true),
            |viewport_ui, _class| {
                if viewport_ui.ctx().input(|i| i.viewport().close_requested()) {
                    return false;
                }
                let mut pop_in = false;
                egui::CentralPanel::default().show_inside(viewport_ui, |ui| {
                    if dock_control_bar(ui, "Pop in") {
                        pop_in = true;
                    }
                    render_plugin_content(ui, plugin.as_mut(), pool, frame_data, running, actions);
                });
                !pop_in
            },
        );
        if !keep_popped {
            dock.set_popped(&plugin_id, false);
        }
    }
}

fn render_plugin_content(
    ui: &mut Ui,
    plugin: &mut dyn MemRWPlugin,
    pool: &VariablePool,
    frame_data: &FrameData,
    running: bool,
    actions: &mut Vec<PluginAction>,
) {
    actions.extend(plugin.render(
        ui,
        PluginRenderContext {
            pool,
            frame_data,
            running,
        },
    ));
}

fn dock_control_bar(ui: &mut Ui, button: &str) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            clicked = ui.button(button).clicked();
        });
    });
    ui.separator();
    clicked
}
