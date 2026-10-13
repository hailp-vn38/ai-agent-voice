/// Speech provenance stays with the actor and never selects a provider or Device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpeechOrigin {
    Llm,
    ToolDirect,
    ClientDirect,
    AdminDirect,
    Reminder {
        reminder_id: String,
        attempt_id: String,
    },
}

#[derive(Clone, Debug)]
pub struct DirectSpeechRequest {
    pub request_id: String,
    pub text: String,
    pub origin: SpeechOrigin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectSpeechError {
    InvalidText,
    InvalidRequestId,
    InvalidSession,
    Unauthorized,
    UnsupportedFeature,
    SessionBusy,
    CapacityReached,
    TtsUnavailable,
    DeviceOffline,
    RateLimited,
    ShuttingDown,
}

impl DirectSpeechError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidText => "invalid_text",
            Self::InvalidRequestId => "invalid_request_id",
            Self::InvalidSession => "invalid_session",
            Self::Unauthorized => "unauthorized",
            Self::UnsupportedFeature => "unsupported_feature",
            Self::SessionBusy => "session_busy",
            Self::CapacityReached => "capacity_reached",
            Self::TtsUnavailable => "tts_unavailable",
            Self::DeviceOffline => "device_offline",
            Self::RateLimited => "rate_limited",
            Self::ShuttingDown => "shutting_down",
        }
    }
}

pub fn validate_direct_speech(request: &DirectSpeechRequest) -> Result<(), DirectSpeechError> {
    if request.request_id.is_empty()
        || request.request_id.len() > 128
        || !request
            .request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
    {
        return Err(DirectSpeechError::InvalidRequestId);
    }
    if request.text.trim().is_empty()
        || request.text.chars().count() > 1_024
        || request.text.len() > 4_096
        || request
            .text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(DirectSpeechError::InvalidText);
    }
    Ok(())
}
