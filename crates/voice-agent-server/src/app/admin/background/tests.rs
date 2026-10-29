use super::attachments::{
    JobAttachmentMediaKind, decode_attachment_base64, sanitize_job_attachment_name,
    sanitize_wav_attachment_name, valid_attachment_mime_for_kind, valid_mp4_mime, valid_wav_mime,
};

#[test]
fn wav_and_mp4_attachment_validation_sanitizes_name_mime_and_base64() {
    assert_eq!(
        sanitize_wav_attachment_name("C:\\fakepath\\cuoc-hop.WAV").as_deref(),
        Some("cuoc-hop.WAV")
    );
    assert_eq!(
        sanitize_wav_attachment_name("/tmp/recordings/voice_note.wav").as_deref(),
        Some("voice_note.wav")
    );
    assert_eq!(
        sanitize_job_attachment_name("/tmp/videos/demo-clip.MP4"),
        Some(("demo-clip.MP4".to_owned(), JobAttachmentMediaKind::Mp4))
    );
    assert!(sanitize_job_attachment_name("not-supported.mp3").is_none());
    assert!(sanitize_job_attachment_name("").is_none());

    assert!(valid_wav_mime("audio/wav"));
    assert!(valid_wav_mime("audio/x-wav"));
    assert!(valid_wav_mime("audio/wave"));
    assert!(!valid_wav_mime("audio/mpeg"));
    assert!(valid_mp4_mime("video/mp4"));
    assert!(!valid_mp4_mime("video/webm"));
    assert!(valid_attachment_mime_for_kind(
        "video/mp4",
        JobAttachmentMediaKind::Mp4
    ));
    assert!(!valid_attachment_mime_for_kind(
        "audio/wav",
        JobAttachmentMediaKind::Mp4
    ));

    let decoded = decode_attachment_base64("UklGRg==").expect("base64");
    assert_eq!(decoded, b"RIFF");
}
