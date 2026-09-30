use std::time::Duration;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use voice_reference_client::admin::{
    AdminBaseUrl, AdminClient, AdminError,
    models::{BindTemplateProviderRequest, CreateAgentRequest, McpBindingRequest},
};

#[tokio::test]
async fn admin_client_preserves_the_nested_server_error_code() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request).await.unwrap();
        let body = r#"{"error":{"code":"revision_conflict","request_id":"test"}}"#;
        let response = format!(
            "HTTP/1.1 409 Conflict\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });

    let base = AdminBaseUrl::parse(&format!("http://{address}/api/admin/")).unwrap();
    let client = AdminClient::new(base, "test-token").unwrap();
    let error = client
        .create_agent(CreateAgentRequest {
            key: "qualification_agent".into(),
            name: "Qualification Agent".into(),
            description: None,
        })
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        AdminError::Rejected { status: 409, ref code } if code == "revision_conflict"
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn admin_client_rejects_dynamic_path_segments_before_network_io() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let accepted = tokio::time::timeout(Duration::from_millis(100), listener.accept()).await;
        let Ok(Ok((mut stream, _))) = accepted else {
            return false;
        };
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request).await.unwrap();
        let body =
            r#"{"id":1,"key":"qualification","name":"Qualification","enabled":true,"revision":1}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        true
    });
    let base = AdminBaseUrl::parse(&format!("http://{address}/api/admin/")).unwrap();
    let client = AdminClient::new(base, "test-token").unwrap();

    let template_result = client
        .bind_template_provider(
            "../../providers",
            "llm",
            1,
            BindTemplateProviderRequest {
                provider_key: "qualification_llm".into(),
            },
        )
        .await;
    let kind_result = client
        .bind_template_provider(
            "qualification_template",
            "../../providers",
            1,
            BindTemplateProviderRequest {
                provider_key: "qualification_llm".into(),
            },
        )
        .await;
    let agent_result = client
        .bind_agent_mcp(
            "../../providers",
            "qualification_mcp",
            1,
            McpBindingRequest {
                enabled: true,
                required: false,
            },
        )
        .await;
    let server_result = client
        .bind_agent_mcp(
            "qualification_agent",
            "../../providers",
            1,
            McpBindingRequest {
                enabled: true,
                required: false,
            },
        )
        .await;

    for result in [template_result, kind_result] {
        assert!(matches!(result, Err(AdminError::Wire)));
    }
    for result in [agent_result, server_result] {
        assert!(matches!(result, Err(AdminError::Wire)));
    }
    assert!(
        !server.await.unwrap(),
        "invalid resource identity must fail before a bearer-authenticated request is sent"
    );
}
