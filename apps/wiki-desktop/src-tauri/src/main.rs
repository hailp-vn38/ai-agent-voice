use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use wiki_core::{
    Applied, Change, DocPath, Graph, Origin, Page, ReadWindow, Scope, TreeNode, Vault, VaultConfig,
    VaultError, VaultStatus,
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

fn activate_vault(
    app: tauri::AppHandle,
    state: &DesktopState,
    path: String,
) -> Result<String, CommandError> {
    let result = open_vault_state(state, path)?;
    forward_vault_events(&app, state)?;
    // MCP is optional: a busy or invalid port must never keep the vault closed.
    if let Err(error) = start_mcp_server(app.clone(), state, None) {
        eprintln!("wiki MCP server did not start: {error:?}");
    }
    restart_watcher(state)?;
    restart_lock_retry(&app, state)?;
    Ok(result)
}

fn forward_vault_events(app: &tauri::AppHandle, state: &DesktopState) -> Result<(), CommandError> {
    let mut events = state.active_vault()?.subscribe();
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

#[tauri::command]
fn vault_pick(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
) -> Result<Option<String>, CommandError> {
    let Some(path) = app
        .dialog()
        .file()
        .set_title("Choose a Wiki vault")
        .blocking_pick_folder()
    else {
        return Ok(None);
    };
    let path = path.into_path().map_err(CommandError::io)?;
    activate_vault(app, &state, path.to_string_lossy().into_owned()).map(Some)
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
                && let Ok(mut runtime) = state.mcp_server.lock()
                && let Some(runtime) = runtime.take()
            {
                runtime.shutdown();
            }
        });
}
