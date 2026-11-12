use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::StatusCode;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

use super::harness::*;

#[tokio::test]
async fn admin_direct_speech_applies_the_reported_template_boundary_before_admission() {
    let (base, url, task, _, mut boundary) = start_controlled_switch(&["sales"]).await;
    let admin = crate::admin_web::admin_web_token(&reqwest::Client::new(), &base).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;
    let mut socket = admit_with_direct(&base, true).await;
    let switching = boundary
        .run_turn_and_hold_at_its_boundary(&mut socket)
        .await;
    assert_eq!(switching.answer, "default-llm[TEMPLATE-PROMPT-V1]");
    boundary.hold.store(false, Ordering::SeqCst);
    boundary.release.notify_one();
    let response = reqwest::Client::new()
        .post(format!("{base}/api/admin/devices/device/speak"))
        .bearer_auth(&admin)
        .json(&serde_json::json!({
            "request_id": "boundary-direct",
            "text": "Direct text after switching."
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let mut audio = 0;
    loop {
        let frame = timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match frame {
            Message::Binary(_) => audio += 1,
            Message::Text(text) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_ne!(value["type"], "llm");
                assert_ne!(value["type"], "stt");
                if value["type"] == "tts" && value["state"] == "result" {
                    assert_eq!(value["request_id"], "boundary-direct");
                    assert_eq!(value["outcome"], "completed");
                    break;
                }
            }
            other => panic!("unexpected frame {other:?}"),
        }
    }
    assert!(audio > 0);
    task.abort();
}
