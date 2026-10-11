use super::*;
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Response},
    routing::get,
};

async fn service(body: &'static str, status: u16) -> (Value, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/v1/voices",
        get(move |headers: HeaderMap| async move {
            assert_eq!(headers["authorization"], "Bearer fixture-secret");
            Response::builder()
                .status(status)
                .body(Body::from(body))
                .unwrap()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let selection = json!({"base_url": format!("http://{}/v1", listener.local_addr().unwrap())});
    (
        selection,
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }),
    )
}

#[tokio::test]
async fn discovers_unicode_and_custom_voices_without_a_selected_voice() {
    let (selection, server) = service(r#"{"data":[{"id":"Hải Đăng","name":"Hải Đăng","aliases":["Minh Quân"]},{"id":"Custom","name":"Custom"}]}"#, 200).await;
    let result = discover(&selection, Some(&SecretValue::new("fixture-secret".into())))
        .await
        .unwrap();
    assert_eq!(
        result["voices"][0],
        json!({"id":"Hải Đăng","name":"Hải Đăng","languages":["vi-VN"]})
    );
    assert_eq!(result["voices"][1]["id"], "Custom");
    assert!(!result.to_string().contains("fixture-secret"));
    server.abort();
}

#[tokio::test]
async fn rejects_untrusted_lists_and_non_success_responses() {
    for (body, status) in [
        (r#"{"data":[{"id":"","name":"Invalid"}]}"#, 200),
        (
            r#"{"data":[{"id":"same","name":"One"},{"id":"same","name":"Two"}]}"#,
            200,
        ),
        ("not JSON", 200),
        ("private upstream error", 401),
    ] {
        let (selection, server) = service(body, status).await;
        assert!(matches!(
            discover(&selection, Some(&SecretValue::new("fixture-secret".into()))).await,
            Err(ProviderInspectError::RemoteUnavailable)
        ));
        server.abort();
    }
}

#[tokio::test]
async fn rejects_invalid_urls_and_credentials_in_selection() {
    for selection in [
        json!({"base_url":"file:///tmp/voices"}),
        json!({"base_url":"http://user:secret@localhost/v1"}),
        json!({"base_url":"http://localhost/v1","api_key":"secret"}),
    ] {
        assert!(matches!(
            discover(&selection, None).await,
            Err(ProviderInspectError::InvalidSelection)
        ));
    }
}
