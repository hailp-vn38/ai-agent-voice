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

    pub(super) fn push_provider_pcm(&mut self, pcm: PcmF32Mono) -> Result<(), SpeechOutputError> {
        for packet in self
            .downlink_pipeline
            .push_provider_pcm(pcm)
            .map_err(|_| SpeechOutputError::Synthesis)?
        {
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
        for packet in self
            .downlink_pipeline
            .finish()
            .map_err(|_| SpeechOutputError::Synthesis)?
        {
            self.packets.push_back(packet.as_bytes().to_vec());
        }
        Ok(())
    }
}
