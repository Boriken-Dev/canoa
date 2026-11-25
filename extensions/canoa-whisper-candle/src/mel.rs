//! The log-mel spectrogram Whisper reads, as OpenAI's reference makes it: a Hann-windowed STFT of 400 samples every 160 (reflection
//! padded, the last frame dropped), the power on a Slaney mel scale, then `log10`, clamped 8 below the clip's loudest value and
//! scaled to roughly -1 to 1. Single threaded, so it runs where threads do not.

use core::f32::consts::PI;

/// The samples in one window.
pub(crate) const N_FFT: usize = 400;
/// The samples between two windows.
pub(crate) const HOP: usize = 160;
/// The rate Whisper reads.
pub(crate) const SAMPLE_RATE: f64 = 16_000.0;

/// The bins of the spectrum a window has.
const BINS: usize = N_FFT / 2 + 1;

/// Hertz per mel below 1 kHz on the Slaney scale.
const LINEAR: f64 = 200.0 / 3.0;

fn hz_to_mel(hz: f64) -> f64 {
    if hz < 1000.0 {
        hz / LINEAR
    } else {
        1000.0 / LINEAR + (hz / 1000.0).ln() / (6.4_f64.ln() / 27.0)
    }
}

fn mel_to_hz(mel: f64) -> f64 {
    let knee = 1000.0 / LINEAR;
    if mel < knee {
        mel * LINEAR
    } else {
        1000.0 * ((mel - knee) * (6.4_f64.ln() / 27.0)).exp()
    }
}

/// The triangular filters of a Slaney mel scale from 0 Hz to half the rate, each scaled to unit area, as `librosa.filters.mel` makes
/// them: `n_mels` rows of [`BINS`] weights.
pub(crate) fn filters(n_mels: usize) -> Vec<f32> {
    let top = hz_to_mel(SAMPLE_RATE / 2.0);
    let points: Vec<f64> = (0..n_mels + 2)
        .map(|index| mel_to_hz(top * index as f64 / (n_mels + 1) as f64))
        .collect();
    let mut weights = vec![0.0_f32; n_mels * BINS];
    for mel in 0..n_mels {
        let (left, centre, right) = (points[mel], points[mel + 1], points[mel + 2]);
        let norm = 2.0 / (right - left);
        for bin in 0..BINS {
            let hz = bin as f64 * (SAMPLE_RATE / 2.0) / (BINS - 1) as f64;
            let lower = (hz - left) / (centre - left);
            let upper = (right - hz) / (right - centre);
            weights[mel * BINS + bin] = (lower.min(upper).max(0.0) * norm) as f32;
        }
    }
    weights
}

type Complex = (f32, f32);

/// The 400th roots of unity, which every length that divides 400 reads its own from.
fn roots() -> Vec<Complex> {
    (0..N_FFT)
        .map(|k| {
            let angle = -2.0 * PI * k as f32 / N_FFT as f32;
            (angle.cos(), angle.sin())
        })
        .collect()
}

/// The discrete Fourier transform of `input`, whose length divides 400: even lengths split in two, the rest are summed out.
/// `twiddles` is [`roots`], and `stride` is 400 over the length.
fn fft(input: &[Complex], twiddles: &[Complex], stride: usize) -> Vec<Complex> {
    let n = input.len();
    if n == 1 {
        return input.to_vec();
    }
    if n % 2 == 1 {
        return (0..n)
            .map(|k| {
                input
                    .iter()
                    .enumerate()
                    .fold((0.0, 0.0), |sum, (j, value)| {
                        let w = twiddles[(j * k % n) * stride];
                        (
                            sum.0 + value.0 * w.0 - value.1 * w.1,
                            sum.1 + value.0 * w.1 + value.1 * w.0,
                        )
                    })
            })
            .collect();
    }
    let even: Vec<Complex> = input.iter().step_by(2).copied().collect();
    let odd: Vec<Complex> = input.iter().skip(1).step_by(2).copied().collect();
    let (even, odd) = (
        fft(&even, twiddles, stride * 2),
        fft(&odd, twiddles, stride * 2),
    );
    let half = n / 2;
    let mut out = vec![(0.0, 0.0); n];
    for k in 0..half {
        let w = twiddles[k * stride];
        let t = (
            odd[k].0 * w.0 - odd[k].1 * w.1,
            odd[k].0 * w.1 + odd[k].1 * w.0,
        );
        out[k] = (even[k].0 + t.0, even[k].1 + t.1);
        out[k + half] = (even[k].0 - t.0, even[k].1 - t.1);
    }
    out
}

/// The log-mel spectrogram of 16 kHz mono `samples`: `n_mels` rows of `samples.len() / 160` frames, row after row, ready to be a
/// tensor of shape (1, n_mels, frames). `filters` is [`filters`] for the same `n_mels`.
pub(crate) fn log_mel(samples: &[f32], filters: &[f32], n_mels: usize) -> (Vec<f32>, usize) {
    let frames = samples.len() / HOP;
    if frames == 0 {
        return (Vec::new(), 0);
    }
    let twiddles = roots();
    let window: Vec<f32> = (0..N_FFT)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / N_FFT as f32).cos())
        .collect();
    // The signal reflected at both ends by half a window, so a frame is centred on its hop.
    let half = (N_FFT / 2) as isize;
    let last = samples.len() as isize - 1;
    let at = |index: isize| -> f32 {
        let mut index = index.abs();
        if index > last {
            index = (2 * last - index).max(0);
        }
        samples[index as usize]
    };
    let mut mel = vec![0.0_f32; n_mels * frames];
    let mut power = vec![0.0_f32; BINS];
    for frame in 0..frames {
        let start = (frame * HOP) as isize - half;
        let input: Vec<Complex> = (0..N_FFT)
            .map(|i| (at(start + i as isize) * window[i], 0.0))
            .collect();
        let spectrum = fft(&input, &twiddles, 1);
        for (bin, slot) in power.iter_mut().enumerate() {
            *slot = spectrum[bin].0 * spectrum[bin].0 + spectrum[bin].1 * spectrum[bin].1;
        }
        for row in 0..n_mels {
            let weights = &filters[row * BINS..(row + 1) * BINS];
            let sum: f32 = weights.iter().zip(&power).map(|(w, p)| w * p).sum();
            mel[row * frames + frame] = sum.max(1e-10).log10();
        }
    }
    let loudest = mel.iter().copied().fold(f32::MIN, f32::max);
    for value in &mut mel {
        *value = (value.max(loudest - 8.0) + 4.0) / 4.0;
    }
    (mel, frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filters_are_a_slaney_bank_whose_rows_each_hold_their_own_band() {
        let bank = filters(80);
        assert_eq!(bank.len(), 80 * BINS);
        // The first filter starts at 0 Hz, peaks at the second mel point (about 1.6 bins) and each row is a triangle of unit area
        // over the scale's width, which is 2 / (right - left) high.
        let rows: Vec<&[f32]> = bank.chunks(BINS).collect();
        for (index, row) in rows.iter().enumerate() {
            assert!(row.iter().any(|weight| *weight > 0.0), "row {index}");
        }
        let peak = |row: &[f32]| {
            row.iter()
                .position(|w| *w == row.iter().copied().fold(0.0, f32::max))
                .unwrap()
        };
        assert!(peak(rows[10]) < peak(rows[40]) && peak(rows[40]) < peak(rows[79]));
        // The width of a mel step is 12.5 Hz at the bottom and 3.5 times as much near 8 kHz, so the area per bin falls with the row.
        assert!(
            rows[0].iter().copied().fold(0.0, f32::max)
                > rows[79].iter().copied().fold(0.0, f32::max)
        );
    }

    #[test]
    fn a_pure_tone_lands_in_the_mel_row_that_holds_its_frequency() {
        let samples: Vec<f32> = (0..16_000)
            .map(|i| (2.0 * PI * 1000.0 * i as f32 / 16_000.0).sin())
            .collect();
        let (mel, frames) = log_mel(&samples, &filters(80), 80);
        assert_eq!(frames, 100);
        let middle = 50;
        let loudest = (0..80)
            .max_by(|a, b| mel[a * frames + middle].total_cmp(&mel[b * frames + middle]))
            .unwrap();
        // 1 kHz is mel 15 on a scale of 45.2 mel at 8 kHz, split in 81 steps: row 26 holds it.
        assert!((25..=27).contains(&loudest), "{loudest}");
        assert!(mel.iter().all(|value| (-1.0..=2.0).contains(value)));
    }

    #[test]
    fn the_fft_of_an_impulse_is_flat_and_of_a_constant_is_one_bin() {
        let twiddles = roots();
        let mut impulse = vec![(0.0, 0.0); N_FFT];
        impulse[0] = (1.0, 0.0);
        let flat = fft(&impulse, &twiddles, 1);
        assert!(
            flat.iter()
                .all(|v| (v.0 - 1.0).abs() < 1e-5 && v.1.abs() < 1e-5)
        );
        let constant = vec![(1.0, 0.0); N_FFT];
        let one = fft(&constant, &twiddles, 1);
        assert!((one[0].0 - 400.0).abs() < 1e-2);
        assert!(
            one[1..]
                .iter()
                .all(|v| v.0.abs() < 1e-2 && v.1.abs() < 1e-2)
        );
    }
}
