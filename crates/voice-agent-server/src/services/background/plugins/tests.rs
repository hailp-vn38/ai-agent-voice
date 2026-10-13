use super::protocol::parse_response;
use serde_json::json;

#[tokio::test]
async fn worker_saturation_waits_for_release_and_expires_at_job_deadline() {
    let slots = tokio::sync::Semaphore::new(1);
    let held = slots.acquire().await.unwrap();
    let deadline = crate::database::unix_seconds().unwrap() + 2;
    let waiting = super::dispatch::acquire_worker_until(&slots, deadline);
    tokio::pin!(waiting);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut waiting)
            .await
            .is_err()
    );
    drop(held);
    assert!(waiting.await.is_ok());

    let held = slots.acquire().await.unwrap();
    let expired =
        super::dispatch::acquire_worker_until(&slots, crate::database::unix_seconds().unwrap() - 1)
            .await;
    assert_eq!(
        expired.err(),
        Some(super::PluginError::JobRejected(
            crate::services::background::JobError::DeadlineExceeded
        ))
    );
    drop(held);
}

#[test]
fn worker_timeout_distinguishes_job_deadline_from_step_limit() {
    let now = crate::database::unix_seconds().unwrap();
    assert_eq!(
        super::dispatch::classify_worker_timeout(now - 1),
        super::PluginError::WorkerDeadlineExceeded
    );
    assert_eq!(
        super::dispatch::classify_worker_timeout(now + 30),
        super::PluginError::WorkerTimeout
    );
}

#[test]
fn worker_response_requires_one_bounded_matching_schema_line() {
    let schema = json!({"type":"object","required":["message"],"properties":{"message":{"type":"string"}},"additionalProperties":false});
    let valid = b"{\"schema_version\":1,\"request_id\":\"attempt-1\",\"ok\":true,\"result\":{\"message\":\"hello\"},\"artifacts\":[]}\n";
    assert!(parse_response(valid, "attempt-1", &schema).is_ok());
    assert!(parse_response(valid, "attempt-2", &schema).is_err());
    assert!(
        parse_response(
            &[valid.as_slice(), valid.as_slice()].concat(),
            "attempt-1",
            &schema
        )
        .is_err()
    );
    let wrong = b"{\"schema_version\":1,\"request_id\":\"attempt-1\",\"ok\":true,\"result\":{\"message\":4},\"artifacts\":[]}\n";
    assert!(parse_response(wrong, "attempt-1", &schema).is_err());
    assert!(parse_response(&vec![b'x'; 65537], "attempt-1", &schema).is_err());
}
