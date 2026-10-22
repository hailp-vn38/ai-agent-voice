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

#[test]
fn settings_extension_is_optional_and_legacy_request_stays_unchanged() {
    let old = br#"{"schema_version":1,"request_id":"test","operation":"extract","arguments":{},"artifact_dir":"/work/out"}
"#;
    assert!(parse_request(old.as_slice()).unwrap().settings.is_none());
    let extended = br#"{"schema_version":1,"request_id":"test","operation":"extract","arguments":{},"artifact_dir":"/work/out","settings":{"max_links":4}}
"#;
    let req = parse_request(extended.as_slice()).unwrap();
    assert_eq!(req.settings.unwrap()["max_links"], 4);
    let unknown = br#"{"schema_version":1,"request_id":"test","operation":"extract","arguments":{},"artifact_dir":"/work/out","unexpected":1}
"#;
    assert!(parse_request(unknown.as_slice()).is_err());
}

#[test]
fn request_debug_never_exposes_worker_secrets() {
    let secret = "SECRET_WORKER_VALUE_SHOULD_NOT_APPEAR";
    let line = format!(
        "{{\"schema_version\":1,\"request_id\":\"test\",\"operation\":\"fetch\",\"arguments\":{{}},\"artifact_dir\":\"/work/out\",\"settings\":{{\"api_key\":\"{secret}\"}}}}\n"
    );
    let parsed = parse_request(line.as_bytes()).unwrap();
    let debug = format!("{parsed:?}");
    assert!(!debug.contains(secret));
    assert!(debug.contains("[REDACTED]"));
}

#[test]
fn streamed_file_attachment_keeps_bytes_and_digest() {
    let directory = std::env::temp_dir().join(format!("sdk-stream-{}", uuid_like()));
    fs::create_dir(&directory).unwrap();
    let src = directory.join("source");
    let payload = "Vietnamese Unicode 🦀\n".repeat(60_000);
    fs::write(&src, payload.as_bytes()).unwrap();
    let mut result = Outcome::new(Value::Null);
    result.attach_file(directory.to_str().unwrap(), "output.md", "text/markdown", &src).unwrap();
    assert_eq!(result.artifacts[0].size_bytes, payload.len() as u64);
    assert_eq!(result.artifacts[0].sha256, Sha256::digest(payload.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>());
    assert_eq!(fs::read(directory.join("output.md")).unwrap(), payload.as_bytes());
    fs::remove_dir_all(directory).unwrap();
}
fn uuid_like() -> String {
    format!("{}-{}", std::process::id(), std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())
}

#[test]
fn streamed_large_file_does_not_use_one_megabyte_attachment_limit() {
    let directory = std::env::temp_dir().join(format!("sdk-binary-{}", uuid_like()));
    fs::create_dir(&directory).unwrap();
    let src = directory.join("large-source.bin");
    let file = fs::File::create(&src).unwrap();
    file.set_len(100 * 1024 * 1024).unwrap();
    drop(file);
    let mut result = Outcome::new(Value::Null);
    result.attach_file(directory.to_str().unwrap(), "large.bin", "application/octet-stream", &src)
        .unwrap();
    assert_eq!(result.artifacts[0].size_bytes, 100 * 1024 * 1024);
    assert_eq!(fs::metadata(directory.join("large.bin")).unwrap().len(), result.artifacts[0].size_bytes);
    fs::remove_dir_all(&directory).unwrap();
}
