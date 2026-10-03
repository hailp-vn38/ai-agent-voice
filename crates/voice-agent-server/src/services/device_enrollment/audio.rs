use crate::audio::{
    DOWNLINK_FRAME_SAMPLES, DownlinkOpusEncoder, DownlinkPcmFrame, MAX_DOWNLINK_OPUS_PACKET_BYTES,
    Pcm16Mono,
};
use std::path::Path;
use thiserror::Error;

const RATE: usize = 24_000;
const MAX_PROMPT_SAMPLES: usize = RATE * 15;
const GAP_SAMPLES: usize = RATE / 10;
const EDGE_SAMPLES: usize = RATE / 200;

#[derive(Debug, Error)]
#[error("enrollment_prompt_unavailable")]
pub struct PromptError;

/// Immutable PCM; a fresh encoder processes the entire assembled prompt each time.
pub struct PromptAssets {
    intro: Vec<i16>,
    digits: Vec<Vec<i16>>,
}

impl PromptAssets {
    pub fn load(directory: &Path) -> Result<Self, PromptError> {
        let intro = load_clip(&directory.join("intro.wav"), RATE * 6)?;
        let mut digits = Vec::with_capacity(10);
        for digit in 0..10 {
            digits.push(load_clip(&directory.join(format!("{digit}.wav")), RATE)?);
        }
        Ok(Self { intro, digits })
    }

    fn assemble(&self, code: &str) -> Result<Vec<i16>, PromptError> {
        if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(PromptError);
        }
        let mut pcm = Vec::with_capacity(MAX_PROMPT_SAMPLES);
        append_clip(&mut pcm, &self.intro);
        for digit in code.bytes() {
            pcm.resize(pcm.len() + GAP_SAMPLES, 0);
            append_clip(&mut pcm, &self.digits[usize::from(digit - b'0')]);
        }
        if pcm.len() > MAX_PROMPT_SAMPLES {
            return Err(PromptError);
        }
        // Same terminal 20 ms fade used by production canonical downlink delivery.
        crate::audio::fade_out_tail(&mut pcm);
        let frames = pcm.len().div_ceil(DOWNLINK_FRAME_SAMPLES);
        pcm.resize(frames * DOWNLINK_FRAME_SAMPLES, 0);
        Ok(pcm)
    }

    pub fn encode(&self, code: &str) -> Result<Vec<Vec<u8>>, PromptError> {
        let pcm = self.assemble(code)?;
        let mut encoder =
            DownlinkOpusEncoder::new(MAX_DOWNLINK_OPUS_PACKET_BYTES).map_err(|_| PromptError)?;
        pcm.chunks_exact(DOWNLINK_FRAME_SAMPLES)
            .map(|samples| {
                let frame = DownlinkPcmFrame::try_new(Pcm16Mono::new(samples.to_vec()))
                    .map_err(|_| PromptError)?;
                encoder
                    .encode(frame)
                    .map(|packet| packet.as_bytes().to_vec())
                    .map_err(|_| PromptError)
            })
            .collect()
    }
}

fn load_clip(path: &Path, max_samples: usize) -> Result<Vec<i16>, PromptError> {
    let file = std::fs::File::open(path).map_err(|_| PromptError)?;
    let metadata = file.metadata().map_err(|_| PromptError)?;
    if !metadata.is_file() || metadata.len() > 2 * 1024 * 1024 {
        return Err(PromptError);
    }
    let mut reader =
        hound::WavReader::new(std::io::BufReader::new(file)).map_err(|_| PromptError)?;
    let spec = reader.spec();
    if spec.sample_rate != RATE as u32
        || spec.channels != 1
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
        || reader.duration() == 0
        || reader.duration() as usize > max_samples
    {
        return Err(PromptError);
    }
    let samples = reader
        .samples::<i16>()
        .take(max_samples + 1)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| PromptError)?;
    if samples.is_empty()
        || samples.len() > max_samples
        || samples.iter().all(|sample| *sample == 0)
    {
        return Err(PromptError);
    }
    Ok(samples)
}

fn append_clip(pcm: &mut Vec<i16>, clip: &[i16]) {
    let start = pcm.len();
    pcm.extend_from_slice(clip);
    let edge = EDGE_SAMPLES.min(clip.len() / 2);
    for offset in 0..edge {
        let gain = offset as f32 / edge as f32;
        pcm[start + offset] = (f32::from(pcm[start + offset]) * gain).round() as i16;
        let end = pcm.len() - offset - 1;
        pcm[end] = (f32::from(pcm[end]) * gain).round() as i16;
    }
}

#[cfg(test)]
#[path = "audio_tests.rs"]
mod tests;
