use serde::Deserialize;

pub(super) const MAX_JOB_ATTACHMENTS: usize = 4;
pub(super) const MAX_JOB_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum JobAttachmentMediaKind {
    Wav,
    Mp4,
}

impl JobAttachmentMediaKind {
    pub(super) const fn canonical_mime(self) -> &'static str {
        match self {
            Self::Wav => "audio/wav",
            Self::Mp4 => "video/mp4",
        }
    }
}

fn default_wav_mime() -> String {
    "audio/wav".to_owned()
}

#[derive(Deserialize)]
pub(super) struct JobAttachmentInput {
    pub name: String,
    #[serde(default = "default_wav_mime")]
    pub mime_type: String,
    pub data_base64: String,
}

pub(super) fn sanitize_job_attachment_name(raw: &str) -> Option<(String, JobAttachmentMediaKind)> {
    let base = raw.trim().rsplit(['/', '\\']).next().unwrap_or("").trim();
    if base.is_empty() || base.chars().count() > 128 {
        return None;
    }
    if base
        .chars()
        .any(|c| c.is_control() || matches!(c, '/' | '\\' | '\0'))
    {
        return None;
    }
    let lower = base.to_ascii_lowercase();
    if lower.ends_with(".wav") {
        Some((base.to_owned(), JobAttachmentMediaKind::Wav))
    } else if lower.ends_with(".mp4") {
        Some((base.to_owned(), JobAttachmentMediaKind::Mp4))
    } else {
        None
    }
}

#[cfg(test)]
pub(super) fn sanitize_wav_attachment_name(raw: &str) -> Option<String> {
    sanitize_job_attachment_name(raw)
        .filter(|(_, kind)| *kind == JobAttachmentMediaKind::Wav)
        .map(|(name, _)| name)
}

pub(super) fn valid_wav_mime(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "audio/wav" | "audio/x-wav" | "audio/wave"
    )
}

pub(super) fn valid_mp4_mime(raw: &str) -> bool {
    matches!(raw.trim().to_ascii_lowercase().as_str(), "video/mp4")
}

pub(super) fn valid_attachment_mime_for_kind(raw: &str, kind: JobAttachmentMediaKind) -> bool {
    match kind {
        JobAttachmentMediaKind::Wav => valid_wav_mime(raw),
        JobAttachmentMediaKind::Mp4 => valid_mp4_mime(raw),
    }
}

pub(super) fn decode_attachment_base64(raw: &str) -> Result<Vec<u8>, &'static str> {
    use base64::Engine as _;
    let payload = raw.trim();
    if payload.is_empty() {
        return Err("invalid_attachment");
    }
    base64::engine::general_purpose::STANDARD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(payload))
        .map_err(|_| "invalid_attachment")
}
