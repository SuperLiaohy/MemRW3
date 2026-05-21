use eframe::egui::Color32;
use std::collections::VecDeque;

pub struct ChartLegend {
    pub variable_id: usize,
    pub curve_name: String,
    pub color: Color32,
    pub visible: bool,
    pub buffer_size: usize,
    pub data_history: VecDeque<(f64, f64)>,
}

impl ChartLegend {
    pub fn new(variable_id: usize, curve_name: String) -> Self {
        Self {
            variable_id,
            curve_name,
            color: default_color(variable_id),
            visible: true,
            buffer_size: 5000,
            data_history: VecDeque::with_capacity(5000),
        }
    }

    pub fn push_value(&mut self, time: f64, value: f64) {
        if self.data_history.len() >= self.buffer_size {
            self.data_history.pop_front();
        }
        self.data_history.push_back((time, value));
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
