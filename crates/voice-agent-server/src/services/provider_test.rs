//! Source validation and managed execution of ephemeral Provider Test Drafts.
use super::{
    provider_diagnostic::{ProviderDiagnosticRequestError, ProviderDiagnosticService},
    test_credentials::{InlineCredential, SavedCredential},
};
use crate::{
    audio::PcmF32Mono,
    database::{
        Database, DesiredProvider, provider_config,
        secrets::{SecretResolver, SecretValue},
    },
    providers::TtsDiagnosticRequest,
};
use std::sync::Arc;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderTestDraft {
    #[serde(rename = "type")]
    pub kind: String,
    pub adapter: String,
    pub config_json: serde_json::Value,
    pub api_key: Option<SecretValue>,
    pub saved_credential: Option<SavedCredential>,
}
pub enum ProviderTestInput {
    Llm(String),
    Tts(TtsDiagnosticRequest),
    Asr(PcmF32Mono),
}
pub enum ProviderTestOutput {
    Text {
        text: String,
        language: Option<String>,
    },
    Wav(Vec<u8>),
}
#[derive(Debug, thiserror::Error)]
pub enum ProviderTestError {
    #[error("provider_config_invalid")]
    Config,
    #[error("credential_invalid")]
    Credential,
    #[error(transparent)]
    Diagnostic(#[from] ProviderDiagnosticRequestError),
}
pub struct ProviderTestRunner<'a> {
    pub diagnostics: &'a ProviderDiagnosticService,
    pub database: Option<&'a Database>,
    pub secrets: Arc<dyn SecretResolver>,
}
impl ProviderTestRunner<'_> {
    pub async fn run(
        &self,
        draft: ProviderTestDraft,
        input: ProviderTestInput,
    ) -> Result<ProviderTestOutput, ProviderTestError> {
        let expected = match &input {
            ProviderTestInput::Llm(_) => "llm",
            ProviderTestInput::Tts(_) => "tts",
            ProviderTestInput::Asr(_) => "asr",
        };
        let registry = crate::providers::compiled_provider_adapter_registry();
        let descriptor = registry
            .get(&draft.adapter)
            .ok_or(ProviderTestError::Config)?;
        if draft.kind != expected || descriptor.provider_type.as_str() != expected {
            return Err(ProviderDiagnosticRequestError::TypeMismatch.into());
        }
        let config_json = provider_config::validate(&draft.adapter, &draft.config_json)
            .map_err(|_| ProviderTestError::Config)?;
        match &input {
            ProviderTestInput::Llm(text) if text.is_empty() || text.len() > 8192 => {
                return Err(ProviderDiagnosticRequestError::InvalidInput.into());
            }
            ProviderTestInput::Tts(request)
                if request.text.is_empty()
                    || request.text.len() > 4096
                    || [&request.voice, &request.language].iter().any(|value| {
                        value
                            .as_ref()
                            .is_some_and(|v| v.is_empty() || v.len() > 128)
                    }) =>
            {
                return Err(ProviderDiagnosticRequestError::InvalidInput.into());
            }
            ProviderTestInput::Asr(pcm)
                if pcm.samples().is_empty()
                    || !descriptor
                        .capabilities
                        .input_sample_rates
                        .is_some_and(|rates| rates.contains(&pcm.sample_rate_hz())) =>
            {
                return Err(ProviderDiagnosticRequestError::InvalidInput.into());
            }
            _ => {}
        }
        if draft.api_key.is_some() && draft.saved_credential.is_some() {
            return Err(ProviderTestError::Credential);
        }
        let credential = if let Some(saved) = draft.saved_credential {
            let database = self
                .database
                .ok_or(ProviderDiagnosticRequestError::DatabaseUnavailable)?;
            let (_, _, kind, adapter, _, revision, _, record) = database
                .diagnostic_provider_row(&saved.key)
                .await
                .map_err(|e| match e {
                    sqlx::Error::RowNotFound => ProviderDiagnosticRequestError::NotFound,
                    _ => ProviderDiagnosticRequestError::DatabaseUnavailable,
                })?;
            if revision != saved.expected_revision {
                return Err(ProviderDiagnosticRequestError::RevisionConflict.into());
            }
            if kind != draft.kind || adapter != draft.adapter {
                return Err(ProviderTestError::Credential);
            }
            let reference = crate::database::credentials::reference(
                &format!("provider:{}", saved.key),
                record.as_deref(),
                crate::database::secrets::provider_secret_env(&saved.key, &adapter),
            )
            .ok_or(ProviderTestError::Credential)?;
            Some(
                self.secrets
                    .resolve(
                        &crate::database::secrets::SecretRef::parse(reference)
                            .map_err(|_| ProviderTestError::Credential)?,
                    )
                    .map_err(|_| ProviderTestError::Credential)?,
            )
        } else {
            draft.api_key
        };
        if credential
            .as_ref()
            .is_some_and(|value| !crate::database::credentials::valid_input(value))
            || (credential.is_some()
                && !matches!(draft.adapter.as_str(), "openai" | "chillaudio_ws"))
        {
            return Err(ProviderTestError::Credential);
        }
        let has_credential = credential.is_some();
        let secrets: Arc<dyn SecretResolver> = match credential {
            Some(value) => Arc::new(InlineCredential(value)),
            None => self.secrets.clone(),
        };
        let snapshot = DesiredProvider {
            id: 0,
            revision: 1,
            key: "draft".into(),
            kind: draft.kind,
            adapter: draft.adapter,
            config_json,
            secret_ref: has_credential.then(|| "DRAFT".into()),
        };
        let service = self.diagnostics.draft_request(snapshot, secrets).await?;
        match input {
            ProviderTestInput::Llm(text) => {
                let result = service.execute_llm("draft", text).await?;
                Ok(ProviderTestOutput::Text {
                    text: result.text,
                    language: None,
                })
            }
            ProviderTestInput::Tts(request) => Ok(ProviderTestOutput::Wav(
                service.execute_tts("draft", request).await?.wav,
            )),
            ProviderTestInput::Asr(pcm) => {
                let result = service.execute_asr("draft", pcm).await?;
                Ok(ProviderTestOutput::Text {
                    text: result.text,
                    language: Some(result.language),
                })
            }
        }
    }
}
