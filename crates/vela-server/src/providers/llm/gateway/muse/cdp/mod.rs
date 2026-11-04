#![cfg_attr(test, allow(dead_code))]

mod bootstrap;
mod engine;
mod session;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

pub(crate) use self::bootstrap::extract_wss_via_chromium;
#[cfg_attr(test, allow(unused_imports))]
pub(crate) use self::engine::CdpBrowserEngine;

#[derive(Clone, PartialEq)]
pub(crate) struct CookieParam {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub expires: f64,
}

impl std::fmt::Debug for CookieParam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CookieParam")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("secure", &self.secure)
            .field("expires", &self.expires)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CdpError {
    #[error("browser process unavailable: {0}")]
    BrowserUnavailable(String),
    #[error("cdp protocol error: {0}")]
    Protocol(String),
    #[error("javascript evaluation error: {0}")]
    JsException(String),
    #[error("cdp operation timed out")]
    Timeout,
}

#[async_trait::async_trait]
pub(crate) trait BrowserPage: Send + Sync {
    async fn navigate(&self, url: &str) -> Result<(), CdpError>;
    async fn evaluate_json(
        &self,
        expression: &str,
        await_promise: bool,
        timeout: Duration,
    ) -> Result<Value, CdpError>;
    async fn get_cookies(&self, _urls: &[&str]) -> Result<BTreeMap<String, String>, CdpError> {
        Ok(BTreeMap::new())
    }
    async fn close(&self);
}

#[async_trait::async_trait]
pub(crate) trait BrowserEngine: Send + Sync {
    /// Open a new tab inside the isolated `BrowserContext` of `context_key` and load cookies.
    async fn open_page(
        &self,
        context_key: &str,
        cookies: &[CookieParam],
    ) -> Result<Arc<dyn BrowserPage>, CdpError>;

    /// Dispose the `BrowserContext` associated with `context_key` (`Target.disposeBrowserContext`).
    async fn dispose_context(&self, context_key: &str);
}
