//! Canonical audio domain types and capture state.

use thiserror::Error;

mod opus;
mod resampler;
mod vad_segmenter;

pub use opus::{
    AudioFrameDropReason, DOWNLINK_ENCODE_BUFFER_BYTES, DecodeOutcome, DownlinkOpusEncoder,
    MAX_UPLINK_OPUS_PACKET_BYTES, OpusPacket, UplinkOpusDecoder,
};
pub use resampler::DownlinkResampler;
pub use vad_segmenter::{VadBoundary, VadSegmenter, VadSegmenterConfig, VadSegmenterError};

pub const UPLINK_FRAME_SAMPLES: usize = 960;
pub const DOWNLINK_FRAME_SAMPLES: usize = 1_440;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcm16Mono(Vec<i16>);

impl Pcm16Mono {
    pub fn new(samples: Vec<i16>) -> Self {
        Self(samples)
    }

    pub fn as_slice(&self) -> &[i16] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn into_samples(self) -> Vec<i16> {
        self.0
    }
}

/// Float PCM passed to inference providers with an explicit canonical rate.
#[derive(Clone, Debug, PartialEq)]
pub struct PcmF32Mono {
    samples: Vec<f32>,
    sample_rate_hz: u32,
}

impl PcmF32Mono {
    pub fn new(samples: Vec<f32>, sample_rate_hz: u32) -> Self {
        Self {
            samples,
            sample_rate_hz,
        }
    }
    pub fn from_uplink(frame: &UplinkPcmFrame) -> Self {
        Self {
            samples: frame
                .samples()
                .iter()
                .map(|sample| f32::from(*sample) / f32::from(i16::MAX))
                .collect(),
            sample_rate_hz: 16_000,
        }
    }

    pub fn from_samples(samples: Vec<f32>) -> Self {
        Self::new(samples, 16_000)
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UplinkPcmFrame(Pcm16Mono);

impl UplinkPcmFrame {
    pub fn try_new(pcm: Pcm16Mono) -> Result<Self, AudioError> {
        if pcm.len() != UPLINK_FRAME_SAMPLES {
            return Err(AudioError::InvalidUplinkFrameSamples(pcm.len()));
        }
        Ok(Self(pcm))
    }

    fn into_pcm(self) -> Pcm16Mono {
        self.0
    }

    pub fn samples(&self) -> &[i16] {
        self.0.as_slice()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownlinkPcmFrame(Pcm16Mono);

impl DownlinkPcmFrame {
    pub fn try_new(pcm: Pcm16Mono) -> Result<Self, AudioError> {
        if pcm.len() != DOWNLINK_FRAME_SAMPLES {
            return Err(AudioError::InvalidDownlinkFrameSamples(pcm.len()));
        }
        Ok(Self(pcm))
    }

    pub fn samples(&self) -> &[i16] {
        self.0.as_slice()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UplinkAudioUtterance(Pcm16Mono);

impl UplinkAudioUtterance {
    pub fn samples(&self) -> &[i16] {
        self.0.as_slice()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureOutcome {
    Utterance(UplinkAudioUtterance),
    Empty,
    Overflowed,
}

#[derive(Debug)]
pub struct ManualCapture {
    samples: Vec<i16>,
    max_frames: usize,
    received_frames: usize,
    active: bool,
    overflowed: bool,
}

impl ManualCapture {
    pub fn new(max_frames: usize) -> Result<Self, AudioError> {
        let max_samples = max_frames
            .checked_mul(UPLINK_FRAME_SAMPLES)
            .ok_or(AudioError::CaptureCapacityOverflow)?;
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(max_samples)
            .map_err(|_| AudioError::CaptureAllocation)?;
        Ok(Self {
            samples,
            max_frames,
            received_frames: 0,
            active: false,
            overflowed: false,
        })
    }

    pub fn start(&mut self) {
        self.reset();
        self.active = true;
    }

    pub fn restart(&mut self) {
        self.start();
    }

    /// Returns false after the capture has overflowed, so callers can stop downstream work.
    pub fn push(&mut self, frame: UplinkPcmFrame) -> bool {
        if !self.active || self.overflowed {
            return false;
        }
        if self.received_frames == self.max_frames {
            self.samples.clear();
            self.overflowed = true;
            return false;
        }
        self.samples.extend(frame.into_pcm().into_samples());
        self.received_frames += 1;
        true
    }

    pub fn stop(&mut self) -> CaptureOutcome {
        if !self.active {
            return CaptureOutcome::Empty;
        }
        self.active = false;
        if self.overflowed {
            self.reset();
            return CaptureOutcome::Overflowed;
        }
        if self.samples.is_empty() {
            return CaptureOutcome::Empty;
        }
        self.received_frames = 0;
        CaptureOutcome::Utterance(UplinkAudioUtterance(Pcm16Mono::new(std::mem::take(
            &mut self.samples,
        ))))
    }

    /// Returns storage from an utterance that has no downstream consumer yet.
    pub fn recycle(&mut self, utterance: UplinkAudioUtterance) {
        if self.samples.is_empty() {
            self.samples = utterance.0.into_samples();
            self.samples.clear();
        }
    }

    pub fn abort(&mut self) {
        self.reset();
    }

    fn reset(&mut self) {
        self.samples.clear();
        self.received_frames = 0;
        self.overflowed = false;
        self.active = false;
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AudioError {
    #[error("uplink PCM frame must contain {UPLINK_FRAME_SAMPLES} samples, got {0}")]
    InvalidUplinkFrameSamples(usize),
    #[error("downlink PCM frame must contain {DOWNLINK_FRAME_SAMPLES} samples, got {0}")]
    InvalidDownlinkFrameSamples(usize),
    #[error("manual capture capacity overflow")]
    CaptureCapacityOverflow,
    #[error("manual capture allocation failed")]
    CaptureAllocation,
    #[error("cannot initialize uplink Opus decoder")]
    DecoderInit,
    #[error("cannot initialize downlink Opus encoder")]
    EncoderInit,
    #[error("cannot encode Opus packet")]
    EncodeFailed,
    #[error("Opus encoder produced an empty packet")]
    EmptyEncodedPacket,
    #[error("Opus packet is {actual} bytes, above WebSocket cap {max}")]
    EncodedPacketTooLarge { actual: usize, max: usize },
    #[error("downlink PCM must be finite")]
    InvalidDownlinkPcm,
}
