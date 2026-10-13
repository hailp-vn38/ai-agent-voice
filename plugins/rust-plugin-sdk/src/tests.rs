use super::*;

#[test]
fn request_rejects_extra_lines_and_oversized_input() {
    let line = b"{\"schema_version\":1,\"request_id\":\"test\",\"operation\":\"extract\",\"arguments\":{},\"artifact_dir\":\"/work/out\"}\n";
    assert!(parse_request(line.as_slice()).is_ok());
    assert!(parse_request([line.as_slice(), line.as_slice()].concat().as_slice()).is_err());
    assert!(parse_request(vec![b'x'; 65536].as_slice()).is_err());
}

#[test]
fn unicode_error_message_fits_worker_byte_limit() {
    let message = "Việt Nam 🦀".repeat(100);
    let bounded = error_message(&message);
    assert!(bounded.len() <= 256);
    assert!(message.starts_with(bounded));
}
#[test]
fn clips_at_unicode_boundary() {
    assert_eq!(clipped("Việt Nam", 4), ("Việt".to_string(), true));
}
#[test]
fn enforces_artifact_safety() {
    let mut result = Outcome::new(Value::Null);
    assert!(result
        .attach("/work/out", "../escape", "text/plain", b"x")
        .is_err());
}
