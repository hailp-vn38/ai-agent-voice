use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    audio::{DOWNLINK_FRAME_SAMPLES, DownlinkOpusEncoder, DownlinkPcmFrame, Pcm16Mono, PcmF32Mono},
    providers::TtsProvider,
};

const PROVIDER_SAMPLE_RATE_HZ: u32 = 48_000;
const PACED_FRAME_DURATION: Duration = Duration::from_millis(60);

/// The actor observes these events; this module never owns a WebSocket sender.
pub enum SpeechOutputEvent {
    Started,
    AudioPacket(Vec<u8>),
    Drained,
}

/// Owns synthesis, canonical conversion, Opus encoding, and audio pacing for one turn.
pub struct SpeechOutput {
    tts: Arc<dyn TtsProvider>,
    encoder: DownlinkOpusEncoder,
    packets: Vec<Vec<u8>>,
    next_packet: usize,
    finish_input: bool,
    started: bool,
    next_deadline: Option<Instant>,
}

impl SpeechOutput {
    pub fn new(tts: Arc<dyn TtsProvider>) -> Result<Self, crate::audio::AudioError> {
        Ok(Self {
            tts,
            encoder: DownlinkOpusEncoder::new(4_000)?,
            packets: Vec::new(),
            next_packet: 0,
            finish_input: false,
            started: false,
            next_deadline: None,
        })
    }

    pub fn submit(&mut self, text: &str) -> Result<(), ()> {
        let pcm = self.tts.synthesize(text).map_err(|_| ())?;
        let downlink = resample_to_downlink(pcm).ok_or(())?;
        for samples in downlink.chunks(DOWNLINK_FRAME_SAMPLES) {
            let mut frame = samples.to_vec();
            frame.resize(DOWNLINK_FRAME_SAMPLES, 0);
            let packet = self
                .encoder
                .encode(DownlinkPcmFrame::try_new(Pcm16Mono::new(frame)).map_err(|_| ())?)
                .map_err(|_| ())?;
            self.packets.push(packet.as_bytes().to_vec());
        }
        if self.packets.is_empty() {
            return Err(());
        }
        Ok(())
    }

    pub fn finish_input(&mut self) {
        self.finish_input = true;
    }

    pub fn cancel(&mut self) {
        self.packets.clear();
        self.next_packet = 0;
        self.finish_input = false;
        self.started = false;
        self.next_deadline = None;
    }

    pub fn poll(&mut self) -> Option<SpeechOutputEvent> {
        if self.next_packet == self.packets.len() {
            if self.finish_input
                && self.started
                && self
                    .next_deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
            {
                self.cancel();
                return Some(SpeechOutputEvent::Drained);
            }
            return None;
        }
        if self
            .next_deadline
            .is_some_and(|deadline| Instant::now() < deadline)
        {
            return None;
        }
        if !self.started {
            self.started = true;
            return Some(SpeechOutputEvent::Started);
        }
        let packet = self.packets[self.next_packet].clone();
        self.next_packet += 1;
        self.next_deadline = Some(Instant::now() + PACED_FRAME_DURATION);
        Some(SpeechOutputEvent::AudioPacket(packet))
    }
}

fn resample_to_downlink(pcm: PcmF32Mono) -> Option<Vec<i16>> {
    if pcm.sample_rate_hz() != PROVIDER_SAMPLE_RATE_HZ || pcm.samples().is_empty() {
        return None;
    }
    let samples = pcm.samples();
    let mut output = Vec::with_capacity(samples.len() / 2);
    let (pairs, _) = samples.as_chunks::<2>();
    for pair in pairs {
        let sample = (pair[0] + pair[1]) * 0.5;
        if !sample.is_finite() {
            return None;
        }
        output.push((sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16);
    }
    (!output.is_empty()).then_some(output)
}
