use std::time::Duration;

use super::*;
use crate::{
    audio::PcmF32Mono,
    providers::{
        ProviderType, TtsDiagnosticRequest, TtsDiagnosticValidationError, llm::LlmRequest,
    },
};

impl ProviderDiagnosticService {
    /// Executes a tool-free LLM diagnostic against the exact runtime loaded at process startup.
    /// This is the service boundary for desired-row lookup, type validation, runtime lookup,
    /// admission and timeout ownership; HTTP only supplies already-validated text and maps errors.
    pub async fn execute_llm(
        &self,
        key: &str,
        input: String,
    ) -> Result<LlmDiagnosticResult, ProviderDiagnosticRequestError> {
        if self.manager.is_some() {
            let service = self.managed_request(key, "llm").await?;
            let mut result = Box::pin(service.execute_llm(key, input)).await?;
            result.provider_key = key.to_owned();
            return Ok(result);
        }
        let snapshot = self.request_snapshot(key, "llm").await?;
        let provider_key = snapshot.key.clone();
        let revision = snapshot.revision;
        let operation = self
            .registry
            .llm_diagnostic(&provider_key, LlmRequest::text_turn(input), 32 * 1024)
            .map_err(|_| ProviderDiagnosticError::RuntimeNotLoaded)?;
        let media_slot = operation.media_slot();
        let (text, runtime) = self
            .execute(
                ProviderDiagnosticTarget {
                    key: provider_key.clone(),
                    provider_type: ProviderType::Llm,
                    desired_revision: revision,
                },
                operation,
                Duration::from_millis(100),
            )
            .await?;
        let media = media_slot.lock().ok().and_then(|mut guard| guard.take());
        Ok(LlmDiagnosticResult {
            provider_key,
            text,
            media,
            runtime,
        })
    }

    pub async fn execute_tts(
        &self,
        key: &str,
        request: TtsDiagnosticRequest,
    ) -> Result<TtsDiagnosticResult, ProviderDiagnosticRequestError> {
        if self.manager.is_some() {
            let service = self.managed_request(key, "tts").await?;
            let mut result = Box::pin(service.execute_tts(key, request)).await?;
            result.provider_key = key.to_owned();
            return Ok(result);
        }
        let snapshot = self.request_snapshot(key, "tts").await?;
        let provider_key = snapshot.key.clone();
        let revision = snapshot.revision;
        self.registry
            .validate_tts_diagnostic(&provider_key, &request)
            .map_err(|error| match error {
                TtsDiagnosticValidationError::NotLoaded => {
                    ProviderDiagnosticRequestError::Diagnostic(
                        ProviderDiagnosticError::RuntimeNotLoaded,
                    )
                }
                TtsDiagnosticValidationError::InvalidInput => {
                    ProviderDiagnosticRequestError::InvalidInput
                }
            })?;
        let operation = self
            .registry
            .tts_diagnostic(&provider_key, request)
            .map_err(|_| ProviderDiagnosticError::RuntimeNotLoaded)?;
        let (output, runtime) = self
            .execute(
                ProviderDiagnosticTarget {
                    key: provider_key.clone(),
                    provider_type: ProviderType::Tts,
                    desired_revision: revision,
                },
                operation,
                Duration::from_millis(100),
            )
            .await?;
        Ok(TtsDiagnosticResult {
            provider_key,
            wav: output.wav,
            runtime,
        })
    }

    /// Executes exactly one canonical silence frame against the VAD runtime loaded at startup.
    pub async fn execute_vad(
        &self,
        key: &str,
    ) -> Result<VadDiagnosticResult, ProviderDiagnosticRequestError> {
        if self.manager.is_some() {
            let service = self.managed_request(key, "vad").await?;
            let mut result = Box::pin(service.execute_vad(key)).await?;
            result.provider_key = key.to_owned();
            return Ok(result);
        }
        let snapshot = self.request_snapshot(key, "vad").await?;
        let provider_key = snapshot.key.clone();
        let revision = snapshot.revision;
        let operation = self
            .registry
            .vad_diagnostic(&provider_key)
            .map_err(|_| ProviderDiagnosticError::RuntimeNotLoaded)?;
        let (probability, runtime) = self
            .execute(
                ProviderDiagnosticTarget {
                    key: provider_key.clone(),
                    provider_type: ProviderType::Vad,
                    desired_revision: revision,
                },
                operation,
                Duration::from_millis(100),
            )
            .await?;
        Ok(VadDiagnosticResult {
            provider_key,
            probability: probability.probability,
            start_sample: probability.start_sample,
            end_sample: probability.end_sample,
            runtime,
        })
    }

    /// Executes one bounded ASR diagnostic against the runtime loaded at process startup.
    pub async fn execute_asr(
        &self,
        key: &str,
        pcm: PcmF32Mono,
    ) -> Result<AsrDiagnosticResult, ProviderDiagnosticRequestError> {
        if self.manager.is_some() {
            let service = self.managed_request(key, "asr").await?;
            let mut result = Box::pin(service.execute_asr(key, pcm)).await?;
            result.provider_key = key.to_owned();
            return Ok(result);
        }
        let snapshot = self.request_snapshot(key, "asr").await?;
        let provider_key = snapshot.key.clone();
        let revision = snapshot.revision;
        let adapter = snapshot.adapter;
        let config_json = snapshot.config_json;
        let accepts_sample_rate = crate::providers::compiled_provider_adapter_registry()
            .get(&adapter)
            .filter(|descriptor| descriptor.provider_type == ProviderType::Asr)
            .and_then(|descriptor| descriptor.capabilities.input_sample_rates)
            .is_some_and(|rates| rates.contains(&pcm.sample_rate_hz()));
        if !accepts_sample_rate {
            return Err(ProviderDiagnosticRequestError::InvalidInput);
        }
        let language = serde_json::from_str::<serde_json::Value>(&config_json)
            .ok()
            .and_then(|config| config.get("language")?.as_str().map(str::to_owned))
            .unwrap_or_else(|| "und".to_owned());
        let operation = self
            .registry
            .asr_diagnostic(&provider_key, pcm)
            .map_err(|_| ProviderDiagnosticError::RuntimeNotLoaded)?;
        let (text, runtime) = self
            .execute(
                ProviderDiagnosticTarget {
                    key: provider_key.clone(),
                    provider_type: ProviderType::Asr,
                    desired_revision: revision,
                },
                operation,
                Duration::from_millis(100),
            )
            .await?;
        Ok(AsrDiagnosticResult {
            provider_key,
            text,
            language,
            runtime,
        })
    }
}
