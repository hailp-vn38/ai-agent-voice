use super::harness::*;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Error, Message, client::IntoClientRequest},
};

#[tokio::test]
async fn websocket_admits_mixed_sources_and_refuses_a_removed_explicit_config_binding() {
    let (base, db, _builds, task) = server(true).await;
    sqlx::raw_sql(r#"
INSERT INTO providers(key,name,type,adapter,config_json,created_at,updated_at) VALUES ('vad','VAD','vad','silero_onnx','{}',1,1),('asr','ASR','asr','gipformer_sherpa_offline','{"decoding_method":"greedy_search","max_active_paths":4}',1,1),('tts','TTS','tts','zerotts_onnx','{"voice":"maichi"}',1,1);
INSERT INTO template_provider_bindings(template_id,provider_type,provider_id,created_at,updated_at) SELECT 1,type,id,1,1 FROM providers WHERE key IN ('vad','asr','tts');
INSERT INTO template_provider_bindings(template_id,provider_type,provider_source,config_provider_key,created_at,updated_at) VALUES(1,'llm','config','primary',1,1);
"#).execute(db.pool()).await.unwrap();
    let request = || {
        let mut request = format!(
            "{}/voice/v1/",
            base.trim_end_matches("/api/admin")
                .replace("http://", "ws://")
        )
        .into_client_request()
        .unwrap();
        request
            .headers_mut()
            .insert("Protocol-Version", "1".parse().unwrap());
        request
            .headers_mut()
            .insert("Device-Id", "device".parse().unwrap());
        request
            .headers_mut()
            .insert("Client-Id", "config-bindings-test".parse().unwrap());
        request
    };
    let (mut socket, response) = connect_async(request()).await.unwrap();
    assert_eq!(response.status().as_u16(), 101);
    // Mutation affects a new admission, while the admitted session keeps its frozen profile.
    sqlx::query("UPDATE template_provider_bindings SET config_provider_key='removed' WHERE provider_source='config'").execute(db.pool()).await.unwrap();
    socket
        .send(Message::Text(r#"{"type":"hello"}"#.into()))
        .await
        .unwrap();
    let hello = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(hello, Message::Text(_)));
    match connect_async(request()).await.unwrap_err() {
        Error::Http(r) => assert_eq!(r.status().as_u16(), 503),
        other => panic!("{other:?}"),
    }
    socket.close(None).await.unwrap();
    task.abort();
}
