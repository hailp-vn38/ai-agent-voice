//! Typed, origin-pinned access to the separately authenticated Admin API.

use std::net::IpAddr;

use thiserror::Error;
use url::Url;
pub mod models;
use models::*;

#[derive(Clone)]
pub struct AdminClient {
    base: AdminBaseUrl,
    http: reqwest::Client,
    token: String,
}
#[derive(Debug, Error)]
pub enum AdminError {
    #[error("admin request failed")]
    Transport,
    #[error("admin rejected request: {status} {code}")]
    Rejected { status: u16, code: String },
    #[error("admin response is invalid")]
    Wire,
}

impl AdminClient {
    pub fn new(base: AdminBaseUrl, token: impl Into<String>) -> Result<Self, AdminError> {
        let token = token.into();
        if token.is_empty() {
            return Err(AdminError::Wire);
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| AdminError::Transport)?;
        Ok(Self { base, http, token })
    }
    async fn send<T: serde::Serialize, R: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&T>,
        revision: Option<u64>,
    ) -> Result<R, AdminError> {
        let url = self.base.join(path).map_err(|_| AdminError::Wire)?;
        let mut request = self.http.request(method, url).bearer_auth(&self.token);
        if let Some(revision) = revision {
            request = request.header("if-match", revision);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.map_err(|_| AdminError::Transport)?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let code = response
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v.get("code").and_then(|v| v.as_str()).map(str::to_owned))
                .unwrap_or_else(|| "unknown".into());
            return Err(AdminError::Rejected { status, code });
        }
        response.json().await.map_err(|_| AdminError::Wire)
    }
    pub async fn create_agent(&self, value: CreateAgentRequest) -> Result<AgentView, AdminError> {
        self.send(reqwest::Method::POST, "agents", Some(&value), None)
            .await
    }
    pub async fn create_template(
        &self,
        value: CreateTemplateRequest,
    ) -> Result<TemplateView, AdminError> {
        self.send(reqwest::Method::POST, "templates", Some(&value), None)
            .await
    }
    pub async fn create_provider(
        &self,
        value: CreateProviderRequest,
    ) -> Result<ProviderView, AdminError> {
        self.send(reqwest::Method::POST, "providers", Some(&value), None)
            .await
    }
    pub async fn bind_template_provider(
        &self,
        template: &str,
        kind: &str,
        revision: u64,
        value: BindTemplateProviderRequest,
    ) -> Result<TemplateView, AdminError> {
        self.send(
            reqwest::Method::PUT,
            &format!("templates/{template}/providers/{kind}"),
            Some(&value),
            Some(revision),
        )
        .await
    }
    pub async fn create_device(
        &self,
        value: CreateDeviceRequest,
    ) -> Result<DeviceView, AdminError> {
        self.send(reqwest::Method::POST, "devices", Some(&value), None)
            .await
    }
    pub async fn create_mcp_server(
        &self,
        value: CreateMcpServerRequest,
    ) -> Result<McpServerView, AdminError> {
        self.send(reqwest::Method::POST, "mcp-servers", Some(&value), None)
            .await
    }
    pub async fn bind_agent_mcp(
        &self,
        agent: &str,
        server: &str,
        revision: u64,
        value: McpBindingRequest,
    ) -> Result<AgentView, AdminError> {
        self.send(
            reqwest::Method::PUT,
            &format!("agents/{agent}/mcp-bindings/{server}"),
            Some(&value),
            Some(revision),
        )
        .await
    }
}

/// A validated Admin API collection base. Resource paths are always joined relatively.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminBaseUrl(Url);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AdminUrlError {
    #[error("admin URL is invalid")]
    Invalid,
    #[error("admin URL must use http or https")]
    Scheme,
    #[error("admin URL must not contain credentials, a query, or a fragment")]
    Components,
    #[error("admin URL must end with a slash")]
    TrailingSlash,
    #[error("plaintext Admin HTTP is only allowed for explicit local origins")]
    InsecureOrigin,
}

impl AdminBaseUrl {
    pub fn parse(value: &str) -> Result<Self, AdminUrlError> {
        let url = Url::parse(value).map_err(|_| AdminUrlError::Invalid)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(AdminUrlError::Scheme);
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(AdminUrlError::Components);
        }
        if !url.path().ends_with('/') {
            return Err(AdminUrlError::TrailingSlash);
        }
        if url.scheme() == "http" && !is_explicit_local_origin(&url) {
            return Err(AdminUrlError::InsecureOrigin);
        }
        Ok(Self(url))
    }

    pub fn join(&self, relative: &str) -> Result<Url, AdminUrlError> {
        if relative.is_empty()
            || relative.starts_with('/')
            || relative.contains('?')
            || relative.contains('#')
        {
            return Err(AdminUrlError::Invalid);
        }
        self.0.join(relative).map_err(|_| AdminUrlError::Invalid)
    }

    pub fn as_url(&self) -> &Url {
        &self.0
    }
}

fn is_explicit_local_origin(url: &Url) -> bool {
    match url.host_str() {
        Some("localhost") => true,
        Some(host) => match host.trim_matches(['[', ']']).parse::<IpAddr>() {
            Ok(IpAddr::V4(address)) => address.octets()[0] == 127,
            Ok(IpAddr::V6(address)) => address.is_loopback(),
            Err(_) => false,
        },
        None => false,
    }
}
