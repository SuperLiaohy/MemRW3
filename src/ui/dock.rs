use std::collections::HashMap;

use eframe::egui::{self, Ui};

use crate::model::{DockTab, VariablePool};
use crate::ui::chart_plugin::{self, ChartPluginState};
use crate::ui::table_plugin::{self, TablePluginState};

pub type FrameData = HashMap<usize, Vec<(f64, [u8; 8])>>;

#[derive(Debug, Clone)]
pub struct DockLayoutState {
    chart_popped: bool,
    table_popped: bool,
    split_ratio: f32,
    split_drag_start: Option<(f32, f32)>,
}

impl Default for DockLayoutState {
    fn default() -> Self {
        Self {
            chart_popped: false,
            table_popped: false,
            split_ratio: 0.5,
            split_drag_start: None,
        }
    }
}

pub fn show_chart_table_dock(
    ui: &mut Ui,
    dock: &mut DockLayoutState,
    chart_state: &mut ChartPluginState,
    table_state: &mut TablePluginState,
    pool: &VariablePool,
    frame_data: &FrameData,
    running: bool,
    open_tree: &mut Option<DockTab>,
) {
    let chart_docked = !dock.chart_popped;
    let table_docked = !dock.table_popped;

    match (chart_docked, table_docked) {
        (true, true) => show_split_dock(
            ui,
            dock,
            chart_state,
            table_state,
            pool,
            frame_data,
            running,
            open_tree,
        ),
        (true, false) => {
            show_chart_docked(ui, dock, chart_state, pool, frame_data, running, open_tree)
        }
        (false, true) => show_table_docked(ui, dock, table_state, pool, frame_data, open_tree),
        (false, false) => {
            ui.centered_and_justified(|ui| {
                ui.label("Chart 和 Table 已弹出为独立窗口，可在窗口内点击 Pop in 返回主区域。");
            });
        }
    }

    show_popout_viewports(
        ui,
        dock,
        chart_state,
        table_state,
        pool,
        frame_data,
        running,
        open_tree,
    );
}

fn show_split_dock(
    ui: &mut Ui,
    dock: &mut DockLayoutState,
    chart_state: &mut ChartPluginState,
    table_state: &mut TablePluginState,
    pool: &VariablePool,
    frame_data: &FrameData,
    running: bool,
    open_tree: &mut Option<DockTab>,
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
            dock.chart_popped = true;
        }
        render_chart_content(ui, chart_state, pool, frame_data, running, open_tree);
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
            dock.table_popped = true;
        }
        render_table_content(ui, table_state, pool, frame_data, open_tree);
    });
}

fn show_chart_docked(
    ui: &mut Ui,
    dock: &mut DockLayoutState,
    chart_state: &mut ChartPluginState,
    pool: &VariablePool,
    frame_data: &FrameData,
    running: bool,
    open_tree: &mut Option<DockTab>,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_height(ui.available_height());
        if dock_control_bar(ui, "Pop out") {
            dock.chart_popped = true;
        }
        render_chart_content(ui, chart_state, pool, frame_data, running, open_tree);
    });
}

fn show_table_docked(
    ui: &mut Ui,
    dock: &mut DockLayoutState,
    table_state: &mut TablePluginState,
    pool: &VariablePool,
    frame_data: &FrameData,
    open_tree: &mut Option<DockTab>,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_height(ui.available_height());
        if dock_control_bar(ui, "Pop out") {
            dock.table_popped = true;
        }
        render_table_content(ui, table_state, pool, frame_data, open_tree);
    });
}

fn show_splitter(
    ui: &mut Ui,
    dock: &mut DockLayoutState,
    splitter_rect: egui::Rect,
    content_w: f32,
) {
    let splitter_id = ui.make_persistent_id("chart_table_splitter");
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
    chart_state: &mut ChartPluginState,
    table_state: &mut TablePluginState,
    pool: &VariablePool,
    frame_data: &FrameData,
    running: bool,
    open_tree: &mut Option<DockTab>,
) {
    if dock.chart_popped {
        let keep_popped = ui.ctx().show_viewport_immediate(
            egui::ViewportId::from_hash_of("chart_popout_viewport"),
            egui::ViewportBuilder::default()
                .with_title("Chart 实时数据")
                .with_inner_size(egui::vec2(720.0, 420.0))
                .with_min_inner_size(egui::vec2(360.0, 240.0))
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
                    render_chart_content(ui, chart_state, pool, frame_data, running, open_tree);
                });
                !pop_in
            },
        );
        if !keep_popped {
            dock.chart_popped = false;
        }
    }

    if dock.table_popped {
        let keep_popped = ui.ctx().show_viewport_immediate(
            egui::ViewportId::from_hash_of("table_popout_viewport"),
            egui::ViewportBuilder::default()
                .with_title("Table 读写数据")
                .with_inner_size(egui::vec2(520.0, 360.0))
                .with_min_inner_size(egui::vec2(320.0, 220.0))
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
                    render_table_content(ui, table_state, pool, frame_data, open_tree);
                });
                !pop_in
            },
        );
        if !keep_popped {
            dock.table_popped = false;
        }
    }
}

fn render_chart_content(
    ui: &mut Ui,
    chart_state: &mut ChartPluginState,
    pool: &VariablePool,
    frame_data: &FrameData,
    running: bool,
    open_tree: &mut Option<DockTab>,
) {
    let action = chart_plugin::chart_panel(ui, chart_state, pool, frame_data, running);
    if action == chart_plugin::PanelAction::OpenTree {
        *open_tree = Some(DockTab::Chart);
    }
}

fn render_table_content(
    ui: &mut Ui,
    table_state: &mut TablePluginState,
    pool: &VariablePool,
    frame_data: &FrameData,
    open_tree: &mut Option<DockTab>,
) {
    let action = table_plugin::table_panel(ui, table_state, pool, frame_data);
    if action == table_plugin::PanelAction::OpenTree {
        *open_tree = Some(DockTab::Table);
    }
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
