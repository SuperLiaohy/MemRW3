use std::collections::VecDeque;
use std::f64::consts::PI;
use std::ops::{Add, Mul, Sub};
use std::sync::Arc;

use egui_plot::PlotPoint;

#[derive(Clone, Copy)]
struct Complex {
    re: f64,
    im: f64,
}

impl Complex {
    fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }
    fn norm(self) -> f64 {
        self.re.hypot(self.im)
    }
}

impl Add for Complex {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self {
            re: self.re + rhs.re,
            im: self.im + rhs.im,
        }
    }
}

impl Sub for Complex {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self {
            re: self.re - rhs.re,
            im: self.im - rhs.im,
        }
    }
}

impl Mul for Complex {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self {
            re: self.re * rhs.re - self.im * rhs.im,
            im: self.re * rhs.im + self.im * rhs.re,
        }
    }
}

fn fft(data: &mut [Complex]) {
    let n = data.len();
    if n <= 1 {
        return;
    }
    assert!(n.is_power_of_two(), "FFT size must be a power of two");

    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            data.swap(i, j);
        }
    }

    let mut len = 2usize;
    while len <= n {
        let angle = -2.0 * PI / len as f64;
        let wlen = Complex::new(angle.cos(), angle.sin());
        for i in (0..n).step_by(len) {
            let mut w = Complex::new(1.0, 0.0);
            for jj in 0..len / 2 {
                let u = data[i + jj];
                let v = data[i + jj + len / 2] * w;
                data[i + jj] = u + v;
                data[i + jj + len / 2] = u - v;
                w = w * wlen;
            }
        }
        len <<= 1;
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum FftWindowType {
    Rectangular,
    Hann,
    Hamming,
    Blackman,
}

impl FftWindowType {
    pub fn label(&self) -> &'static str {
        match self {
            FftWindowType::Rectangular => "Rectangular",
            FftWindowType::Hann => "Hann",
            FftWindowType::Hamming => "Hamming",
            FftWindowType::Blackman => "Blackman",
        }
    }

    pub const ALL: &'static [FftWindowType] = &[
        FftWindowType::Rectangular,
        FftWindowType::Hann,
        FftWindowType::Hamming,
        FftWindowType::Blackman,
    ];
}

fn window_value(win_type: FftWindowType, size: usize, index: usize) -> f64 {
    if size < 2 {
        return 1.0;
    }
    let x = 2.0 * PI * index as f64 / (size as f64 - 1.0);
    match win_type {
        FftWindowType::Rectangular => 1.0,
        FftWindowType::Hann => 0.5 * (1.0 - x.cos()),
        FftWindowType::Hamming => 0.54 - 0.46 * x.cos(),
        FftWindowType::Blackman => 0.42 - 0.5 * x.cos() + 0.08 * (2.0 * x).cos(),
    }
}

pub struct FftResult {
    pub points: Vec<PlotPoint>,
    pub sample_rate: f64,
}

/// Reuse the spectrum while its input window is unchanged (for example while
/// inspecting a paused chart). Compare the actual samples so clearing/reloading
/// history or replacing a value with the same timestamp cannot leave stale FFTs.
#[derive(Default)]
pub struct FftCache {
    samples: Vec<PlotPoint>,
    window_type: Option<FftWindowType>,
    result: Option<Arc<FftResult>>,
}

impl FftCache {
    pub fn get(
        &mut self,
        data: &VecDeque<PlotPoint>,
        sample_count: usize,
        window_type: FftWindowType,
    ) -> Option<Arc<FftResult>> {
        let take = sample_count.clamp(4, MAX_FFT_SIZE).min(data.len());
        let offset = data.len() - take;
        let unchanged = self.window_type == Some(window_type)
            && self.samples.len() == take
            && self
                .samples
                .iter()
                .zip(data.iter().skip(offset))
                .all(|(a, b)| a.x.to_bits() == b.x.to_bits() && a.y.to_bits() == b.y.to_bits());
        if !unchanged {
            self.samples.clear();
            self.samples.extend(data.iter().skip(offset).copied());
            self.window_type = Some(window_type);
            self.result = compute_fft(data, sample_count, window_type).map(Arc::new);
        }
        self.result.clone()
    }
}

const MAX_FFT_SIZE: usize = 65536;
const GAP_FACTOR: f64 = 3.0;
const GAP_BASELINE_INTERVALS: usize = 3;

/// Compute the FFT magnitude spectrum from time-series (timestamp, value) pairs.
///
/// `sample_count`: take at most this many points from the **end** of the data
/// (clamped to `[4, data.len()]`).  `window_type` selects the window function.
///
/// The most recent continuous segment is resampled onto a uniform time grid before the FFT.
/// Returns `None` if there are fewer than 4 usable points or timestamps are invalid.
pub fn compute_fft(
    data: &VecDeque<PlotPoint>,
    sample_count: usize,
    window_type: FftWindowType,
) -> Option<FftResult> {
    let samples = recent_contiguous_samples(data, sample_count)?;
    let (uniform_values, sample_rate) = resample_uniform(&samples)?;
    let take = uniform_values.len();
    let n = take.next_power_of_two();

    let mut signal: Vec<Complex> = vec![Complex::new(0.0, 0.0); n];
    let mut window_sum = 0.0;
    for (i, value) in uniform_values.into_iter().enumerate() {
        let window = window_value(window_type, take, i);
        window_sum += window;
        signal[i] = Complex::new(value * window, 0.0);
    }
    if !window_sum.is_finite() || window_sum.abs() <= f64::EPSILON {
        return None;
    }

    fft(&mut signal);

    let n_half = n / 2;
    let mut points = Vec::with_capacity(n_half + 1);
    for (k, value) in signal.iter().take(n_half + 1).enumerate() {
        let one_sided_scale = if k == 0 || k == n_half { 1.0 } else { 2.0 };
        let magnitude = value.norm() * one_sided_scale / window_sum;
        if !magnitude.is_finite() {
            return None;
        }
        points.push(PlotPoint::new(k as f64 * sample_rate / n as f64, magnitude));
    }

    Some(FftResult {
        points,
        sample_rate,
    })
}

fn recent_contiguous_samples(
    data: &VecDeque<PlotPoint>,
    sample_count: usize,
) -> Option<Vec<PlotPoint>> {
    if data.len() < 4 {
        return None;
    }
    let take = sample_count.clamp(4, MAX_FFT_SIZE).min(data.len());
    let offset = data.len() - take;
    let mut samples = data.iter().skip(offset).copied().collect::<Vec<_>>();
    if samples
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return None;
    }

    let deltas = samples
        .windows(2)
        .map(|pair| pair[1].x - pair[0].x)
        .collect::<Vec<_>>();
    if deltas
        .iter()
        .any(|delta| !delta.is_finite() || *delta <= 0.0)
    {
        return None;
    }
    let segment_start = latest_segment_start(&deltas)?;
    samples.drain(..segment_start);
    (samples.len() >= 4).then_some(samples)
}

fn latest_segment_start(deltas: &[f64]) -> Option<usize> {
    let baseline_start = deltas.len().saturating_sub(GAP_BASELINE_INTERVALS);
    let baseline = median(&deltas[baseline_start..])?;
    for (index, &delta) in deltas.iter().enumerate().rev() {
        let ratio = delta / baseline;
        if !(1.0 / GAP_FACTOR..=GAP_FACTOR).contains(&ratio) {
            return Some(index + 1);
        }
    }
    Some(0)
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    let value = if sorted.len() % 2 == 0 {
        (sorted[middle - 1] + sorted[middle]) * 0.5
    } else {
        sorted[middle]
    };
    (value.is_finite() && value > 0.0).then_some(value)
}

fn resample_uniform(samples: &[PlotPoint]) -> Option<(Vec<f64>, f64)> {
    if samples.len() < 4 {
        return None;
    }
    let first_time = samples.first()?.x;
    let last_time = samples.last()?.x;
    let duration = last_time - first_time;
    if !duration.is_finite() || duration <= 0.0 {
        return None;
    }
    let step = duration / (samples.len() - 1) as f64;
    let sample_rate = 1.0 / step;
    if !sample_rate.is_finite() || sample_rate <= 0.0 {
        return None;
    }

    let mut values = Vec::with_capacity(samples.len());
    let mut left_index = 0usize;
    for index in 0..samples.len() {
        let target_time = if index + 1 == samples.len() {
            last_time
        } else {
            first_time + index as f64 * step
        };
        while left_index + 1 < samples.len() - 1 && samples[left_index + 1].x < target_time {
            left_index += 1;
        }
        let left = samples[left_index];
        let right = samples[left_index + 1];
        let span = right.x - left.x;
        let alpha = ((target_time - left.x) / span).clamp(0.0, 1.0);
        let value = left.y + (right.y - left.y) * alpha;
        if !value.is_finite() {
            return None;
        }
        values.push(value);
    }
    Some((values, sample_rate))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use egui_plot::PlotPoint;

    use std::f64::consts::PI;

    use super::{FftCache, FftWindowType, compute_fft};

    #[test]
    fn reuses_spectrum_while_the_selected_input_window_is_unchanged() {
        let mut history = sine_history(32, 32.0, 4.0, 1.0);
        let mut cache = FftCache::default();
        let first = cache.get(&history, 16, FftWindowType::Hann).unwrap();
        let again = cache.get(&history, 16, FftWindowType::Hann).unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &again));

        // Older points outside the requested FFT window cannot affect it.
        history[0].y = 100.0;
        let unchanged = cache.get(&history, 16, FftWindowType::Hann).unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &unchanged));

        // Even an in-place value replacement with the same timestamp invalidates it.
        history[20].y += 10.0;
        let changed = cache.get(&history, 16, FftWindowType::Hann).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&first, &changed));
        assert_ne!(first.points, changed.points);
    }

    #[test]
    fn invalidates_cached_spectrum_for_options_and_history_reset() {
        let mut history = sine_history(32, 32.0, 4.0, 1.0);
        let mut cache = FftCache::default();
        let first = cache.get(&history, 16, FftWindowType::Hann).unwrap();
        let resized = cache.get(&history, 32, FftWindowType::Hann).unwrap();
        assert_ne!(first.points.len(), resized.points.len());
        let window = cache.get(&history, 32, FftWindowType::Blackman).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&resized, &window));

        history[20].y = f64::NAN;
        assert!(cache.get(&history, 32, FftWindowType::Blackman).is_none());
        history.clear();
        assert!(cache.get(&history, 32, FftWindowType::Blackman).is_none());
        history = sine_history(32, 64.0, 4.0, 2.0);
        let reloaded = cache.get(&history, 32, FftWindowType::Blackman).unwrap();
        assert_eq!(reloaded.sample_rate, 64.0);
        assert!(!std::sync::Arc::ptr_eq(&window, &reloaded));
    }

    fn sine_history(
        count: usize,
        sample_rate: f64,
        frequency: f64,
        amplitude: f64,
    ) -> VecDeque<PlotPoint> {
        (0..count)
            .map(|index| {
                let time = index as f64 / sample_rate;
                PlotPoint::new(time, amplitude * (2.0 * PI * frequency * time).sin())
            })
            .collect()
    }

    fn peak(result: &super::FftResult) -> PlotPoint {
        result
            .points
            .iter()
            .skip(1)
            .copied()
            .max_by(|left, right| left.y.total_cmp(&right.y))
            .unwrap()
    }

    #[test]
    fn computes_spectrum_directly_from_plot_history() {
        let history = (0..8)
            .map(|index| {
                let time = index as f64 * 0.001;
                PlotPoint::new(time, (index as f64).sin())
            })
            .collect::<VecDeque<_>>();

        let result = compute_fft(&history, 8, FftWindowType::Rectangular).unwrap();
        assert_eq!(result.points.len(), 5);
        assert!((result.sample_rate - 1000.0).abs() < 1e-6);
        assert!(result.points.windows(2).all(|pair| pair[0].x < pair[1].x));
        assert!(result.points.iter().all(|point| point.y.is_finite()));
    }

    #[test]
    fn windowed_sine_amplitude_is_coherent_gain_corrected() {
        let amplitude = 2.5;
        let history = sine_history(1024, 1024.0, 64.0, amplitude);

        for &window in FftWindowType::ALL {
            let result = compute_fft(&history, 1024, window).unwrap();
            let peak = peak(&result);
            assert!((peak.x - 64.0).abs() < 1e-9, "{}", window.label());
            assert!((peak.y - amplitude).abs() < 0.01, "{}", window.label());
        }
    }

    #[test]
    fn dc_and_nyquist_bins_are_present_and_not_doubled() {
        let dc = (0..1024)
            .map(|index| PlotPoint::new(index as f64 / 1024.0, 3.0))
            .collect::<VecDeque<_>>();
        let nyquist = (0..1024)
            .map(|index| {
                let value = if index % 2 == 0 { 2.0 } else { -2.0 };
                PlotPoint::new(index as f64 / 1024.0, value)
            })
            .collect::<VecDeque<_>>();

        for &window in FftWindowType::ALL {
            let dc_result = compute_fft(&dc, 1024, window).unwrap();
            assert_eq!(dc_result.points.len(), 513);
            assert!((dc_result.points[0].y - 3.0).abs() < 1e-12);
            assert!((dc_result.points.last().unwrap().x - 512.0).abs() < 1e-12);

            let nyquist_result = compute_fft(&nyquist, 1024, window).unwrap();
            assert!((nyquist_result.points.last().unwrap().y - 2.0).abs() < 1e-12);
        }
    }

    #[test]
    fn uses_only_the_latest_continuous_segment_after_a_pause() {
        let mut history = sine_history(8, 1000.0, 125.0, 1.0);
        history.extend((0..8).map(|index| {
            let time = 1.0 + index as f64 / 2000.0;
            PlotPoint::new(time, (2.0 * PI * 250.0 * time).sin())
        }));

        let result = compute_fft(&history, 16, FftWindowType::Rectangular).unwrap();
        assert!((result.sample_rate - 2000.0).abs() < 1e-9);
        assert_eq!(result.points.len(), 5);
    }

    #[test]
    fn detects_a_pause_even_when_the_new_segment_has_a_slower_rate() {
        let mut history = sine_history(16, 1000.0, 125.0, 1.0);
        history.extend((0..4).map(|index| {
            let time = 1.0 + index as f64 / 200.0;
            PlotPoint::new(time, (2.0 * PI * 25.0 * time).sin())
        }));

        let result = compute_fft(&history, 20, FftWindowType::Rectangular).unwrap();
        assert!((result.sample_rate - 200.0).abs() < 1e-9);
        assert_eq!(result.points.len(), 3);
    }

    #[test]
    fn resamples_timestamp_jitter_before_transforming() {
        let history = (0..1024)
            .map(|index| {
                let jitter = if index % 2 == 0 { 0.0 } else { 0.00015 };
                let time = index as f64 / 1000.0 + jitter;
                PlotPoint::new(time, (2.0 * PI * 50.0 * time).sin())
            })
            .collect::<VecDeque<_>>();

        let result = compute_fft(&history, 1024, FftWindowType::Hann).unwrap();
        assert!((peak(&result).x - 50.0).abs() < 1.0);
    }

    #[test]
    fn rejects_non_finite_or_non_increasing_samples() {
        let mut duplicate_time = sine_history(8, 1000.0, 125.0, 1.0);
        duplicate_time[4].x = duplicate_time[3].x;
        assert!(compute_fft(&duplicate_time, 8, FftWindowType::Hann).is_none());

        let mut non_finite = sine_history(8, 1000.0, 125.0, 1.0);
        non_finite[4].y = f64::NAN;
        assert!(compute_fft(&non_finite, 8, FftWindowType::Hann).is_none());
    }
}
