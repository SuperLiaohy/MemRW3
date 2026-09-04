use std::collections::HashMap;

use eframe::egui::{self, RichText, Ui};

use crate::model::VariablePool;
use crate::ui::plugin::{MemRWPlugin, PluginAction, PluginRenderContext};
use crate::ui::theme;
use crate::ui::variable_tree_panel::VariableTreePanel;

#[derive(Debug, Clone, Default)]
pub struct DockLayoutState {
    popped: HashMap<String, bool>,
    paused: HashMap<String, bool>,
    active_plugin: Option<String>,
    pending_pop_in: Option<String>,
}

impl DockLayoutState {
    fn is_popped(&self, plugin_id: &str) -> bool {
        self.popped.get(plugin_id).copied().unwrap_or(false)
    }

    fn set_popped(&mut self, plugin_id: &str, popped: bool) {
        self.popped.insert(plugin_id.to_owned(), popped);
    }

    pub fn is_plugin_paused(&self, plugin_id: &str) -> bool {
        self.paused.get(plugin_id).copied().unwrap_or(false)
    }

    fn toggle_plugin_paused(&mut self, plugin_id: &str) {
        let paused = !self.is_plugin_paused(plugin_id);
        self.paused.insert(plugin_id.to_owned(), paused);
    }

    fn active_plugin_id(&self) -> Option<&str> {
        self.active_plugin.as_deref()
    }

    fn set_active_plugin(&mut self, plugin_id: impl Into<String>) {
        self.active_plugin = Some(plugin_id.into());
    }

    pub fn focus_plugin(&mut self, plugin_id: &str) {
        self.set_popped(plugin_id, false);
        self.set_active_plugin(plugin_id);
    }

    fn request_pop_in(&mut self, plugin_id: &str) {
        self.pending_pop_in = Some(plugin_id.to_owned());
    }

    fn confirm_pop_in(&mut self) {
        if let Some(plugin_id) = self.pending_pop_in.take() {
            self.focus_plugin(&plugin_id);
        }
    }
}

pub fn show_plugin_activity_bar(
    ui: &mut Ui,
    dock: &mut DockLayoutState,
    plugins: &mut [Box<dyn MemRWPlugin>],
) {
    if plugins.is_empty() {
        return;
    }
    ensure_active_plugin(dock, plugins);
    show_activity_bar(ui, dock, plugins);
    show_pop_in_confirmation(ui.ctx(), dock, plugins);
}

pub fn show_active_plugin_content(
    ui: &mut Ui,
    dock: &mut DockLayoutState,
    plugins: &mut [Box<dyn MemRWPlugin>],
    pool: &mut VariablePool,
    running: bool,
    interaction_enabled: bool,
    variable_tree: &mut VariableTreePanel,
) -> Vec<PluginAction> {
    let mut actions = Vec::new();

    if plugins.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label("没有已加载的插件。");
        });
        return actions;
    }

    ensure_active_plugin(dock, plugins);
    let active_id = dock.active_plugin.clone();
    let Some(active_idx) = active_id
        .as_deref()
        .and_then(|id| plugins.iter().position(|plugin| plugin.id() == id))
    else {
        show_empty_dock(ui);
        return actions;
    };

    let plugin = plugins[active_idx].as_mut();
    show_plugin_docked(
        ui,
        dock,
        plugin,
        pool,
        running,
        interaction_enabled,
        variable_tree,
        &mut actions,
    );
    actions
}

pub fn show_plugin_popouts(
    ui: &mut Ui,
    dock: &mut DockLayoutState,
    plugins: &mut [Box<dyn MemRWPlugin>],
    pool: &mut VariablePool,
    running: bool,
    interaction_enabled: bool,
    variable_tree: &mut VariableTreePanel,
) -> Vec<PluginAction> {
    let mut actions = Vec::new();
    show_popout_viewports(
        ui,
        dock,
        plugins,
        pool,
        running,
        interaction_enabled,
        variable_tree,
        &mut actions,
    );
    actions
}

fn show_empty_dock(ui: &mut Ui) {
    ui.centered_and_justified(|ui| {
        ui.label("所有插件已弹出为独立窗口，可在窗口内点击 Pop in 返回主区域。");
    });
}

fn ensure_active_plugin(dock: &mut DockLayoutState, plugins: &[Box<dyn MemRWPlugin>]) {
    let active_is_available = dock.active_plugin_id().is_some_and(|active_id| {
        plugins
            .iter()
            .any(|plugin| plugin.id() == active_id && !dock.is_popped(plugin.id()))
    });

    if active_is_available {
        return;
    }

    dock.active_plugin = plugins
        .iter()
        .find(|plugin| !dock.is_popped(plugin.id()))
        .map(|plugin| plugin.id().to_owned());
}

fn show_activity_bar(ui: &mut Ui, dock: &mut DockLayoutState, plugins: &[Box<dyn MemRWPlugin>]) {
    let colors = theme::palette(ui);
    egui::Frame::NONE
        .fill(colors.sidebar_bg)
        .stroke(theme::panel_stroke(ui))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.set_height(ui.available_height());
            ui.add_space(6.0);

            for plugin in plugins {
                let plugin_id = plugin.id();
                let is_active = dock.active_plugin_id() == Some(plugin_id);
                let is_popped = dock.is_popped(plugin_id);
                let is_paused = dock.is_plugin_paused(plugin_id);
                let response =
                    activity_button(ui, plugin.as_ref(), is_active, is_popped, is_paused);
                let button_rect = response.rect;

                if response.clicked() {
                    if is_popped {
                        dock.request_pop_in(plugin_id);
                    } else {
                        dock.set_active_plugin(plugin_id);
                    }
                }

                if is_active {
                    let indicator = egui::Rect::from_min_size(
                        button_rect.left_center() - egui::vec2(0.0, 14.0),
                        egui::vec2(3.0, 28.0),
                    );
                    ui.painter()
                        .rect_filled(indicator, egui::CornerRadius::same(2), colors.accent);
                }
            }

            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                ui.add_space(6.0);
            });
        });
}

fn show_pop_in_confirmation(
    ctx: &egui::Context,
    dock: &mut DockLayoutState,
    plugins: &[Box<dyn MemRWPlugin>],
) {
    let Some(plugin_id) = dock.pending_pop_in.clone() else {
        return;
    };
    if !dock.is_popped(&plugin_id) {
        dock.pending_pop_in = None;
        return;
    }
    let title = plugins
        .iter()
        .find(|plugin| plugin.id() == plugin_id)
        .map(|plugin| plugin.title())
        .unwrap_or(plugin_id.as_str());
    let mut confirm = false;
    let mut cancel = false;
    let response = egui::Modal::new(egui::Id::new("confirm_plugin_pop_in")).show(ctx, |ui| {
        ui.set_min_width(300.0);
        ui.heading("确认返回主窗口");
        ui.label(format!("是否将“{title}”从独立窗口移回主窗口？"));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            cancel = ui.button("取消").clicked();
            confirm = ui.button("确认 Pop in").clicked();
        });
    });
    cancel |= response.backdrop_response.clicked()
        || ctx.input(|input| input.key_pressed(egui::Key::Escape));
    if confirm {
        dock.confirm_pop_in();
    } else if cancel {
        dock.pending_pop_in = None;
    }
}

fn activity_button(
    ui: &mut Ui,
    plugin: &dyn MemRWPlugin,
    is_active: bool,
    is_popped: bool,
    is_paused: bool,
) -> egui::Response {
    let colors = theme::palette(ui);
    let fill = if is_active {
        colors.accent_weak
    } else {
        egui::Color32::TRANSPARENT
    };
    let text_color = if is_paused {
        colors.text_muted
    } else if is_active {
        colors.accent_hover
    } else if is_popped {
        colors.text_muted
    } else {
        colors.text
    };

    ui.add_sized(
        [44.0, 42.0],
        egui::Button::new(
            RichText::new(plugin_icon(plugin.id(), plugin.title()))
                .size(20.0)
                .color(text_color),
        )
        .fill(fill)
        .frame(is_active),
    )
    .on_hover_text(if is_paused {
        format!("{} 已暂停，点击打开", plugin.title())
    } else if is_popped {
        format!("{} 已弹出，点击返回主区域", plugin.title())
    } else {
        plugin.title().to_owned()
    })
}

fn plugin_icon(plugin_id: &str, title: &str) -> String {
    match plugin_id {
        "chart" => "📈".to_owned(),
        "table" => "📋".to_owned(),
        "debug" => "🐞".to_owned(),
        _ => title.chars().next().unwrap_or('□').to_string(),
    }
}

fn native_window_title(plugin_id: &str, title: &str) -> String {
    match plugin_id {
        "chart" => "Chart".to_owned(),
        "table" => "Table".to_owned(),
        "debug" => "Debug".to_owned(),
        _ => {
            let title = title
                .chars()
                .filter(|ch| ch.is_ascii_graphic() || ch.is_ascii_whitespace())
                .collect::<String>();
            if title.trim().is_empty() {
                plugin_id.to_owned()
            } else {
                title.trim().to_owned()
            }
        }
    }
}

fn show_plugin_docked(
    ui: &mut Ui,
    dock: &mut DockLayoutState,
    plugin: &mut dyn MemRWPlugin,
    pool: &mut VariablePool,
    running: bool,
    interaction_enabled: bool,
    variable_tree: &mut VariableTreePanel,
    actions: &mut Vec<PluginAction>,
) {
    let colors = theme::palette(ui);
    egui::Frame::NONE
        .fill(colors.panel_bg)
        .stroke(theme::panel_stroke(ui))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_height(ui.available_height());
            let overlay_open = variable_tree.is_open_in(ui.ctx().viewport_id());
            let paused = dock.is_plugin_paused(plugin.id());
            let controls = dock_control_bar(ui, Some(plugin.title()), "Pop out", paused);
            if controls.toggle_paused {
                dock.toggle_plugin_paused(plugin.id());
                actions.extend(plugin.on_enabled_changed(!dock.is_plugin_paused(plugin.id())));
            }
            if controls.move_viewport {
                dock.set_popped(plugin.id(), true);
            } else {
                let paused = dock.is_plugin_paused(plugin.id());
                ui.add_enabled_ui(interaction_enabled && !overlay_open && !paused, |ui| {
                    render_plugin_content(ui, plugin, pool, running, actions);
                });
            }
            variable_tree.show(ui, plugin, pool, actions);
        });
}

fn show_popout_viewports(
    ui: &mut Ui,
    dock: &mut DockLayoutState,
    plugins: &mut [Box<dyn MemRWPlugin>],
    pool: &mut VariablePool,
    running: bool,
    interaction_enabled: bool,
    variable_tree: &mut VariableTreePanel,
    actions: &mut Vec<PluginAction>,
) {
    for plugin in plugins.iter_mut() {
        if !dock.is_popped(plugin.id()) {
            continue;
        }

        let plugin_id = plugin.id().to_owned();
        let viewport_id = egui::ViewportId::from_hash_of(format!("{plugin_id}_popout_viewport"));
        let title = native_window_title(plugin.id(), plugin.title());
        let keep_popped = ui.ctx().show_viewport_immediate(
            viewport_id,
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
                    let overlay_open = variable_tree.is_open_in(ui.ctx().viewport_id());
                    let paused = dock.is_plugin_paused(plugin.id());
                    let controls = dock_control_bar(ui, Some(plugin.title()), "Pop in", paused);
                    if controls.toggle_paused {
                        dock.toggle_plugin_paused(plugin.id());
                        actions
                            .extend(plugin.on_enabled_changed(!dock.is_plugin_paused(plugin.id())));
                    }
                    if controls.move_viewport {
                        pop_in = true;
                    }
                    let paused = dock.is_plugin_paused(plugin.id());
                    ui.add_enabled_ui(interaction_enabled && !overlay_open && !paused, |ui| {
                        render_plugin_content(ui, plugin.as_mut(), pool, running, actions);
                    });
                });
                variable_tree.show(viewport_ui, plugin.as_mut(), pool, actions);
                !pop_in
            },
        );
        if !keep_popped {
            variable_tree.close_in(viewport_id);
            dock.set_popped(&plugin_id, false);
            if dock.pending_pop_in.as_deref() == Some(plugin_id.as_str()) {
                dock.pending_pop_in = None;
            }
        }
    }
}

fn render_plugin_content(
    ui: &mut Ui,
    plugin: &mut dyn MemRWPlugin,
    pool: &VariablePool,
    running: bool,
    actions: &mut Vec<PluginAction>,
) {
    actions.extend(plugin.render(
        ui,
        PluginRenderContext {
            pool,
            running,
            viewport_id: ui.ctx().viewport_id(),
        },
    ));
}

#[derive(Default)]
struct DockControlAction {
    move_viewport: bool,
    toggle_paused: bool,
}

fn dock_control_bar(
    ui: &mut Ui,
    title: Option<&str>,
    button: &str,
    paused: bool,
) -> DockControlAction {
    let colors = theme::palette(ui);
    let mut action = DockControlAction::default();
    ui.horizontal(|ui| {
        if let Some(title) = title {
            ui.label(RichText::new(title).strong().color(colors.text));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            action.move_viewport = ui.button(button).clicked();
            let pause_label = if paused {
                "启用插件"
            } else {
                "暂停插件"
            };
            action.toggle_paused = ui
                .add_sized([72.0, 22.0], egui::Button::new(pause_label))
                .on_hover_text(if paused {
                    "恢复此插件的数据更新和交互"
                } else {
                    "暂停此插件的数据更新和交互，保留当前画面"
                })
                .clicked();
            if paused {
                ui.label(RichText::new("已暂停").color(colors.warning));
            }
        });
    });
    ui.separator();
    action
}

#[cfg(test)]
mod tests {
    use super::DockLayoutState;

    #[test]
    fn plugins_are_active_by_default_and_pause_independently() {
        let mut dock = DockLayoutState::default();
        assert!(!dock.is_plugin_paused("chart"));
        assert!(!dock.is_plugin_paused("table"));

        dock.toggle_plugin_paused("chart");
        assert!(dock.is_plugin_paused("chart"));
        assert!(!dock.is_plugin_paused("table"));

        dock.toggle_plugin_paused("chart");
        assert!(!dock.is_plugin_paused("chart"));
    }

    #[test]
    fn focusing_plugin_for_variable_tree_pops_it_back_in() {
        let mut dock = DockLayoutState::default();
        dock.set_popped("table", true);

        dock.focus_plugin("table");

        assert_eq!(dock.active_plugin_id(), Some("table"));
        assert!(!dock.is_popped("table"));
    }

    #[test]
    fn activity_bar_pop_in_requires_explicit_confirmation() {
        let mut dock = DockLayoutState::default();
        dock.set_popped("debug", true);
        dock.request_pop_in("debug");

        assert!(dock.is_popped("debug"));
        assert_eq!(dock.pending_pop_in.as_deref(), Some("debug"));

        dock.confirm_pop_in();
        assert!(!dock.is_popped("debug"));
        assert_eq!(dock.active_plugin_id(), Some("debug"));
        assert!(dock.pending_pop_in.is_none());
    }
}
