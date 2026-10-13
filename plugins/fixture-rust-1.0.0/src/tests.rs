use super::*;

#[test]
fn rejects_invalid_envelopes_before_writing_artifacts() {
    let request = "{\"schema_version\":2,\"request_id\":\"test\",\"operation\":\"collect\",\"arguments\":{\"topic\":\"AI\"},\"artifact_dir\":\"/work/out\"}\n";
    assert_eq!(run(request), Err("invalid_request"));
    assert_eq!(run(&request.replace("\n", "")), Err("invalid_request"));
    assert_eq!(run(&format!("{request}{request}")), Err("invalid_request"));
}
