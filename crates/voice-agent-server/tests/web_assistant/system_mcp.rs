use super::*;
use std::sync::Mutex;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use voice_agent_server::providers::llm::{
    ChatMessage, LlmEvent, LlmEventStream, LlmRequest, ToolCall,
};

#[derive(Default)]
struct ManagementLlm(Mutex<Vec<LlmRequest>>);
#[async_trait::async_trait]
impl LlmProvider for ManagementLlm {
    fn adapter(&self) -> &'static str {
        "test-system-mcp"
    }
    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        let continuation = request
            .messages
            .iter()
            .any(|message| matches!(message, ChatMessage::ToolResult { .. }));
        let management = request
            .tools
            .iter()
            .any(|tool| tool.name == "system_get_status");
        self.0.lock().unwrap().push(request);
        let events = if continuation || !management {
            vec![
                LlmEvent::TextDelta("Đã kiểm tra hệ thống.".into()),
                LlmEvent::Finished,
            ]
        } else {
            [
                ("system_get_status", serde_json::json!({})),
                ("system_get_resources", serde_json::json!({})),
                ("mcp_list_servers", serde_json::json!({})),
                (
                    "mcp_get_server",
                    serde_json::json!({"server_key":"weather"}),
                ),
                ("mcp_list_tools", serde_json::json!({"agent_key":"studio"})),
                (
                    "mcp_get_agent_bindings",
                    serde_json::json!({"agent_key":"studio"}),
                ),
                (
                    "mcp_get_server",
                    serde_json::json!({"server_key":"missing"}),
                ),
            ]
            .into_iter()
            .enumerate()
            .map(|(index, (name, arguments))| {
                LlmEvent::ToolCall(ToolCall {
                    id: format!("call-{index}"),
                    name: name.into(),
                    arguments,
                })
            })
            .chain(std::iter::once(LlmEvent::Finished))
            .collect()
        };
        Ok(Box::pin(futures_util::stream::iter(
            events.into_iter().map(Ok),
        )))
    }
}

async fn connect_assistant(base: &str) -> Socket {
    let ticket: serde_json::Value = Client::new()
        .post(format!("{base}/api/admin/assistant/ws-ticket"))
        .bearer_auth("admin-test-token")
        .header("Origin", "http://localhost")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let mut request = format!(
        "{}/voice/v1/?device-id={}&client-id={}&ticket={}",
        base.replacen("http", "ws", 1),
        ticket["device_id"].as_str().unwrap(),
        ticket["client_id"].as_str().unwrap(),
        ticket["ticket"].as_str().unwrap()
    )
    .into_client_request()
    .unwrap();
    request
        .headers_mut()
        .insert("Origin", "http://localhost".parse().unwrap());
    tokio_tungstenite::connect_async(request).await.unwrap().0
}

async fn run_turn(socket: &mut Socket) -> Vec<serde_json::Value> {
    send(
        socket,
        serde_json::json!({"type":"hello","features":{"mcp":false,"tool_status":true}}),
    )
    .await;
    assert_eq!(next_json(socket).await["type"], "hello");
    send(
        socket,
        serde_json::json!({"type":"listen","state":"start","mode":"manual"}),
    )
    .await;
    send(
        socket,
        serde_json::json!({"type":"listen","state":"detect","text":"Kiểm tra hệ thống và MCP."}),
    )
    .await;
    let mut frames = Vec::new();
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let tokio_tungstenite::tungstenite::Message::Text(text) = message else {
            continue;
        };
        let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
        let done = frame["type"] == "tts" && frame["state"] == "stop";
        frames.push(frame);
        if done {
            return frames;
        }
    }
}

#[tokio::test]
async fn verified_assistant_reads_six_tools_with_status_results_and_tts() {
    let llm = Arc::new(ManagementLlm::default());
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(VoiceVad),
        Arc::new(VoiceAsr),
        llm.clone(),
        Arc::new(VoiceTts),
    ));
    let (base, task) = start_with_system_mcp(providers, "physical-token", true).await;
    let client = Client::new();
    for (path, body) in [
        (
            "agents",
            serde_json::json!({"key":"studio","name":"Studio"}),
        ),
        (
            "mcp-servers",
            serde_json::json!({"key":"weather","name":"Weather","url":"http://internal.example/private","auth":{"type":"none"}}),
        ),
    ] {
        client
            .post(format!("{base}/api/admin/{path}"))
            .bearer_auth("admin-test-token")
            .json(&body)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }
    client
        .put(format!(
            "{base}/api/admin/agents/studio/mcp-bindings/weather"
        ))
        .bearer_auth("admin-test-token")
        .header("If-Match", "\"1\"")
        .json(&serde_json::json!({"enabled":false}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    client
        .put(format!("{base}/api/admin/system/assistant-device"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"agent_key":"studio","enabled":true}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert_eq!(
        client
            .get(format!("{base}/mcp/system"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let frames = run_turn(&mut connect_assistant(&base).await).await;
    let status: Vec<_> = frames
        .iter()
        .filter(|frame| frame["type"] == "tool")
        .collect();
    assert_eq!(status.len(), 14);
    for (index, pair) in status.chunks(2).enumerate() {
        assert_eq!(pair[0]["state"], "started");
        assert_eq!(
            pair[1]["state"],
            if index == 6 { "failed" } else { "completed" }
        );
        assert_eq!(pair[0]["name"], pair[1]["name"]);
        assert!(pair[0].get("arguments").is_none());
    }
    let requests = llm.0.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0]
            .tools
            .iter()
            .filter(|tool| tool.name.starts_with("system_") || tool.name.starts_with("mcp_"))
            .count(),
        6
    );
    let results: Vec<_> = requests[1]
        .messages
        .iter()
        .filter_map(|message| {
            if let ChatMessage::ToolResult { content, .. } = message {
                Some(serde_json::from_str::<serde_json::Value>(content).unwrap())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(results.len(), 7);
    for result in &results[..6] {
        assert_eq!(result["ok"], true);
    }
    let data: Vec<serde_json::Value> = results[..6]
        .iter()
        .map(|result| serde_json::from_str(result["content"].as_str().unwrap()).unwrap())
        .collect();
    assert_eq!(data[0]["database"]["status"], "ready");
    assert!(data[1]["system"].get("memory_total_bytes").is_some());
    assert_eq!(data[2]["items"][0]["key"], "weather");
    assert_eq!(data[3]["key"], "weather");
    assert_eq!(data[4]["items"], serde_json::json!([]));
    assert_eq!(data[5]["items"][0]["server_key"], "weather");
    assert_eq!(results[6]["code"], "resource_not_found");
    assert!(
        !serde_json::to_string(&results)
            .unwrap()
            .contains("internal.example")
    );
    assert!(frames.iter().any(|frame| frame["type"] == "llm"));
    task.abort();
}

#[tokio::test]
async fn disabled_assistant_and_physical_device_do_not_advertise_system_tools() {
    for (enabled, physical) in [(false, false), (true, true)] {
        let llm = Arc::new(ManagementLlm::default());
        let providers = Arc::new(ProviderSet::with_all(
            Arc::new(VoiceVad),
            Arc::new(VoiceAsr),
            llm.clone(),
            Arc::new(VoiceTts),
        ));
        let (base, task) = start_with_system_mcp(providers, "physical-token", enabled).await;
        let client = Client::new();
        client
            .post(format!("{base}/api/admin/agents"))
            .bearer_auth("admin-test-token")
            .json(&serde_json::json!({"key":"studio","name":"Studio"}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let mut socket = if physical {
            client
                .post(format!("{base}/api/admin/devices"))
                .bearer_auth("admin-test-token")
                .json(&serde_json::json!({"device_id":"physical","agent_key":"studio"}))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
            tokio_tungstenite::connect_async(format!("{}/voice/v1/?device-id=physical&client-id=test&authorization=Bearer%20physical-token", base.replacen("http", "ws", 1))).await.unwrap().0
        } else {
            client
                .put(format!("{base}/api/admin/system/assistant-device"))
                .bearer_auth("admin-test-token")
                .json(&serde_json::json!({"agent_key":"studio","enabled":true}))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
            connect_assistant(&base).await
        };
        let frames = run_turn(&mut socket).await;
        assert!(frames.iter().all(|frame| frame["type"] != "tool"));
        let requests = llm.0.lock().unwrap();
        assert!(
            requests[0]
                .tools
                .iter()
                .all(|tool| !tool.name.starts_with("system_") && !tool.name.starts_with("mcp_"))
        );
        assert_eq!(requests.len(), 1);
        task.abort();
    }
}
