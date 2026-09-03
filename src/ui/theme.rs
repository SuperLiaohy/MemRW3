use eframe::egui::{self, Color32, Stroke, Ui};

#[derive(Clone, Copy)]
pub struct Palette {
    pub app_bg: Color32,
    pub sidebar_bg: Color32,
    pub panel_bg: Color32,
    pub surface_bg: Color32,
    pub elevated_bg: Color32,
    pub field_bg: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_weak: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub modal_overlay: Color32,
    pub tooltip_bg: Color32,
}

pub fn install(ctx: &egui::Context) {
    set_dark_mode(ctx, true);
}

pub fn set_dark_mode(ctx: &egui::Context, dark_mode: bool) {
    let palette = if dark_mode {
        dark_palette()
    } else {
        light_palette()
    };
    let mut style = (*ctx.global_style()).clone();
    let visuals = &mut style.visuals;

    visuals.dark_mode = dark_mode;
    visuals.override_text_color = Some(palette.text);
    visuals.weak_text_color = Some(palette.text_muted);
    visuals.hyperlink_color = palette.accent_hover;
    visuals.faint_bg_color = palette.panel_bg;
    visuals.extreme_bg_color = palette.field_bg;
    visuals.text_edit_bg_color = Some(palette.field_bg);
    visuals.code_bg_color = palette.field_bg;
    visuals.warn_fg_color = palette.warning;
    visuals.error_fg_color = palette.danger;
    visuals.panel_fill = palette.app_bg;
    visuals.window_fill = palette.elevated_bg;
    visuals.window_stroke = panel_stroke_for(palette);
    visuals.window_corner_radius = egui::CornerRadius::same(6);
    visuals.menu_corner_radius = egui::CornerRadius::same(4);
    visuals.selection.bg_fill = palette.accent_weak;
    visuals.selection.stroke = Stroke::new(1.0, palette.text);
    visuals.button_frame = true;
    visuals.striped = true;
    visuals.slider_trailing_fill = true;

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = palette.panel_bg;
    widgets.noninteractive.weak_bg_fill = palette.panel_bg;
    widgets.noninteractive.bg_stroke = panel_stroke_for(palette);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.noninteractive.corner_radius = egui::CornerRadius::same(4);

    widgets.inactive.bg_fill = palette.field_bg;
    widgets.inactive.weak_bg_fill = palette.surface_bg;
    widgets.inactive.bg_stroke = panel_stroke_for(palette);
    widgets.inactive.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.inactive.corner_radius = egui::CornerRadius::same(4);

    widgets.hovered.bg_fill = palette.surface_bg;
    widgets.hovered.weak_bg_fill = palette.surface_bg;
    widgets.hovered.bg_stroke = Stroke::new(1.0, palette.accent);
    widgets.hovered.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.hovered.corner_radius = egui::CornerRadius::same(4);

    widgets.active.bg_fill = palette.accent_weak;
    widgets.active.weak_bg_fill = palette.accent_weak;
    widgets.active.bg_stroke = Stroke::new(1.0, palette.accent);
    widgets.active.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.active.corner_radius = egui::CornerRadius::same(4);

    widgets.open.bg_fill = palette.surface_bg;
    widgets.open.weak_bg_fill = palette.surface_bg;
    widgets.open.bg_stroke = Stroke::new(1.0, palette.accent);
    widgets.open.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.open.corner_radius = egui::CornerRadius::same(4);

    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(8.0, 3.0);
    style.spacing.window_margin = egui::Margin::same(8);

    ctx.set_global_style(style);
}

pub fn palette(ui: &Ui) -> Palette {
    if ui.visuals().dark_mode {
        dark_palette()
    } else {
        light_palette()
    }
}

pub fn panel_stroke(ui: &Ui) -> Stroke {
    panel_stroke_for(palette(ui))
}

pub fn muted_text(ui: &Ui) -> Color32 {
    palette(ui).text_muted
}

pub fn danger_text(ui: &Ui) -> Color32 {
    palette(ui).danger
}

fn panel_stroke_for(palette: Palette) -> Stroke {
    Stroke::new(1.0, palette.border)
}

fn dark_palette() -> Palette {
    Palette {
        app_bg: Color32::from_rgb(23, 24, 27),
        sidebar_bg: Color32::from_rgb(30, 31, 35),
        panel_bg: Color32::from_rgb(34, 36, 40),
        surface_bg: Color32::from_rgb(41, 43, 48),
        elevated_bg: Color32::from_rgb(46, 48, 54),
        field_bg: Color32::from_rgb(28, 29, 33),
        border: Color32::from_rgb(65, 69, 77),
        border_strong: Color32::from_rgb(85, 91, 101),
        text: Color32::from_rgb(230, 232, 236),
        text_muted: Color32::from_rgb(152, 158, 168),
        accent: Color32::from_rgb(82, 171, 164),
        accent_hover: Color32::from_rgb(112, 196, 188),
        accent_weak: Color32::from_rgb(38, 82, 80),
        success: Color32::from_rgb(94, 181, 110),
        warning: Color32::from_rgb(218, 163, 65),
        danger: Color32::from_rgb(219, 105, 105),
        modal_overlay: Color32::from_black_alpha(130),
        tooltip_bg: Color32::from_rgba_premultiplied(26, 27, 31, 232),
    }
}

fn light_palette() -> Palette {
    Palette {
        app_bg: Color32::from_rgb(242, 243, 245),
        sidebar_bg: Color32::from_rgb(231, 233, 236),
        panel_bg: Color32::from_rgb(248, 249, 250),
        surface_bg: Color32::from_rgb(255, 255, 255),
        elevated_bg: Color32::from_rgb(255, 255, 255),
        field_bg: Color32::from_rgb(236, 238, 241),
        border: Color32::from_rgb(207, 212, 219),
        border_strong: Color32::from_rgb(178, 185, 194),
        text: Color32::from_rgb(32, 35, 40),
        text_muted: Color32::from_rgb(94, 101, 112),
        accent: Color32::from_rgb(28, 133, 128),
        accent_hover: Color32::from_rgb(19, 111, 107),
        accent_weak: Color32::from_rgb(210, 235, 233),
        success: Color32::from_rgb(44, 132, 76),
        warning: Color32::from_rgb(174, 112, 25),
        danger: Color32::from_rgb(190, 67, 67),
        modal_overlay: Color32::from_black_alpha(96),
        tooltip_bg: Color32::from_rgba_premultiplied(255, 255, 255, 236),
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui;

    use super::set_dark_mode;

    #[test]
    fn switches_the_complete_global_visual_theme() {
        let context = egui::Context::default();

        set_dark_mode(&context, false);
        assert!(!context.global_style().visuals.dark_mode);
        let light_background = context.global_style().visuals.panel_fill;

        set_dark_mode(&context, true);
        assert!(context.global_style().visuals.dark_mode);
        assert_ne!(context.global_style().visuals.panel_fill, light_background);
    }
}
