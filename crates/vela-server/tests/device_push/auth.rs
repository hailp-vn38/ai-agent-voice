use sqlx::SqlitePool;

use crate::harness::{hex, insert_device, ota, push_config, start};
async fn post(base: &str, path: &str, body: serde_json::Value) -> u16 {
    reqwest::Client::new()
        .post(format!("{base}/mqtt/auth/{path}"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn check_user(base: &str, username: &str, password: &str) -> u16 {
    post(
        base,
        "user",
        serde_json::json!({"username": username, "password": password, "clientid": "c"}),
    )
    .await
}

async fn check_acl(base: &str, username: &str, topic: &str, acc: u8) -> u16 {
    post(
        base,
        "acl",
        serde_json::json!({"username": username, "clientid": "c", "topic": topic, "acc": acc}),
    )
    .await
}

fn service_password() -> String {
    let root = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, b"push-secret-0123456789");
    let service_key = ring::hmac::Key::new(
        ring::hmac::HMAC_SHA256,
        ring::hmac::sign(&root, b"vela-device-push/service-key").as_ref(),
    );
    hex(ring::hmac::sign(&service_key, b"v1:vela-server").as_ref())
}

#[tokio::test]
async fn credentials_from_ota_pass_user_and_topic_checks_then_block_denies() {
    let (base, db, task) = start(push_config(), false).await;
    insert_device(&db, "device-1", true).await;
    let (_, body) = ota(&base, "device-1").await;
    let push = &body["push"];
    let (user, pass, topic) = (
        push["username"].as_str().unwrap(),
        push["password"].as_str().unwrap(),
        push["topic"].as_str().unwrap(),
    );
    assert_eq!(check_user(&base, user, pass).await, 200);
    assert_eq!(check_acl(&base, user, topic, 4).await, 200);
    assert_eq!(check_acl(&base, user, topic, 1).await, 200);
    let pool = SqlitePool::connect(&db).await.unwrap();
    sqlx::query("UPDATE devices SET enabled=0 WHERE device_id='device-1'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(check_user(&base, user, pass).await, 403);
    assert_eq!(check_acl(&base, user, topic, 4).await, 403);
    task.abort();
}

#[tokio::test]
async fn wrong_password_unknown_device_are_denied() {
    let (base, db, task) = start(push_config(), false).await;
    insert_device(&db, "device-1", true).await;
    assert_eq!(check_user(&base, "device-1", "nope").await, 403);
    let (_, body) = ota(&base, "device-1").await;
    let pass = body["push"]["password"].as_str().unwrap().to_owned();
    assert_eq!(check_user(&base, "stranger", &pass).await, 403);
    assert_eq!(check_user(&base, "device-1", "").await, 403);
    task.abort();
}

#[tokio::test]
async fn device_acl_is_read_only_on_own_topic() {
    let (base, db, task) = start(push_config(), false).await;
    insert_device(&db, "device-1", true).await;
    insert_device(&db, "device-2", true).await;
    let own = vela_server::services::device_push::DevicePush::signal_topic("device-1");
    let other = vela_server::services::device_push::DevicePush::signal_topic("device-2");
    assert_eq!(check_acl(&base, "device-1", &own, 4).await, 200);
    assert_eq!(check_acl(&base, "device-1", &own, 2).await, 403);
    assert_eq!(check_acl(&base, "device-1", &own, 3).await, 403);
    assert_eq!(check_acl(&base, "device-1", &other, 4).await, 403);
    assert_eq!(check_acl(&base, "device-1", "#", 4).await, 403);
    task.abort();
}

#[tokio::test]
async fn service_account_publishes_signal_topics_only() {
    let (base, _db, task) = start(push_config(), false).await;
    let topic = vela_server::services::device_push::DevicePush::signal_topic("device-9");
    assert_eq!(
        check_user(&base, "vela-server", &service_password()).await,
        200
    );
    assert_eq!(check_user(&base, "vela-server", "bad").await, 403);
    assert_eq!(check_acl(&base, "vela-server", &topic, 2).await, 200);
    assert_eq!(check_acl(&base, "vela-server", &topic, 4).await, 403);
    assert_eq!(
        check_acl(&base, "vela-server", "vela/v1/devices/x/other", 2).await,
        403
    );
    assert_eq!(check_acl(&base, "vela-server", "#", 2).await, 403);
    assert_eq!(
        post(
            &base,
            "superuser",
            serde_json::json!({"username": "vela-server"})
        )
        .await,
        403
    );
    assert_eq!(
        post(
            &base,
            "superuser",
            serde_json::json!({"username": "device-1"})
        )
        .await,
        403
    );
    task.abort();
}

#[tokio::test]
async fn endpoint_absent_when_feature_disabled() {
    let (base, _db, task) = start(Default::default(), false).await;
    assert_eq!(
        check_user(&base, "vela-server", &service_password()).await,
        404
    );
    task.abort();
}

#[tokio::test]
async fn database_error_denies() {
    let (base, db, task) = start(push_config(), false).await;
    insert_device(&db, "device-1", true).await;
    let (_, body) = ota(&base, "device-1").await;
    let pass = body["push"]["password"].as_str().unwrap().to_owned();
    let pool = SqlitePool::connect(&db).await.unwrap();
    sqlx::query("ALTER TABLE devices RENAME TO devices_gone")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(check_user(&base, "device-1", &pass).await, 403);
    task.abort();
}
