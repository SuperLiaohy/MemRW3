use std::collections::VecDeque;
use std::f64::consts::PI;
use std::ops::{Add, Mul, Sub};

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
        (self.re * self.re + self.im * self.im).sqrt()
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

/// Compute the FFT magnitude spectrum from time-series (timestamp, value) pairs.
///
/// `sample_count`: take at most this many points from the **end** of the data
/// (clamped to `[4, data.len()]`).  `window_type` selects the window function.
///
/// Returns `None` if there are fewer than 4 usable points.
pub fn compute_fft(
    data: &VecDeque<PlotPoint>,
    sample_count: usize,
    window_type: FftWindowType,
) -> Option<FftResult> {
    let total = data.len();
    if total < 4 {
        return None;
    }
    let desired_take = sample_count.min(total).max(4);
    let n = (desired_take.next_power_of_two()).min(65536);
    let take = desired_take.min(n);

    let offset = total - take;
    let t_first = data.get(offset)?.x;
    let t_last = data.back()?.x;
    let duration = (t_last - t_first).max(0.0);
    let sample_rate = if duration > 0.0 {
        (take - 1) as f64 / duration
    } else {
        1.0
    };

    let mut signal: Vec<Complex> = vec![Complex::new(0.0, 0.0); n];
    for (i, point) in data.iter().skip(offset).take(take).enumerate() {
        signal[i] = Complex::new(point.y * window_value(window_type, take, i), 0.0);
    }

    fft(&mut signal);

    let n_half = n / 2;
    let mut points = Vec::with_capacity(n_half);
    for (k, value) in signal.iter().take(n_half).enumerate() {
        points.push(PlotPoint::new(
            k as f64 * sample_rate / n as f64,
            (*value).norm() / take as f64 * 2.0,
        ));
    }

    Some(FftResult {
        points,
        sample_rate,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use egui_plot::PlotPoint;

    use super::{FftWindowType, compute_fft};

    #[test]
    fn computes_spectrum_directly_from_plot_history() {
        let history = (0..8)
            .map(|index| {
                let time = index as f64 * 0.001;
                PlotPoint::new(time, (index as f64).sin())
            })
            .collect::<VecDeque<_>>();

        let result = compute_fft(&history, 8, FftWindowType::Rectangular).unwrap();
        assert_eq!(result.points.len(), 4);
        assert!((result.sample_rate - 1000.0).abs() < 1e-6);
        assert!(result.points.windows(2).all(|pair| pair[0].x < pair[1].x));
        assert!(result.points.iter().all(|point| point.y.is_finite()));
    }
}
