use super::*;

#[test]
fn request_rejects_extra_lines_and_oversized_input() {
    let line = b"{\"schema_version\":1,\"request_id\":\"test\",\"operation\":\"extract\",\"arguments\":{},\"artifact_dir\":\"/work/out\"}\n";
    assert!(parse_request(line.as_slice()).is_ok());
    assert!(parse_request([line.as_slice(), line.as_slice()].concat().as_slice()).is_err());
    assert!(parse_request(vec![b'x'; 65536].as_slice()).is_err());
}

#[test]
fn request_accepts_runner_artifact_directory_and_rejects_unsafe_paths() {
    for (directory, allowed) in [
        ("/attempts/4fe75065-927f-43a3-9e56-08bdfd0a976e/ea9a0301-0058-48e0-ae09-2576909caa5d/out", true),
        ("relative/out", false),
        ("/attempts/../etc", false),
    ] {
        let line = format!("{{\"schema_version\":1,\"request_id\":\"test\",\"operation\":\"extract\",\"arguments\":{{}},\"artifact_dir\":\"{directory}\"}}\n");
        assert_eq!(parse_request(line.as_bytes()).is_ok(), allowed, "{directory}");
    }
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

#[test]
fn writes_artifacts_to_runner_provided_directory() {
    let directory = std::env::temp_dir().join(format!("plugin-sdk-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let mut result = Outcome::new(Value::Null);
    result
        .attach(
            directory.to_str().unwrap(),
            "result.txt",
            "text/plain",
            b"hello",
        )
        .unwrap();
    assert_eq!(fs::read(directory.join("result.txt")).unwrap(), b"hello");
    assert_eq!(result.artifacts[0].size_bytes, 5);
    fs::remove_dir_all(directory).unwrap();
}
