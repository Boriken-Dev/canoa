//! The mel filter bank that a GGML file carries: `librosa.filters.mel(sr=16000, n_fft=400, n_mels=n)` with its defaults.

use alloc::vec;
use alloc::vec::Vec;

/// The number of FFT bins a filter spans: `n_fft / 2 + 1` for `n_fft` 400.
pub(crate) const BINS: usize = 201;

/// `n_mels` filters of [`BINS`] weights each (Slaney scale and norm), rounded to f32 as numpy does: the triangles first, then
/// the band normalisation in f64, then f32 again.
pub(crate) fn filters(n_mels: usize) -> Vec<f32> {
    const SAMPLE_RATE: f64 = 16_000.0;
    let hz_to_mel = |hz: f64| {
        if hz >= 1000.0 {
            15.0 + (hz / 1000.0).ln() / (6.4f64.ln() / 27.0)
        } else {
            hz / (200.0 / 3.0)
        }
    };
    let mel_to_hz = |mel: f64| {
        if mel >= 15.0 {
            1000.0 * ((6.4f64.ln() / 27.0) * (mel - 15.0)).exp()
        } else {
            mel * (200.0 / 3.0)
        }
    };
    let fft_freqs: Vec<f64> = (0..BINS)
        .map(|bin| (SAMPLE_RATE / 2.0) * bin as f64 / (BINS - 1) as f64)
        .collect();
    let (low, high) = (hz_to_mel(0.0), hz_to_mel(SAMPLE_RATE / 2.0));
    let centres: Vec<f64> = (0..n_mels + 2)
        .map(|index| mel_to_hz(low + (high - low) * index as f64 / (n_mels + 1) as f64))
        .collect();
    let mut out = vec![0f32; n_mels * BINS];
    for mel in 0..n_mels {
        let norm = 2.0 / (centres[mel + 2] - centres[mel]);
        for (bin, freq) in fft_freqs.iter().enumerate() {
            let lower = -(centres[mel] - freq) / (centres[mel + 1] - centres[mel]);
            let upper = (centres[mel + 2] - freq) / (centres[mel + 2] - centres[mel + 1]);
            let weight = lower.min(upper).max(0.0) as f32;
            out[mel * BINS + bin] = (f64::from(weight) * norm) as f32;
        }
    }
    out
}
