use eframe::egui::{self, Rect, Ui};
use serde::{Deserialize, Serialize};

const SPLITTER_SIZE: f32 = 5.0;
const NAVIGATOR_MIN_WIDTH: f32 = 160.0;
const INSPECTOR_MIN_WIDTH: f32 = 190.0;
const EDITOR_MIN_WIDTH: f32 = 260.0;
const EDITOR_MIN_HEIGHT: f32 = 150.0;
const CALL_STACK_MIN_HEIGHT: f32 = 72.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct WorkspaceLayoutState {
    pub navigator_width: f32,
    pub inspector_width: f32,
    pub call_stack_height: f32,
    pub bottom_split_ratio: f32,
}

impl Default for WorkspaceLayoutState {
    fn default() -> Self {
        Self {
            navigator_width: 230.0,
            inspector_width: 280.0,
            call_stack_height: 140.0,
            bottom_split_ratio: 0.55,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct WorkspaceRects {
    pub navigator: Rect,
    pub editor: Rect,
    pub inspector: Rect,
    pub bottom_left: Rect,
    pub bottom_right: Rect,
    navigator_splitter: Rect,
    inspector_splitter: Rect,
    call_stack_splitter: Rect,
    bottom_splitter: Rect,
}

impl WorkspaceLayoutState {
    pub fn layout(&mut self, bounds: Rect) -> WorkspaceRects {
        let column_width = (bounds.width() - SPLITTER_SIZE * 2.0).max(0.0);
        let minimum_width = NAVIGATOR_MIN_WIDTH + EDITOR_MIN_WIDTH + INSPECTOR_MIN_WIDTH;

        let (navigator_width, inspector_width) = if column_width >= minimum_width {
            let navigator_max =
                (column_width - EDITOR_MIN_WIDTH - INSPECTOR_MIN_WIDTH).max(NAVIGATOR_MIN_WIDTH);
            let navigator = self
                .navigator_width
                .clamp(NAVIGATOR_MIN_WIDTH, navigator_max.min(360.0));
            let inspector_max =
                (column_width - navigator - EDITOR_MIN_WIDTH).max(INSPECTOR_MIN_WIDTH);
            let inspector = self
                .inspector_width
                .clamp(INSPECTOR_MIN_WIDTH, inspector_max.min(420.0));
            (navigator, inspector)
        } else {
            // Keep the editor useful in very small docked windows. Side panes shrink
            // proportionally, but every rectangle remains inside the workspace.
            (column_width * 0.24, column_width * 0.28)
        };
        self.navigator_width = navigator_width;
        self.inspector_width = inspector_width;

        let row_height = (bounds.height() - SPLITTER_SIZE).max(0.0);
        let call_stack_height = if row_height >= EDITOR_MIN_HEIGHT + CALL_STACK_MIN_HEIGHT {
            self.call_stack_height.clamp(
                CALL_STACK_MIN_HEIGHT,
                (row_height - EDITOR_MIN_HEIGHT).min(260.0),
            )
        } else {
            row_height * 0.28
        };
        self.call_stack_height = call_stack_height;

        let main_bottom = bounds.bottom() - call_stack_height - SPLITTER_SIZE;
        let navigator_right = bounds.left() + navigator_width;
        let inspector_left = bounds.right() - inspector_width;
        self.bottom_split_ratio = self.bottom_split_ratio.clamp(0.25, 0.75);
        let bottom_content_width = (bounds.width() - SPLITTER_SIZE).max(0.0);
        let bottom_split_x = bounds.left() + bottom_content_width * self.bottom_split_ratio;

        WorkspaceRects {
            navigator: Rect::from_min_max(bounds.min, egui::pos2(navigator_right, main_bottom)),
            navigator_splitter: Rect::from_min_max(
                egui::pos2(navigator_right, bounds.top()),
                egui::pos2(navigator_right + SPLITTER_SIZE, main_bottom),
            ),
            editor: Rect::from_min_max(
                egui::pos2(navigator_right + SPLITTER_SIZE, bounds.top()),
                egui::pos2(inspector_left - SPLITTER_SIZE, main_bottom),
            ),
            inspector_splitter: Rect::from_min_max(
                egui::pos2(inspector_left - SPLITTER_SIZE, bounds.top()),
                egui::pos2(inspector_left, main_bottom),
            ),
            inspector: Rect::from_min_max(
                egui::pos2(inspector_left, bounds.top()),
                egui::pos2(bounds.right(), main_bottom),
            ),
            call_stack_splitter: Rect::from_min_max(
                egui::pos2(bounds.left(), main_bottom),
                egui::pos2(bounds.right(), main_bottom + SPLITTER_SIZE),
            ),
            bottom_left: Rect::from_min_max(
                egui::pos2(bounds.left(), main_bottom + SPLITTER_SIZE),
                egui::pos2(bottom_split_x, bounds.bottom()),
            ),
            bottom_splitter: Rect::from_min_max(
                egui::pos2(bottom_split_x, main_bottom + SPLITTER_SIZE),
                egui::pos2(bottom_split_x + SPLITTER_SIZE, bounds.bottom()),
            ),
            bottom_right: Rect::from_min_max(
                egui::pos2(bottom_split_x + SPLITTER_SIZE, main_bottom + SPLITTER_SIZE),
                bounds.max,
            ),
        }
    }

    pub fn show_splitters(
        &mut self,
        ui: &mut Ui,
        viewport_id: egui::ViewportId,
        rects: WorkspaceRects,
    ) {
        let navigator = ui.interact(
            rects.navigator_splitter.expand2(egui::vec2(2.0, 0.0)),
            ui.id().with(("debug_navigator_splitter", viewport_id)),
            egui::Sense::drag(),
        );
        if navigator.dragged() {
            self.navigator_width += navigator.drag_delta().x;
        }

        let inspector = ui.interact(
            rects.inspector_splitter.expand2(egui::vec2(2.0, 0.0)),
            ui.id().with(("debug_inspector_splitter", viewport_id)),
            egui::Sense::drag(),
        );
        if inspector.dragged() {
            self.inspector_width -= inspector.drag_delta().x;
        }

        let call_stack = ui.interact(
            rects.call_stack_splitter.expand2(egui::vec2(0.0, 2.0)),
            ui.id().with(("debug_call_stack_splitter", viewport_id)),
            egui::Sense::drag(),
        );
        if call_stack.dragged() {
            self.call_stack_height -= call_stack.drag_delta().y;
        }

        let bottom = ui.interact(
            rects.bottom_splitter.expand2(egui::vec2(2.0, 0.0)),
            ui.id().with(("debug_bottom_splitter", viewport_id)),
            egui::Sense::drag(),
        );
        if bottom.dragged() && rects.bottom_splitter.left() > rects.bottom_left.left() {
            let total_width = rects.bottom_right.right() - rects.bottom_left.left() - SPLITTER_SIZE;
            if total_width > 0.0 {
                self.bottom_split_ratio += bottom.drag_delta().x / total_width;
            }
        }

        let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
        for (response, rect) in [
            (&navigator, rects.navigator_splitter),
            (&inspector, rects.inspector_splitter),
            (&call_stack, rects.call_stack_splitter),
            (&bottom, rects.bottom_splitter),
        ] {
            let color = if response.dragged() || response.hovered() {
                ui.visuals().selection.stroke.color
            } else {
                stroke.color
            };
            ui.painter().rect_filled(rect, 0.0, color);
        }
    }
}

pub(super) fn show_pane(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    rect: Rect,
    add_contents: impl FnOnce(&mut Ui),
) {
    let mut pane_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    pane_ui.set_clip_rect(rect);
    egui::Frame::NONE
        .fill(ui.visuals().panel_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .inner_margin(egui::Margin::same(6))
        .show(&mut pane_ui, |ui| {
            ui.set_min_size((rect.size() - egui::vec2(12.0, 12.0)).max(egui::Vec2::ZERO));
            add_contents(ui);
        });
}

#[cfg(test)]
mod tests {
    use super::{SPLITTER_SIZE, WorkspaceLayoutState};
    use eframe::egui::{Rect, pos2};

    fn assert_rect_is_inside(inner: Rect, outer: Rect) {
        assert!(inner.left() >= outer.left());
        assert!(inner.right() <= outer.right());
        assert!(inner.top() >= outer.top());
        assert!(inner.bottom() <= outer.bottom());
        assert!(inner.width() >= 0.0);
        assert!(inner.height() >= 0.0);
    }

    #[test]
    fn layout_stays_inside_normal_and_small_workspaces() {
        for size in [(1100.0, 620.0), (760.0, 350.0), (320.0, 180.0)] {
            let bounds = Rect::from_min_max(pos2(10.0, 20.0), pos2(10.0 + size.0, 20.0 + size.1));
            let rects = WorkspaceLayoutState::default().layout(bounds);
            for rect in [
                rects.navigator,
                rects.editor,
                rects.inspector,
                rects.bottom_left,
                rects.bottom_right,
                rects.navigator_splitter,
                rects.inspector_splitter,
                rects.call_stack_splitter,
                rects.bottom_splitter,
            ] {
                assert_rect_is_inside(rect, bounds);
            }
            assert!((rects.bottom_left.top() - rects.editor.bottom() - SPLITTER_SIZE).abs() < 0.01);
        }
    }

    #[test]
    fn stale_oversized_pane_preferences_are_clamped() {
        let bounds = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 420.0));
        let mut layout = WorkspaceLayoutState {
            navigator_width: 10_000.0,
            inspector_width: 10_000.0,
            call_stack_height: 10_000.0,
            bottom_split_ratio: 10_000.0,
        };
        let rects = layout.layout(bounds);
        assert!(rects.editor.width() >= 260.0);
        assert!(rects.editor.height() >= 150.0);
        assert!(rects.bottom_left.height() <= 260.0);
        assert!(rects.bottom_left.width() >= 0.0);
        assert!(rects.bottom_right.width() >= 0.0);
    }
}
