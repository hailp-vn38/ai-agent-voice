//! Vendor-neutral LLM streaming boundary.

use std::pin::Pin;

use futures_util::Stream;
use serde_json::Value;
use thiserror::Error;

pub(crate) mod gateway;
pub(crate) mod openai;
pub(crate) mod opencode_free;
#[cfg(feature = "qualification-providers")]
pub mod qualification;

pub use gateway::{LlmGatewayConfig, LlmGatewayProvider};

pub(crate) const REASONING_EFFORTS: &[&str] = &["low", "medium", "high"];
pub const LLM_CAPABILITY_TAGS: &[&str] = &[
    "chat",
    "vision",
    "image",
    "video",
    "audio",
    "reasoning",
    "embedding",
];

const DIAGNOSTIC_PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmCapabilityTag {
    Chat,
    Vision,
    Image,
    Video,
    Audio,
    Reasoning,
    Embedding,
}

impl LlmCapabilityTag {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Vision => "vision",
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Reasoning => "reasoning",
            Self::Embedding => "embedding",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "chat" => Some(Self::Chat),
            "vision" => Some(Self::Vision),
            "image" => Some(Self::Image),
            "video" => Some(Self::Video),
            "audio" | "tts" | "TTS" => Some(Self::Audio),
            "reasoning" => Some(Self::Reasoning),
            "embedding" => Some(Self::Embedding),
            _ => None,
        }
    }
}

pub fn effective_capability_tags(tags: Option<&[String]>) -> Vec<LlmCapabilityTag> {
    let parsed: Vec<LlmCapabilityTag> = tags
        .unwrap_or(&[])
        .iter()
        .filter_map(|tag| LlmCapabilityTag::parse(tag))
        .collect();
    if parsed.is_empty() {
        vec![LlmCapabilityTag::Chat]
    } else {
        parsed
    }
}

pub fn capability_tags_support(tags: &[LlmCapabilityTag], tag: LlmCapabilityTag) -> bool {
    tags.contains(&tag)
        || (tag == LlmCapabilityTag::Chat && tags.contains(&LlmCapabilityTag::Reasoning))
}

pub fn valid_llm_tags(tags: Option<&[String]>) -> bool {
    let Some(tags) = tags else {
        return true;
    };
    if tags.is_empty() || tags.len() > LLM_CAPABILITY_TAGS.len() {
        return false;
    }
    for (idx, tag) in tags.iter().enumerate() {
        let Some(parsed) = LlmCapabilityTag::parse(tag) else {
            return false;
        };
        if tags[..idx]
            .iter()
            .any(|earlier| earlier == tag || LlmCapabilityTag::parse(earlier) == Some(parsed))
        {
            return false;
        }
    }
    true
}

pub fn effective_llm_tags(tags: Option<&[String]>) -> Vec<String> {
    match tags {
        Some(list) if !list.is_empty() => list.to_vec(),
        _ => vec!["chat".to_owned()],
    }
}

pub fn llm_config_tags(config_json: &str) -> Vec<String> {
    let Ok(parsed) = serde_json::from_str::<Value>(config_json) else {
        return vec!["chat".to_owned()];
    };
    let is_muse_gateway = parsed
        .get("gateway_binding")
        .and_then(Value::as_str)
        .is_some_and(|b| b.trim() == "pool:muse")
        || parsed
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(|m| m.trim().starts_with("muse-"));
    let Some(arr) = parsed.get("tags").and_then(Value::as_array) else {
        return if is_muse_gateway {
            vec![
                "chat".to_owned(),
                "vision".to_owned(),
                "image".to_owned(),
                "video".to_owned(),
            ]
        } else {
            vec!["chat".to_owned()]
        };
    };
    let tags: Vec<String> = arr
        .iter()
        .filter_map(Value::as_str)
        .filter(|t| LlmCapabilityTag::parse(t).is_some())
        .map(str::to_owned)
        .collect();
    if tags.is_empty() {
        if is_muse_gateway {
            vec![
                "chat".to_owned(),
                "vision".to_owned(),
                "image".to_owned(),
                "video".to_owned(),
            ]
        } else {
            vec!["chat".to_owned()]
        }
    } else {
        tags
    }
}

pub fn llm_config_supports_tag(config_json: &str, required_tag: &str) -> bool {
    let Some(required) = LlmCapabilityTag::parse(required_tag) else {
        return false;
    };
    let tags = llm_config_tags(config_json);
    let parsed: Vec<LlmCapabilityTag> = tags
        .iter()
        .filter_map(|t| LlmCapabilityTag::parse(t))
        .collect();
    capability_tags_support(&parsed, required)
}

pub fn validate_template_provider_capability(
    provider_type: &str,
    config_json: &str,
) -> Result<(), &'static str> {
    if provider_type == "llm" && !llm_config_supports_tag(config_json, "chat") {
        return Err("provider_capability_mismatch");
    }
    Ok(())
}

pub(crate) fn valid_generation_bounds(
    max_tokens: Option<u32>,
    context_limit_tokens: Option<u32>,
    reasoning_effort: Option<&str>,
    temperature_percent: Option<u16>,
) -> bool {
    max_tokens.is_none_or(|v| v >= 1)
        && context_limit_tokens.is_none_or(|v| (4_096..=2_000_000).contains(&v))
        && reasoning_effort.is_none_or(|v| REASONING_EFFORTS.contains(&v))
        && temperature_percent.is_none_or(|v| v <= 200)
}

pub type LlmEventStream = Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send>>;

#[derive(Clone, Debug, PartialEq)]
pub enum LlmEvent {
    TextDelta(String),
    ToolCall(ToolCall),
    Finished,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ChatMessage {
    System {
        content: String,
    },
    User {
        content: String,
    },
    AssistantText {
        content: String,
    },
    AssistantToolCalls {
        calls: Vec<ToolCall>,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LlmRequest {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
}

impl LlmRequest {
    pub fn text_turn(text: String) -> Self {
        Self {
            messages: vec![ChatMessage::User { content: text }],
            tools: Vec::new(),
        }
    }
}

impl From<String> for LlmRequest {
    fn from(value: String) -> Self {
        Self::text_turn(value)
    }
}

impl From<&str> for LlmRequest {
    fn from(value: &str) -> Self {
        Self::text_turn(value.to_owned())
    }
}

/// Số lượng ảnh tham chiếu tối đa được đính kèm trong một yêu cầu sinh ảnh.
pub const MAX_REFERENCE_IMAGES: usize = 4;

/// Ảnh đầu vào đã được xác thực MIME và giải mã thành byte nguyên bản.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReferenceImage {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

impl ReferenceImage {
    /// Chuẩn hóa MIME từ magic bytes (`image/png`, `image/jpeg`, `image/webp`, `image/gif`).
    pub fn from_bytes(bytes: Vec<u8>, fallback_mime: &str) -> Result<Self, LlmError> {
        if bytes.is_empty() {
            return Err(LlmError::Rejected);
        }
        let mime = sniff_media_mime_type(&bytes, fallback_mime);
        if !mime.starts_with("image/") {
            return Err(LlmError::Rejected);
        }
        Ok(Self {
            bytes,
            mime_type: mime.to_owned(),
        })
    }
}

/// Unified single-shot operation envelope across all non-streaming LLM capability tags.
#[derive(Clone, Debug, PartialEq)]
pub enum LlmOperation {
    Chat {
        request: LlmRequest,
        reasoning: bool,
    },
    Vision {
        prompt: String,
        image_bytes: Vec<u8>,
        mime_type: String,
    },
    ImageGeneration {
        prompt: String,
        width: u32,
        height: u32,
        /// Từ 0 đến `MAX_REFERENCE_IMAGES` (4) ảnh tham chiếu.
        reference_images: Vec<ReferenceImage>,
        /// Yêu cầu nền trong suốt (`true` khi `background == "transparent"` hoặc format yêu cầu tách nền).
        transparent_background: bool,
    },
    VideoGeneration {
        prompt: String,
        width: u32,
        height: u32,
        duration_secs: u32,
        /// Ảnh khung hình đầu tiên (Image-to-Video) nếu có.
        first_frame: Option<ReferenceImage>,
    },
    AudioSpeech {
        text: String,
        voice: Option<String>,
        speed: Option<f32>,
    },
    AudioTranscription {
        wav_bytes: Vec<u8>,
        language: Option<String>,
    },
    Embedding {
        inputs: Vec<String>,
    },
}

impl LlmOperation {
    pub const fn required_tag(&self) -> LlmCapabilityTag {
        match self {
            Self::Chat {
                reasoning: false, ..
            } => LlmCapabilityTag::Chat,
            Self::Chat {
                reasoning: true, ..
            } => LlmCapabilityTag::Reasoning,
            Self::Vision { .. } => LlmCapabilityTag::Vision,
            Self::ImageGeneration { .. } => LlmCapabilityTag::Image,
            Self::VideoGeneration { .. } => LlmCapabilityTag::Video,
            Self::AudioSpeech { .. } | Self::AudioTranscription { .. } => LlmCapabilityTag::Audio,
            Self::Embedding { .. } => LlmCapabilityTag::Embedding,
        }
    }

    pub fn diagnostic_for(tag: LlmCapabilityTag, input: String) -> Self {
        let prompt = if input.trim().is_empty() {
            "diagnostic".to_owned()
        } else {
            input
        };
        match tag {
            LlmCapabilityTag::Chat => Self::Chat {
                request: LlmRequest::text_turn(prompt),
                reasoning: false,
            },
            LlmCapabilityTag::Reasoning => Self::Chat {
                request: LlmRequest::text_turn(prompt),
                reasoning: true,
            },
            LlmCapabilityTag::Vision => Self::Vision {
                prompt,
                image_bytes: DIAGNOSTIC_PNG_1X1.to_vec(),
                mime_type: "image/png".to_owned(),
            },
            LlmCapabilityTag::Image => Self::ImageGeneration {
                prompt,
                width: 1024,
                height: 1024,
                reference_images: Vec::new(),
                transparent_background: false,
            },
            LlmCapabilityTag::Video => Self::VideoGeneration {
                prompt,
                width: 1280,
                height: 720,
                duration_secs: 5,
                first_frame: None,
            },
            LlmCapabilityTag::Audio => Self::AudioSpeech {
                text: prompt,
                voice: None,
                speed: None,
            },
            LlmCapabilityTag::Embedding => Self::Embedding {
                inputs: vec![prompt],
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LlmDiagnosticMedia {
    pub mime_type: String,
    pub data_url: String,
}

pub(crate) fn sniff_media_mime_type<'a>(bytes: &[u8], fallback: &'a str) -> &'a str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        "audio/wav"
    } else if bytes.len() >= 8 && &bytes[4..8] == b"ftyp" {
        "video/mp4"
    } else {
        fallback
    }
}

/// Individual binary media part returned by a multi-clip/multi-asset LLM media operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaPart {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

/// Normalized output across all non-streaming LLM capability operations.
#[derive(Clone, Debug, PartialEq)]
pub enum LlmOutput {
    Text(String),
    Media {
        bytes: Vec<u8>,
        mime_type: String,
        extra_parts: Vec<MediaPart>,
        assistant_message: Option<String>,
    },
    Embeddings {
        vectors: Vec<Vec<f32>>,
        dimensions: usize,
    },
}

impl LlmOutput {
    pub fn single_media(bytes: Vec<u8>, mime_type: impl Into<String>) -> Self {
        Self::Media {
            bytes,
            mime_type: mime_type.into(),
            extra_parts: Vec::new(),
            assistant_message: None,
        }
    }

    pub fn from_media_parts(
        mut parts: Vec<MediaPart>,
        assistant_message: Option<String>,
    ) -> Option<Self> {
        if parts.is_empty() {
            return None;
        }
        let first = parts.remove(0);
        Some(Self::Media {
            bytes: first.bytes,
            mime_type: first.mime_type,
            extra_parts: parts,
            assistant_message,
        })
    }

    pub fn into_media_parts(self) -> Option<(Vec<MediaPart>, Option<String>)> {
        match self {
            Self::Media {
                bytes,
                mime_type,
                extra_parts,
                assistant_message,
            } => {
                let mut all = Vec::with_capacity(1 + extra_parts.len());
                all.push(MediaPart { bytes, mime_type });
                all.extend(extra_parts);
                Some((all, assistant_message))
            }
            _ => None,
        }
    }

    pub fn diagnostic_summary(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Media {
                bytes,
                mime_type,
                extra_parts,
                ..
            } => {
                if extra_parts.is_empty() {
                    format!("Generated {mime_type} ({} bytes)", bytes.len())
                } else {
                    let total_bytes: usize =
                        bytes.len() + extra_parts.iter().map(|p| p.bytes.len()).sum::<usize>();
                    format!(
                        "Generated {} {mime_type} clip(s) ({total_bytes} bytes total)",
                        1 + extra_parts.len()
                    )
                }
            }
            Self::Embeddings {
                vectors,
                dimensions,
            } => {
                format!(
                    "Embedded {} input(s) (dimensions={dimensions})",
                    vectors.len()
                )
            }
        }
    }

    pub fn diagnostic_media(&self) -> Option<LlmDiagnosticMedia> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        match self {
            Self::Media {
                bytes, mime_type, ..
            } if !bytes.is_empty() => {
                let detected_mime = sniff_media_mime_type(bytes, mime_type);
                let data_url = format!("data:{detected_mime};base64,{}", STANDARD.encode(bytes));
                Some(LlmDiagnosticMedia {
                    mime_type: detected_mime.to_owned(),
                    data_url,
                })
            }
            _ => None,
        }
    }
}

#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    fn supports_tools(&self) -> bool {
        false
    }
    fn adapter(&self) -> &'static str;

    fn capability_tags(&self) -> &[LlmCapabilityTag] {
        &[LlmCapabilityTag::Chat]
    }

    fn supports_tag(&self, tag: LlmCapabilityTag) -> bool {
        capability_tags_support(self.capability_tags(), tag)
    }

    fn ensure_tag(&self, tag: LlmCapabilityTag) -> Result<(), LlmError> {
        if self.supports_tag(tag) {
            Ok(())
        } else {
            Err(LlmError::UnsupportedCapability(tag.as_str()))
        }
    }

    fn complete(&self, _: &LlmRequest) -> Result<String, LlmError> {
        Err(LlmError::Failed)
    }

    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        self.ensure_tag(LlmCapabilityTag::Chat)?;
        let response = self.complete(&request)?;
        Ok(Box::pin(futures_util::stream::iter([
            Ok(LlmEvent::TextDelta(response)),
            Ok(LlmEvent::Finished),
        ])))
    }

    async fn invoke(&self, op: LlmOperation) -> Result<LlmOutput, LlmError> {
        self.ensure_tag(op.required_tag())?;
        self.execute_operation(op).await
    }

    async fn execute_operation(&self, op: LlmOperation) -> Result<LlmOutput, LlmError> {
        match op {
            LlmOperation::Chat { request, .. } => self.complete(&request).map(LlmOutput::Text),
            _ => Err(LlmError::UnsupportedCapability(op.required_tag().as_str())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum LlmError {
    #[error("LLM completion failed")]
    Failed,
    /// A terminal provider response definitively rejected or failed to produce usable output.
    #[error("LLM returned a terminal invalid response")]
    Rejected,
    #[error("LLM provider does not support capability tag `{0}`")]
    UnsupportedCapability(&'static str),
}

pub struct UnavailableLlm;

impl LlmProvider for UnavailableLlm {
    fn adapter(&self) -> &'static str {
        "unavailable"
    }
}

pub(crate) use openai::provider::ConfiguredOpenAiLlm;

pub(crate) async fn collect_chat_stream(mut stream: LlmEventStream) -> Result<LlmOutput, LlmError> {
    use futures_util::StreamExt;
    let mut text = String::new();
    while let Some(item) = stream.next().await {
        match item? {
            LlmEvent::TextDelta(delta) => text.push_str(&delta),
            LlmEvent::ToolCall(_) => return Err(LlmError::Rejected),
            LlmEvent::Finished => break,
        }
    }
    if text.trim().is_empty() {
        return Err(LlmError::Rejected);
    }
    Ok(LlmOutput::Text(text))
}

pub(crate) async fn stream_openai_request(
    client: &dyn llm::chat::ChatProvider,
    request: LlmRequest,
    adapter: &'static str,
) -> Result<LlmEventStream, LlmError> {
    use futures_util::StreamExt;
    let messages = request
        .messages
        .iter()
        .filter(|message| !matches!(message, ChatMessage::System { .. }))
        .map(to_llm_message)
        .collect::<Vec<_>>();
    let tools = request
        .tools
        .iter()
        .map(|tool| llm::chat::Tool {
            tool_type: "function".into(),
            function: llm::chat::FunctionTool {
                name: tool.name.clone(),
                description: tool.description.clone(),
                parameters: tool.parameters.clone(),
            },
            cache_control: None,
        })
        .collect::<Vec<_>>();
    let stream = client
        .chat_stream_with_tools(&messages, (!tools.is_empty()).then_some(tools.as_slice()))
        .await
        .map_err(|error| {
            tracing::warn!(
                event = "llm_provider_open_stream_failed",
                adapter,
                "OpenAI-compatible LLM stream could not be opened"
            );
            match error {
                llm::error::LLMError::ResponseFormatError { .. }
                | llm::error::LLMError::AuthError(_)
                | llm::error::LLMError::InvalidRequest(_)
                | llm::error::LLMError::ProviderError(_)
                | llm::error::LLMError::JsonError(_) => LlmError::Rejected,
                _ => LlmError::Failed,
            }
        })?;
    Ok(Box::pin(stream.filter_map(|chunk| async move {
        adapt_stream_chunk(chunk)
    })))
}

/// Maps provider transport chunks to the small domain vocabulary used by the actor.
/// Tool start and argument deltas are intentionally not terminal events: the `llm`
/// crate assembles them and emits the completed call later in the same stream.
fn adapt_stream_chunk(
    chunk: Result<llm::chat::StreamChunk, llm::error::LLMError>,
) -> Option<Result<LlmEvent, LlmError>> {
    match chunk {
        Ok(llm::chat::StreamChunk::Text(text)) => Some(Ok(LlmEvent::TextDelta(text))),
        Ok(llm::chat::StreamChunk::ToolUseStart { index, .. }) => {
            tracing::info!(
                event = "llm_tool_use_started",
                tool_index = index,
                "LLM tool use started"
            );
            None
        }
        Ok(llm::chat::StreamChunk::ToolUseInputDelta { .. }) => None,
        Ok(llm::chat::StreamChunk::ToolUseComplete { tool_call, .. }) => {
            let name = tool_call.function.name.clone();
            let result = serde_json::from_str(&tool_call.function.arguments)
                .map(|arguments| {
                    LlmEvent::ToolCall(ToolCall {
                        id: tool_call.id,
                        name,
                        arguments,
                    })
                })
                .map_err(|_| LlmError::Failed);
            if result.is_ok() {
                tracing::info!(event = "llm_tool_call_complete", "LLM tool call completed");
            }
            Some(result)
        }
        Ok(llm::chat::StreamChunk::Done { .. }) => Some(Ok(LlmEvent::Finished)),
        Err(_) => {
            tracing::warn!(
                event = "llm_provider_stream_chunk_failed",
                "OpenAI-compatible LLM stream chunk failed"
            );
            Some(Err(LlmError::Failed))
        }
    }
}

fn to_llm_message(message: &ChatMessage) -> llm::chat::ChatMessage {
    match message {
        // The upstream `llm` crate has no system-role ChatMessage. The configured
        // OpenAI client carries the system instruction separately; this arm is
        // unreachable after `ConfiguredOpenAiLlm::stream` removes it.
        ChatMessage::System { .. } => unreachable!("system prompt is configured separately"),
        ChatMessage::User { content } => llm::chat::ChatMessage::user().content(content).build(),
        ChatMessage::AssistantText { content } => {
            llm::chat::ChatMessage::assistant().content(content).build()
        }
        ChatMessage::AssistantToolCalls { calls } => llm::chat::ChatMessage::assistant()
            .tool_use(calls.iter().map(to_llm_call).collect())
            .build(),
        ChatMessage::ToolResult {
            tool_call_id,
            content,
        } => llm::chat::ChatMessage::user()
            .tool_result(vec![llm::ToolCall {
                id: tool_call_id.clone(),
                call_type: "function".into(),
                function: llm::FunctionCall {
                    name: "tool_result".into(),
                    arguments: content.clone(),
                },
            }])
            .build(),
    }
}

fn to_llm_call(call: &ToolCall) -> llm::ToolCall {
    llm::ToolCall {
        id: call.id.clone(),
        call_type: "function".into(),
        function: llm::FunctionCall {
            name: call.name.clone(),
            arguments: call.arguments.to_string(),
        },
    }
}

#[cfg(test)]
mod tests;
