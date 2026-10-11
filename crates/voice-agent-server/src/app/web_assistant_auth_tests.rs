use super::*;

fn fixture_device() -> Device {
    Device {
        id: 1,
        device_id: "managed".into(),
        kind: "web_assistant".into(),
        agent_key: "studio".into(),
        template_key: None,
        name: None,
        description: None,
        enabled: 1,
        metadata_json: None,
        revision: 1,
        created_at: 1,
        updated_at: 1,
    }
}

#[tokio::test(start_paused = true)]
async fn tickets_expire_once_and_bind_the_device_revision_client_and_origin() {
    let auth = WebAssistantAuth::default();
    let mut device = fixture_device();
    let ticket = auth.issue(&device, "http://localhost").unwrap();
    tokio::time::advance(Duration::from_secs(30)).await;
    assert!(
        auth.consume(
            &ticket.ticket,
            &device,
            &ticket.client_id,
            "http://localhost"
        )
        .is_err()
    );
    for mismatch in [
        "client", "origin", "revision", "device", "disabled", "physical",
    ] {
        let ticket = auth.issue(&device, "http://localhost").unwrap();
        let client = if mismatch == "client" {
            "other"
        } else {
            &ticket.client_id
        };
        let origin = if mismatch == "origin" {
            "http://127.0.0.1"
        } else {
            "http://localhost"
        };
        let old_revision = device.revision;
        if mismatch == "revision" {
            device.revision += 1;
        }
        if mismatch == "device" {
            device.id += 1;
        }
        if mismatch == "disabled" {
            device.enabled = 0;
        }
        if mismatch == "physical" {
            device.kind = "physical".into();
        }
        assert!(
            auth.consume(&ticket.ticket, &device, client, origin)
                .is_err()
        );
        device = fixture_device();
        device.revision = old_revision;
        assert!(
            auth.consume(
                &ticket.ticket,
                &device,
                &ticket.client_id,
                "http://localhost"
            )
            .is_err()
        );
    }
}

#[tokio::test(start_paused = true)]
async fn ticket_rate_and_connection_capacity_are_bounded_and_revocable() {
    let auth = WebAssistantAuth::default();
    let device = fixture_device();
    let ticket = auth.issue(&device, "http://localhost").unwrap();
    let connection = auth
        .consume(
            &ticket.ticket,
            &device,
            &ticket.client_id,
            "http://localhost",
        )
        .unwrap();
    assert_eq!(auth.active(), 1);
    let pending = auth.issue(&device, "http://localhost").unwrap();
    auth.invalidate();
    assert!(connection.close.is_cancelled());
    assert!(
        auth.consume(
            &pending.ticket,
            &device,
            &pending.client_id,
            "http://localhost"
        )
        .is_err()
    );
    drop(connection);
    assert_eq!(auth.active(), 0);
    for _ in 2..20 {
        auth.issue(&device, "http://localhost").unwrap();
    }
    assert!(auth.issue(&device, "http://localhost").is_err());
    tokio::time::advance(Duration::from_secs(60)).await;
    assert!(auth.issue(&device, "http://localhost").is_ok());
}
