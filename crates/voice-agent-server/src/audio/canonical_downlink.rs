use crate::audio::{
    AudioError, DOWNLINK_FRAME_SAMPLES, DownlinkOpusEncoder, DownlinkPcmFrame, DownlinkResampler,
    OpusPacket, Pcm16Mono, PcmF32Mono,
};

const ZEROTTS_SAMPLE_RATE_HZ: u32 = 48_000;
const CANONICAL_DOWNLINK_SAMPLE_RATE_HZ: u32 = 24_000;
const FADE_IN_SAMPLES: usize = 48_000 * 8 / 1_000;
/// The production terminal fade is applied after resampling, so this is 20 ms at 24 kHz.
const FADE_OUT_SAMPLES: usize = 480;

/// Deterministic provider-PCM to canonical downlink-Opus conversion shared by delivery paths.
pub struct CanonicalDownlinkPipeline {
    encoder: DownlinkOpusEncoder,
    resampler: DownlinkResampler,
    provider_sample_rate_hz: Option<u32>,
    first_pcm_chunk: bool,
    downlink_tail: Vec<i16>,
}

impl CanonicalDownlinkPipeline {
    pub fn new(max_packet_bytes: usize) -> Result<Self, AudioError> {
        Ok(Self {
            encoder: DownlinkOpusEncoder::new(max_packet_bytes)?,
            resampler: DownlinkResampler::new_48k_to_24k(),
            provider_sample_rate_hz: None,
            first_pcm_chunk: true,
            downlink_tail: Vec::new(),
        })
    }

    pub fn push_provider_pcm(
        &mut self,
        mut pcm: PcmF32Mono,
    ) -> Result<Vec<OpusPacket>, AudioError> {
        let sample_rate_hz = pcm.sample_rate_hz();
        if !matches!(
            sample_rate_hz,
            ZEROTTS_SAMPLE_RATE_HZ | CANONICAL_DOWNLINK_SAMPLE_RATE_HZ
        ) || pcm.samples().is_empty()
            || self
                .provider_sample_rate_hz
                .is_some_and(|rate| rate != sample_rate_hz)
        {
            return Err(AudioError::InvalidProviderPcm);
        }
        self.provider_sample_rate_hz = Some(sample_rate_hz);
        if self.first_pcm_chunk {
            let fade_samples = FADE_IN_SAMPLES.min(pcm.samples().len());
            for (index, sample) in pcm.samples_mut()[..fade_samples].iter_mut().enumerate() {
                *sample *= index as f32 / fade_samples as f32;
            }
            self.first_pcm_chunk = false;
        }
        let downlink_pcm = match sample_rate_hz {
            ZEROTTS_SAMPLE_RATE_HZ => self.resampler.process(pcm.samples())?,
            CANONICAL_DOWNLINK_SAMPLE_RATE_HZ => {
                if pcm.samples().iter().any(|sample| !sample.is_finite()) {
                    return Err(AudioError::InvalidProviderPcm);
                }
                pcm.samples().to_vec()
            }
            _ => unreachable!("rate validated above"),
        };
        self.downlink_tail
            .extend(downlink_pcm.into_iter().map(float_to_i16));
        self.take_complete_packets()
    }

    pub fn finish(&mut self) -> Result<Vec<OpusPacket>, AudioError> {
        if self.provider_sample_rate_hz == Some(ZEROTTS_SAMPLE_RATE_HZ) {
            self.downlink_tail
                .extend(self.resampler.flush().into_iter().map(float_to_i16));
        }
        if self.downlink_tail.is_empty() {
            return Ok(Vec::new());
        }
        fade_out_tail(&mut self.downlink_tail);
        self.downlink_tail.resize(DOWNLINK_FRAME_SAMPLES, 0);
        let frame = std::mem::take(&mut self.downlink_tail);
        Ok(vec![self.encoder.encode(DownlinkPcmFrame::try_new(
            Pcm16Mono::new(frame),
        )?)?])
    }

    fn take_complete_packets(&mut self) -> Result<Vec<OpusPacket>, AudioError> {
        let mut packets = Vec::new();
        while self.downlink_tail.len() >= DOWNLINK_FRAME_SAMPLES {
            let frame = self
                .downlink_tail
                .drain(..DOWNLINK_FRAME_SAMPLES)
                .collect::<Vec<_>>();
            packets.push(
                self.encoder
                    .encode(DownlinkPcmFrame::try_new(Pcm16Mono::new(frame))?)?,
            );
        }
        Ok(packets)
    }
}

fn float_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16
}

pub(crate) fn fade_out_tail(samples: &mut [i16]) {
    let fade_samples = FADE_OUT_SAMPLES.min(samples.len());
    if fade_samples <= 1 {
        return;
    }
    let fade_start = samples.len() - fade_samples;
    for (index, sample) in samples[fade_start..].iter_mut().enumerate() {
        *sample =
            (f32::from(*sample) * (1.0 - index as f32 / (fade_samples - 1) as f32)).round() as i16;
    }
}

#[cfg(test)]
mod tests {
    use super::CanonicalDownlinkPipeline;
    use crate::audio::PcmF32Mono;

    #[test]
    fn finalizes_a_partial_provider_tail_as_one_canonical_packet() {
        let mut pipeline = CanonicalDownlinkPipeline::new(4_000).unwrap();
        assert!(
            pipeline
                .push_provider_pcm(PcmF32Mono::new(vec![0.25; 960], 48_000))
                .unwrap()
                .is_empty()
        );
        assert_eq!(pipeline.finish().unwrap().len(), 1);
    }

    #[test]
    fn accepts_canonical_24khz_provider_pcm_without_resampling() {
        let mut pipeline = CanonicalDownlinkPipeline::new(4_000).unwrap();
        assert_eq!(
            pipeline
                .push_provider_pcm(PcmF32Mono::new(vec![0.25; 1_440], 24_000))
                .unwrap()
                .len(),
            1
        );
    }
}
