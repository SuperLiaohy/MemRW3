use eframe::egui::Color32;
use egui_plot::PlotPoint;
use std::collections::VecDeque;

pub struct ChartLegend {
    pub variable_id: usize,
    pub curve_name: String,
    pub color: Color32,
    pub visible: bool,
    pub buffer_size: usize,
    pub data_history: VecDeque<PlotPoint>,
    pub plot_bridge: Option<[PlotPoint; 2]>,
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
            plot_bridge: None,
        }
    }

    /// Make room for an incoming batch and return how many oldest incoming
    /// samples can be skipped because they would be evicted immediately.
    pub fn prepare_batch(&mut self, incoming_len: usize) -> usize {
        if self.buffer_size == 0 {
            self.data_history.clear();
            return incoming_len;
        }

        if incoming_len >= self.buffer_size {
            self.data_history.clear();
            return incoming_len - self.buffer_size;
        }

        let overflow = self
            .data_history
            .len()
            .saturating_add(incoming_len)
            .saturating_sub(self.buffer_size);
        if overflow > 0 {
            drop(self.data_history.drain(..overflow));
        }
        0
    }

    pub fn push_prepared(&mut self, time: f64, value: f64) {
        debug_assert!(self.data_history.len() < self.buffer_size);
        self.data_history.push_back(PlotPoint::new(time, value));
    }

    pub fn refresh_plot_bridge(&mut self) {
        let (first, second) = self.data_history.as_slices();
        self.plot_bridge = first
            .last()
            .copied()
            .zip(second.first().copied())
            .map(|(left, right)| [left, right]);
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

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use egui_plot::PlotPoint;

    use super::ChartLegend;

    #[test]
    fn prepares_space_for_a_batch_once() {
        let mut legend = ChartLegend::new(0, "test".to_owned());
        legend.buffer_size = 4;
        legend.data_history.extend(
            [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0)].map(|(x, y)| egui_plot::PlotPoint::new(x, y)),
        );

        assert_eq!(legend.prepare_batch(2), 0);
        legend.push_prepared(4.0, 4.0);
        legend.push_prepared(5.0, 5.0);

        assert_eq!(
            legend
                .data_history
                .iter()
                .map(|point| (point.x, point.y))
                .collect::<Vec<_>>(),
            vec![(2.0, 2.0), (3.0, 3.0), (4.0, 4.0), (5.0, 5.0)]
        );
    }

    #[test]
    fn skips_samples_that_cannot_fit() {
        let mut legend = ChartLegend::new(0, "test".to_owned());
        legend.buffer_size = 2;
        legend
            .data_history
            .push_back(egui_plot::PlotPoint::new(1.0, 1.0));

        assert_eq!(legend.prepare_batch(4), 2);
        legend.push_prepared(4.0, 4.0);
        legend.push_prepared(5.0, 5.0);

        assert_eq!(
            legend
                .data_history
                .iter()
                .map(|point| (point.x, point.y))
                .collect::<Vec<_>>(),
            vec![(4.0, 4.0), (5.0, 5.0)]
        );
    }

    #[test]
    fn bridges_wrapped_plot_slices() {
        let mut legend = ChartLegend::new(0, "test".to_owned());
        legend.buffer_size = 4;
        legend.data_history = VecDeque::with_capacity(4);
        for value in 0..4 {
            legend.data_history.push_back(PlotPoint::new(value, value));
        }
        legend.prepare_batch(1);
        legend.push_prepared(4.0, 4.0);

        let (_, wrapped) = legend.data_history.as_slices();
        assert!(!wrapped.is_empty());
        legend.refresh_plot_bridge();

        assert_eq!(
            legend.plot_bridge,
            Some([PlotPoint::new(3.0, 3.0), PlotPoint::new(4.0, 4.0)])
        );
    }
}
