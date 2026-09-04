use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread::JoinHandle;
#[cfg(target_os = "linux")]
use std::time::Duration;

use eframe::egui::{self, Color32, Stroke, Ui};

const THEME_UNKNOWN: u8 = 0;
const THEME_DARK: u8 = 1;
const THEME_LIGHT: u8 = 2;

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
    let dark_style = configured_style(
        (*ctx.style_of(egui::Theme::Dark)).clone(),
        dark_palette(),
        true,
    );
    let light_style = configured_style(
        (*ctx.style_of(egui::Theme::Light)).clone(),
        light_palette(),
        false,
    );
    ctx.set_style_of(egui::Theme::Dark, dark_style);
    ctx.set_style_of(egui::Theme::Light, light_style);
    ctx.options_mut(|options| options.fallback_theme = egui::Theme::Light);
    ctx.set_theme(egui::ThemePreference::Light);
}

pub fn set_theme_preference(ctx: &egui::Context, preference: egui::ThemePreference) {
    ctx.set_theme(preference);
}

pub fn next_theme_preference(preference: egui::ThemePreference) -> egui::ThemePreference {
    match preference {
        egui::ThemePreference::Dark => egui::ThemePreference::Light,
        egui::ThemePreference::Light => egui::ThemePreference::System,
        egui::ThemePreference::System => egui::ThemePreference::Dark,
    }
}

pub struct SystemThemeMonitor {
    detected: Arc<AtomicU8>,
    applied: AtomicU8,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SystemThemeMonitor {
    pub fn new(repaint_ctx: egui::Context) -> Self {
        let detected = Arc::new(AtomicU8::new(theme_code(
            detect_desktop_theme().unwrap_or(egui::Theme::Light),
        )));
        let stop = Arc::new(AtomicBool::new(false));

        #[cfg(target_os = "linux")]
        let handle = {
            let detected = Arc::clone(&detected);
            let stop = Arc::clone(&stop);
            Some(std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    if let Some(theme) = detect_desktop_theme() {
                        let code = theme_code(theme);
                        if detected.swap(code, Ordering::AcqRel) != code {
                            repaint_ctx.request_repaint();
                        }
                    }
                    for _ in 0..20 {
                        if stop.load(Ordering::Acquire) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            }))
        };
        #[cfg(not(target_os = "linux"))]
        let handle = {
            let _ = repaint_ctx;
            None
        };

        Self {
            detected,
            applied: AtomicU8::new(THEME_UNKNOWN),
            stop,
            handle,
        }
    }

    pub fn apply(&self, ctx: &egui::Context) {
        let code = self.detected.load(Ordering::Acquire);
        if code == THEME_UNKNOWN || self.applied.swap(code, Ordering::AcqRel) == code {
            return;
        }
        let theme = if code == THEME_DARK {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        };
        ctx.options_mut(|options| options.fallback_theme = theme);
    }
}

impl Drop for SystemThemeMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn theme_code(theme: egui::Theme) -> u8 {
    match theme {
        egui::Theme::Dark => THEME_DARK,
        egui::Theme::Light => THEME_LIGHT,
    }
}

#[cfg(target_os = "linux")]
fn detect_desktop_theme() -> Option<egui::Theme> {
    let color_scheme = gsettings_value("color-scheme");
    let gtk_theme = gsettings_value("gtk-theme");
    desktop_theme_from_values(color_scheme.as_deref(), gtk_theme.as_deref())
}

#[cfg(not(target_os = "linux"))]
fn detect_desktop_theme() -> Option<egui::Theme> {
    None
}

#[cfg(target_os = "linux")]
fn gsettings_value(key: &str) -> Option<String> {
    let output = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().trim_matches('\'').to_ascii_lowercase())
}

fn desktop_theme_from_values(
    color_scheme: Option<&str>,
    gtk_theme: Option<&str>,
) -> Option<egui::Theme> {
    if color_scheme.is_some_and(|value| value.contains("prefer-dark")) {
        return Some(egui::Theme::Dark);
    }
    if color_scheme.is_some_and(|value| value.contains("prefer-light")) {
        return Some(egui::Theme::Light);
    }
    if let Some(gtk_theme) = gtk_theme {
        return Some(if gtk_theme.contains("dark") {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
    }
    color_scheme.map(|_| egui::Theme::Light)
}

fn configured_style(mut style: egui::Style, palette: Palette, dark_mode: bool) -> egui::Style {
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

    // Keep hover/press paint inside the widget allocation. Expansion makes
    // adjacent controls appear to move even though their logical rect is stable.
    widgets.noninteractive.expansion = 0.0;
    widgets.inactive.expansion = 0.0;
    widgets.hovered.expansion = 0.0;
    widgets.active.expansion = 0.0;
    widgets.open.expansion = 0.0;

    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(8.0, 3.0);
    style.spacing.window_margin = egui::Margin::same(8);

    style
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

    use super::{desktop_theme_from_values, install, next_theme_preference, set_theme_preference};

    #[test]
    fn switches_the_complete_global_visual_theme() {
        let context = egui::Context::default();

        install(&context);
        let dark = context.style_of(egui::Theme::Dark);
        let light = context.style_of(egui::Theme::Light);

        assert!(dark.visuals.dark_mode);
        assert!(!light.visuals.dark_mode);
        assert_ne!(dark.visuals.panel_fill, light.visuals.panel_fill);
        for style in [&dark, &light] {
            assert_eq!(style.visuals.widgets.hovered.expansion, 0.0);
            assert_eq!(style.visuals.widgets.active.expansion, 0.0);
            assert_eq!(style.visuals.widgets.open.expansion, 0.0);
        }
        assert_eq!(
            context.options(|options| options.theme_preference),
            egui::ThemePreference::Light
        );

        set_theme_preference(&context, egui::ThemePreference::System);
        assert_eq!(
            context.options(|options| options.theme_preference),
            egui::ThemePreference::System
        );
    }

    #[test]
    fn theme_preference_cycles_through_all_three_modes() {
        assert_eq!(
            next_theme_preference(egui::ThemePreference::Dark),
            egui::ThemePreference::Light
        );
        assert_eq!(
            next_theme_preference(egui::ThemePreference::Light),
            egui::ThemePreference::System
        );
        assert_eq!(
            next_theme_preference(egui::ThemePreference::System),
            egui::ThemePreference::Dark
        );
    }

    #[test]
    fn parses_gnome_and_gtk_theme_preferences() {
        assert_eq!(
            desktop_theme_from_values(Some("prefer-dark"), Some("Yaru")),
            Some(egui::Theme::Dark)
        );
        assert_eq!(
            desktop_theme_from_values(Some("default"), Some("Yaru-dark")),
            Some(egui::Theme::Dark)
        );
        assert_eq!(
            desktop_theme_from_values(Some("default"), Some("Yaru")),
            Some(egui::Theme::Light)
        );
        assert_eq!(
            desktop_theme_from_values(Some("prefer-light"), Some("Yaru-dark")),
            Some(egui::Theme::Light)
        );
    }
}
