//! Pure-Rust DSP post-processing for ViFastTts:
//! - 3×3 box-blur unsharp masking on mel spectrograms (`sharpen_mel`)
//! - 1024-point radix-2 inverse STFT with periodic Hann window (`istft`)
//! - Linear fade-in / fade-out (`fade`)

use std::f64::consts::PI;

pub const DEFAULT_SHARPEN_ALPHA: f32 = 1.5;
pub const DEFAULT_FADE_IN_MS: f32 = 10.0;
pub const DEFAULT_FADE_OUT_MS: f32 = 20.0;

pub fn sharpen_mel(mel: &mut [f32], frames: usize, n_mels: usize, alpha: f32) {
    if frames == 0 || n_mels == 0 || mel.len() != frames.saturating_mul(n_mels) {
        return;
    }
    let orig = mel.to_vec();
    for t in 0..frames {
        let t_min = t.saturating_sub(1);
        let t_max = (t + 1).min(frames - 1);
        for m in 0..n_mels {
            let m_min = m.saturating_sub(1);
            let m_max = (m + 1).min(n_mels - 1);
            let mut sum = 0.0_f32;
            for ti in t_min..=t_max {
                let row = ti * n_mels;
                for mi in m_min..=m_max {
                    sum += orig[row + mi];
                }
            }
            let blur = sum / 9.0;
            let val = orig[t * n_mels + m];
            mel[t * n_mels + m] = val + alpha * (val - blur);
        }
    }
}

fn ifft_radix2(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    debug_assert_eq!(n, im.len());
    if n <= 1 {
        return;
    }

    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    let mut len = 2usize;
    while len <= n {
        let half = len / 2;
        let angle_step = 2.0 * PI / (len as f64);
        for start in (0..n).step_by(len) {
            for k in 0..half {
                let angle = angle_step * (k as f64);
                let (w_im, w_re) = angle.sin_cos();
                let w_re = w_re as f32;
                let w_im = w_im as f32;

                let idx_even = start + k;
                let idx_odd = idx_even + half;

                let u_re = re[idx_even];
                let u_im = im[idx_even];
                let v_re = re[idx_odd] * w_re - im[idx_odd] * w_im;
                let v_im = re[idx_odd] * w_im + im[idx_odd] * w_re;

                re[idx_even] = u_re + v_re;
                im[idx_even] = u_im + v_im;
                re[idx_odd] = u_re - v_re;
                im[idx_odd] = u_im - v_im;
            }
        }
        len <<= 1;
    }

    let inv_n = 1.0 / (n as f32);
    for (r, i) in re.iter_mut().zip(im.iter_mut()) {
        *r *= inv_n;
        *i *= inv_n;
    }
}

pub fn istft(
    spec_real: &[f32],
    spec_imag: &[f32],
    freq_bins: usize,
    frames: usize,
    n_fft: usize,
    hop_length: usize,
) -> Vec<f32> {
    if frames == 0
        || n_fft == 0
        || hop_length == 0
        || !n_fft.is_power_of_two()
        || freq_bins != n_fft / 2 + 1
        || spec_real.len() != freq_bins.saturating_mul(frames)
        || spec_imag.len() != freq_bins.saturating_mul(frames)
    {
        return Vec::new();
    }

    let window: Vec<f32> = (0..n_fft)
        .map(|i| (0.5 - 0.5 * (2.0 * PI * (i as f64) / (n_fft as f64)).cos()) as f32)
        .collect();
    let window_sq: Vec<f32> = window.iter().map(|&w| w * w).collect();

    let total_len = n_fft + frames.saturating_sub(1) * hop_length;
    let mut output = vec![0.0_f32; total_len];
    let mut envelope = vec![0.0_f32; total_len];

    let mut re = vec![0.0_f32; n_fft];
    let mut im = vec![0.0_f32; n_fft];

    for t in 0..frames {
        for k in 0..freq_bins {
            let idx = k * frames + t;
            re[k] = spec_real[idx];
            im[k] = spec_imag[idx];
        }
        for k in 1..(n_fft / 2) {
            re[n_fft - k] = re[k];
            im[n_fft - k] = -im[k];
        }

        ifft_radix2(&mut re, &mut im);

        let start = t * hop_length;
        for i in 0..n_fft {
            output[start + i] += re[i] * window[i];
            envelope[start + i] += window_sq[i];
        }
    }

    for (out, &env) in output.iter_mut().zip(envelope.iter()) {
        *out /= env.max(1e-11);
    }

    let pad = n_fft / 2;
    if total_len > 2 * pad {
        output[pad..total_len - pad].to_vec()
    } else {
        output
    }
}

pub fn fade(audio: &mut [f32], sample_rate: u32, fade_in_ms: f32, fade_out_ms: f32) {
    let n = audio.len();
    if n == 0 {
        return;
    }
    let fi = ((sample_rate as f32 * fade_in_ms / 1000.0) as usize).min(n);
    let fo = ((sample_rate as f32 * fade_out_ms / 1000.0) as usize).min(n);

    if fi > 1 {
        let denom = (fi - 1) as f32;
        for (i, sample) in audio[..fi].iter_mut().enumerate() {
            *sample *= (i as f32) / denom;
        }
    }
    if fo > 1 {
        let denom = (fo - 1) as f32;
        let start = n - fo;
        for (i, sample) in audio[start..].iter_mut().enumerate() {
            *sample *= ((fo - 1 - i) as f32) / denom;
        }
    }
}
