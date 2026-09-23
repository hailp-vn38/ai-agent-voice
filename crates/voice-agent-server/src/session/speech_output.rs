use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::config::SpeechOutputConfig;
use unicode_normalization::UnicodeNormalization;

use crate::{
    audio::{
        DOWNLINK_FRAME_SAMPLES, DownlinkOpusEncoder, DownlinkPcmFrame, DownlinkResampler,
        Pcm16Mono, PcmF32Mono,
    },
    providers::TtsProvider,
    workers::{TtsLease, TtsStreamId, TtsWorkerEvent, TtsWorkerRuntime},
};

const PROVIDER_SAMPLE_RATE_HZ: u32 = 48_000;
const FADE_IN_SAMPLES: usize = 48_000 * 8 / 1_000;
const PACED_FRAME_DURATION: Duration = Duration::from_millis(60);
const PREBUFFER_PACKETS: usize = 5;
const MAX_BUFFERED_PACKETS: usize = 32;

/// The actor observes these events; this module never owns a WebSocket sender.
pub enum SpeechOutputEvent {
    SegmentReady { text: String },
    Started,
    AudioPacket(Vec<u8>),
    Drained,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeechOutputError {
    Backpressure,
    Synthesis,
}

/// Owns synthesis, canonical conversion, Opus encoding, and audio pacing for one turn.
pub struct SpeechOutput {
    tts: Arc<dyn TtsProvider>,
    tts_runtime: Option<Arc<TtsWorkerRuntime>>,
    tts_stream: Option<TtsStreamId>,
    active_worker: Option<TtsLease>,
    encoder: DownlinkOpusEncoder,
    pending: VecDeque<SpeechSegment>,
    downlink_resampler: DownlinkResampler,
    first_pcm_chunk: bool,
    downlink_tail: Vec<i16>,
    packets: VecDeque<Vec<u8>>,
    packets_sent: usize,
    finish_input: bool,
    started: bool,
    playback_origin: Option<Instant>,
    playback_end_deadline: Option<Instant>,
    json_filter: JsonFilter,
    segmenter: SentenceSegmenter,
    max_pending: usize,
}

struct SpeechSegment {
    display_text: String,
    tts_text: String,
    announced: bool,
}

impl SpeechOutput {
    pub fn with_config(
        tts: Arc<dyn TtsProvider>,
        config: SpeechOutputConfig,
    ) -> Result<Self, crate::audio::AudioError> {
        let max_pending = config.pending_segments;
        Ok(Self {
            tts,
            tts_runtime: None,
            tts_stream: None,
            active_worker: None,
            encoder: DownlinkOpusEncoder::new(4_000)?,
            downlink_resampler: DownlinkResampler::new_48k_to_24k(),
            first_pcm_chunk: true,
            pending: VecDeque::new(),
            downlink_tail: Vec::new(),
            packets: VecDeque::new(),
            packets_sent: 0,
            finish_input: false,
            started: false,
            playback_origin: None,
            playback_end_deadline: None,
            json_filter: JsonFilter::default(),
            segmenter: SentenceSegmenter::new(config),
            max_pending,
        })
    }

    pub fn with_worker(
        tts: Arc<dyn TtsProvider>,
        runtime: Arc<TtsWorkerRuntime>,
        config: SpeechOutputConfig,
    ) -> Result<Self, crate::audio::AudioError> {
        let mut output = Self::with_config(tts, config)?;
        output.tts_stream = Some(runtime.begin_stream());
        output.tts_runtime = Some(runtime);
        Ok(output)
    }

    /// Accepts LLM text incrementally. Every completed segment is admitted atomically.
    pub fn push_delta(&mut self, text: &str) -> Result<(), SpeechOutputError> {
        let text = self.json_filter.push(text);
        for segment in self.segmenter.push(&text) {
            self.enqueue(segment)?;
        }
        // Emergency bound for an unfinished sentence; never synthesize a partial sentence.
        if self.segmenter.buffer.chars().count() + self.json_filter.candidate.chars().count()
            > self.segmenter.config.max_chars.saturating_mul(2)
        {
            return Err(SpeechOutputError::Backpressure);
        }
        Ok(())
    }

    pub fn finish_input(&mut self) -> Result<(), SpeechOutputError> {
        let remaining = self.json_filter.finish();
        for segment in self.segmenter.push(&remaining) {
            self.enqueue(segment)?;
        }
        if let Some(segment) = self.segmenter.finish() {
            self.enqueue(segment)?;
        }
        if !self.started
            && self.pending.is_empty()
            && self.active_worker.is_none()
            && self.packets.is_empty()
        {
            return Err(SpeechOutputError::Synthesis);
        }
        self.finish_input = true;
        Ok(())
    }

    fn enqueue(&mut self, text: String) -> Result<(), SpeechOutputError> {
        let display_text = text.trim().to_owned();
        let tts_text = sanitize_tts_text(&display_text);
        if display_text.is_empty() || tts_text.is_empty() {
            return Ok(());
        }
        // This is the semantic bound for text awaiting synthesis. Audio has its own bounded
        // transport queue, so a long valid segment must not consume future segment capacity.
        if self.pending.len() >= self.max_pending {
            return Err(SpeechOutputError::Backpressure);
        }
        self.pending.push_back(SpeechSegment {
            display_text,
            tts_text,
            announced: false,
        });
        Ok(())
    }

    fn synthesize_next(&mut self) -> Result<(), SpeechOutputError> {
        let Some(segment) = self.pending.pop_front() else {
            return Ok(());
        };
        let text = segment.tts_text;
        if let Some(runtime) = &self.tts_runtime {
            let stream = self.tts_stream.expect("TTS worker requires a stream");
            tracing::info!(?stream, "TTS segment submission requested");
            let lease = runtime
                .start_in_stream(stream, text)
                .map_err(|_| SpeechOutputError::Synthesis)?;
            self.active_worker = Some(lease);
            return Ok(());
        }
        tracing::info!(
            tts_input = %text,
            chars = text.chars().count(),
            delivery = "direct",
            "TTS synthesis input"
        );
        let pcm = self
            .tts
            .synthesize(&text)
            .map_err(|_| SpeechOutputError::Synthesis)?;
        self.push_provider_pcm(pcm)
    }

    fn push_provider_pcm(&mut self, mut pcm: PcmF32Mono) -> Result<(), SpeechOutputError> {
        if pcm.sample_rate_hz() != PROVIDER_SAMPLE_RATE_HZ || pcm.samples().is_empty() {
            return Err(SpeechOutputError::Synthesis);
        }
        if self.first_pcm_chunk {
            let fade_samples = FADE_IN_SAMPLES.min(pcm.samples().len());
            for (index, sample) in pcm.samples_mut()[..fade_samples].iter_mut().enumerate() {
                *sample *= index as f32 / fade_samples as f32;
            }
            self.first_pcm_chunk = false;
        }
        let downlink = self
            .downlink_resampler
            .process(pcm.samples())
            .map_err(|_| SpeechOutputError::Synthesis)?
            .into_iter()
            .map(float_to_i16)
            .collect::<Vec<_>>();
        self.downlink_tail.extend(downlink);
        while self.downlink_tail.len() >= DOWNLINK_FRAME_SAMPLES {
            let frame = self
                .downlink_tail
                .drain(..DOWNLINK_FRAME_SAMPLES)
                .collect::<Vec<_>>();
            let packet = self
                .encoder
                .encode(
                    DownlinkPcmFrame::try_new(Pcm16Mono::new(frame))
                        .map_err(|_| SpeechOutputError::Synthesis)?,
                )
                .map_err(|_| SpeechOutputError::Synthesis)?;
            if !self.started && self.packets.is_empty() {
                tracing::info!(
                    packet_bytes = packet.as_bytes().len(),
                    "First downlink Opus packet ready"
                );
            }
            self.packets.push_back(packet.as_bytes().to_vec());
        }
        tracing::debug!(
            buffered_packets = self.packets.len(),
            buffered_ms = self.packets.len() * PACED_FRAME_DURATION.as_millis() as usize,
            "TTS Opus buffer"
        );
        Ok(())
    }

    fn flush_downlink_tail(&mut self) -> Result<(), SpeechOutputError> {
        if self.downlink_tail.is_empty() {
            return Ok(());
        }
        for sample in self.downlink_resampler.flush() {
            self.downlink_tail.push(float_to_i16(sample));
        }
        self.downlink_tail.resize(DOWNLINK_FRAME_SAMPLES, 0);
        let frame = std::mem::take(&mut self.downlink_tail);
        let packet = self
            .encoder
            .encode(
                DownlinkPcmFrame::try_new(Pcm16Mono::new(frame))
                    .map_err(|_| SpeechOutputError::Synthesis)?,
            )
            .map_err(|_| SpeechOutputError::Synthesis)?;
        self.packets.push_back(packet.as_bytes().to_vec());
        Ok(())
    }

    pub fn cancel(&mut self) {
        if let Some(lease) = self.active_worker.take()
            && let Some(runtime) = &self.tts_runtime
        {
            let _ = runtime.cancel_and_detach(lease);
        }
        self.pending.clear();
        self.downlink_tail.clear();
        self.downlink_resampler = DownlinkResampler::new_48k_to_24k();
        self.first_pcm_chunk = true;
        self.packets.clear();
        self.packets_sent = 0;
        self.finish_input = false;
        self.started = false;
        self.playback_origin = None;
        self.playback_end_deadline = None;
        self.json_filter.reset();
        self.segmenter.reset();
        if let (Some(runtime), Some(stream)) = (&self.tts_runtime, self.tts_stream.take()) {
            runtime.close_stream(stream);
            self.tts_stream = Some(runtime.begin_stream());
        }
    }

    fn packet_send_deadline(&self) -> Option<Instant> {
        if self.packets_sent < PREBUFFER_PACKETS {
            return None;
        }
        let origin = self
            .playback_origin
            .expect("sent packets require playback origin");
        let paced_index = self.packets_sent + 1 - PREBUFFER_PACKETS;
        Some(
            origin
                + PACED_FRAME_DURATION * u32::try_from(paced_index).expect("packet index fits u32"),
        )
    }

    pub fn poll(&mut self) -> Result<Option<SpeechOutputEvent>, SpeechOutputError> {
        // Only one segment is synthesized at a time; packet pacing may overlap the next poll.
        if self.packets.len() < MAX_BUFFERED_PACKETS
            && self.active_worker.is_none()
            && !self.pending.is_empty()
        {
            let segment = self.pending.front_mut().expect("checked non-empty");
            if !segment.announced {
                segment.announced = true;
                tracing::info!(text = %segment.display_text, "Speech segment ready");
                return Ok(Some(SpeechOutputEvent::SegmentReady {
                    text: segment.display_text.clone(),
                }));
            }
            self.synthesize_next()?;
        }
        // Keep native PCM production behind the audio high-water mark. The runtime's
        // event channel supplies backpressure while paced packets leave this queue.
        if let Some(lease) = self
            .active_worker
            .filter(|_| self.packets.len() < MAX_BUFFERED_PACKETS)
        {
            let event = self
                .tts_runtime
                .as_ref()
                .expect("active TTS worker requires a runtime")
                .poll(lease)
                .map_err(|_| SpeechOutputError::Synthesis)?;
            match event {
                None => {}
                Some(TtsWorkerEvent::Pcm(pcm)) => self.push_provider_pcm(pcm)?,
                Some(TtsWorkerEvent::Finished) => self.active_worker = None,
                Some(
                    TtsWorkerEvent::Cancelled
                    | TtsWorkerEvent::Failed
                    | TtsWorkerEvent::CleanupTimedOut,
                ) => {
                    self.active_worker = None;
                    return Err(SpeechOutputError::Synthesis);
                }
                Some(TtsWorkerEvent::TimedOut) => {
                    self.active_worker = None;
                    // Actor failure stops polling this lease, so cleanup must continue at the
                    // runtime boundary and quarantine the slot if native acknowledgement stalls.
                    let _ = self
                        .tts_runtime
                        .as_ref()
                        .expect("active TTS worker requires a runtime")
                        .cancel_and_detach(lease);
                    return Err(SpeechOutputError::Synthesis);
                }
            }
        }
        if self.packets.is_empty()
            && self.finish_input
            && self.pending.is_empty()
            && self.active_worker.is_none()
        {
            self.flush_downlink_tail()?;
        }
        if self.packets.is_empty() {
            if self.finish_input
                && self.pending.is_empty()
                && self.active_worker.is_none()
                && self.started
                && self
                    .playback_end_deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
            {
                self.cancel();
                return Ok(Some(SpeechOutputEvent::Drained));
            }
            return Ok(None);
        }
        if !self.started {
            // A five-packet burst alone cannot cover the native decoder's next-chunk
            // latency. Finish short initial segments before playback; a long segment
            // starts at the bounded audio high-water mark so the worker can continue.
            if self.packets.len() < MAX_BUFFERED_PACKETS
                && (self.active_worker.is_some() || !self.pending.is_empty())
            {
                return Ok(None);
            }
            tracing::info!(
                buffered_packets = self.packets.len(),
                active_worker = self.active_worker.is_some(),
                "TTS initial buffer ready"
            );
            self.started = true;
            return Ok(Some(SpeechOutputEvent::Started));
        }
        if self
            .packet_send_deadline()
            .is_some_and(|deadline| Instant::now() < deadline)
        {
            return Ok(None);
        }
        let packet = self.packets.pop_front().expect("checked non-empty");
        let now = Instant::now();
        let origin = *self.playback_origin.get_or_insert(now);
        self.packets_sent += 1;
        let packet_count = u32::try_from(self.packets_sent).expect("packet count fits u32");
        let expected_end = origin + PACED_FRAME_DURATION * packet_count;
        self.playback_end_deadline = Some(expected_end.max(now + PACED_FRAME_DURATION));
        tracing::debug!(
            packet_seq = self.packets_sent,
            opus_queue = self.packets.len(),
            active_worker = self.active_worker.is_some(),
            "Downlink Opus packet ready for WebSocket"
        );
        Ok(Some(SpeechOutputEvent::AudioPacket(packet)))
    }
}

/// Removes complete JSON objects and arrays from streamed LLM text before sentence splitting.
#[derive(Default)]
struct JsonFilter {
    candidate: String,
    depth: usize,
    in_string: bool,
    escaped: bool,
    last_emitted: Option<char>,
    skip_next_space: bool,
}

impl JsonFilter {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn push(&mut self, delta: &str) -> String {
        let mut output = String::new();
        for ch in delta.chars() {
            if self.depth == 0 {
                if matches!(ch, '{' | '[') {
                    self.depth = 1;
                    self.candidate.push(ch);
                } else {
                    if self.skip_next_space && ch == ' ' {
                        self.skip_next_space = false;
                        continue;
                    }
                    self.skip_next_space = false;
                    output.push(ch);
                    self.last_emitted = Some(ch);
                }
                continue;
            }

            self.candidate.push(ch);
            if self.escaped {
                self.escaped = false;
            } else if ch == '\\' && self.in_string {
                self.escaped = true;
            } else if ch == '"' {
                self.in_string = !self.in_string;
            } else if !self.in_string {
                match ch {
                    '{' | '[' => self.depth += 1,
                    '}' | ']' => self.depth -= 1,
                    _ => {}
                }
            }
            if self.depth == 0 {
                if serde_json::from_str::<serde_json::Value>(&self.candidate).is_ok() {
                    if self.last_emitted.is_some_and(char::is_alphanumeric) {
                        output.push(' ');
                        self.last_emitted = Some(' ');
                    }
                    self.skip_next_space = self.last_emitted == Some(' ');
                } else {
                    output.push_str(&self.candidate);
                    self.last_emitted = Some(ch);
                    self.skip_next_space = false;
                }
                self.candidate.clear();
            }
        }
        output
    }

    fn finish(&mut self) -> String {
        self.depth = 0;
        self.in_string = false;
        self.escaped = false;
        std::mem::take(&mut self.candidate)
    }
}

/// Pure V1 sentence delivery policy. It never knows about synthesis or transport.
struct SentenceSegmenter {
    config: SpeechOutputConfig,
    buffer: String,
}

impl SentenceSegmenter {
    fn new(config: SpeechOutputConfig) -> Self {
        Self {
            config,
            buffer: String::new(),
        }
    }
    fn reset(&mut self) {
        self.buffer.clear();
    }
    fn push(&mut self, delta: &str) -> Vec<String> {
        self.buffer.push_str(delta);
        let mut segments = Vec::new();
        loop {
            let boundary = self.buffer.char_indices().find_map(|(index, ch)| {
                let next = self.buffer[index + ch.len_utf8()..].chars().next();
                let previous = self.buffer[..index].chars().next_back();
                // A trailing digit-dot may become a decimal or version separator in
                // the next delta. A trailing word-dot is released immediately so a
                // complete sentence can start audio before LLM EOF.
                let sentence_dot = ch == '.'
                    && match next {
                        Some(next) => {
                            next.is_whitespace()
                                || matches!(next, '"' | '\'' | ')' | ']' | '”' | '’')
                        }
                        None => previous.is_some_and(|previous| !previous.is_ascii_digit()),
                    };
                let hard =
                    sentence_dot || matches!(ch, '!' | '?' | '\u{3002}' | '\u{ff01}' | '\u{ff1f}');
                hard.then_some(index + ch.len_utf8())
            });
            let Some(split) = boundary else { break };
            let segment = self.buffer[..split].trim().to_owned();
            self.buffer.drain(..split);
            if !segment.is_empty() {
                segments.push(segment);
            }
        }
        segments
    }
    fn finish(&mut self) -> Option<String> {
        let text = self.buffer.trim().to_owned();
        self.buffer.clear();
        (!text.is_empty()).then_some(text)
    }
}

fn sanitize_tts_text(input: &str) -> String {
    let normalized = input.nfc().collect::<String>();
    let mut output = String::with_capacity(normalized.len());
    let mut pending_space = false;
    for (index, ch) in normalized.char_indices() {
        let between_digits = normalized[..index]
            .chars()
            .next_back()
            .is_some_and(|previous| previous.is_ascii_digit())
            && normalized[index + ch.len_utf8()..]
                .chars()
                .next()
                .is_some_and(|next| next.is_ascii_digit());
        if ch.is_alphanumeric() {
            if pending_space && !output.is_empty() {
                output.push(' ');
            }
            pending_space = false;
            output.push(ch);
        } else if matches!(ch, '/' | '-') && between_digits {
            pending_space = false;
            output.push(ch);
        } else if ch.is_whitespace() || matches!(ch, '_' | '-' | '–' | '—') {
            pending_space = true;
        } else if matches!(ch, '.' | ',' | '!' | '?' | ';' | ':') {
            pending_space = false;
            output.push(ch);
        }
    }
    output.trim().to_owned()
}

fn float_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

#[cfg(test)]
mod tests {
    use super::{SentenceSegmenter, sanitize_tts_text};
    use crate::config::SpeechOutputConfig;

    #[test]
    fn short_streaming_segment_waits_for_completion_before_starting_playback() {
        use super::{SpeechOutput, SpeechOutputEvent};
        use crate::{
            audio::PcmF32Mono,
            providers::{TtsError, TtsProvider, TtsWorker},
            workers::{TtsWorkerRuntime, WorkerRuntimeConfig},
        };
        use std::{
            sync::{Arc, Mutex, atomic::AtomicBool, mpsc},
            time::Duration,
        };

        struct GatedProvider {
            emitted: mpsc::Sender<usize>,
            release: Arc<Mutex<mpsc::Receiver<()>>>,
        }
        struct GatedWorker {
            emitted: mpsc::Sender<usize>,
            release: Arc<Mutex<mpsc::Receiver<()>>>,
        }
        impl TtsProvider for GatedProvider {
            fn adapter(&self) -> &'static str {
                "gated"
            }
            fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
                Ok(Box::new(GatedWorker {
                    emitted: self.emitted.clone(),
                    release: Arc::clone(&self.release),
                }))
            }
        }
        impl TtsWorker for GatedWorker {
            fn synthesize(
                &mut self,
                _: &str,
                _: &AtomicBool,
                on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
            ) -> Result<(), TtsError> {
                for frame in 1..=5 {
                    on_pcm(PcmF32Mono::new(vec![0.1; 2_880], 48_000))?;
                    self.emitted.send(frame).unwrap();
                    self.release.lock().unwrap().recv().unwrap();
                }
                Ok(())
            }
            fn reset(&mut self) -> Result<(), TtsError> {
                Ok(())
            }
        }

        let (emitted_tx, emitted_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let provider: Arc<dyn TtsProvider> = Arc::new(GatedProvider {
            emitted: emitted_tx,
            release: Arc::new(Mutex::new(release_rx)),
        });
        let runtime = Arc::new(TtsWorkerRuntime::new(
            Arc::clone(&provider),
            WorkerRuntimeConfig {
                max_workers: 1,
                command_capacity: 4,
                final_timeout: Duration::from_secs(2),
                cleanup_grace: Duration::from_secs(1),
            },
        ));
        let mut output =
            SpeechOutput::with_worker(provider, runtime, SpeechOutputConfig::default()).unwrap();
        output
            .push_delta("Một câu có âm thanh phát từng phần.")
            .unwrap();
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::SegmentReady { .. })
        ));
        assert!(output.poll().unwrap().is_none());
        for expected in 1..=5 {
            if expected > 1 {
                release_tx.send(()).unwrap();
            }
            assert_eq!(
                emitted_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
                expected
            );
            for _ in 0..10_000 {
                let event = output.poll().unwrap();
                if output.packets.len() == expected {
                    assert!(event.is_none(), "Started while worker still has more PCM");
                    break;
                }
                assert!(
                    event.is_none(),
                    "Started before packet {expected} reached the queue"
                );
                std::thread::yield_now();
            }
            assert_eq!(output.packets.len(), expected);
            assert!(
                output.poll().unwrap().is_none(),
                "Started with worker active at packet {expected}"
            );
        }
        release_tx.send(()).unwrap();
        let mut started = false;
        for _ in 0..10_000 {
            if matches!(output.poll().unwrap(), Some(SpeechOutputEvent::Started)) {
                started = true;
                break;
            }
            std::thread::yield_now();
        }
        assert!(
            started,
            "short segment did not start after worker completion"
        );
        for _ in 0..5 {
            assert!(matches!(
                output.poll().unwrap(),
                Some(SpeechOutputEvent::AudioPacket(_))
            ));
        }
    }

    #[test]
    fn long_streaming_segment_starts_at_bounded_initial_buffer() {
        use super::{MAX_BUFFERED_PACKETS, SpeechOutput, SpeechOutputEvent};
        use crate::{
            audio::PcmF32Mono,
            providers::{TtsError, TtsProvider, TtsWorker},
            workers::{TtsWorkerRuntime, WorkerRuntimeConfig},
        };
        use std::{
            sync::{Arc, Mutex, atomic::AtomicBool, mpsc},
            time::Duration,
        };

        struct LargeChunkProvider(Arc<Mutex<mpsc::Receiver<()>>>);
        struct LargeChunkWorker(Arc<Mutex<mpsc::Receiver<()>>>);
        impl TtsProvider for LargeChunkProvider {
            fn adapter(&self) -> &'static str {
                "large-chunk"
            }
            fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
                Ok(Box::new(LargeChunkWorker(Arc::clone(&self.0))))
            }
        }
        impl TtsWorker for LargeChunkWorker {
            fn synthesize(
                &mut self,
                _: &str,
                _: &AtomicBool,
                on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
            ) -> Result<(), TtsError> {
                on_pcm(PcmF32Mono::new(
                    vec![0.1; 2_880 * MAX_BUFFERED_PACKETS],
                    48_000,
                ))?;
                self.0.lock().unwrap().recv().unwrap();
                Ok(())
            }
            fn reset(&mut self) -> Result<(), TtsError> {
                Ok(())
            }
        }

        let (release_tx, release_rx) = mpsc::channel();
        let provider: Arc<dyn TtsProvider> =
            Arc::new(LargeChunkProvider(Arc::new(Mutex::new(release_rx))));
        let runtime = Arc::new(TtsWorkerRuntime::new(
            Arc::clone(&provider),
            WorkerRuntimeConfig {
                max_workers: 1,
                command_capacity: 4,
                final_timeout: Duration::from_secs(2),
                cleanup_grace: Duration::from_secs(1),
            },
        ));
        let mut output =
            SpeechOutput::with_worker(provider, runtime, SpeechOutputConfig::default()).unwrap();
        output
            .push_delta("Một câu đủ dài để kiểm tra bộ đệm đầu lượt.")
            .unwrap();
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::SegmentReady { .. })
        ));
        let mut started = false;
        for _ in 0..10_000 {
            if matches!(output.poll().unwrap(), Some(SpeechOutputEvent::Started)) {
                started = true;
                break;
            }
            std::thread::yield_now();
        }
        assert!(
            started,
            "bounded long segment must start without waiting for worker completion"
        );
        assert!(output.active_worker.is_some());
        assert!(output.packets.len() >= MAX_BUFFERED_PACKETS);
        release_tx.send(()).unwrap();
    }

    #[test]
    fn first_five_packets_are_sent_before_realtime_pacing() {
        use super::{SpeechOutput, SpeechOutputEvent};
        use crate::{
            audio::PcmF32Mono,
            providers::{TtsError, TtsProvider},
        };
        use std::sync::Arc;

        struct TenFrames;
        impl TtsProvider for TenFrames {
            fn adapter(&self) -> &'static str {
                "ten-frames"
            }
            fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
                Ok(PcmF32Mono::new(vec![0.1; 2_880 * 10], 48_000))
            }
        }
        let mut output =
            SpeechOutput::with_config(Arc::new(TenFrames), SpeechOutputConfig::default()).unwrap();
        output.push_delta("Một câu đủ dài.").unwrap();
        output.finish_input().unwrap();
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::SegmentReady { .. })
        ));
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::Started)
        ));
        for packet in 1..=5 {
            assert!(
                matches!(
                    output.poll().unwrap(),
                    Some(SpeechOutputEvent::AudioPacket(_))
                ),
                "packet {packet} should prebuffer immediately"
            );
        }
        assert!(
            output.poll().unwrap().is_none(),
            "sixth packet must wait for pacing"
        );
        let origin = std::time::Instant::now() - std::time::Duration::from_millis(61);
        output.playback_origin = Some(origin);
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::AudioPacket(_))
        ));
        assert_eq!(
            output.packet_send_deadline(),
            Some(origin + std::time::Duration::from_millis(120)),
            "late polls must retain the absolute cadence"
        );
    }

    #[test]
    fn paced_deadlines_are_anchored_to_first_packet_even_if_prebuffer_is_delayed() {
        use super::SpeechOutput;
        use crate::providers::tts::UnavailableTts;
        use std::{
            sync::Arc,
            time::{Duration, Instant},
        };

        let mut output =
            SpeechOutput::with_config(Arc::new(UnavailableTts), SpeechOutputConfig::default())
                .unwrap();
        let first_packet_at = Instant::now();
        output.playback_origin = Some(first_packet_at);
        output.packets_sent = 5;
        assert_eq!(
            output.packet_send_deadline(),
            Some(first_packet_at + Duration::from_millis(60))
        );
        output.packets_sent = 6;
        assert_eq!(
            output.packet_send_deadline(),
            Some(first_packet_at + Duration::from_millis(120))
        );
    }

    #[test]
    fn next_segment_starts_while_previous_audio_is_queued() {
        use super::{SpeechOutput, SpeechOutputEvent};
        use crate::{
            audio::PcmF32Mono,
            providers::{TtsError, TtsProvider},
        };
        use std::sync::{Arc, Mutex};

        struct RecordingTts(Arc<Mutex<Vec<String>>>);
        impl TtsProvider for RecordingTts {
            fn adapter(&self) -> &'static str {
                "recording"
            }
            fn synthesize(&self, text: &str) -> Result<PcmF32Mono, TtsError> {
                self.0.lock().unwrap().push(text.into());
                Ok(PcmF32Mono::new(vec![0.1; 2_880 * 10], 48_000))
            }
        }
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut output = SpeechOutput::with_config(
            Arc::new(RecordingTts(Arc::clone(&calls))),
            SpeechOutputConfig::default(),
        )
        .unwrap();
        output.push_delta("Câu thứ nhất. Câu thứ hai.").unwrap();
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::SegmentReady { .. })
        ));
        assert!(output.poll().unwrap().is_none());
        assert!(!output.packets.is_empty());
        assert!(
            matches!(
                output.poll().unwrap(),
                Some(SpeechOutputEvent::SegmentReady { .. })
            ),
            "next segment must be announced before first audio queue drains"
        );
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::Started)
        ));
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::AudioPacket(_))
        ));
        assert_eq!(
            calls.lock().unwrap().len(),
            2,
            "second synthesis must overlap first playback"
        );
    }

    #[test]
    fn prebuffered_audio_does_not_drain_before_nominal_playback_ends() {
        use super::{SpeechOutput, SpeechOutputEvent};
        use crate::{
            audio::PcmF32Mono,
            providers::{TtsError, TtsProvider},
        };
        use std::{
            sync::Arc,
            time::{Duration, Instant},
        };

        struct ShortTts;
        impl TtsProvider for ShortTts {
            fn adapter(&self) -> &'static str {
                "short"
            }
            fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
                Ok(PcmF32Mono::new(vec![0.1; 2_880 * 2], 48_000))
            }
        }
        let mut output =
            SpeechOutput::with_config(Arc::new(ShortTts), SpeechOutputConfig::default()).unwrap();
        output.push_delta("Một câu ngắn.").unwrap();
        output.finish_input().unwrap();
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::SegmentReady { .. })
        ));
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::Started)
        ));
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::AudioPacket(_))
        ));
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::AudioPacket(_))
        ));
        assert!(output.playback_end_deadline.unwrap() > Instant::now() + Duration::from_millis(60));
        assert!(
            output.poll().unwrap().is_none(),
            "tts:stop must wait for buffered playback"
        );
        output.playback_end_deadline = Some(Instant::now() - Duration::from_millis(1));
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::Drained)
        ));
    }

    #[test]
    fn splits_unicode_only_at_complete_sentence_boundary() {
        let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig {
            min_chars: 4,
            soft_break_min_chars: 8,
            max_chars: 10,
            pending_segments: 2,
        });
        assert_eq!(segmenter.push("Xin chao。Tiep"), ["Xin chao。"]);
        assert!(segmenter.push(" tuc rat dai").is_empty());
        assert_eq!(segmenter.finish(), Some("Tiep tuc rat dai".into()));
    }

    #[test]
    fn holds_fragmented_vietnamese_deltas_until_the_complete_sentence_boundary() {
        let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig::default());

        for delta in ["Xin", " ch", "ào"] {
            assert!(
                segmenter.push(delta).is_empty(),
                "fragment {delta:?} must not be submitted to TTS"
            );
        }
        assert_eq!(segmenter.push("!"), ["Xin chào!"]);
        for delta in [
            " M", "ình", " có", " thể", " giúp", " gì", " cho", " bạn", " hôm", " nay",
        ] {
            assert!(segmenter.push(delta).is_empty());
        }

        assert_eq!(
            segmenter.push("?"),
            ["Mình có thể giúp gì cho bạn hôm nay?"]
        );
        assert_eq!(segmenter.finish(), None);
    }

    #[test]
    fn does_not_send_a_partial_version_to_tts_when_decimal_dots_span_deltas() {
        let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig::default());
        assert!(segmenter.push("Phiên bản phần mềm hiện là v1.").is_empty());
        assert_eq!(
            segmenter.push("2.3 và đã sẵn sàng."),
            ["Phiên bản phần mềm hiện là v1.2.3 và đã sẵn sàng."]
        );
        assert_eq!(segmenter.finish(), None);
    }

    #[test]
    fn emits_a_complete_sentence_before_the_llm_finishes() {
        let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig::default());
        assert_eq!(
            segmenter.push("Phản hồi âm thanh hoàn chỉnh cho Reference Client."),
            ["Phản hồi âm thanh hoàn chỉnh cho Reference Client."]
        );
        assert_eq!(segmenter.finish(), None);
    }

    #[test]
    fn holds_a_long_sentence_until_eof() {
        let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig {
            min_chars: 4,
            soft_break_min_chars: 8,
            max_chars: 10,
            pending_segments: 2,
        });
        assert!(segmenter.push("abcdefghij").is_empty());
        assert!(segmenter.push("klm next").is_empty());
        assert_eq!(segmenter.finish(), Some("abcdefghijklm next".into()));
    }

    #[test]
    fn rejects_a_token_too_long_to_split_safely() {
        use super::{SpeechOutput, SpeechOutputError};
        use crate::providers::tts::UnavailableTts;
        use std::sync::Arc;

        let mut output = SpeechOutput::with_config(
            Arc::new(UnavailableTts),
            SpeechOutputConfig {
                min_chars: 4,
                soft_break_min_chars: 8,
                max_chars: 10,
                pending_segments: 2,
            },
        )
        .unwrap();
        assert_eq!(
            output.push_delta("abcdefghijklmnopqrstu"),
            Err(SpeechOutputError::Backpressure)
        );
    }

    #[test]
    fn short_hard_sentence_is_flushed_immediately() {
        let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig::default());
        for delta in ["Xin", " ch", "ào"] {
            assert!(segmenter.push(delta).is_empty());
        }
        assert_eq!(segmenter.push("!"), ["Xin chào!"]);
        assert!(
            segmenter
                .push(" Mình có thể giúp gì cho bạn hôm nay")
                .is_empty()
        );
        assert_eq!(
            segmenter.push("?"),
            ["Mình có thể giúp gì cho bạn hôm nay?"]
        );
    }

    #[test]
    fn soft_punctuation_and_length_do_not_split_unfinished_sentence() {
        let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig {
            min_chars: 4,
            soft_break_min_chars: 8,
            max_chars: 10,
            pending_segments: 2,
        });
        assert!(
            segmenter
                .push("Nếu bạn muốn, mình có thể kiểm tra")
                .is_empty()
        );
        assert_eq!(
            segmenter.push(" ngay bây giờ."),
            ["Nếu bạn muốn, mình có thể kiểm tra ngay bây giờ."]
        );
    }

    #[test]
    fn sanitizes_markdown_and_emoji_for_tts() {
        assert_eq!(sanitize_tts_text("**Xin chào!** 😊"), "Xin chào!");
        assert_eq!(
            sanitize_tts_text("### Kết quả:\n- Bạn có thể **khởi động lại** server 🚀"),
            "Kết quả: Bạn có thể khởi động lại server"
        );
        assert_eq!(
            sanitize_tts_text("Mình có thể giúp bạn #hôm_nay?"),
            "Mình có thể giúp bạn hôm nay?"
        );
    }

    #[test]
    fn keeps_date_and_time_separators_for_zerotts_normalization() {
        let text = sanitize_tts_text("Hôm nay là Tuesday, 23/09/2026 12:54:56 UTC.");
        assert_eq!(text, "Hôm nay là Tuesday, 23/09/2026 12:54:56 UTC.");
    }

    #[test]
    fn removes_json_command_across_deltas_before_sentence_delivery() {
        let mut filter = super::JsonFilter::default();
        let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig::default());
        let mut segments = Vec::new();
        for delta in [
            "{\"cmd\": \"date '+%A, %d/%m/",
            "%Y %H:%M:%S %Z'\"}Hôm nay là Tuesday, ",
            "23/09/2026 12:54:56 UTC.",
        ] {
            segments.extend(segmenter.push(&filter.push(delta)));
        }
        segments.extend(segmenter.push(&filter.finish()));
        segments.extend(segmenter.finish());
        assert_eq!(segments, ["Hôm nay là Tuesday, 23/09/2026 12:54:56 UTC."]);
    }

    #[test]
    fn json_filter_handles_escaped_braces_and_keeps_non_json_braces() {
        let mut filter = super::JsonFilter::default();
        assert_eq!(
            filter.push("Xin {\"note\": \"brace } and \\\"quote\\\"\"}chào {bạn}."),
            "Xin chào {bạn}."
        );
        assert_eq!(filter.push("Mời [1, {\"cmd\": \"skip\"}] bạn."), "Mời bạn.");
        assert_eq!(filter.finish(), "");
    }

    #[test]
    fn segment_ready_keeps_display_text_while_tts_receives_sanitized_text() {
        use super::{SpeechOutput, SpeechOutputEvent};
        use crate::{
            audio::PcmF32Mono,
            providers::{TtsError, TtsProvider},
        };
        use std::sync::{Arc, Mutex};

        struct RecordingTts(Arc<Mutex<Vec<String>>>);
        impl TtsProvider for RecordingTts {
            fn adapter(&self) -> &'static str {
                "recording_tts"
            }
            fn synthesize(&self, text: &str) -> Result<PcmF32Mono, TtsError> {
                self.0.lock().unwrap().push(text.to_owned());
                Ok(PcmF32Mono::new(vec![0.1; 2_880], 48_000))
            }
        }

        let inputs = Arc::new(Mutex::new(Vec::new()));
        let mut output = SpeechOutput::with_config(
            Arc::new(RecordingTts(Arc::clone(&inputs))),
            SpeechOutputConfig::default(),
        )
        .unwrap();
        output.push_delta("**Xin chào!** 😊").unwrap();
        output.finish_input().unwrap();
        assert!(
            matches!(output.poll().unwrap(), Some(SpeechOutputEvent::SegmentReady { text }) if text == "**Xin chào!")
        );
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::Started)
        ));
        assert_eq!(*inputs.lock().unwrap(), ["Xin chào!"]);

        output.cancel();
        output.push_delta("{\"cmd\": \"date '+%A, %d/%m/").unwrap();
        output
            .push_delta("%Y %H:%M:%S %Z'\"}Hôm nay là Tuesday, 23/09/2026 12:54:56 UTC.")
            .unwrap();
        output.finish_input().unwrap();
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::SegmentReady { text })
                if text == "Hôm nay là Tuesday, 23/09/2026 12:54:56 UTC."
        ));
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::Started)
        ));
        assert_eq!(
            *inputs.lock().unwrap(),
            ["Xin chào!", "Hôm nay là Tuesday, 23/09/2026 12:54:56 UTC."]
        );
    }

    #[test]
    fn unspeakable_response_fails_instead_of_waiting_forever() {
        use super::{SpeechOutput, SpeechOutputError};
        use crate::providers::tts::UnavailableTts;
        use std::sync::Arc;

        let mut output =
            SpeechOutput::with_config(Arc::new(UnavailableTts), SpeechOutputConfig::default())
                .unwrap();
        output.push_delta("😊🚀").unwrap();
        assert_eq!(output.finish_input(), Err(SpeechOutputError::Synthesis));
    }
}
