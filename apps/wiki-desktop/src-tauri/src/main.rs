use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use wiki_core::{
    Applied, Change, DocPath, Graph, Origin, Page, ReadWindow, Scope, SyncConfig, SyncError,
    SyncReport, SyncStatus, TreeNode, Vault, VaultConfig, VaultError, VaultStatus, VaultSync,
};

mod claude_config;
mod mcp_commands;
mod mcp_server;
mod watcher;

const RECENT_VAULT_LIMIT: usize = 12;

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Settings {
    pub(crate) recent_vaults: Vec<String>,
    pub(crate) current_vault: Option<String>,
    #[serde(default)]
    pub(crate) mcp_port: Option<u16>,
    #[serde(default)]
    pub(crate) mcp_allow_write: bool,
    #[serde(default)]
    pub(crate) sync: HashMap<String, SyncSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SyncSettings {
    #[serde(default)]
    pub(crate) auto: bool,
    #[serde(default = "default_sync_interval")]
    pub(crate) interval_min: u64,
}

#[derive(Debug, Clone, Serialize)]
struct SyncConfigurationView {
    remote_url: String,
    branch: String,
    auto: bool,
    interval_min: u64,
    public_github_remote: bool,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            auto: false,
            interval_min: default_sync_interval(),
        }
    }
}

fn default_sync_interval() -> u64 {
    5
}

impl Settings {
    fn remember_vault(&mut self, path: &str) {
        self.recent_vaults.retain(|recent| recent != path);
        self.recent_vaults.insert(0, path.to_owned());
        self.recent_vaults.truncate(RECENT_VAULT_LIMIT);
        self.current_vault = Some(path.to_owned());
    }
}

/// Stops a background task when dropped, so replacing the slot that holds it
/// is enough to cancel the previous task.
struct TaskCancel(tokio::sync::watch::Sender<bool>);

impl TaskCancel {
    fn new() -> (Self, tokio::sync::watch::Receiver<bool>) {
        let (sender, receiver) = tokio::sync::watch::channel(false);
        (Self(sender), receiver)
    }
}

impl Drop for TaskCancel {
    fn drop(&mut self) {
        let _ = self.0.send(true);
    }
}

async fn cancelled(receiver: &mut tokio::sync::watch::Receiver<bool>) {
    while !*receiver.borrow() {
        if receiver.changed().await.is_err() {
            return;
        }
    }
}

pub(crate) struct DesktopState {
    pub(crate) vault: Mutex<Option<Arc<Vault>>>,
    pub(crate) settings: Mutex<Settings>,
    pub(crate) settings_file: PathBuf,
    event_forwarding: Mutex<Option<TaskCancel>>,
    pub(crate) mcp_server: Mutex<Option<mcp_server::McpServerRuntime>>,
    /// Why the MCP server is not running, shown in MCP settings.
    pub(crate) mcp_error: Mutex<Option<String>>,
    pub(crate) claude_preview: Mutex<Option<claude_config::Preview>>,
    watcher: Mutex<Option<watcher::WatchHandle>>,
    lock_retry: Mutex<Option<TaskCancel>>,
    sync: Mutex<Option<Arc<VaultSync>>>,
    sync_status_forwarding: Mutex<Option<TaskCancel>>,
    sync_scheduler: Mutex<Option<TaskCancel>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub(crate) enum CommandError {
    NotFound { message: String },
    InvalidPath { message: String },
    InvalidInput { message: String },
    Conflict { message: String, current: String },
    ReadOnly { message: String },
    Io { message: String },
    Index { message: String },
}

impl CommandError {
    pub(crate) fn io(message: impl ToString) -> Self {
        Self::Io {
            message: message.to_string(),
        }
    }
}

impl From<VaultError> for CommandError {
    fn from(error: VaultError) -> Self {
        let message = error.to_string();
        match error {
            VaultError::NotFound(_) => Self::NotFound { message },
            VaultError::InvalidPath(_) => Self::InvalidPath { message },
            VaultError::Conflict { current } => Self::Conflict {
                message,
                current: current.0,
            },
            VaultError::ReadOnly => Self::ReadOnly { message },
            VaultError::Io(_) => Self::Io { message },
            VaultError::Index(_) => Self::Index { message },
        }
    }
}

impl From<std::io::Error> for CommandError {
    fn from(error: std::io::Error) -> Self {
        Self::io(error)
    }
}

impl From<serde_json::Error> for CommandError {
    fn from(error: serde_json::Error) -> Self {
        Self::io(error)
    }
}

/// Locks shell state; a poisoned lock becomes an `Io` error naming `what`.
pub(crate) fn locked<'a, T>(
    mutex: &'a Mutex<T>,
    what: &str,
) -> Result<MutexGuard<'a, T>, CommandError> {
    mutex
        .lock()
        .map_err(|_| CommandError::io(format!("{what} lock poisoned")))
}

fn emit_lock(app: &tauri::AppHandle, writable: bool) -> bool {
    app.emit("vault://lock", serde_json::json!({ "writable": writable }))
        .is_ok()
}

impl DesktopState {
    fn persist(&self) -> Result<(), CommandError> {
        let json = serde_json::to_vec_pretty(&*locked(&self.settings, "settings")?)?;
        if let Some(parent) = self.settings_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.settings_file, json)?;
        Ok(())
    }

    fn active_vault(&self) -> Result<Arc<Vault>, CommandError> {
        locked(&self.vault, "vault")?
            .clone()
            .ok_or_else(|| CommandError::NotFound {
                message: "No knowledge vault is open".into(),
            })
    }

    fn current_vault_path(&self) -> Result<PathBuf, CommandError> {
        locked(&self.settings, "settings")?
            .current_vault
            .as_ref()
            .map(PathBuf::from)
            .ok_or_else(|| CommandError::NotFound {
                message: "No knowledge vault is open".into(),
            })
    }
}

fn open_vault_state(state: &DesktopState, path: String) -> Result<String, CommandError> {
    let root = PathBuf::from(&path).canonicalize()?;
    let canonical = root.to_string_lossy().into_owned();
    let already_open = locked(&state.settings, "settings")?
        .current_vault
        .as_deref()
        == Some(canonical.as_str());
    if already_open && locked(&state.vault, "vault")?.is_some() {
        return Ok(canonical);
    }
    let vault = Arc::new(Vault::open(VaultConfig::new(&root))?);
    *locked(&state.vault, "vault")? = Some(vault);
    locked(&state.settings, "settings")?.remember_vault(&canonical);
    state.persist()?;
    Ok(canonical)
}

fn restart_watcher(state: &DesktopState) -> Result<(), CommandError> {
    locked(&state.watcher, "file watcher")?.take();
    let handle = watcher::start(state.current_vault_path()?, state.active_vault()?)
        .map_err(CommandError::io)?;
    *locked(&state.watcher, "file watcher")? = Some(handle);
    Ok(())
}

fn restart_lock_retry(app: &tauri::AppHandle, state: &DesktopState) -> Result<(), CommandError> {
    locked(&state.lock_retry, "vault lock retry")?.take();
    let vault = state.active_vault()?;
    emit_lock(app, vault.is_writable());
    if vault.is_writable() {
        return Ok(());
    }
    let (cancel, mut stop) = TaskCancel::new();
    *locked(&state.lock_retry, "vault lock retry")? = Some(cancel);
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.tick().await;
        loop {
            tokio::select! {
                _ = cancelled(&mut stop) => break,
                _ = interval.tick() => match vault.try_acquire_write() {
                    // `try_acquire_write` publishes `VaultEvent::Lock`, which
                    // `forward_vault_events` turns into `vault://lock`.
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(error) => eprintln!("wiki vault lock retry failed: {error}"),
                }
            }
        }
    });
    Ok(())
}

fn install_sync(
    app: &tauri::AppHandle,
    state: &DesktopState,
    sync: Arc<VaultSync>,
) -> Result<(), CommandError> {
    locked(&state.sync_scheduler, "sync scheduler")?.take();
    locked(&state.sync_status_forwarding, "sync events")?.take();
    *locked(&state.sync, "vault sync")? = Some(Arc::clone(&sync));
    let mut updates = sync.subscribe();
    let (cancel, mut stop) = TaskCancel::new();
    *locked(&state.sync_status_forwarding, "sync events")? = Some(cancel);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                _ = cancelled(&mut stop) => break,
                status = updates.recv() => match status {
                    Ok(status) if app.emit("sync://status", &status).is_ok() => {},
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        }
    });
    restart_sync_scheduler(state, sync)
}

fn restart_sync_scheduler(state: &DesktopState, sync: Arc<VaultSync>) -> Result<(), CommandError> {
    locked(&state.sync_scheduler, "sync scheduler")?.take();
    let path = state.current_vault_path()?.to_string_lossy().into_owned();
    let settings = locked(&state.settings, "settings")?
        .sync
        .get(&path)
        .cloned()
        .unwrap_or_default();
    if !settings.auto {
        return Ok(());
    }
    let vault = state.active_vault()?;
    let mut vault_events = vault.subscribe();
    let (cancel, mut stop) = TaskCancel::new();
    *locked(&state.sync_scheduler, "sync scheduler")? = Some(cancel);
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(
            settings.interval_min.max(1).saturating_mul(60),
        ));
        interval.tick().await;
        if vault.is_writable() {
            run_sync(Arc::clone(&sync)).await;
        }
        let mut debounce_at = None;
        loop {
            let debounce = async {
                if let Some(deadline) = debounce_at {
                    tokio::time::sleep_until(deadline).await;
                } else {
                    std::future::pending::<()>().await;
                }
            };
            tokio::select! {
                _ = cancelled(&mut stop) => break,
                _ = interval.tick() => {
                    if vault.is_writable() { run_sync(Arc::clone(&sync)).await; }
                }
                _ = debounce => {
                    debounce_at = None;
                    if vault.is_writable() { run_sync(Arc::clone(&sync)).await; }
                }
                event = vault_events.recv() => match event {
                    Ok(wiki_core::VaultEvent::Changed { origin: Origin::Ui | Origin::Mcp, .. }) => {
                        debounce_at = Some(tokio::time::Instant::now() + std::time::Duration::from_secs(30));
                    }
                    Ok(wiki_core::VaultEvent::Lock { writable: true }) if vault.is_writable() => {
                        run_sync(Arc::clone(&sync)).await;
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    });
    Ok(())
}

async fn run_sync(sync: Arc<VaultSync>) {
    let _ = tauri::async_runtime::spawn_blocking(move || sync.sync_now()).await;
}

fn flush_sync_on_exit(state: &DesktopState) {
    let auto = state
        .current_vault_path()
        .ok()
        .and_then(|path| {
            state
                .settings
                .lock()
                .ok()?
                .sync
                .get(&path.to_string_lossy().into_owned())
                .map(|settings| settings.auto)
        })
        .unwrap_or(false);
    if !auto {
        return;
    }
    let Ok(sync) = active_sync(state) else { return };
    let task = tauri::async_runtime::spawn_blocking(move || sync.sync_now());
    let _ = tauri::async_runtime::block_on(tokio::time::timeout(
        std::time::Duration::from_secs(10),
        task,
    ));
}

fn activate_vault(
    app: tauri::AppHandle,
    state: &DesktopState,
    path: String,
) -> Result<String, CommandError> {
    let result = open_vault_state(state, path)?;
    // Sync setup runs git (and may touch the network), so it must never block or fail opening
    // the vault. Vaults without a configured remote are left untouched.
    let vault = state.active_vault()?;
    let sync_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let sync_vault = Arc::clone(&vault);
        let configured = tauri::async_runtime::spawn_blocking(move || {
            match VaultSync::current_remote(&sync_vault) {
                Ok(Some(_)) => configure_open_vault_sync(sync_vault).map(Some),
                Ok(None) => Ok(None),
                Err(error) => Err(error),
            }
        })
        .await;
        let sync = match configured {
            Ok(Ok(Some(sync))) => sync,
            Ok(Ok(None)) => return,
            Ok(Err(error)) => {
                eprintln!("wiki vault sync was not started: {error}");
                return;
            }
            Err(error) => {
                eprintln!("wiki vault sync setup failed: {error}");
                return;
            }
        };
        let state = sync_app.state::<DesktopState>();
        // The user may have switched vaults while git was running.
        if !state
            .active_vault()
            .is_ok_and(|active| Arc::ptr_eq(&active, &vault))
        {
            return;
        }
        if let Err(error) = install_sync(&sync_app, &state, Arc::new(sync)) {
            eprintln!("wiki vault sync could not be installed: {error:?}");
        }
    });
    forward_vault_events(&app, state)?;
    // MCP is optional: a busy or invalid port must never keep the vault closed.
    if let Err(error) = start_mcp_server(app.clone(), state, None) {
        eprintln!("wiki MCP server did not start: {error:?}");
    }
    restart_watcher(state)?;
    restart_lock_retry(&app, state)?;
    Ok(result)
}

fn configure_open_vault_sync(vault: Arc<Vault>) -> Result<VaultSync, SyncError> {
    let branch = VaultSync::current_branch(&vault)
        .ok()
        .flatten()
        .unwrap_or_else(|| "main".into());
    VaultSync::configure(
        vault,
        SyncConfig {
            branch,
            ..SyncConfig::default()
        },
    )
}

fn forward_vault_events(app: &tauri::AppHandle, state: &DesktopState) -> Result<(), CommandError> {
    let mut events = state.active_vault()?.subscribe();
    let sync = locked(&state.sync, "vault sync")?.clone();
    let (cancel, mut stop) = TaskCancel::new();
    *locked(&state.event_forwarding, "event forwarding")? = Some(cancel);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                _ = cancelled(&mut stop) => break,
                event = events.recv() => match event {
                    Ok(wiki_core::VaultEvent::Lock { writable }) => {
                        if !emit_lock(&app, writable) { break; }
                        let status = match (writable, sync.as_ref()) {
                            (true, Some(sync)) => {
                                let sync = Arc::clone(sync);
                                tauri::async_runtime::spawn_blocking(move || sync.refresh_status())
                                    .await
                                    .ok()
                                    .and_then(Result::ok)
                                    .unwrap_or(SyncStatus::Disabled { reason: wiki_core::DisabledReason::NotConfigured })
                            }
                            (true, None) => SyncStatus::Disabled { reason: wiki_core::DisabledReason::NotConfigured },
                            (false, _) => SyncStatus::Disabled { reason: wiki_core::DisabledReason::ReadOnly },
                        };
                        if app.emit("sync://status", &status).is_err() { break; }
                    }
                    Ok(event) => {
                        if app.emit("vault://changed", &event).is_err() { break; }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    });
    Ok(())
}

/// Starts MCP for the open vault. With `port: None` the saved port is tried
/// first and any free port is used if it is taken; an explicit port must bind
/// or the running server is left untouched.
pub(crate) fn start_mcp_server(
    app: tauri::AppHandle,
    state: &DesktopState,
    port: Option<u16>,
) -> Result<(), CommandError> {
    let vault_changed = port.is_none();
    if vault_changed {
        // The old server serves the old vault and may hold the saved port.
        locked(&state.mcp_server, "MCP server")?.take();
    }
    let (saved_port, allow_write) = {
        let settings = locked(&state.settings, "settings")?;
        (settings.mcp_port, settings.mcp_allow_write)
    };
    let vault = state.active_vault()?;
    let vault_path = state.current_vault_path()?;
    let runtime_file = state.settings_file.with_file_name("runtime.json");
    let start = |port| -> Result<_, CommandError> {
        let launch = mcp_server::McpLaunch {
            vault: vault.clone(),
            vault_path: vault_path.clone(),
            port,
            allow_write,
            runtime_file: runtime_file.clone(),
        };
        tauri::async_runtime::block_on(mcp_server::start(app.clone(), launch))
            .map_err(CommandError::io)
    };
    let started = match port {
        Some(port) => start(Some(port)),
        None => start(saved_port).or_else(|error| match saved_port {
            Some(saved) => {
                eprintln!("wiki MCP port {saved} unavailable ({error:?}); using a free port");
                start(None)
            }
            None => Err(error),
        }),
    };
    let runtime = match started {
        Ok(runtime) => runtime,
        Err(error) => {
            if let CommandError::Io { message } = &error {
                *locked(&state.mcp_error, "MCP error")? = Some(message.clone());
            }
            return Err(error);
        }
    };
    let bound = runtime.info().map_err(CommandError::io)?.port;
    locked(&state.settings, "settings")?.mcp_port = Some(bound);
    state.persist()?;
    *locked(&state.mcp_error, "MCP error")? = None;
    *locked(&state.mcp_server, "MCP server")? = Some(runtime);
    Ok(())
}

#[tauri::command]
fn vault_open(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    path: String,
) -> Result<String, CommandError> {
    activate_vault(app, &state, path)
}

/// Async so it runs off the main thread: the native folder dialog needs the
/// main thread, and a sync command blocking it there deadlocks the app.
#[tauri::command]
async fn vault_pick(app: tauri::AppHandle) -> Result<Option<String>, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = app
            .dialog()
            .file()
            .set_title("Choose a Wiki vault")
            .blocking_pick_folder()
        else {
            return Ok(None);
        };
        let path = path.into_path().map_err(CommandError::io)?;
        let state = app.state::<DesktopState>();
        activate_vault(app.clone(), &state, path.to_string_lossy().into_owned()).map(Some)
    })
    .await
    .map_err(CommandError::io)?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VaultSession {
    current: Option<String>,
    recent: Vec<String>,
    writable: bool,
}

/// What the window needs on load: the open vault, recent vaults and whether
/// the vault is writable (so a `vault://lock` emitted during startup is not missed).
#[tauri::command]
fn vault_session(state: State<'_, DesktopState>) -> Result<VaultSession, CommandError> {
    let settings = locked(&state.settings, "settings")?;
    let writable = locked(&state.vault, "vault")?
        .as_ref()
        .is_some_and(|vault| vault.is_writable());
    Ok(VaultSession {
        current: settings.current_vault.clone(),
        recent: settings.recent_vaults.clone(),
        writable,
    })
}

#[tauri::command]
fn vault_tree(state: State<'_, DesktopState>, scope: Scope) -> Result<TreeNode, CommandError> {
    Ok(state.active_vault()?.tree(scope)?)
}
#[tauri::command]
fn vault_read(
    state: State<'_, DesktopState>,
    path: DocPath,
    window: Option<ReadWindow>,
) -> Result<Page, CommandError> {
    Ok(state
        .active_vault()?
        .read(&path, window.unwrap_or_default())?)
}
#[tauri::command]
fn vault_search(
    state: State<'_, DesktopState>,
    query: String,
) -> Result<Vec<wiki_core::Hit>, CommandError> {
    Ok(state.active_vault()?.search(&query)?)
}
#[tauri::command]
fn vault_graph(
    state: State<'_, DesktopState>,
    focus: Option<DocPath>,
) -> Result<Graph, CommandError> {
    Ok(state.active_vault()?.graph(focus.as_ref())?)
}
#[tauri::command]
fn vault_status(state: State<'_, DesktopState>) -> Result<VaultStatus, CommandError> {
    Ok(state.active_vault()?.status()?)
}

#[tauri::command]
fn vault_is_writable(state: State<'_, DesktopState>) -> Result<bool, CommandError> {
    Ok(state.active_vault()?.is_writable())
}

#[tauri::command]
fn vault_apply(state: State<'_, DesktopState>, change: Change) -> Result<Applied, CommandError> {
    Ok(state.active_vault()?.apply_from(Origin::Ui, change)?)
}

#[tauri::command]
fn sync_status(state: State<'_, DesktopState>) -> Result<SyncStatus, SyncError> {
    current_sync_status(&state)
}

fn current_sync_status(state: &DesktopState) -> Result<SyncStatus, SyncError> {
    let vault = state
        .active_vault()
        .map_err(|error| SyncError::Vault(format!("{error:?}")))?;
    if !vault.is_writable() {
        return Ok(SyncStatus::Disabled {
            reason: wiki_core::DisabledReason::ReadOnly,
        });
    }
    let sync = state
        .sync
        .lock()
        .map_err(|_| SyncError::Git("sync state lock poisoned".into()))?;
    let status = sync.as_ref().map_or(
        SyncStatus::Disabled {
            reason: wiki_core::DisabledReason::NotConfigured,
        },
        |sync| sync.status(),
    );
    if matches!(
        &status,
        SyncStatus::Disabled {
            reason: wiki_core::DisabledReason::ReadOnly
        }
    ) {
        return sync
            .as_ref()
            .map_or(Ok(status), |sync| sync.refresh_status());
    }
    Ok(status)
}

#[tauri::command]
async fn sync_now(state: State<'_, DesktopState>) -> Result<SyncReport, SyncError> {
    let sync = active_sync(&state)?;
    tauri::async_runtime::spawn_blocking(move || sync.sync_now())
        .await
        .map_err(|error| SyncError::Git(error.to_string()))?
}

#[tauri::command]
async fn sync_configuration(
    state: State<'_, DesktopState>,
) -> Result<SyncConfigurationView, SyncError> {
    let vault = state
        .active_vault()
        .map_err(|error| SyncError::Vault(format!("{error:?}")))?;
    let path = state
        .current_vault_path()
        .map_err(|error| SyncError::Vault(format!("{error:?}")))?
        .to_string_lossy()
        .into_owned();
    let settings = state
        .settings
        .lock()
        .map_err(|_| SyncError::Git("settings lock poisoned".into()))?
        .sync
        .get(&path)
        .cloned()
        .unwrap_or_default();
    let (remote_url, branch, public_github_remote) =
        tauri::async_runtime::spawn_blocking(move || {
            let root = std::path::PathBuf::from(path);
            let remote = match VaultSync::current_remote(&vault) {
                Ok(value) => value.unwrap_or_default(),
                Err(SyncError::GitMissing) => String::new(),
                Err(SyncError::UnsafeRemote(_)) => String::new(),
                Err(error) => return Err(error),
            };
            let branch = match VaultSync::current_branch(&vault) {
                Ok(value) => value.unwrap_or_else(|| "main".into()),
                Err(SyncError::GitMissing) => "main".into(),
                Err(error) => return Err(error),
            };
            let public_github_remote = public_github_remote_warning(&root);
            Ok((remote, branch, public_github_remote))
        })
        .await
        .map_err(|error| SyncError::Git(error.to_string()))??;
    Ok(SyncConfigurationView {
        remote_url,
        branch,
        auto: settings.auto,
        interval_min: settings.interval_min,
        public_github_remote,
    })
}

fn public_github_remote_warning(root: &std::path::Path) -> bool {
    public_github_remote_warning_with("gh", root)
}

fn public_github_remote_warning_with(
    gh_binary: impl AsRef<std::ffi::OsStr>,
    root: &std::path::Path,
) -> bool {
    use std::{
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    let child = Command::new(gh_binary)
        .args([
            "repo",
            "view",
            "--json",
            "visibility",
            "--jq",
            ".visibility",
        ])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok();
    let Some(mut child) = child else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let Ok(output) = child.wait_with_output() else {
                    return false;
                };
                return String::from_utf8_lossy(&output.stdout).trim() == "PUBLIC";
            }
            Ok(Some(_)) | Err(_) => return false,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

fn active_sync(state: &DesktopState) -> Result<Arc<VaultSync>, SyncError> {
    state
        .sync
        .lock()
        .map_err(|_| SyncError::Git("sync state lock poisoned".into()))?
        .clone()
        .ok_or(SyncError::ReadOnly)
}

#[tauri::command]
async fn sync_configure(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    remote_url: String,
    branch: String,
    auto: bool,
    interval_min: u64,
) -> Result<SyncStatus, SyncError> {
    configure_sync(app, &state, remote_url, branch, auto, interval_min).await
}

async fn configure_sync(
    app: tauri::AppHandle,
    state: &DesktopState,
    remote_url: String,
    branch: String,
    auto: bool,
    interval_min: u64,
) -> Result<SyncStatus, SyncError> {
    let vault = state
        .active_vault()
        .map_err(|error| SyncError::Vault(format!("{error:?}")))?;
    let config = SyncConfig {
        remote_url: Some(remote_url),
        branch,
        ..SyncConfig::default()
    };
    let sync = tauri::async_runtime::spawn_blocking(move || VaultSync::configure(vault, config))
        .await
        .map_err(|error| SyncError::Git(error.to_string()))??;
    let status = sync.status();
    let path = state
        .current_vault_path()
        .map_err(|error| SyncError::Git(format!("{error:?}")))?
        .to_string_lossy()
        .into_owned();
    // Settings must be stored before install_sync: the scheduler reads them when it starts.
    state
        .settings
        .lock()
        .map_err(|_| SyncError::Git("settings lock poisoned".into()))?
        .sync
        .insert(
            path,
            SyncSettings {
                auto,
                interval_min: interval_min.max(1),
            },
        );
    install_sync(&app, state, Arc::new(sync))
        .map_err(|error| SyncError::Git(format!("{error:?}")))?;
    state
        .persist()
        .map_err(|error| SyncError::Git(format!("{error:?}")))?;
    Ok(status)
}

#[tauri::command]
fn sync_disable(state: State<'_, DesktopState>) -> Result<(), CommandError> {
    disable_sync(&state)
}

fn disable_sync(state: &DesktopState) -> Result<(), CommandError> {
    locked(&state.sync_scheduler, "sync scheduler")?.take();
    let path = state.current_vault_path()?.to_string_lossy().into_owned();
    let mut settings = locked(&state.settings, "settings")?;
    let entry = settings.sync.entry(path).or_default();
    entry.auto = false;
    drop(settings);
    state.persist()
}

/// Window labels matching `wiki-*` share the `default` capability with `main`.
fn build_window(app: &tauri::AppHandle, label: &str) -> tauri::Result<tauri::WebviewWindow> {
    use tauri::window::{Effect, EffectState, EffectsBuilder};
    tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App("index.html".into()))
        .title("Wiki")
        .inner_size(1280.0, 820.0)
        .min_inner_size(900.0, 620.0)
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .transparent(true)
        .effects(
            EffectsBuilder::new()
                .effect(Effect::Sidebar)
                .state(EffectState::FollowsWindowActiveState)
                .build(),
        )
        .build()
}

#[tauri::command]
fn window_new(app: tauri::AppHandle) -> Result<String, CommandError> {
    let label = format!("wiki-{}", uuid::Uuid::new_v4().simple());
    build_window(&app, &label).map_err(CommandError::io)?;
    Ok(label)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            build_window(app.handle(), "main")?;
            let mut settings_file = app.path().app_data_dir()?;
            std::fs::create_dir_all(&settings_file)?;
            settings_file.push("settings.json");
            let settings = std::fs::read(&settings_file)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
                .unwrap_or_default();
            let current = settings.current_vault.clone();
            let state = DesktopState {
                vault: Mutex::new(None),
                settings: Mutex::new(settings),
                settings_file,
                event_forwarding: Mutex::new(None),
                mcp_server: Mutex::new(None),
                mcp_error: Mutex::new(None),
                claude_preview: Mutex::new(None),
                watcher: Mutex::new(None),
                lock_retry: Mutex::new(None),
                sync: Mutex::new(None),
                sync_status_forwarding: Mutex::new(None),
                sync_scheduler: Mutex::new(None),
            };
            if let Some(current) = current
                && activate_vault(app.handle().clone(), &state, current).is_err()
                && let Ok(mut settings) = state.settings.lock()
            {
                settings.current_vault = None;
            }
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            vault_open,
            vault_pick,
            vault_session,
            vault_tree,
            vault_read,
            vault_search,
            vault_graph,
            vault_status,
            vault_is_writable,
            vault_apply,
            sync_status,
            sync_configuration,
            sync_now,
            sync_configure,
            sync_disable,
            window_new,
            mcp_commands::mcp_info,
            mcp_commands::mcp_set_allow_write,
            mcp_commands::mcp_set_port,
            mcp_commands::mcp_rotate_token,
            mcp_commands::mcp_claude_config_preview,
            mcp_commands::mcp_claude_config_install
        ])
        .build(tauri::generate_context!())
        .expect("error while building Wiki desktop app")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit)
                && let Some(state) = app.try_state::<DesktopState>()
            {
                if let Ok(mut scheduler) = state.sync_scheduler.lock() {
                    scheduler.take();
                }
                if let Ok(mut events) = state.sync_status_forwarding.lock() {
                    events.take();
                }
                if let Ok(mut runtime) = state.mcp_server.lock()
                    && let Some(runtime) = runtime.take()
                {
                    runtime.shutdown();
                }
                flush_sync_on_exit(&state);
            }
        });
}

#[cfg(test)]
mod sync_settings_tests {
    use super::{Settings, public_github_remote_warning_with};
    use std::{fs, os::unix::fs::PermissionsExt};

    fn fake_gh(output: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("wiki-gh-warning-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let binary = root.join("gh");
        fs::write(&binary, format!("#!/bin/sh\nprintf '%s\\n' '{output}'\n")).unwrap();
        let mut permissions = fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&binary, permissions).unwrap();
        (root, binary)
    }

    #[test]
    fn old_settings_without_sync_still_load() {
        let settings: Settings = serde_json::from_str(
            r#"{"recent_vaults":[],"current_vault":null,"mcp_port":null,"mcp_allow_write":false}"#,
        )
        .expect("legacy settings remain readable");
        assert!(settings.sync.is_empty());
    }

    #[test]
    fn public_github_remote_is_reported_but_private_or_missing_gh_is_silent() {
        let (root, gh) = fake_gh("PUBLIC");
        assert!(public_github_remote_warning_with(&gh, &root));
        fs::remove_dir_all(&root).unwrap();

        let (root, gh) = fake_gh("PRIVATE");
        assert!(!public_github_remote_warning_with(&gh, &root));
        fs::remove_dir_all(&root).unwrap();

        let root = std::env::temp_dir().join(format!("wiki-gh-missing-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        assert!(!public_github_remote_warning_with(
            root.join("missing-gh"),
            &root
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
