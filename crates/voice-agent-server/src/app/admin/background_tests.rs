use super::background::{
    decode_attachment_base64, sanitize_wav_attachment_name, valid_wav_mime,
};

#[test]
fn wav_attachment_validation_sanitizes_name_mime_and_base64() {
    assert_eq!(
        sanitize_wav_attachment_name("C:\\fakepath\\cuoc-hop.WAV").as_deref(),
        Some("cuoc-hop.WAV")
    );
    assert_eq!(
        sanitize_wav_attachment_name("/tmp/recordings/voice_note.wav").as_deref(),
        Some("voice_note.wav")
    );
    assert!(sanitize_wav_attachment_name("not-wav.mp3").is_none());
    assert!(sanitize_wav_attachment_name("").is_none());

    assert!(valid_wav_mime("audio/wav"));
    assert!(valid_wav_mime("audio/x-wav"));
    assert!(valid_wav_mime("audio/wave"));
    assert!(!valid_wav_mime("audio/mpeg"));

    let decoded = decode_attachment_base64("UklGRg==").expect("base64");
    assert_eq!(decoded, b"RIFF");
}
