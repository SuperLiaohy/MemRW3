use std::f64::consts::PI;
use std::ops::{Add, Mul, Sub};

#[derive(Clone, Copy)]
struct Complex {
    re: f64,
    im: f64,
}

impl Complex {
    fn new(re: f64, im: f64) -> Self { Self { re, im } }
    fn norm(self) -> f64 { (self.re * self.re + self.im * self.im).sqrt() }
}

impl Add for Complex {
    type Output = Self;
    fn add(self, rhs: Self) -> Self { Self { re: self.re + rhs.re, im: self.im + rhs.im } }
}

impl Sub for Complex {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self { Self { re: self.re - rhs.re, im: self.im - rhs.im } }
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

fn generate_window(win_type: FftWindowType, size: usize) -> Vec<f64> {
    if size < 2 {
        return vec![1.0; size];
    }
    let n = size as f64;
    match win_type {
        FftWindowType::Rectangular => vec![1.0; size],
        FftWindowType::Hann => (0..size)
            .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f64 / (n - 1.0)).cos()))
            .collect(),
        FftWindowType::Hamming => (0..size)
            .map(|i| 0.54 - 0.46 * (2.0 * PI * i as f64 / (n - 1.0)).cos())
            .collect(),
        FftWindowType::Blackman => {
            let a0 = 0.42;
            let a1 = 0.5;
            let a2 = 0.08;
            (0..size)
                .map(|i| {
                    let x = 2.0 * PI * i as f64 / (n - 1.0);
                    a0 - a1 * x.cos() + a2 * (2.0 * x).cos()
                })
                .collect()
        }
    }
}

pub struct FftResult {
    pub frequencies: Vec<f64>,
    pub magnitudes: Vec<f64>,
    pub sample_rate: f64,
}

/// Compute the FFT magnitude spectrum from time-series (timestamp, value) pairs.
///
/// `sample_count`: take at most this many points from the **end** of the data
/// (clamped to `[4, data.len()]`).  `window_type` selects the window function.
///
/// Returns `None` if there are fewer than 4 usable points.
pub fn compute_fft(data: &[(f64, f64)], sample_count: usize, window_type: FftWindowType) -> Option<FftResult> {
    let total = data.len();
    if total < 4 {
        return None;
    }
    let desired_take = sample_count.min(total).max(4);
    let n = (desired_take.next_power_of_two()).min(65536);
    let take = desired_take.min(n);

    let offset = total - take;
    let slice = &data[offset..];

    let t_first = slice.first()?.0;
    let t_last = slice.last()?.0;
    let duration = (t_last - t_first).max(0.0);
    let sample_rate = if duration > 0.0 {
        (take - 1) as f64 / duration
    } else {
        1.0
    };

    let window = generate_window(window_type, take);

    let mut signal: Vec<Complex> = vec![Complex::new(0.0, 0.0); n];
    for i in 0..take {
        signal[i] = Complex::new(slice[i].1 * window[i], 0.0);
    }

    fft(&mut signal);

    let n_half = n / 2;
    let mut frequencies = Vec::with_capacity(n_half);
    let mut magnitudes = Vec::with_capacity(n_half);
    for k in 0..n_half {
        frequencies.push(k as f64 * sample_rate / n as f64);
        magnitudes.push(signal[k].norm() / take as f64 * 2.0);
    }

    Some(FftResult {
        frequencies,
        magnitudes,
        sample_rate,
    })
}
