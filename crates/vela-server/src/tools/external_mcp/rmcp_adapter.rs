//! The only HTTP route RMCP may use for External MCP.
//!
//! RMCP owns JSON-RPC, lifecycle, pagination and SSE framing.  This adapter deliberately owns
//! only HTTP response handling: redirects remain disabled by the caller-owned reqwest client,
//! and credentials never escape the resolved session snapshot.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use futures_util::{StreamExt, stream::BoxStream};
use http::{HeaderName, HeaderValue};
use rmcp::{
    ErrorData,
    model::{ClientJsonRpcMessage, ServerJsonRpcMessage},
    transport::streamable_http_client::{
        StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
    },
};
use sse_stream::{Error as SseError, Sse, SseStream};
use url::Url;

use super::transport::{ReadRejection, read_bounded};

#[derive(Debug, thiserror::Error)]
#[error("external_mcp_transport")]
pub(crate) struct AdapterError;

/// A cloneable RMCP backend, constructed only from a materialized session snapshot.
#[derive(Clone)]
pub(crate) struct PolicyHttpClient {
    endpoint: Url,
    http: reqwest::Client,
    response_cap: usize,
    auth_failed: Arc<AtomicBool>,
    invalid_response: Arc<AtomicBool>,
    unavailable: Arc<AtomicBool>,
}

impl PolicyHttpClient {
    pub(crate) fn new(endpoint: Url, http: reqwest::Client, response_cap: usize) -> Self {
        Self {
            endpoint,
            http,
            response_cap,
            auth_failed: Arc::new(AtomicBool::new(false)),
            invalid_response: Arc::new(AtomicBool::new(false)),
            unavailable: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn take_auth_failed(&self) -> bool {
        self.auth_failed.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn take_invalid_response(&self) -> bool {
        self.invalid_response.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn take_unavailable(&self) -> bool {
        self.unavailable.swap(false, Ordering::AcqRel)
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, StreamableHttpError<AdapterError>> {
        request
            .send()
            .await
            .map_err(|_| StreamableHttpError::Client(AdapterError))
    }

    fn post(
        &self,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> reqwest::RequestBuilder {
        let mut request = self
            .http
            .post(self.endpoint.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .json(&message);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        if let Some(session_id) = session_id {
            request = request.header("mcp-session-id", session_id.as_ref());
        }
        if let Some(auth) = auth {
            request = request.bearer_auth(auth);
        }
        request
    }

    fn status_error(
        &self,
        response: &reqwest::Response,
    ) -> Option<StreamableHttpError<AdapterError>> {
        if response.status() == reqwest::StatusCode::UNAUTHORIZED
            || response.status() == reqwest::StatusCode::FORBIDDEN
        {
            self.auth_failed.store(true, Ordering::Release);
            return Some(StreamableHttpError::UnexpectedServerResponse(
                "authentication failed".into(),
            ));
        }
        (!response.status().is_success()).then(|| {
            StreamableHttpError::UnexpectedServerResponse("non-success HTTP status".into())
        })
    }
}

impl StreamableHttpClient for PolicyHttpClient {
    type Error = AdapterError;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        self.post_message_with_max_sse_event_size(
            uri,
            message,
            session_id,
            auth,
            headers,
            self.response_cap,
        )
        .await
    }

    async fn post_message_with_max_sse_event_size(
        &self,
        _uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
        _max_sse_event_size: usize,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let was_notification = matches!(message, ClientJsonRpcMessage::Notification(_));
        let request_message = message.clone();
        let response = self
            .send(self.post(message, session_id, auth, headers))
            .await?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED
            || response.status() == reqwest::StatusCode::FORBIDDEN
        {
            self.auth_failed.store(true, Ordering::Release);
            return Ok(synthetic_tool_failure(request_message));
        }
        if !response.status().is_success() {
            self.unavailable.store(true, Ordering::Release);
            return Ok(synthetic_tool_failure(request_message));
        }
        if response.status() == reqwest::StatusCode::ACCEPTED
            || response.status() == reqwest::StatusCode::NO_CONTENT
        {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        let session = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if content_type
            .split(';')
            .next()
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("text/event-stream"))
        {
            let stream: BoxStream<'static, Result<Sse, SseError>> =
                SseStream::from_bytes_stream(response.bytes_stream()).boxed();
            return Ok(StreamableHttpPostResponse::Sse(stream, session));
        }
        if !content_type
            .split(';')
            .next()
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
        {
            self.invalid_response.store(true, Ordering::Release);
            return Err(StreamableHttpError::UnexpectedContentType(Some(
                content_type.to_owned(),
            )));
        }
        let bytes = match read_bounded(response, self.response_cap).await {
            Ok(bytes) => bytes,
            Err(ReadRejection::TooLarge) => {
                self.invalid_response.store(true, Ordering::Release);
                return Err(StreamableHttpError::UnexpectedServerResponse(
                    "response exceeds configured cap".into(),
                ));
            }
            Err(ReadRejection::Unavailable) => {
                return Err(StreamableHttpError::Client(AdapterError));
            }
        };
        if was_notification && bytes.is_empty() {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        let message = serde_json::from_slice::<ServerJsonRpcMessage>(&bytes).map_err(|error| {
            self.invalid_response.store(true, Ordering::Release);
            StreamableHttpError::Deserialize(error)
        })?;
        Ok(StreamableHttpPostResponse::Json(message, session))
    }

    async fn delete_session(
        &self,
        _uri: Arc<str>,
        session_id: Arc<str>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        let mut request = self
            .http
            .delete(self.endpoint.clone())
            .header("mcp-session-id", session_id.as_ref());
        for (name, value) in headers {
            request = request.header(name, value);
        }
        if let Some(auth) = auth {
            request = request.bearer_auth(auth);
        }
        let response = self.send(request).await?;
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Ok(());
        }
        self.status_error(&response).map_or(Ok(()), Err)
    }

    async fn get_stream(
        &self,
        _uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        let mut request = self
            .http
            .get(self.endpoint.clone())
            .header(reqwest::header::ACCEPT, "text/event-stream");
        for (name, value) in headers {
            request = request.header(name, value);
        }
        if let Some(session_id) = session_id {
            request = request.header("mcp-session-id", session_id.as_ref());
        }
        if let Some(last_event_id) = last_event_id {
            request = request.header("last-event-id", last_event_id);
        }
        if let Some(auth) = auth {
            request = request.bearer_auth(auth);
        }
        let response = self.send(request).await?;
        if let Some(error) = self.status_error(&response) {
            return Err(error);
        }
        Ok(SseStream::from_bytes_stream(response.bytes_stream()).boxed())
    }
}

/// A remote status has no JSON-RPC response, but returning a bounded synthetic protocol error to
/// RMCP completes precisely this request without tearing down the immutable session snapshot.
/// The application still maps the outcome to its typed, content-free domain failure.
fn synthetic_tool_failure(message: ClientJsonRpcMessage) -> StreamableHttpPostResponse {
    let ClientJsonRpcMessage::Request(request) = message else {
        return StreamableHttpPostResponse::Accepted;
    };
    let method = serde_json::to_value(&request.request)
        .ok()
        .and_then(|value| {
            value
                .get("method")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    if method.as_deref() != Some("tools/call") {
        return StreamableHttpPostResponse::Json(
            ServerJsonRpcMessage::error(
                ErrorData::internal_error("external MCP HTTP failure", None),
                Some(request.id),
            ),
            None,
        );
    }
    let id = serde_json::to_value(request.id).expect("RMCP request ids serialize");
    let message = serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0", "id": id,
        "result": {"content": [{"type": "text", "text": "external MCP HTTP failure"}], "isError": true}
    })).expect("synthetic RMCP tool result matches the SDK model");
    StreamableHttpPostResponse::Json(message, None)
}
