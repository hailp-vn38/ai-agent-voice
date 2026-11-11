use super::harness::*;

#[tokio::test]
async fn reminders_create_replay_and_restart_are_saved_not_played() {
    let uri = database_url();
    let (base, task) = server_with_database(true, &uri).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    client
        .post(format!("{base}/api/admin/agents"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"key":"reminder_agent","name":"Reminders"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    client
        .post(format!("{base}/api/admin/devices"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"device_id":"clock-device","agent_key":"reminder_agent"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let url = format!("{base}/api/admin/devices/clock-device/reminders");
    assert_eq!(
        client.get(&url).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let payload = serde_json::json!({"message":"Kiểm tra máy giặt", "delay_seconds":600});
    let created = client
        .post(&url)
        .bearer_auth(&web_token)
        .header("Idempotency-Key", "create-one")
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let saved: serde_json::Value = created.json().await.unwrap();
    assert_eq!(saved["status"], "pending");
    assert_eq!(saved["sync_status"], "pending");
    assert_eq!(saved["row_revision"], 1);
    assert_eq!(saved["schedule_revision"], 1);
    let replay: serde_json::Value = client
        .post(&url)
        .bearer_auth(&web_token)
        .header("Idempotency-Key", "create-one")
        .json(&payload)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(replay, saved);
    task.abort();
    let (base, task) = server_with_database(true, &uri).await;
    let url = format!("{base}/api/admin/devices/clock-device/reminders");
    let replay: serde_json::Value = client
        .post(&url)
        .bearer_auth(&web_token)
        .header("Idempotency-Key", "create-one")
        .json(&payload)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(replay, saved);
    let list: serde_json::Value = client
        .get(&url)
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    assert_eq!(list["schedule_summary"]["schedule_revision"], 1);
    assert_eq!(list["schedule_summary"]["last_acked_revision"], 0);
    let conflict = client
        .post(&url)
        .bearer_auth(&web_token)
        .header("Idempotency-Key", "create-one")
        .json(&serde_json::json!({"message":"changed", "delay_seconds":600}))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    task.abort();
}

#[tokio::test]
async fn reminder_server_clock_validation_quota_and_agent_permission() {
    use vela_server::database::notifications::{CreateReminder, DeviceNotifications};
    let uri = database_url();
    let (base, task) = server_with_database(true, &uri).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    client
        .post(format!("{base}/api/admin/agents"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"key":"timing_agent","name":"Timing"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    client
        .post(format!("{base}/api/admin/devices"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"device_id":"timing-device","agent_key":"timing_agent"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let policy_url = format!("{base}/api/admin/agents/timing_agent/reminder-policy");
    let policy: serde_json::Value = client
        .get(&policy_url)
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(policy["reminders_enabled"], false);
    assert_eq!(
        client
            .patch(&policy_url)
            .bearer_auth(&web_token)
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"reminders_enabled":true}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .patch(&policy_url)
            .bearer_auth(&web_token)
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"reminders_enabled":false}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    let config = vela_server::config::DatabaseConfig {
        url: uri,
        ..Default::default()
    };
    let database = Database::connect(&config).await.unwrap();
    let notifications = DeviceNotifications::with_clock(database, Arc::new(|| 1_800_000_000_000));
    let input = CreateReminder {
        message: "Wake".into(),
        delay_seconds: Some(600),
        scheduled_at: None,
        timezone: None,
    };
    let audit_first = uuid::Uuid::new_v4().to_string();
    let (reminder, _) = notifications
        .create_with_context(
            "timing-device",
            &vela_server::database::notifications::MutationContext {
                caller: "clock-test",
                request_id: "first",
                audit_request_id: &audit_first,
                source: vela_server::database::notifications::ReminderSource::Web,
                agent_scope: None,
            },
            input.clone(),
        )
        .await
        .unwrap();
    assert_eq!(reminder.due_at_ms, 1_800_000_600_000);
    let mut invalid = input.clone();
    invalid.scheduled_at = Some("2026-10-12T07:00:00+07:00".into());
    let audit_bad = uuid::Uuid::new_v4().to_string();
    assert!(
        notifications
            .create_with_context(
                "timing-device",
                &vela_server::database::notifications::MutationContext {
                    caller: "clock-test",
                    request_id: "bad",
                    audit_request_id: &audit_bad,
                    source: vela_server::database::notifications::ReminderSource::Web,
                    agent_scope: None,
                },
                invalid,
            )
            .await
            .is_err()
    );
    let url = format!("{base}/api/admin/devices/timing-device/reminders");
    for body in [
        serde_json::json!({"message":"\u{0007}","delay_seconds":1}),
        serde_json::json!({"message":"wake","delay_seconds":0}),
        serde_json::json!({"message":"wake","scheduled_at":"2026-10-12T07:00:00"}),
    ] {
        assert_eq!(
            client
                .post(&url)
                .bearer_auth(&web_token)
                .header("idempotency-key", uuid::Uuid::new_v4().to_string())
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    for i in 1..500 {
        let req_id = format!("quota-{i}");
        let audit_id = uuid::Uuid::new_v4().to_string();
        notifications
            .create_with_context(
                "timing-device",
                &vela_server::database::notifications::MutationContext {
                    caller: "clock-test",
                    request_id: &req_id,
                    audit_request_id: &audit_id,
                    source: vela_server::database::notifications::ReminderSource::Web,
                    agent_scope: None,
                },
                input.clone(),
            )
            .await
            .unwrap();
    }
    let audit_overflow = uuid::Uuid::new_v4().to_string();
    assert!(
        notifications
            .create_with_context(
                "timing-device",
                &vela_server::database::notifications::MutationContext {
                    caller: "clock-test",
                    request_id: "overflow",
                    audit_request_id: &audit_overflow,
                    source: vela_server::database::notifications::ReminderSource::Web,
                    agent_scope: None,
                },
                input,
            )
            .await
            .is_err()
    );
    let list: serde_json::Value = client
        .get(format!("{url}?page=10&page_size=50"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["items"].as_array().unwrap().len(), 50);
    assert_eq!(list["schedule_summary"]["schedule_revision"], 500);
    assert_eq!(list["total"], 500);
    assert_eq!(list["has_more"], false);
    task.abort();
}

#[tokio::test]
async fn admin_device_memory_mode_is_set_validated_and_returned() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    let token = &web_token;
    let devices = format!("{base}/api/admin/devices");
    client
        .post(format!("{base}/api/admin/agents"))
        .bearer_auth(token)
        .json(&serde_json::json!({"key":"home","name":"Home"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    let default_off: serde_json::Value = client
        .post(&devices)
        .bearer_auth(token)
        .json(&serde_json::json!({"device_id":"plain","agent_key":"home"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(default_off["memory_mode"], "off");

    let created: serde_json::Value = client
        .post(&devices)
        .bearer_auth(token)
        .json(&serde_json::json!({"device_id":"reader","agent_key":"home","memory_mode":"read"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(created["memory_mode"], "read");

    let rejected = client
        .post(&devices)
        .bearer_auth(token)
        .json(&serde_json::json!({"device_id":"bad","agent_key":"home","memory_mode":"admin"}))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

    let reader = format!("{devices}/reader");
    for bad in [serde_json::json!("admin"), serde_json::Value::Null] {
        let rejected = client
            .patch(&reader)
            .bearer_auth(token)
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({ "memory_mode": bad }))
            .send()
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    }

    let unrelated_patch: serde_json::Value = client
        .patch(&reader)
        .bearer_auth(token)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"description":"kept"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(unrelated_patch["memory_mode"], "read");

    let patched: serde_json::Value = client
        .patch(&reader)
        .bearer_auth(token)
        .header("if-match", "\"2\"")
        .json(&serde_json::json!({"memory_mode":"read_write"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(patched["memory_mode"], "read_write");
    assert_eq!(patched["revision"], 3);

    let stale = client
        .patch(&reader)
        .bearer_auth(token)
        .header("if-match", "\"2\"")
        .json(&serde_json::json!({"memory_mode":"off"}))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);

    let fetched: serde_json::Value = client
        .get(&reader)
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(fetched["memory_mode"], "read_write");

    let listed: serde_json::Value = client
        .get(&devices)
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["items"][0]["device_id"], "plain");
    assert_eq!(listed["items"][0]["memory_mode"], "off");
    assert_eq!(listed["items"][1]["memory_mode"], "read_write");

    let assistant = format!("{base}/api/admin/system/assistant-device");
    let saved: serde_json::Value = client
        .put(&assistant)
        .bearer_auth(token)
        .json(&serde_json::json!({"agent_key":"home","enabled":true,"memory_mode":"read"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(saved["memory_mode"], "read");
    let kept: serde_json::Value = client
        .put(&assistant)
        .bearer_auth(token)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"agent_key":"home","enabled":false}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(kept["memory_mode"], "read");
    let rejected = client
        .put(&assistant)
        .bearer_auth(token)
        .header("if-match", "\"2\"")
        .json(&serde_json::json!({"agent_key":"home","enabled":true,"memory_mode":"admin"}))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

    task.abort();
}

#[tokio::test]
async fn reminder_concurrent_retry_and_reassignment_preserve_device_ownership() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    for key in ["original_agent", "next_agent"] {
        client
            .post(format!("{base}/api/admin/agents"))
            .bearer_auth(&web_token)
            .json(&serde_json::json!({"key":key,"name":key}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }
    client
        .post(format!("{base}/api/admin/devices"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"device_id":"owned-device","agent_key":"original_agent"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let virtual_device: serde_json::Value = client
        .put(format!("{base}/api/admin/system/assistant-device"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"agent_key":"original_agent","enabled":true}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        client
            .post(format!(
                "{base}/api/admin/devices/{}/reminders",
                virtual_device["device_id"].as_str().unwrap()
            ))
            .bearer_auth(&web_token)
            .header("idempotency-key", "virtual")
            .json(&serde_json::json!({"message":"no wake","delay_seconds":600}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let url = format!("{base}/api/admin/devices/owned-device/reminders");
    let payload = serde_json::json!({"message":"Persistent owner","delay_seconds":600});
    let (left, right) = tokio::join!(
        client
            .post(&url)
            .bearer_auth(&web_token)
            .header("idempotency-key", "same-concurrent")
            .json(&payload)
            .send(),
        client
            .post(&url)
            .bearer_auth(&web_token)
            .header("idempotency-key", "same-concurrent")
            .json(&payload)
            .send()
    );
    let left: serde_json::Value = left
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let right: serde_json::Value = right
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(left, right);
    client
        .patch(format!("{base}/api/admin/devices/owned-device"))
        .bearer_auth(&web_token)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"agent_key":"next_agent"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let list: serde_json::Value = client
        .get(&url)
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    assert_eq!(list["items"][0]["device_id"], "owned-device");
    assert_eq!(list["schedule_summary"]["schedule_revision"], 1);
    assert_eq!(
        client
            .get(format!("{url}?page_size=201"))
            .bearer_auth(&web_token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        client
            .post(format!("{base}/api/admin/devices/not-a-device/reminders"))
            .bearer_auth(&web_token)
            .header("idempotency-key", "missing")
            .json(&payload)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    task.abort();
}

#[tokio::test]
async fn reminder_edit_and_cancel_are_scoped_revisioned_and_replayed_after_restart() {
    let uri = database_url();
    let (base, task) = server_with_database(true, &uri).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    client
        .post(format!("{base}/api/admin/agents"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"key":"edit_agent","name":"Edit"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    for device in ["edit-device", "other-device"] {
        client
            .post(format!("{base}/api/admin/devices"))
            .bearer_auth(&web_token)
            .json(&serde_json::json!({"device_id":device,"agent_key":"edit_agent"}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }
    let list_url = format!("{base}/api/admin/devices/edit-device/reminders");
    let saved: serde_json::Value = client
        .post(&list_url)
        .bearer_auth(&web_token)
        .header("idempotency-key", "new-edit")
        .json(&serde_json::json!({"message":"Before","delay_seconds":600}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let reminder = saved["id"].as_str().unwrap();
    let url = format!("{list_url}/{reminder}");
    assert_eq!(
        client
            .patch(&url)
            .json(&serde_json::json!({"message":"No auth"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let update = serde_json::json!({"message":"After","scheduled_at":"2030-01-02T07:00:00+07:00"});
    let changed: serde_json::Value = client
        .patch(&url)
        .bearer_auth(&web_token)
        .header("if-match", "\"1\"")
        .header("idempotency-key", "edit-once")
        .json(&update)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(changed["message"], "After");
    assert_eq!(changed["row_revision"], 2);
    assert_eq!(changed["schedule_revision"], 2);
    assert_eq!(
        client
            .patch(&url)
            .bearer_auth(&web_token)
            .header("if-match", "\"1\"")
            .header("idempotency-key", "stale-writer")
            .json(&update)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        client
            .patch(format!(
                "{base}/api/admin/devices/other-device/reminders/{reminder}"
            ))
            .bearer_auth(&web_token)
            .header("if-match", "\"2\"")
            .header("idempotency-key", "wrong-owner")
            .json(&update)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let cancelled: serde_json::Value = client
        .post(format!("{url}/cancel"))
        .bearer_auth(&web_token)
        .header("if-match", "\"2\"")
        .header("idempotency-key", "cancel-once")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cancelled["status"], "cancelled");
    assert_eq!(cancelled["row_revision"], 3);
    assert_eq!(cancelled["schedule_revision"], 3);
    task.abort();
    let (base, task) = server_with_database(true, &uri).await;
    let url = format!("{base}/api/admin/devices/edit-device/reminders/{reminder}");
    let replay: serde_json::Value = client
        .post(format!("{url}/cancel"))
        .bearer_auth(&web_token)
        .header("if-match", "\"2\"")
        .header("idempotency-key", "cancel-once")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(replay, cancelled);
    let edit_replay: serde_json::Value = client
        .patch(&url)
        .bearer_auth(&web_token)
        .header("if-match", "\"1\"")
        .header("idempotency-key", "edit-once")
        .json(&update)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(edit_replay, changed);
    assert_eq!(
        client
            .patch(&url)
            .bearer_auth(&web_token)
            .header("if-match", "\"3\"")
            .header("idempotency-key", "terminal-edit")
            .json(&update)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    let list: serde_json::Value = client
        .get(format!(
            "{base}/api/admin/devices/edit-device/reminders?status=cancelled"
        ))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    assert_eq!(list["schedule_summary"]["schedule_revision"], 3);
    task.abort();
}

#[tokio::test]
async fn reminder_concurrent_edits_conflict_without_lost_update_and_delete_cascades() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    for agent in ["owner_before", "owner_after"] {
        client
            .post(format!("{base}/api/admin/agents"))
            .bearer_auth(&web_token)
            .json(&serde_json::json!({"key":agent,"name":agent}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }
    client
        .post(format!("{base}/api/admin/devices"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"device_id":"race-edit","agent_key":"owner_before"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let list_url = format!("{base}/api/admin/devices/race-edit/reminders");
    let saved: serde_json::Value = client
        .post(&list_url)
        .bearer_auth(&web_token)
        .header("idempotency-key", "race-new")
        .json(&serde_json::json!({"message":"Before","delay_seconds":600}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = saved["id"].as_str().unwrap();
    let url = format!("{list_url}/{id}");
    let (left, right) = tokio::join!(
        client
            .patch(&url)
            .bearer_auth(&web_token)
            .header("if-match", "\"1\"")
            .header("idempotency-key", "writer-a")
            .json(&serde_json::json!({"message":"A"}))
            .send(),
        client
            .patch(&url)
            .bearer_auth(&web_token)
            .header("if-match", "\"1\"")
            .header("idempotency-key", "writer-b")
            .json(&serde_json::json!({"message":"B"}))
            .send()
    );
    let (a, b) = (left.unwrap(), right.unwrap());
    assert!(
        (a.status() == StatusCode::OK && b.status() == StatusCode::CONFLICT)
            || (b.status() == StatusCode::OK && a.status() == StatusCode::CONFLICT)
    );
    let (winner, key) = if a.status() == StatusCode::OK {
        (a.json::<serde_json::Value>().await.unwrap(), "writer-a")
    } else {
        (b.json::<serde_json::Value>().await.unwrap(), "writer-b")
    };
    assert_eq!(winner["due_at_ms"], saved["due_at_ms"]);
    assert_eq!(
        client
            .patch(&url)
            .bearer_auth(&web_token)
            .header("if-match", "\"1\"")
            .header("idempotency-key", key)
            .json(&serde_json::json!({"message":"changed payload"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        client
            .patch(&url)
            .bearer_auth(&web_token)
            .header("if-match", "\"2\"")
            .header("idempotency-key", "bad-time")
            .json(&serde_json::json!({"delay_seconds":600,"scheduled_at":"2030-01-02T00:00:00Z"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    client
        .patch(format!("{base}/api/admin/devices/race-edit"))
        .bearer_auth(&web_token)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"agent_key":"owner_after"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let list: serde_json::Value = client
        .get(&list_url)
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["items"][0]["id"], id);
    assert_eq!(list["schedule_summary"]["schedule_revision"], 2);
    client
        .delete(format!("{base}/api/admin/devices/race-edit"))
        .bearer_auth(&web_token)
        .header("if-match", "\"2\"")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    client
        .post(format!("{base}/api/admin/devices"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"device_id":"race-edit","agent_key":"owner_after"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let list: serde_json::Value = client
        .get(&list_url)
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["items"].as_array().unwrap().len(), 0);
    assert_eq!(list["schedule_summary"]["schedule_revision"], 0);
    assert_eq!(
        client
            .post(format!("{url}/cancel"))
            .bearer_auth(&web_token)
            .header("if-match", "\"2\"")
            .header("idempotency-key", "deleted-reminder")
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    task.abort();
}

#[tokio::test]
async fn agent_reminders_join_current_assignment_and_scope_mutations() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    for key in ["aggregate_a", "aggregate_b"] {
        client
            .post(format!("{base}/api/admin/agents"))
            .bearer_auth(&web_token)
            .json(&serde_json::json!({"key":key,"name":key}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }
    for (device, key) in [
        ("agg-one", "aggregate_a"),
        ("agg-two", "aggregate_a"),
        ("agg-other", "aggregate_b"),
    ] {
        client
            .post(format!("{base}/api/admin/devices"))
            .bearer_auth(&web_token)
            .json(&serde_json::json!({"device_id":device,"agent_key":key}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }
    client
        .put(format!("{base}/api/admin/system/assistant-device"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"agent_key":"aggregate_a","enabled":true}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let url = format!("{base}/api/admin/agents/aggregate_a/reminders");
    assert_eq!(
        client.get(&url).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let mut ids = Vec::new();
    for (i, device) in ["agg-one", "agg-two", "agg-other"].into_iter().enumerate() {
        let v: serde_json::Value = client
            .post(format!("{base}/api/admin/devices/{device}/reminders"))
            .bearer_auth(&web_token)
            .header("idempotency-key", format!("agg-{i}"))
            .json(&serde_json::json!({"message":device,"scheduled_at":"2030-01-02T00:00:00Z"}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        ids.push(v["id"].as_str().unwrap().to_string());
    }
    let page: serde_json::Value = client
        .get(format!("{url}?page_size=1"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["total"], 2);
    assert_eq!(page["has_more"], true);
    assert_eq!(page["kpi"]["pending"], 2);
    assert_eq!(page["devices"].as_array().unwrap().len(), 2);
    let filtered: serde_json::Value = client
        .get(format!(
            "{url}?device_id=agg-two&due_from_ms=1893542400000&due_to_ms=1893542400000"
        ))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(filtered["items"][0]["device_id"], "agg-two");
    assert_eq!(filtered["total"], 1);
    let second: serde_json::Value = client
        .get(format!("{url}?page=2&page_size=1"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_ne!(second["items"][0]["id"], page["items"][0]["id"]);
    assert_eq!(second["has_more"], false);
    client
        .post(format!(
            "{base}/api/admin/agents/aggregate_a/devices/agg-two/reminders/{}/cancel",
            ids[1]
        ))
        .bearer_auth(&web_token)
        .header("if-match", "\"1\"")
        .header("idempotency-key", "agg-cancel")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let cancelled: serde_json::Value = client
        .get(format!("{url}?status=cancelled"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cancelled["total"], 1);
    assert_eq!(cancelled["kpi"]["pending"], 1);
    assert_eq!(cancelled["kpi"]["cancelled"], 1);
    assert_eq!(
        client
            .get(format!("{url}?due_from_ms=2&due_to_ms=1"))
            .bearer_auth(&web_token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    let scoped = format!(
        "{base}/api/admin/agents/aggregate_a/devices/agg-one/reminders/{}",
        ids[0]
    );
    client
        .patch(format!("{base}/api/admin/devices/agg-one"))
        .bearer_auth(&web_token)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"agent_key":"aggregate_b"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert_eq!(
        client
            .patch(&scoped)
            .bearer_auth(&web_token)
            .header("if-match", "\"1\"")
            .header("idempotency-key", "stale-agent-edit")
            .json(&serde_json::json!({"message":"wrong"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .post(format!(
                "{base}/api/admin/agents/aggregate_a/devices/agg-other/reminders"
            ))
            .bearer_auth(&web_token)
            .header("idempotency-key", "wrong-agent-create")
            .json(&serde_json::json!({"message":"wrong","delay_seconds":600}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let moved: serde_json::Value = client
        .get(format!("{base}/api/admin/agents/aggregate_b/reminders"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(moved["total"], 2);
    assert_eq!(moved["kpi"]["pending"], 2);
    assert_eq!(
        client
            .get(format!("{url}?device_id=agg-one"))
            .bearer_auth(&web_token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .put(format!(
                "{base}/api/admin/system/assistant-device/reminder-target"
            ))
            .bearer_auth(&web_token)
            .json(&serde_json::json!({"session_id":"non-existent-session","device_id":"agg-two"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    task.abort();
}
