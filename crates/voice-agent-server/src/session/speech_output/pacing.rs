use super::*;

impl SpeechOutput {
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

    pub(super) fn packet_send_deadline(&self) -> Option<Instant> {
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
