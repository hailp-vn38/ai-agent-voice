use std::sync::Arc;

use sha2::Digest;
use tokio::net::TcpListener;
use vela_server::{app::bootstrap_with_providers, providers::ProviderSet};

use crate::harness::{config, database_url, hex, insert_device, ota, push_config, start};
#[tokio::test]
async fn registered_device_receives_push_block_when_enabled() {
    let (base, db, task) = start(push_config(), false).await;
    insert_device(&db, "device-1", true).await;
    let (status, body) = ota(&base, "device-1").await;
    assert_eq!(status, 200);
    let push = &body["push"];
    let key = hex(&sha2::Sha256::digest(b"device-1"))[..32].to_owned();
    assert_eq!(push["endpoint"], "push.example.test:8883");
    assert_eq!(push["username"], "device-1");
    assert_eq!(push["client_id"], format!("vela-{key}"));
    assert_eq!(push["topic"], format!("vela/v1/devices/{key}/signal"));
    assert_eq!(push["keepalive"], 45);
    let expected = ring::hmac::sign(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, b"push-secret-0123456789"),
        b"v1:device-1",
    );
    assert_eq!(push["password"], hex(expected.as_ref()));
    assert!(body.get("mqtt").is_none());
    task.abort();
}

#[tokio::test]
async fn device_named_like_the_service_account_gets_no_push_block() {
    let (base, db, task) = start(push_config(), false).await;
    insert_device(&db, "vela-server", true).await;
    let (status, body) = ota(&base, "vela-server").await;
    assert_eq!(status, 200);
    assert!(body.get("push").is_none());
    task.abort();
}

#[tokio::test]
async fn no_push_block_when_feature_disabled() {
    let (base, db, task) = start(Default::default(), false).await;
    insert_device(&db, "device-1", true).await;
    let (status, body) = ota(&base, "device-1").await;
    assert_eq!(status, 200);
    assert!(body.get("push").is_none());
    assert!(body.get("mqtt").is_none());
    assert!(body.get("websocket").is_some());
    task.abort();
}

#[tokio::test]
async fn no_push_block_for_pending_or_blocked_devices() {
    let (base, db, task) = start(push_config(), true).await;
    let (status, pending) = ota(&base, "new-device").await;
    assert_eq!(status, 200);
    assert!(pending.get("activation").is_some());
    assert!(pending.get("push").is_none());
    assert!(pending.get("mqtt").is_none());
    insert_device(&db, "blocked-device", false).await;
    let (status, blocked) = ota(&base, "blocked-device").await;
    assert_eq!(status, 403);
    assert!(blocked.get("push").is_none());
    task.abort();
}

#[tokio::test]
async fn awkward_device_ids_still_yield_valid_topic_and_client_id() {
    let (base, db, task) = start(push_config(), false).await;
    insert_device(&db, "a/b+c#d", true).await;
    let (_, body) = ota(&base, "a/b+c#d").await;
    let push = &body["push"];
    assert_eq!(push["username"], "a/b+c#d");
    let topic = push["topic"].as_str().unwrap();
    let client_id = push["client_id"].as_str().unwrap();
    assert!(!topic.contains(['+', '#']));
    assert_eq!(topic.matches('/').count(), 4);
    assert!(
        client_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    );
    task.abort();
}

#[tokio::test]
async fn startup_fails_when_push_enabled_but_broker_url_is_unusable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut app_config = config(listener.local_addr().unwrap(), database_url());
    app_config.device_push = push_config();
    app_config.device_push.broker_url = "not-a-broker-url".into();
    let result = bootstrap_with_providers(app_config, Arc::new(ProviderSet::unavailable())).await;
    assert!(result.is_err());
}
