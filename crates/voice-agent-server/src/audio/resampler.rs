//! Stateful downlink sample-rate conversion.

use std::collections::VecDeque;

use super::AudioError;

const TAPS: usize = 63;
const CUTOFF: f32 = 0.225;

/// A fixed 48 kHz to 24 kHz low-pass FIR decimator.  The delay line deliberately lives for
/// the whole delivery generation: resetting it for every provider chunk creates clicks at the
/// chunk boundary and permits high-frequency aliasing into the downlink band.
pub struct DownlinkResampler {
    coefficients: [f32; TAPS],
    history: VecDeque<f32>,
    input_index: usize,
}

impl DownlinkResampler {
    pub fn new_48k_to_24k() -> Self {
        let mut coefficients = [0.0; TAPS];
        let midpoint = (TAPS - 1) as f32 / 2.0;
        let mut sum = 0.0;
        for (index, coefficient) in coefficients.iter_mut().enumerate() {
            let offset = index as f32 - midpoint;
            let sinc = if offset == 0.0 {
                2.0 * CUTOFF
            } else {
                (2.0 * std::f32::consts::PI * CUTOFF * offset).sin()
                    / (std::f32::consts::PI * offset)
            };
            let window =
                0.54 - 0.46 * (2.0 * std::f32::consts::PI * index as f32 / (TAPS - 1) as f32).cos();
            *coefficient = sinc * window;
            sum += *coefficient;
        }
        for coefficient in &mut coefficients {
            *coefficient /= sum;
        }
        Self {
            coefficients,
            history: VecDeque::from(vec![0.0; TAPS]),
            input_index: 0,
        }
    }

    pub fn process(&mut self, input: &[f32]) -> Result<Vec<f32>, AudioError> {
        let mut output = Vec::with_capacity(input.len() / 2);
        for &sample in input {
            if !sample.is_finite() {
                return Err(AudioError::InvalidDownlinkPcm);
            }
            self.history.pop_front();
            self.history.push_back(sample);
            // Decimate only after low-pass filtering.  The parity is generation-global rather
            // than chunk-local, so arbitrary provider chunk sizes cannot shift the waveform.
            if self.input_index.is_multiple_of(2) {
                let filtered = self
                    .history
                    .iter()
                    .zip(self.coefficients.iter())
                    .map(|(sample, coefficient)| sample * coefficient)
                    .sum();
                output.push(filtered);
            }
            self.input_index += 1;
        }
        Ok(output)
    }

    /// The causal filter has already emitted the downlink samples aligned with every admitted
    /// input sample.  We intentionally do not append zero-tail audio at a response boundary.
    pub fn flush(&mut self) -> Vec<f32> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::DownlinkResampler;

    #[test]
    fn keeps_phase_across_arbitrary_chunks() {
        let input = (0..960).map(|i| (i as f32 / 8.0).sin()).collect::<Vec<_>>();
        let mut whole = DownlinkResampler::new_48k_to_24k();
        let expected = whole.process(&input).unwrap();
        let mut chunked = DownlinkResampler::new_48k_to_24k();
        let mut actual = chunked.process(&input[..137]).unwrap();
        actual.extend(chunked.process(&input[137..]).unwrap());
        assert_eq!(actual, expected);
    }

    #[test]
    fn attenuates_content_above_the_24khz_nyquist_band() {
        let sine = |hz: f32| {
            (0..4_800)
                .map(|index| (2.0 * std::f32::consts::PI * hz * index as f32 / 48_000.0).sin())
                .collect::<Vec<_>>()
        };
        let rms = |samples: &[f32]| {
            (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32)
                .sqrt()
        };
        let mut pass = DownlinkResampler::new_48k_to_24k();
        let low = pass.process(&sine(1_000.0)).unwrap();
        let mut stop = DownlinkResampler::new_48k_to_24k();
        let high = stop.process(&sine(18_000.0)).unwrap();
        assert!(rms(&high[64..]) < rms(&low[64..]) * 0.05);
    }
}
