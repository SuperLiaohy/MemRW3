use eframe::egui;
use egui::{Color32, FontData, FontDefinitions, FontFamily, Ui};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer, tab_viewer};
use std::{fs, sync::Arc};

use crate::model::{AppSession, DockTab};
use crate::probe::ProbeSession;
use crate::types::DwarfApp;
use crate::ui;
use crate::ui::chart_plugin::ChartPluginState;
use crate::ui::table_plugin::TablePluginState;

const CHINESE_FONT_PATH: &str = "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TabKind {
    Chart,
    Table,
}

pub struct MemRW3App {
    tree: DockState<TabKind>,
    pub dwarf_app: DwarfApp,
    pub session: AppSession,
    pub probe: ProbeSession,
    pub chart_state: ChartPluginState,
    pub table_state: TablePluginState,
}

impl MemRW3App {
    pub fn new(dwarf_app: DwarfApp) -> Self {
        let mut tree = DockState::new(vec![TabKind::Chart]);
        tree.main_surface_mut()
            .split_right(NodeIndex::root(), 0.5, vec![TabKind::Table]);

        Self {
            tree,
            dwarf_app,
            session: AppSession {
                bottom_sheet_height: 300.0,
                ..Default::default()
            },
            probe: ProbeSession::default(),
            chart_state: ChartPluginState::default(),
            table_state: TablePluginState::default(),
        }
    }
}

struct TabViewerCtx<'a> {
    dwarf_app: &'a mut DwarfApp,
    session: &'a mut AppSession,
    chart_state: &'a mut ChartPluginState,
    table_state: &'a mut TablePluginState,
}

impl<'a> TabViewerCtx<'a> {
    fn render_main(&mut self, ui: &mut Ui, tab: &TabKind) {
        use crate::model::DockTab;
        match tab {
            TabKind::Chart => {
                let a = ui::chart_plugin::chart_panel(ui, self.chart_state, &self.session.pool);
                if a == ui::chart_plugin::PanelAction::OpenTree {
                    self.session.active_bottom_sheet = Some(DockTab::Chart);
                }
            }
            TabKind::Table => {
                let a = ui::table_plugin::table_panel(ui, self.table_state, &self.session.pool);
                if a == ui::table_plugin::PanelAction::OpenTree {
                    self.session.active_bottom_sheet = Some(DockTab::Table);
                }
            }
        }
    }

    fn render_bottom_sheet(&mut self, ui: &mut Ui) {
        let target_tab = self.session.active_bottom_sheet;

        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("变量列表 (DWARF Tree)");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("关闭").clicked() {
                            self.session.active_bottom_sheet = None;
                        }
                    });
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                let total_w = ui.available_width();
                let left_w = total_w * 0.5;
                let right_w = total_w - left_w;

                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(left_w);
                        ui::vari_tree_ui(ui, self.dwarf_app);
                    });

                    ui.separator();

                    ui.vertical(|ui| {
                        ui.set_width(right_w);
                        if let Some(ref node) = self.dwarf_app.selected_node {
                            let pool = &mut self.session.pool;
                            let already_added =
                                pool.contains(node.id);

                            let added = match target_tab {
                                Some(DockTab::Chart) => {
                                    let mut curve_name = String::new();
                                    let mut color = Color32::from_rgb(66, 133, 244);

                                    ui::vari_properties_ui(ui, node, |ui, node_name| {
                                        ui::chart_plugin::chart_add_config_ui(
                                            ui, node_name, &mut curve_name, &mut color,
                                        );
                                        if ui.button("添加到 Chart").clicked() {
                                            return true;
                                        }
                                        false
                                    })
                                }
                                Some(DockTab::Table) => {
                                    let mut display_name = String::new();

                                    ui::vari_properties_ui(ui, node, |ui, node_name| {
                                        ui::table_plugin::table_add_config_ui(
                                            ui, node_name, &mut display_name,
                                        );
                                        if ui.button("添加到 Table").clicked() {
                                            return true;
                                        }
                                        false
                                    })
                                }
                                None => false,
                            };

                            if added && !already_added {
                                let var_id = self.session.pool.add(node);
                                self.session.selected_variables.insert(var_id);
                                match target_tab {
                                    Some(DockTab::Chart) => {
                                        self.chart_state.add_from_pool(
                                            &self.session.pool,
                                            var_id,
                                        );
                                    }
                                    Some(DockTab::Table) => {
                                        self.table_state.add_from_pool(
                                            &self.session.pool,
                                            var_id,
                                        );
                                    }
                                    None => {}
                                }
                            }
                        } else {
                            ui.label("选择节点以查看属性");
                        }
                    });
                });
            });
    }

    fn any_bottom_sheet_open(&self) -> bool {
        self.session.active_bottom_sheet.is_some()
    }

    fn bs_active_for(&self, tab: &TabKind) -> bool {
        matches!(
            (tab, self.session.active_bottom_sheet),
            (TabKind::Chart, Some(DockTab::Chart))
                | (TabKind::Table, Some(DockTab::Table))
        )
    }
}

impl<'a> TabViewer for TabViewerCtx<'a> {
    type Tab = TabKind;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            TabKind::Chart => "Chart 实时数据".into(),
            TabKind::Table => "Table 读写数据".into(),
        }
    }

    fn on_close(&mut self, _tab: &mut Self::Tab) -> tab_viewer::OnCloseResponse {
        tab_viewer::OnCloseResponse::Ignore
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Self::Tab) {
        let bs_open_here = self.bs_active_for(tab);
        let any_bs_open = self.any_bottom_sheet_open();

        if any_bs_open && !bs_open_here {
            ui.disable();
            ui.vertical(|ui| {
                ui.colored_label(
                    Color32::from_rgb(230, 50, 50),
                    "变量树已在其他标签页中打开，请先关闭。",
                );
                ui.separator();
                self.render_main(ui, tab);
            });
            return;
        }

        if bs_open_here {
            let total_height = ui.available_height();
            self.session.bottom_sheet_height = self
                .session
                .bottom_sheet_height
                .clamp(200.0, (total_height - 60.0).max(200.0));

            let bs_height = self.session.bottom_sheet_height;
            let top_height = (total_height - bs_height).max(0.0);

            ui.vertical(|ui| {
                let (top_rect, _) = ui.allocate_at_least(
                    egui::vec2(ui.available_width(), top_height),
                    egui::Sense::hover(),
                );
                let mut top_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(top_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                self.render_main(&mut top_ui, tab);

                let (bs_container_rect, _) = ui.allocate_at_least(
                    egui::vec2(ui.available_width(), bs_height),
                    egui::Sense::hover(),
                );
                let mut bs_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(bs_container_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );

                let card_bg = ui.visuals().window_fill();
                let card_stroke = ui.visuals().window_stroke();

                egui::Frame::NONE
                    .fill(card_bg)
                    .stroke(card_stroke)
                    .corner_radius(egui::CornerRadius {
                        nw: 16,
                        ne: 16,
                        sw: 0,
                        se: 0,
                    })
                    .show(&mut bs_ui, |ui| {
                        let delta = bottom_sheet_handle(ui);
                        if delta != 0.0 {
                            self.session.bottom_sheet_height -= delta;
                            ui.ctx().request_repaint();
                        }
                        self.render_bottom_sheet(ui);
                    });
            });
        } else {
            self.render_main(ui, tab);
        }
    }
}

fn bottom_sheet_handle(ui: &mut Ui) -> f32 {
    let handle_area = 20.0;
    let (rect, response) = ui.allocate_at_least(
        egui::vec2(ui.available_width(), handle_area),
        egui::Sense::drag(),
    );

    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }

    let handle_color = if response.dragged() {
        ui.visuals().widgets.active.bg_fill
    } else if response.hovered() {
        ui.visuals().widgets.hovered.bg_fill
    } else if ui.visuals().dark_mode {
        Color32::from_gray(80)
    } else {
        Color32::from_gray(200)
    };

    let capsule = egui::Rect::from_center_size(rect.center(), egui::vec2(40.0, 4.0));
    ui.painter()
        .rect_filled(capsule, egui::CornerRadius::same(2), handle_color);

    if response.dragged() {
        response.drag_delta().y
    } else {
        0.0
    }
}

impl eframe::App for MemRW3App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let control_bar_height = 48.0;
        let total_height = ui.available_height();

        ui.vertical(|ui| {
            let (control_rect, _) = ui.allocate_at_least(
                egui::vec2(ui.available_width(), control_bar_height),
                egui::Sense::hover(),
            );
            let mut control_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(control_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            ui::control_bar(&mut control_ui, &mut self.session, &mut self.probe);

            ui.add_space(6.0);

            let remaining = (total_height - control_bar_height - 6.0).max(0.0);
            let (dock_rect, _) = ui.allocate_at_least(
                egui::vec2(ui.available_width(), remaining),
                egui::Sense::hover(),
            );
            let mut dock_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(dock_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );

            self.probe.running = self.session.running;
            self.probe.acquire(&mut self.session.pool, self.session.delay_us);
            if self.session.running {
                ui.ctx().request_repaint();
            }

            let mut viewer = TabViewerCtx {
                dwarf_app: &mut self.dwarf_app,
                session: &mut self.session,
                chart_state: &mut self.chart_state,
                table_state: &mut self.table_state,
            };

            DockArea::new(&mut self.tree)
                .style(egui_dock::Style::from_egui(ui.style()))
                .show_close_buttons(false)
                .show_leaf_collapse_buttons(false)
                .show_inside(&mut dock_ui, &mut viewer);
        });
    }
}

pub fn setup_fonts(ctx: &egui::Context) {
    let font_bytes = fs::read(CHINESE_FONT_PATH).unwrap_or_else(|_| {
        eprintln!("未找到中文字体: {CHINESE_FONT_PATH}，中文将无法显示");
        Vec::new()
    });
    if font_bytes.is_empty() {
        return;
    }

    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "DroidSansFallback".to_owned(),
        Arc::new(FontData::from_owned(font_bytes)),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "DroidSansFallback".to_owned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .push("DroidSansFallback".to_owned());
    ctx.set_fonts(fonts);
}
