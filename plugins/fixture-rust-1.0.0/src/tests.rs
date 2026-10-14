use super::*;

#[test]
fn rejects_invalid_envelopes_before_writing_artifacts() {
    let request = "{\"schema_version\":2,\"request_id\":\"test\",\"operation\":\"collect\",\"arguments\":{\"topic\":\"AI\"},\"artifact_dir\":\"/work/out\"}\n";
    assert_eq!(run(request), Err("invalid_request"));
    assert_eq!(run(&request.replace("\n", "")), Err("invalid_request"));
    assert_eq!(run(&format!("{request}{request}")), Err("invalid_request"));
    for directory in ["relative/out", "/attempts/../etc"] {
        assert_eq!(
            run(&request.replace(":2", ":1").replace("/work/out", directory)),
            Err("invalid_request")
        );
    }
}

#[test]
fn writes_artifacts_to_runner_provided_directory() {
    let directory = std::env::temp_dir().join(format!("fixture-rust-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let request = json!({"schema_version":1,"request_id":"test","operation":"collect","arguments":{"topic":"AI"},"artifact_dir":directory});
    let response = run(&format!("{request}\n")).unwrap();
    assert_eq!(response["ok"], true);
    let records: Value =
        serde_json::from_slice(&fs::read(directory.join("records.json")).unwrap()).unwrap();
    assert_eq!(records["records"][0]["topic"], "AI");
    fs::remove_dir_all(directory).unwrap();
}
