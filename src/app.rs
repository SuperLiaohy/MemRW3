use eframe::egui;
use egui::{Color32, FontData, FontDefinitions, FontFamily, Ui};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer, tab_viewer};
use object::Object;
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
    pub elf_path: String,
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
            elf_path: String::new(),
            session: AppSession {
                bottom_sheet_height: 250.0,
                ..Default::default()
            },
            probe: ProbeSession::default(),
            chart_state: ChartPluginState::default(),
            table_state: TablePluginState::default(),
        }
    }

    fn load_elf(&mut self) {
        self.session.load_error = None;
        let path = self.elf_path.trim().to_string();
        if path.is_empty() {
            self.session.load_error = Some("请输入 ELF 文件路径".into());
            return;
        }
        let data = match fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                self.session.load_error = Some(format!("读取文件失败: {e}"));
                return;
            }
        };
        let object = match object::read::File::parse(&*data) {
            Ok(o) => o,
            Err(e) => {
                self.session.load_error = Some(format!("解析 ELF 失败: {e}"));
                return;
            }
        };
        if object.format() != object::BinaryFormat::Elf {
            self.session.load_error = Some("不是有效的 ELF 文件".into());
            return;
        }
        let endian = match object.endianness() {
            object::Endianness::Little => gimli::RunTimeEndian::Little,
            object::Endianness::Big => gimli::RunTimeEndian::Big,
        };
        let dwarf = match crate::dwarf::load_dwarf(&object, endian) {
            Ok(d) => d,
            Err(e) => {
                self.session.load_error = Some(format!("加载 DWARF 失败: {e}"));
                return;
            }
        };
        let cus = match crate::dwarf::collect_cus(&dwarf) {
            Ok(c) => c,
            Err(e) => {
                self.session.load_error = Some(format!("解析 DWARF 数据失败: {e}"));
                return;
            }
        };
        self.dwarf_app = DwarfApp::new(cus);
        self.session.load_error = None;
    }
}

struct TabViewerCtx<'a> {
    session: &'a mut AppSession,
    chart_state: &'a mut ChartPluginState,
    table_state: &'a mut TablePluginState,
}

impl<'a> TabViewerCtx<'a> {
    fn render_main(&mut self, ui: &mut Ui, tab: &TabKind) {
        match tab {
            TabKind::Chart => {
                let a = ui::chart_plugin::chart_panel(
                    ui,
                    self.chart_state,
                    &self.session.pool,
                    self.session.running,
                );
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
        self.render_main(ui, tab);
    }
}

fn bottom_sheet_handle(ui: &mut Ui, drag_state: &mut Option<(f32, f32)>, current_h: f32) -> f32 {
    let (rect, response) =
        ui.allocate_at_least(egui::vec2(ui.available_width(), 20.0), egui::Sense::drag());
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
        if let Some(pointer) = response.interact_pointer_pos() {
            if drag_state.is_none() {
                *drag_state = Some((pointer.y, current_h));
            }
            let (origin_y, initial_h) = drag_state.unwrap();
            let displacement = origin_y - pointer.y;
            return initial_h + displacement;
        }
        current_h
    } else {
        *drag_state = None;
        current_h
    }
}

impl eframe::App for MemRW3App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let total_h = ui.available_height();
        let ctrl_h = (total_h * 0.06).clamp(40.0, 56.0);
        let bs_open = self.session.active_bottom_sheet.is_some();
        let dialog_open = self.chart_state.show_line_dialog
            || self.table_state.show_entry_dialog
            || self.probe.show_settings;

        ui.vertical(|ui| {
            // ── Control Bar (locked when BottomSheet or dialogs are open) ──
            let (ctrl_rect, _) = ui.allocate_at_least(
                egui::vec2(ui.available_width(), ctrl_h),
                egui::Sense::hover(),
            );
            let mut ctrl_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(ctrl_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            ctrl_ui.add_enabled_ui(!bs_open && !dialog_open, |ui| {
                ui::control_bar(ui, &mut self.session, &mut self.probe);
            });

            self.probe.running = self.session.running;
            self.probe
                .acquire(&mut self.session.pool, self.session.delay_us);
            if self.session.running {
                ui.ctx().request_repaint();
            }

            let remaining = ui.available_height();
            // 1. 将最小高度提高到 250.0，确保树状视图始终有空间显示
            let min_limit = remaining * 0.5;
            let max_h = (remaining * 0.9).max(min_limit);
            let min_h = min_limit.min(max_h);
            let bs_h = if bs_open {
                // 2. 覆盖写回 bottom_sheet_height：防止快速拖拽越界导致数值跑飞而产生“拖动卡死”感
                self.session.bottom_sheet_height = self.session.bottom_sheet_height.clamp(min_h, max_h);
                self.session.bottom_sheet_height
            } else {
                0.0
            };
            if remaining > 0.0 {
                let (dock_rect, _) = ui.allocate_at_least(
                    egui::vec2(ui.available_width(), remaining),
                    egui::Sense::click(),
                );
                let mut dock_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(dock_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );

                if bs_open || dialog_open {
                    dock_ui.disable();
                }

                let mut viewer = TabViewerCtx {
                    session: &mut self.session,
                    chart_state: &mut self.chart_state,
                    table_state: &mut self.table_state,
                };
                DockArea::new(&mut self.tree)
                    .style(egui_dock::Style::from_egui(ui.style()))
                    .show_close_buttons(false)
                    .show_leaf_collapse_buttons(false)
                    .show_inside(&mut dock_ui, &mut viewer);

                // Full-dock click interceptor: rendered AFTER DockArea, BEFORE BottomSheet.
                // egui Z-order: later widgets take input priority → this steals all clicks
                // from exposed dock tabs (which use ui.interact() that ignores enabled state).
                // BottomSheet, rendered after this, overrides for its own area.
                if bs_open || dialog_open {
                    ui.interact(dock_rect, ui.id().with("dock_blocker"), egui::Sense::click_and_drag());
                }
            }

            if bs_open {
                let bs_rect = egui::Rect::from_min_max(
                    egui::pos2(ui.min_rect().left(), ui.min_rect().bottom() - bs_h),
                    egui::pos2(ui.min_rect().right(), ui.min_rect().bottom()),
                );
                let _resp = ui.allocate_ui_at_rect(bs_rect, |ui| {
                    let card_bg = ui.visuals().window_fill();
                    let card_stroke = ui.visuals().window_stroke();
                    let target_tab = self.session.active_bottom_sheet;
                    egui::Frame::NONE
                        .fill(card_bg)
                        .stroke(card_stroke)
                        .corner_radius(egui::CornerRadius {
                            nw: 16,
                            ne: 16,
                            sw: 0,
                            se: 0,
                        })
                        .show(ui, |ui| {
                            let target = bottom_sheet_handle(
                                ui,
                                &mut self.session.bottom_sheet_drag,
                                self.session.bottom_sheet_height,
                            );
                            self.session.bottom_sheet_height = target.clamp(min_h, max_h);
                            egui::Frame::NONE
                                .inner_margin(egui::Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    // ── ELF file path picker (top of BottomSheet) ──
                                    ui.horizontal(|ui| {
                                        ui.label("ELF 文件:");
                                        ui.add_sized(
                                            [ui.available_width() - 120.0, 20.0],
                                            egui::TextEdit::singleline(&mut self.elf_path)
                                                .hint_text("输入 firmware.elf 路径..."),
                                        );
                                        if ui.button("加载").clicked() {
                                            self.load_elf();
                                        }
                                        if let Some(ref err) = self.session.load_error {
                                            ui.colored_label(Color32::from_rgb(255, 80, 80), err);
                                        }
                                    });
                                    ui.add_space(4.0);
                                    ui.separator();
                                    ui.add_space(2.0);

                                    ui.horizontal(|ui| {
                                        ui.heading("变量列表 (DWARF Tree)");
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui.button("关闭").clicked() {
                                                    self.session.active_bottom_sheet = None;
                                                }
                                            },
                                        );
                                    });
                                    ui.add_space(4.0);
                                    ui.separator();
                                    ui.add_space(4.0);
                                    let rem_h = ui.available_height().max(0.0);
                                    let total_w = ui.available_width().max(0.0);
                                    let right_w = (total_w * 0.32).clamp(220.0, 350.0);
                                    let left_w = (total_w - right_w - 8.0).max(200.0);
                                    ui.horizontal(|ui| {
                                        let (left_rect, _) = ui.allocate_exact_size(
                                            egui::vec2(left_w, rem_h),
                                            egui::Sense::hover(),
                                        );
                                        let mut left_ui = ui.new_child(
                                            egui::UiBuilder::new()
                                                .max_rect(left_rect)
                                                .layout(egui::Layout::top_down(egui::Align::Min)),
                                        );
                                        ui::vari_tree_ui(&mut left_ui, &mut self.dwarf_app);
                                        ui.separator();
                                        let (right_rect, _) = ui.allocate_exact_size(
                                            egui::vec2(ui.available_width(), rem_h),
                                            egui::Sense::hover(),
                                        );
                                        let mut right_ui = ui.new_child(
                                            egui::UiBuilder::new()
                                                .max_rect(right_rect)
                                                .layout(egui::Layout::top_down(egui::Align::Min)),
                                        );
                                        if let Some(ref node) = self.dwarf_app.selected_node {
                                            let pool = &mut self.session.pool;
                                            let already_added = pool.contains(node.id);
                                            let extend_name =
                                                self.dwarf_app.compute_extend_name(node.id);
                                            let extend_addr = self
                                                .dwarf_app
                                                .compute_extend_address(node.id)
                                                .unwrap_or(node.address);
                                            let default_type = crate::types::basic_type_to_extend(
                                                &node.basic_type,
                                            );

                                            // Get or create ExtendConfig for this node
                                            let config = self
                                                .session
                                                .extend_configs
                                                .entry(node.id)
                                                .or_insert_with(|| crate::types::ExtendConfig {
                                                    name: extend_name.clone(),
                                                    address: extend_addr,
                                                    ext_type: default_type.clone(),
                                                    size: node.size,
                                                    array_index: None,
                                                    array_count: None,
                                                });

                                            // Array element: set up index/name/address
                                            if let Some((count, elem_size)) =
                                                self.dwarf_app.parent_array_info(node.id)
                                            {
                                                config.array_count = Some(count);
                                                if config.array_index.is_none() {
                                                    config.array_index = Some(0);
                                                }
                                                let idx = config.array_index.unwrap_or(0);
                                                let parent_id = node.parent_id.unwrap();
                                                let parent_name =
                                                    self.dwarf_app.compute_extend_name(parent_id);
                                                let parent_addr = self
                                                    .dwarf_app
                                                    .compute_extend_address(parent_id)
                                                    .unwrap_or(0);
                                                config.name =
                                                    format!("{}[{}]", parent_name, idx);
                                                config.address = parent_addr + elem_size * idx;
                                            }

                                            // Color persistence via egui memory
                                            let color_id = ui.make_persistent_id(format!(
                                                "chart_add_color_{}",
                                                node.id
                                            ));
                                            let mut chart_color = ui.data_mut(|d| {
                                                *d.get_temp_mut_or(
                                                    color_id,
                                                    Color32::from_rgb(66, 133, 244),
                                                )
                                            });
                                            let mut chart_curve_name = String::new();
                                            let mut table_display_name = String::new();
                                            let added = match target_tab {
                                                Some(DockTab::Chart) => {
                                                    let result = ui::vari_properties_ui(
                                                        &mut right_ui,
                                                        node,
                                                        config,
                                                        |ui, node_name| {
                                                            ui::chart_plugin::chart_add_config_ui(
                                                                ui,
                                                                node_name,
                                                                &mut chart_curve_name,
                                                                &mut chart_color,
                                                            );
                                                            ui.button("添加到 Chart").clicked()
                                                        },
                                                    );
                                                    ui.data_mut(|d| {
                                                        d.insert_temp(color_id, chart_color)
                                                    });
                                                    result
                                                }
                                                Some(DockTab::Table) => ui::vari_properties_ui(
                                                    &mut right_ui,
                                                    node,
                                                    config,
                                                    |ui, node_name| {
                                                        ui::table_plugin::table_add_config_ui(
                                                            ui,
                                                            node_name,
                                                            &mut table_display_name,
                                                        );
                                                        ui.button("添加到 Table").clicked()
                                                    },
                                                ),
                                                None => false,
                                            };
                                            if added && !already_added {
                                                let var_id = self.session.pool.add(config);
                                                self.session.selected_variables.insert(var_id);
                                                match target_tab {
                                                    Some(DockTab::Chart) => {
                                                        self.chart_state.add_from_pool(
                                                            &self.session.pool,
                                                            var_id,
                                                        );
                                                        if let Some(legend) =
                                                            self.chart_state.legends.last_mut()
                                                        {
                                                            legend.curve_name =
                                                                chart_curve_name.clone();
                                                            legend.color = chart_color;
                                                        }
                                                    }
                                                    Some(DockTab::Table) => {
                                                        self.table_state.add_from_pool(
                                                            &self.session.pool,
                                                            var_id,
                                                        );
                                                        if let Some(entry) =
                                                            self.table_state.entries.last_mut()
                                                        {
                                                            entry.display_name =
                                                                table_display_name.clone();
                                                        }
                                                    }
                                                    None => {}
                                                }
                                            }
                                        } else {
                                            right_ui.label("选择节点以查看属性");
                                        }
                                    });
                                });
                        });
                });
            }
        });
    }
}

pub fn setup_fonts(ctx: &egui::Context) {
    let font_bytes = fs::read(CHINESE_FONT_PATH).unwrap_or_else(|_| {
        eprintln!("未找到中文字体: {CHINESE_FONT_PATH}");
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
