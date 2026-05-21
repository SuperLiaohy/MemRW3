use eframe::egui::Color32;

pub struct ChartLegend {
    pub variable_id: usize,
    pub curve_name: String,
    pub color: Color32,
    pub visible: bool,
    pub buffer_size: usize,
}

impl ChartLegend {
    pub fn new(variable_id: usize, curve_name: String) -> Self {
        Self {
            variable_id,
            curve_name,
            color: default_color(variable_id),
            visible: true,
            buffer_size: 5000,
        }
    }
}

const PRESET_COLORS: [Color32; 12] = [
    Color32::from_rgb(66, 133, 244),
    Color32::from_rgb(219, 68, 55),
    Color32::from_rgb(244, 180, 0),
    Color32::from_rgb(15, 157, 88),
    Color32::from_rgb(171, 71, 188),
    Color32::from_rgb(0, 172, 193),
    Color32::from_rgb(255, 112, 67),
    Color32::from_rgb(63, 81, 181),
    Color32::from_rgb(139, 195, 74),
    Color32::from_rgb(255, 87, 34),
    Color32::from_rgb(158, 158, 158),
    Color32::from_rgb(121, 85, 72),
];

fn default_color(index: usize) -> Color32 {
    PRESET_COLORS[index % PRESET_COLORS.len()]
}

pub const fn preset_colors() -> &'static [Color32; 12] {
    &PRESET_COLORS
}
