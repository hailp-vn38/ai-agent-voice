use super::*;

impl SpeechOutput {
    pub(super) fn enqueue(&mut self, text: String) -> Result<(), SpeechOutputError> {
        let display_text = text.trim().to_owned();
        let tts_text = sanitize_tts_text(&display_text);
        if display_text.is_empty() || tts_text.is_empty() {
            return Ok(());
        }
        // This is the semantic bound for text awaiting synthesis. Audio has its own bounded
        // transport queue, so a long valid segment must not consume future segment capacity.
        if self.pending.len() >= self.max_pending {
            tracing::warn!(
                pending_segments = self.pending.len(),
                max_pending = self.max_pending,
                "Speech segment queue reached capacity"
            );
            return Err(SpeechOutputError::Backpressure);
        }
        self.pending.push_back(SpeechSegment {
            display_text,
            tts_text,
            announced: false,
        });
        Ok(())
    }

    pub(super) fn synthesize_next(&mut self) -> Result<(), SpeechOutputError> {
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

    pub(super) fn push_provider_pcm(
        &mut self,
        mut pcm: PcmF32Mono,
    ) -> Result<(), SpeechOutputError> {
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

    pub(super) fn flush_downlink_tail(&mut self) -> Result<(), SpeechOutputError> {
        if self.downlink_tail.is_empty() {
            return Ok(());
        }
        for sample in self.downlink_resampler.flush() {
            self.downlink_tail.push(float_to_i16(sample));
        }
        fade_out_tail(&mut self.downlink_tail);
        if self.downlink_tail.is_empty() {
            return Ok(());
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
}

pub(super) fn fade_out_tail(samples: &mut [i16]) {
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
