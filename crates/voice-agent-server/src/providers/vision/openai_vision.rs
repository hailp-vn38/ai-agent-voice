use super::{VisionError, VisionProvider, VisionRequest, VisionResponse};
use crate::config::{OpenAiVisionConfig, SecretString};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use url::Url;

pub struct OpenAiVisionProvider {
    client: reqwest::Client,
    endpoint: Url,
    api_key: SecretString,
    model: String,
    max_tokens: u32,
    temperature: f32,
    top_p: f32,
}

impl OpenAiVisionProvider {
    pub fn new(config: OpenAiVisionConfig) -> Result<Self, VisionError> {
        let mut base = config.base_url;
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let endpoint = base
            .join("chat/completions")
            .map_err(|_| VisionError::InvalidRequest)?;
        Ok(Self {
            client: reqwest::Client::new(),
            endpoint,
            api_key: config.api_key,
            model: config.model,
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            top_p: config.top_p,
        })
    }
}

#[derive(Serialize)]
struct Payload {
    model: String,
    messages: [Message; 1],
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
}
#[derive(Serialize)]
struct Message {
    role: &'static str,
    content: [Content; 2],
}
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Content {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}
#[derive(Serialize)]
struct ImageUrl {
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
) -> Result<String, VisionError> {
    if req.question.trim().is_empty()
        || req.image_bytes.is_empty()
        || req.mime_type.trim().is_empty()
    {
        return Err(VisionError::InvalidRequest);
    }
    let payload = Payload {
        model: req.model.to_owned(),
        stream: false,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        top_p: req.top_p,
        messages: [Message {
            role: "user",
            content: [
                Content::Text {
                    text: req.question.to_owned(),
                },
                Content::ImageUrl {
                    image_url: ImageUrl {
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
        .map_err(|_| VisionError::Transport)?;
    let status = response.status();
    if !status.is_success() {
        return Err(
            if matches!(status.as_u16(), 401 | 403 | 429) || status.is_client_error() {
                VisionError::UpstreamRejected
            } else if matches!(status.as_u16(), 408 | 504) {
                VisionError::Timeout
            } else {
                VisionError::Transport
            },
        );
    }
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|_| VisionError::InvalidResponse)?;
    let text = value
        .pointer("/choices/0/message/content")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(VisionError::InvalidResponse)?;
    Ok(text.to_owned())
}

#[async_trait::async_trait]
impl VisionProvider for OpenAiVisionProvider {
    fn adapter(&self) -> &'static str {
        "openai_vision"
    }
    async fn analyze(&self, request: VisionRequest) -> Result<VisionResponse, VisionError> {
        let text = execute_openai_vision_completion(OpenAiVisionCompletion {
            client: &self.client,
            endpoint: &self.endpoint,
            api_key: self.api_key.expose(),
            model: &self.model,
            question: &request.question,
            image_bytes: request.image.as_ref(),
            mime_type: &request.mime_type,
            max_tokens: Some(self.max_tokens),
            temperature: Some(self.temperature),
            top_p: Some(self.top_p),
        })
        .await?;
        Ok(VisionResponse { text })
    }
}
