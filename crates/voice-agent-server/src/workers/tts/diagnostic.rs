use std::sync::Arc;

use crate::{
    audio::PcmF32Mono,
    providers::{TtsBinding, TtsDiagnosticRequest, TtsError, TtsSynthesisRequest},
    services::provider_diagnostic::{
        ProviderDiagnosticOperation, ProviderDiagnosticOperationError,
    },
};
use tokio_util::sync::CancellationToken;

use super::*;

const MAX_DIAGNOSTIC_WAV_BYTES: usize = 16 * 1024 * 1024;

impl TtsWorkerRuntime {
    /// Acquires capacity for a bounded diagnostic operation.
    pub fn admit_diagnostic(&self) -> Result<ProviderCapacityPermit, ProviderAdmissionError> {
        let state = self.state.lock().expect("TTS worker state poisoned");
        if state.closed {
            return Err(ProviderAdmissionError::Capacity);
        }
        self.admission.try_admit(ProviderWorkloadClass::Diagnostic)
    }

    /// Builds a diagnostic for an already-materialized native worker pool. Capacity admission is
    /// owned by `ProviderDiagnosticService`; this value cannot load a provider or alter config.
    pub fn diagnostic(self: &Arc<Self>, request: TtsDiagnosticRequest) -> TtsDiagnosticOperation {
        TtsDiagnosticOperation {
            runtime: Arc::clone(self),
            request,
            lease: None,
            terminal: false,
        }
    }

    pub fn validate_diagnostic(&self, request: &TtsDiagnosticRequest) -> Result<(), TtsError> {
        self.effective_diagnostic_request(request.clone())
            .map(|_| ())
    }

    pub(super) fn start_diagnostic(
        &self,
        request: TtsDiagnosticRequest,
    ) -> Result<TtsLease, TtsWorkerError> {
        let original = request.clone();
        let request = self
            .effective_diagnostic_request(request)
            .map_err(|_| TtsWorkerError::InvalidConfig)?;
        self.start_internal(
            None,
            TtsWorkRequest::Diagnostic { request, original },
            false,
        )
    }

    fn effective_diagnostic_request(
        &self,
        request: TtsDiagnosticRequest,
    ) -> Result<TtsSynthesisRequest, TtsError> {
        let voice = request
            .voice
            .clone()
            .unwrap_or_else(|| self.binding.voice.clone());
        let language = match &request.language {
            Some(lang) => lang.clone(),
            None if self.provider.adapter() == "edge_tts" && request.voice.is_some() => {
                crate::providers::tts::edge_tts::voices::voice_locale(&voice)
                    .unwrap_or(&self.binding.language)
                    .to_string()
            }
            None => self.binding.language.clone(),
        };
        let selection = TtsBinding { voice, language };
        if self.provider.adapter() == "zerotts_onnx" {
            self.provider.validate_diagnostic(&TtsDiagnosticRequest {
                text: request.text.clone(),
                voice: Some(selection.voice.clone()),
                language: Some(selection.language.clone()),
            })?;
        } else {
            self.provider.validate_diagnostic(&request)?;
        }
        Ok(TtsSynthesisRequest {
            text: request.text,
            selection,
        })
    }

    pub(super) fn quarantine_diagnostic(&self, lease: TtsLease) {
        let mut state = self.state.lock().expect("TTS worker state poisoned");
        if let Some(worker) = state.slots.get_mut(&lease).map(|slot| {
            slot.quarantined = true;
            slot.cleanup_reported = true;
            slot.worker
        }) {
            state.workers[worker].quarantined = true;
        }
    }

    /// Resolves a requested voice ID or gender alias (`"female"`, `"vi-female"`, `"male"`, ...)
    /// into a concrete voice supported by this runtime's TTS adapter/model.
    pub fn resolve_background_voice(&self, voice: Option<&str>) -> String {
        let default_binding_voice = (!self.binding.voice.is_empty()
            && self.binding.voice != "__readiness__")
            .then_some(self.binding.voice.as_str());
        crate::providers::tts::voice_catalog::resolve_tts_voice(
            self.provider.adapter(),
            self.provider.model_id(),
            default_binding_voice,
            voice,
        )
    }

    /// Synthesizes one background TTS clip into a WAV payload (`ProviderWorkloadClass::Background`),
    /// honouring an optional voice override when supported by the provider and falling back to
    /// the runtime's configured [`TtsBinding`].
    pub async fn synthesize_background_wav(
        &self,
        text: &str,
        voice: Option<&str>,
    ) -> Result<Vec<u8>, TtsWorkerError> {
        let resolved_voice = self.resolve_background_voice(voice);
        let synth_req = if !resolved_voice.is_empty() && resolved_voice != self.binding.voice {
            self.effective_diagnostic_request(TtsDiagnosticRequest {
                text: text.to_owned(),
                voice: Some(resolved_voice),
                language: None,
            })
            .ok()
        } else {
            None
        }
        .unwrap_or_else(|| TtsSynthesisRequest {
            text: text.to_owned(),
            selection: self.binding.clone(),
        });

        let workload_class = if self.admission.has_background_capacity() {
            ProviderWorkloadClass::Background
        } else {
            ProviderWorkloadClass::Voice
        };
        let permit = self
            .admission
            .try_admit(workload_class)
            .map_err(|_| TtsWorkerError::Capacity)?;
        let lease = self.start_with_permit(None, TtsWorkRequest::Voice(synth_req), Some(permit))?;
        let mut sample_rate = None;
        let mut samples = Vec::new();
        loop {
            match self.poll(lease)? {
                Some(TtsWorkerEvent::Pcm(pcm)) => {
                    if append_pcm(&mut sample_rate, &mut samples, pcm).is_err() {
                        let _ = self.cancel(lease);
                        return Err(TtsWorkerError::InvalidConfig);
                    }
                }
                Some(TtsWorkerEvent::Finished) => {
                    while self.reset_pending() {
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                    return wav(sample_rate, samples).map_err(|_| TtsWorkerError::InvalidConfig);
                }
                Some(TtsWorkerEvent::Failed)
                | Some(TtsWorkerEvent::Cancelled)
                | Some(TtsWorkerEvent::CleanupTimedOut) => {
                    return Err(TtsWorkerError::Initialization);
                }
                Some(TtsWorkerEvent::TimedOut) => {
                    let _ = self.cancel(lease);
                    return Err(TtsWorkerError::Capacity);
                }
                None => tokio::time::sleep(std::time::Duration::from_millis(1)).await,
            }
        }
    }
}
#[async_trait::async_trait]
impl ProviderDiagnosticOperation for TtsDiagnosticOperation {
    type Output = TtsDiagnosticOutput;

    async fn execute(
        &mut self,
        _: CancellationToken,
    ) -> Result<Self::Output, ProviderDiagnosticOperationError> {
        let lease = self
            .runtime
            .start_diagnostic(self.request.clone())
            .map_err(|_| ProviderDiagnosticOperationError::Unavailable)?;
        self.lease = Some(lease);
        let mut sample_rate = None;
        let mut samples = Vec::new();
        loop {
            match self
                .runtime
                .poll(lease)
                .map_err(|_| ProviderDiagnosticOperationError::Failed)?
            {
                Some(TtsWorkerEvent::Pcm(pcm)) => append_pcm(&mut sample_rate, &mut samples, pcm)?,
                Some(TtsWorkerEvent::Finished) => {
                    self.terminal = true;
                    while self.runtime.reset_pending() {
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                    return wav(sample_rate, samples).map(|wav| TtsDiagnosticOutput { wav });
                }
                Some(TtsWorkerEvent::Failed) | Some(TtsWorkerEvent::Cancelled) => {
                    self.terminal = true;
                    return Err(ProviderDiagnosticOperationError::Failed);
                }
                Some(TtsWorkerEvent::CleanupTimedOut) => {
                    return Err(ProviderDiagnosticOperationError::Unavailable);
                }
                Some(TtsWorkerEvent::TimedOut) | None => {
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await
                }
            }
        }
    }

    fn cancel_exact(&mut self) {
        if let Some(lease) = self.lease {
            let _ = self.runtime.cancel(lease);
        }
    }

    async fn await_terminal_acknowledgement(&mut self) -> bool {
        let Some(lease) = self.lease else {
            return self.terminal;
        };
        while !self.terminal {
            match self.runtime.poll(lease) {
                Ok(Some(event)) if event.is_terminal() => self.terminal = true,
                Ok(Some(TtsWorkerEvent::CleanupTimedOut)) | Err(_) => return false,
                _ => tokio::time::sleep(std::time::Duration::from_millis(1)).await,
            }
        }
        while self.runtime.reset_pending() {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        true
    }

    fn quarantine_exact(&mut self) {
        if let Some(lease) = self.lease {
            self.runtime.quarantine_diagnostic(lease);
        }
    }
}

fn append_pcm(
    sample_rate: &mut Option<u32>,
    output: &mut Vec<i16>,
    pcm: PcmF32Mono,
) -> Result<(), ProviderDiagnosticOperationError> {
    if pcm.sample_rate_hz() == 0 || sample_rate.is_some_and(|rate| rate != pcm.sample_rate_hz()) {
        return Err(ProviderDiagnosticOperationError::InvalidResponse);
    }
    *sample_rate = Some(pcm.sample_rate_hz());
    let added = pcm.samples().len();
    if output.len().saturating_add(added) > (MAX_DIAGNOSTIC_WAV_BYTES - 44) / 2
        || pcm.samples().iter().any(|sample| !sample.is_finite())
    {
        return Err(ProviderDiagnosticOperationError::InvalidResponse);
    }
    output.extend(
        pcm.samples()
            .iter()
            .map(|sample| (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16),
    );
    Ok(())
}

fn wav(
    sample_rate: Option<u32>,
    samples: Vec<i16>,
) -> Result<Vec<u8>, ProviderDiagnosticOperationError> {
    let sample_rate = sample_rate
        .filter(|_| !samples.is_empty())
        .ok_or(ProviderDiagnosticOperationError::InvalidResponse)?;
    let data_bytes = u32::try_from(
        samples
            .len()
            .checked_mul(2)
            .ok_or(ProviderDiagnosticOperationError::InvalidResponse)?,
    )
    .map_err(|_| ProviderDiagnosticOperationError::InvalidResponse)?;
    let mut result = Vec::with_capacity(44 + data_bytes as usize);
    result.extend_from_slice(b"RIFF");
    result.extend_from_slice(
        &(36_u32
            .checked_add(data_bytes)
            .ok_or(ProviderDiagnosticOperationError::InvalidResponse)?)
        .to_le_bytes(),
    );
    result.extend_from_slice(b"WAVEfmt ");
    result.extend_from_slice(&16_u32.to_le_bytes());
    result.extend_from_slice(&1_u16.to_le_bytes());
    result.extend_from_slice(&1_u16.to_le_bytes());
    result.extend_from_slice(&sample_rate.to_le_bytes());
    result.extend_from_slice(
        &(sample_rate
            .checked_mul(2)
            .ok_or(ProviderDiagnosticOperationError::InvalidResponse)?)
        .to_le_bytes(),
    );
    result.extend_from_slice(&2_u16.to_le_bytes());
    result.extend_from_slice(&16_u16.to_le_bytes());
    result.extend_from_slice(b"data");
    result.extend_from_slice(&data_bytes.to_le_bytes());
    for sample in samples {
        result.extend_from_slice(&sample.to_le_bytes());
    }
    Ok(result)
}
