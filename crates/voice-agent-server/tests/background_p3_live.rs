//! Explicit live qualification against an operator-provisioned rootless deployment.
//! Credentials stay in environment variables and are never written to the report.
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

#[path = "background_p3_live/receipt.rs"]
mod receipt;

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing {name}"))
}
async fn checked(request: reqwest::RequestBuilder) -> Value {
    let response = request.send().await.expect("Admin HTTP request");
    assert!(
        response.status().is_success(),
        "Admin HTTP status {}",
        response.status()
    );
    response.json().await.expect("Admin JSON response")
}

#[tokio::test]
#[ignore = "requires a running Linux rootless deployment, real Voice/Orchestrator/Worker LLMs and enrolled Device"]
async fn live_voice_to_python_worker_and_admin_artifact() {
    let base = required("AGENT_P3_BASE_URL")
        .trim_end_matches('/')
        .to_owned();
    let token = required("AGENT_P3_ADMIN_TOKEN");
    let ws_url = required("AGENT_P3_WS_URL");
    let device = required("AGENT_P3_DEVICE_ID");
    let studio = std::env::var("AGENT_P3_STUDIO").is_ok_and(|value| value == "true");
    let mcp = std::env::var("AGENT_P3_MCP_FEATURE").is_ok_and(|value| value == "true");
    let client = Client::builder()
        .timeout(Duration::from_secs(150))
        .build()
        .unwrap();
    let zip = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/fixture-research-1.0.0.zip"
    ));
    let package_digest: String = Sha256::digest(zip)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let package = reqwest::multipart::Part::bytes(zip.to_vec()).file_name("fixture-research.zip");
    let installed = checked(
        client
            .post(format!("{base}/api/admin/plugins/install"))
            .bearer_auth(&token)
            .multipart(reqwest::multipart::Form::new().part("package", package)),
    )
    .await;
    assert_eq!(installed["digest"], package_digest);
    let active = checked(
        client
            .post(format!(
                "{base}/api/admin/plugins/fixture-research/activate"
            ))
            .bearer_auth(&token)
            .header(
                "If-Match",
                format!("\"{}\"", installed["revision"].as_i64().unwrap()),
            )
            .json(&json!({})),
    )
    .await;
    assert_eq!(active["status"], "ready");
    let settings = checked(
        client
            .get(format!("{base}/api/admin/background/settings"))
            .bearer_auth(&token),
    )
    .await;
    let enabled=checked(client.put(format!("{base}/api/admin/background/settings")).bearer_auth(&token).header("If-Match",format!("\"{}\"",settings["revision"].as_i64().unwrap())).json(&json!({"enabled":true,"orchestrator_llm_provider":required("AGENT_P3_ORCHESTRATOR_PROVIDER"),"worker_llm_provider":required("AGENT_P3_WORKER_PROVIDER")}))).await;
    assert_eq!(enabled["runtime_ready"], true);
    let run_id = uuid::Uuid::new_v4().to_string();
    let issue_ticket = || async {
        checked(
            client
                .post(format!("{base}/api/admin/assistant/ws-ticket"))
                .bearer_auth(&token)
                .header("Origin", required("AGENT_P3_WS_ORIGIN")),
        )
        .await
    };
    let first_ticket = if studio {
        Some(issue_ticket().await)
    } else {
        None
    };
    let mut initial_url = reqwest::Url::parse(&ws_url).expect("WS URL");
    if let Some(ticket) = &first_ticket {
        initial_url
            .query_pairs_mut()
            .append_pair("ticket", ticket["ticket"].as_str().unwrap());
    }
    let mut request = initial_url.as_str().into_client_request().expect("WS URL");
    request.headers_mut().insert(
        "Device-Id",
        first_ticket
            .as_ref()
            .map_or(device.as_str(), |ticket| {
                ticket["device_id"].as_str().unwrap()
            })
            .parse()
            .unwrap(),
    );
    request.headers_mut().insert(
        "Client-Id",
        first_ticket
            .as_ref()
            .map(|ticket| ticket["client_id"].as_str().unwrap().to_owned())
            .unwrap_or_else(|| {
                std::env::var("AGENT_P3_CLIENT_ID").unwrap_or_else(|_| "p3-qualification".into())
            })
            .parse()
            .unwrap(),
    );
    request
        .headers_mut()
        .insert("Protocol-Version", "1".parse().unwrap());
    if let Ok(origin) = std::env::var("AGENT_P3_WS_ORIGIN") {
        request
            .headers_mut()
            .insert("Origin", origin.parse().unwrap());
    }
    if let Ok(auth) = std::env::var("AGENT_P3_WS_TOKEN") {
        request
            .headers_mut()
            .insert("Authorization", format!("Bearer {auth}").parse().unwrap());
    }
    let jobs_url = format!("{base}/api/admin/jobs");
    let initial = checked(client.get(&jobs_url).bearer_auth(&token)).await;
    let (mut socket, _) = connect_async(request.clone())
        .await
        .expect("managed Device WS admission");
    let hello=json!({"type":"hello","version":1,"transport":"websocket","features":{"mcp":mcp,"direct_tts":true,"tool_status":true},"audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}}).to_string();
    socket
        .send(Message::Text(hello.clone().into()))
        .await
        .unwrap();
    let server_hello = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(server_hello,Message::Text(ref text) if serde_json::from_str::<Value>(text).is_ok_and(|value|value["type"]=="hello"))
    );
    let Message::Text(ref hello_text) = server_hello else {
        unreachable!()
    };
    let hello_frame: Value = serde_json::from_str(hello_text).unwrap();
    let mut order = receipt::ReceiptOrder::new(hello_frame["session_id"].as_str().unwrap());
    let after_hello = checked(client.get(&jobs_url).bearer_auth(&token)).await;
    assert_eq!(
        after_hello["total"], initial["total"],
        "hello must not create a Job on this isolated deployment"
    );
    let before = checked(
        client
            .get(format!("{jobs_url}?q={run_id}"))
            .bearer_auth(&token),
    )
    .await;
    assert_eq!(before["total"], 0);
    socket
        .send(Message::Text(
            json!({"type":"listen","state":"start","mode":"manual"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let goal = format!(
        "Create one background Job to research topic AI using the offline fixture source and research.collect capability. Include required_sources=['fixture'] and required_capabilities=['research.collect']. Include qualification marker {run_id} in the goal. After the server_create_job result says created=true, acknowledge in English that the Job was queued; do not claim it completed. The background plan should call fixture-research.collect_topic with topic AI and then a Worker LLM summary step."
    );
    socket
        .send(Message::Text(
            json!({"type":"listen","state":"detect","text":goal})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let mut committed_id = None;
    tokio::time::timeout(Duration::from_secs(90), async {
        while let Some(message) = socket.next().await {
            if let Message::Text(text) = message.unwrap() {
                let frame: Value = serde_json::from_str(&text).unwrap();
                if order
                    .observe(&frame)
                    .expect("receipt must precede acknowledgement")
                {
                    let committed = checked(
                        client
                            .get(format!("{jobs_url}?q={run_id}"))
                            .bearer_auth(&token),
                    )
                    .await;
                    assert_eq!(
                        committed["total"], 1,
                        "completed ToolResult must refer to a committed Job"
                    );
                    committed_id = Some(committed["items"][0]["id"].as_str().unwrap().to_owned());
                }
                if order.acknowledged() {
                    break;
                }
            }
        }
    })
    .await
    .expect("real LLM queued acknowledgement");
    assert!(order.acknowledged());
    let page = checked(
        client
            .get(format!("{jobs_url}?q={run_id}"))
            .bearer_auth(&token),
    )
    .await;
    assert_eq!(page["total"], 1, "one durable Voice Job");
    let id = page["items"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(committed_id.as_deref(), Some(id.as_str()));
    assert!(
        page["items"][0]["requester_scope"]
            .as_str()
            .unwrap()
            .starts_with("voice:")
    );
    socket.close(None).await.unwrap();
    let reconnect_request = if studio {
        let ticket = issue_ticket().await;
        let mut url = reqwest::Url::parse(&ws_url).unwrap();
        url.query_pairs_mut()
            .append_pair("ticket", ticket["ticket"].as_str().unwrap());
        let mut fresh = url.as_str().into_client_request().unwrap();
        *fresh.headers_mut() = request.headers().clone();
        fresh.headers_mut().insert(
            "Client-Id",
            ticket["client_id"].as_str().unwrap().parse().unwrap(),
        );
        fresh
    } else if let Ok(url) = std::env::var("AGENT_P3_RECONNECT_WS_URL") {
        let mut fresh = url.into_client_request().unwrap();
        for name in [
            "Device-Id",
            "Client-Id",
            "Protocol-Version",
            "Authorization",
            "Origin",
        ] {
            if let Some(value) = request.headers().get(name) {
                fresh.headers_mut().insert(name, value.clone());
            }
        }
        if let Ok(client) = std::env::var("AGENT_P3_RECONNECT_CLIENT_ID") {
            fresh
                .headers_mut()
                .insert("Client-Id", client.parse().unwrap());
        }
        fresh
    } else {
        request
    };
    let (mut reconnect, _) = connect_async(reconnect_request).await.unwrap();
    reconnect.send(Message::Text(hello.into())).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), reconnect.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    reconnect.close(None).await.unwrap();
    let job = tokio::time::timeout(Duration::from_secs(600), async {
        loop {
            let job = checked(client.get(format!("{jobs_url}/{id}")).bearer_auth(&token)).await;
            if matches!(
                job["status"].as_str(),
                Some("succeeded" | "partial_success" | "failed" | "cancelled")
            ) {
                break job;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
    .await
    .expect("finite terminal Job");
    assert_eq!(
        job["status"], "succeeded",
        "terminal failure code: {}",
        job["error_code"]
    );
    assert_eq!(job["required_sources"], json!(["fixture"]));
    assert_eq!(job["required_capabilities"], json!(["research.collect"]));
    let steps = job["steps"].as_array().unwrap();
    assert!(
        steps
            .iter()
            .any(|step| step["plugin_id"] == "fixture-research" && step["status"] == "succeeded")
    );
    assert!(
        steps
            .iter()
            .any(|step| step["plugin_id"].is_null() && step["status"] == "succeeded")
    );
    let artifacts = job["artifacts"].as_array().unwrap();
    assert!(!artifacts.is_empty());
    for artifact in artifacts {
        let response = client
            .get(format!(
                "{jobs_url}/{id}/artifacts/{}",
                artifact["id"].as_str().unwrap()
            ))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        let bytes = response.bytes().await.unwrap();
        let sha: String = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(sha, artifact["sha256"]);
        assert_eq!(bytes.len() as u64, artifact["size_bytes"].as_u64().unwrap());
    }
    let after = checked(
        client
            .get(format!("{jobs_url}?q={run_id}"))
            .bearer_auth(&token),
    )
    .await;
    assert_eq!(after["total"], 1, "reconnect must not replay the Job");
    if let Ok(path) = std::env::var("AGENT_P3_REPORT") {
        let report = json!({"schema_version":1,"result":"PASS","run_id":run_id,"job_id":id,"mcp_feature":mcp,"studio":studio,"package_digest":package_digest,"image_digest":active["image_digest"],"snapshot":job["snapshot"],"artifact_count":artifacts.len(),"receipt_order_gate":"PASS","browser_gate":"NOT_RUN"});
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .expect("new report path");
        file.write_all(report.to_string().as_bytes()).unwrap();
    }
}
