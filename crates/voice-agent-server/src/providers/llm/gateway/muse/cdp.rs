use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

const CDP_MAX_WS_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CookieParam {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub expires: f64,
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
    async fn current_url(&self) -> Result<String, CdpError>;
    async fn evaluate_json(
        &self,
        expression: &str,
        await_promise: bool,
        timeout: Duration,
    ) -> Result<Value, CdpError>;
    async fn press_enter(&self) -> Result<(), CdpError>;
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

pub(super) struct CdpSession {
    next_id: AtomicU64,
    tx: mpsc::UnboundedSender<String>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, CdpError>>>>>,
    closed: Arc<AtomicBool>,
}

impl CdpSession {
    pub async fn connect(ws_url: &str) -> Result<Arc<Self>, CdpError> {
        let mut ws_config = WebSocketConfig::default();
        ws_config.max_message_size = Some(CDP_MAX_WS_PAYLOAD_BYTES);
        ws_config.max_frame_size = Some(CDP_MAX_WS_PAYLOAD_BYTES);
        let (ws_stream, _) = connect_async_with_config(ws_url, Some(ws_config), false)
            .await
            .map_err(|err| {
                CdpError::BrowserUnavailable(format!("websocket connect {ws_url}: {err}"))
            })?;
        let (mut sink, mut stream) = ws_stream.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, CdpError>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));

        let closed_writer = Arc::clone(&closed);
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if sink.send(Message::Text(msg.into())).await.is_err() {
                    closed_writer.store(true, Ordering::SeqCst);
                    break;
                }
            }
        });

        let pending_reader = Arc::clone(&pending);
        let closed_reader = Arc::clone(&closed);
        tokio::spawn(async move {
            let mut close_reason = "cdp websocket closed".to_string();
            while let Some(frame) = stream.next().await {
                match frame {
                    Ok(Message::Text(text)) => {
                        if let Ok(val) = serde_json::from_str::<Value>(&text) {
                            if let Some(id) = val.get("id").and_then(Value::as_u64) {
                                let sender = pending_reader.lock().await.remove(&id);
                                if let Some(sender) = sender {
                                    if let Some(err) = val.get("error") {
                                        let msg = err
                                            .get("message")
                                            .and_then(Value::as_str)
                                            .unwrap_or("cdp error")
                                            .to_string();
                                        let _ = sender.send(Err(CdpError::Protocol(msg)));
                                    } else {
                                        let res = val.get("result").cloned().unwrap_or(Value::Null);
                                        let _ = sender.send(Ok(res));
                                    }
                                }
                            }
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Err(err) => {
                        close_reason = format!("cdp websocket closed: {err}");
                        break;
                    }
                    _ => {}
                }
            }
            closed_reader.store(true, Ordering::SeqCst);
            let mut map = pending_reader.lock().await;
            for (_, sender) in map.drain() {
                let _ = sender.send(Err(CdpError::Protocol(close_reason.clone())));
            }
        });

        Ok(Arc::new(Self {
            next_id: AtomicU64::new(1),
            tx,
            pending,
            closed,
        }))
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub async fn send(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, CdpError> {
        if self.is_closed() {
            return Err(CdpError::BrowserUnavailable(
                "cdp session is closed".to_string(),
            ));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let payload = json!({
            "id": id,
            "method": method,
            "params": params,
        });
        if self.tx.send(payload.to_string()).is_err() {
            self.pending.lock().await.remove(&id);
            self.closed.store(true, Ordering::SeqCst);
            return Err(CdpError::BrowserUnavailable(
                "cdp writer channel closed".to_string(),
            ));
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err(CdpError::Protocol(
                "cdp response channel dropped".to_string(),
            )),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(CdpError::Timeout)
            }
        }
    }
}

struct CdpBrowserState {
    child: Option<Child>,
    browser_ws: Option<Arc<CdpSession>>,
    contexts: HashMap<String, String>,
}

pub(crate) struct CdpBrowserEngine {
    cdp_port: u16,
    headless: bool,
    profile_dir: PathBuf,
    http: reqwest::Client,
    state: Mutex<CdpBrowserState>,
}

impl CdpBrowserEngine {
    pub fn new(cdp_port: u16, headless: bool) -> Self {
        let profile_dir =
            std::env::temp_dir().join(format!("voice-agent-muse-cdp-profile-{cdp_port}"));
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        Self {
            cdp_port,
            headless,
            profile_dir,
            http,
            state: Mutex::new(CdpBrowserState {
                child: None,
                browser_ws: None,
                contexts: HashMap::new(),
            }),
        }
    }

    async fn fetch_browser_ws_url(&self) -> Option<String> {
        let url = format!("http://127.0.0.1:{}/json/version", self.cdp_port);
        let resp = self.http.get(&url).send().await.ok()?;
        let val = resp.json::<Value>().await.ok()?;
        val.get("webSocketDebuggerUrl")
            .and_then(Value::as_str)
            .map(ToString::to_string)
    }

    fn clean_singleton_locks(&self) {
        let _ = std::fs::create_dir_all(&self.profile_dir);
        for lock_name in ["SingletonLock", "SingletonSocket", "SingletonCookie"] {
            let _ = std::fs::remove_file(self.profile_dir.join(lock_name));
        }
    }

    async fn ensure_connected(&self) -> Result<Arc<CdpSession>, CdpError> {
        let mut guard = self.state.lock().await;
        if let Some(ws) = &guard.browser_ws {
            if !ws.is_closed() {
                return Ok(Arc::clone(ws));
            }
        }

        let ws_url = if let Some(existing) = self.fetch_browser_ws_url().await {
            existing
        } else {
            self.clean_singleton_locks();
            let bin = find_chromium_binary().ok_or_else(|| {
                CdpError::BrowserUnavailable(
                    "Chromium/Chrome binary not found; set CHROMIUM_PATH or install Chrome"
                        .to_string(),
                )
            })?;
            let mut cmd = Command::new(bin);
            cmd.kill_on_drop(true)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .arg(format!("--remote-debugging-port={}", self.cdp_port))
                .arg("--remote-allow-origins=*")
                .arg(format!("--user-data-dir={}", self.profile_dir.display()))
                .arg("--no-first-run")
                .arg("--no-default-browser-check")
                .arg("--disable-dev-shm-usage")
                .arg("--disable-background-networking")
                .arg("--autoplay-policy=no-user-gesture-required")
                .arg("--window-size=1440,2200");
            if self.headless {
                cmd.arg("--headless=new").arg("--disable-gpu");
            }
            let child = cmd.spawn().map_err(|err| {
                CdpError::BrowserUnavailable(format!("failed to spawn chromium: {err}"))
            })?;
            guard.child = Some(child);

            let mut found = None;
            for _ in 0..40 {
                tokio::time::sleep(Duration::from_millis(200)).await;
                if let Some(url) = self.fetch_browser_ws_url().await {
                    found = Some(url);
                    break;
                }
            }
            found.ok_or_else(|| {
                CdpError::BrowserUnavailable(
                    "chromium launched but /json/version did not become ready".to_string(),
                )
            })?
        };

        let browser_ws = CdpSession::connect(&ws_url).await?;
        self.prune_dead_contexts(&browser_ws, &mut guard.contexts)
            .await;
        guard.browser_ws = Some(Arc::clone(&browser_ws));
        Ok(browser_ws)
    }

    async fn prune_dead_contexts(
        &self,
        browser_ws: &Arc<CdpSession>,
        contexts: &mut HashMap<String, String>,
    ) {
        let Ok(res) = browser_ws
            .send(
                "Target.getBrowserContexts",
                json!({}),
                Duration::from_secs(5),
            )
            .await
        else {
            contexts.clear();
            return;
        };

        let live_ids: HashSet<String> = res
            .get("browserContextIds")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default();

        contexts.retain(|_, ctx_id| live_ids.contains(ctx_id));
        let owned_ids: HashSet<&String> = contexts.values().collect();
        for live_id in &live_ids {
            if !owned_ids.contains(live_id) {
                let _ = browser_ws
                    .send(
                        "Target.disposeBrowserContext",
                        json!({ "browserContextId": live_id }),
                        Duration::from_secs(5),
                    )
                    .await;
            }
        }
    }

    async fn get_or_create_context(
        &self,
        browser_ws: &Arc<CdpSession>,
        context_key: &str,
    ) -> Result<String, CdpError> {
        {
            let guard = self.state.lock().await;
            if let Some(existing) = guard.contexts.get(context_key) {
                return Ok(existing.clone());
            }
        }
        let res = browser_ws
            .send(
                "Target.createBrowserContext",
                json!({ "disposeOnDetach": false }),
                Duration::from_secs(10),
            )
            .await?;
        let ctx_id = res
            .get("browserContextId")
            .and_then(Value::as_str)
            .ok_or_else(|| CdpError::Protocol("missing browserContextId".to_string()))?
            .to_string();
        let mut guard = self.state.lock().await;
        guard
            .contexts
            .insert(context_key.to_string(), ctx_id.clone());
        Ok(ctx_id)
    }
}

#[async_trait::async_trait]
impl BrowserEngine for CdpBrowserEngine {
    async fn open_page(
        &self,
        context_key: &str,
        cookies: &[CookieParam],
    ) -> Result<Arc<dyn BrowserPage>, CdpError> {
        let browser_ws = self.ensure_connected().await?;
        let ctx_id = self.get_or_create_context(&browser_ws, context_key).await?;
        let target = browser_ws
            .send(
                "Target.createTarget",
                json!({
                    "url": "about:blank",
                    "browserContextId": ctx_id,
                    "newWindow": true,
                }),
                Duration::from_secs(10),
            )
            .await?;
        let target_id = target
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or_else(|| CdpError::Protocol("missing targetId".to_string()))?
            .to_string();

        let page_ws_url = format!("ws://127.0.0.1:{}/devtools/page/{target_id}", self.cdp_port);
        let page_ws = CdpSession::connect(&page_ws_url).await?;

        for domain in ["Page.enable", "Runtime.enable", "Network.enable"] {
            page_ws
                .send(domain, json!({}), Duration::from_secs(10))
                .await?;
        }
        let _ = page_ws
            .send(
                "Emulation.setFocusEmulationEnabled",
                json!({ "enabled": true }),
                Duration::from_secs(5),
            )
            .await;
        let _ = page_ws
            .send(
                "Page.setWebLifecycleState",
                json!({ "state": "active" }),
                Duration::from_secs(5),
            )
            .await;

        for cookie in cookies {
            page_ws
                .send(
                    "Network.setCookie",
                    json!({
                        "name": cookie.name,
                        "value": cookie.value,
                        "domain": cookie.domain,
                        "path": cookie.path,
                        "secure": cookie.secure,
                        "expires": cookie.expires,
                    }),
                    Duration::from_secs(5),
                )
                .await?;
        }

        Ok(Arc::new(CdpBrowserPage {
            target_id,
            browser_ws,
            page_ws,
        }))
    }

    async fn dispose_context(&self, context_key: &str) {
        let (browser_ws, ctx_id) = {
            let mut guard = self.state.lock().await;
            let ctx_id = guard.contexts.remove(context_key);
            (guard.browser_ws.clone(), ctx_id)
        };
        if let (Some(ws), Some(ctx_id)) = (browser_ws, ctx_id) {
            if !ws.is_closed() {
                let _ = ws
                    .send(
                        "Target.disposeBrowserContext",
                        json!({ "browserContextId": ctx_id }),
                        Duration::from_secs(5),
                    )
                    .await;
            }
        }
    }
}

struct CdpBrowserPage {
    target_id: String,
    browser_ws: Arc<CdpSession>,
    page_ws: Arc<CdpSession>,
}

#[async_trait::async_trait]
impl BrowserPage for CdpBrowserPage {
    async fn navigate(&self, url: &str) -> Result<(), CdpError> {
        self.page_ws
            .send(
                "Page.navigate",
                json!({ "url": url }),
                Duration::from_secs(30),
            )
            .await?;
        Ok(())
    }

    async fn current_url(&self) -> Result<String, CdpError> {
        let val = self
            .evaluate_json("location.href", false, Duration::from_secs(5))
            .await?;
        Ok(val.as_str().unwrap_or_default().to_string())
    }

    async fn evaluate_json(
        &self,
        expression: &str,
        await_promise: bool,
        timeout: Duration,
    ) -> Result<Value, CdpError> {
        let res = self
            .page_ws
            .send(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": await_promise,
                }),
                timeout,
            )
            .await?;
        if let Some(exc) = res.get("exceptionDetails") {
            let text = exc
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("js exception");
            return Err(CdpError::JsException(text.to_string()));
        }
        Ok(res
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    async fn press_enter(&self) -> Result<(), CdpError> {
        self.page_ws
            .send(
                "Input.dispatchKeyEvent",
                json!({
                    "type": "keyDown",
                    "key": "Enter",
                    "code": "Enter",
                    "windowsVirtualKeyCode": 13,
                    "text": "\r"
                }),
                Duration::from_secs(5),
            )
            .await?;
        self.page_ws
            .send(
                "Input.dispatchKeyEvent",
                json!({
                    "type": "keyUp",
                    "key": "Enter",
                    "code": "Enter",
                    "windowsVirtualKeyCode": 13
                }),
                Duration::from_secs(5),
            )
            .await?;
        Ok(())
    }

    async fn get_cookies(&self, urls: &[&str]) -> Result<BTreeMap<String, String>, CdpError> {
        let res = self
            .page_ws
            .send(
                "Network.getCookies",
                json!({ "urls": urls }),
                Duration::from_secs(5),
            )
            .await?;
        let mut map = BTreeMap::new();
        if let Some(arr) = res.get("cookies").and_then(Value::as_array) {
            for item in arr {
                if let (Some(name), Some(value)) = (
                    item.get("name").and_then(Value::as_str),
                    item.get("value").and_then(Value::as_str),
                ) {
                    let name = name.trim();
                    let value = value.trim();
                    if !name.is_empty() && !value.is_empty() {
                        map.insert(name.to_owned(), value.to_owned());
                    }
                }
            }
        }
        Ok(map)
    }

    async fn close(&self) {
        let _ = self
            .browser_ws
            .send(
                "Target.closeTarget",
                json!({ "targetId": self.target_id }),
                Duration::from_secs(5),
            )
            .await;
    }
}

fn find_chromium_binary() -> Option<String> {
    if let Ok(custom) = std::env::var("CHROMIUM_PATH") {
        if !custom.trim().is_empty() && std::path::Path::new(&custom).exists() {
            return Some(custom);
        }
    }
    let candidates = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
    ];
    for candidate in candidates {
        if std::path::Path::new(candidate).exists() {
            return Some(candidate.to_string());
        }
    }
    None
}
