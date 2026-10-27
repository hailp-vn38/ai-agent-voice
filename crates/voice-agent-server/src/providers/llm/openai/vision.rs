use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use url::Url;

use crate::providers::llm::{LlmError, LlmOutput};

use super::provider::ConfiguredOpenAiLlm;

#[derive(Serialize)]
struct VisionPayload {
    model: String,
    messages: [VisionMessage; 1],
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
}

#[derive(Serialize)]
struct VisionMessage {
    role: &'static str,
    content: [VisionContent; 2],
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum VisionContent {
    Text { text: String },
    ImageUrl { image_url: VisionImageUrl },
}

#[derive(Serialize)]
struct VisionImageUrl {
    url: String,
}

pub(crate) struct OpenAiVisionCompletion<'a> {
    pub client: &'a reqwest::Client,
    pub endpoint: &'a Url,
    pub api_key: &'a str,
    pub model: &'a str,
    pub question: &'a str,
    pub image_bytes: &'a [u8],
    pub mime_type: &'a str,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
}

pub(crate) async fn execute_openai_vision_completion(
    req: OpenAiVisionCompletion<'_>,
) -> Result<String, LlmError> {
    if req.question.trim().is_empty()
        || req.image_bytes.is_empty()
        || req.mime_type.trim().is_empty()
    {
        return Err(LlmError::Rejected);
    }
    let payload = VisionPayload {
        model: req.model.to_owned(),
        stream: false,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        top_p: req.top_p,
        messages: [VisionMessage {
            role: "user",
            content: [
                VisionContent::Text {
                    text: req.question.to_owned(),
                },
                VisionContent::ImageUrl {
                    image_url: VisionImageUrl {
                        url: format!(
                            "data:{};base64,{}",
                            req.mime_type,
                            STANDARD.encode(req.image_bytes)
                        ),
                    },
                },
            ],
        }],
    };
    let response = req
        .client
        .post(req.endpoint.clone())
        .bearer_auth(req.api_key)
        .json(&payload)
        .send()
        .await
        .map_err(|_| LlmError::Failed)?;
    let status = response.status();
    if !status.is_success() {
        return Err(
            if matches!(status.as_u16(), 401 | 403 | 429) || status.is_client_error() {
                LlmError::Rejected
            } else {
                LlmError::Failed
            },
        );
    }
    let value: serde_json::Value = response.json().await.map_err(|_| LlmError::Rejected)?;
    let text = value
        .pointer("/choices/0/message/content")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(LlmError::Rejected)?;
    Ok(text.to_owned())
}

impl ConfiguredOpenAiLlm {
    pub(super) async fn complete_vision(
        &self,
        prompt: &str,
        image_bytes: &[u8],
        mime_type: &str,
    ) -> Result<LlmOutput, LlmError> {
        let api_key = self.ensure_api_key()?;
        let endpoint = self.endpoint("chat/completions")?;
        let text = execute_openai_vision_completion(OpenAiVisionCompletion {
            client: &self.client,
            endpoint: &endpoint,
            api_key,
            model: &self.model,
            question: prompt,
            image_bytes,
            mime_type,
            max_tokens: self.max_tokens,
            temperature: self.temperature(),
            top_p: None,
        })
        .await?;
        Ok(LlmOutput::Text(text))
    }
}
