use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use wiki_core::{
    Applied, Change, DocPath, Graph, Origin, Page, ReadWindow, Scope, TreeNode, Vault, VaultConfig,
    VaultError, VaultStatus,
};

mod mcp_commands;
mod mcp_server;
mod watcher;

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Settings {
    pub(crate) recent_vaults: Vec<String>,
    pub(crate) current_vault: Option<String>,
    #[serde(default)]
    pub(crate) mcp_port: Option<u16>,
    #[serde(default)]
    pub(crate) mcp_allow_write: bool,
}

pub(crate) struct DesktopState {
    pub(crate) vault: Mutex<Option<Arc<Vault>>>,
    pub(crate) settings: Mutex<Settings>,
    pub(crate) settings_file: PathBuf,
    event_cancel: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
    pub(crate) mcp_server: Mutex<Option<mcp_server::McpServerRuntime>>,
    watcher: Mutex<Option<watcher::WatchHandle>>,
    lock_retry_cancel: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub(crate) enum CommandError {
    NotFound { message: String },
    InvalidPath { message: String },
    Conflict { message: String, current: String },
    ReadOnly { message: String },
    Io { message: String },
    Index { message: String },
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

impl DesktopState {
    fn persist(&self) -> Result<(), CommandError> {
        let settings = self.settings.lock().map_err(|_| CommandError::Io {
            message: "settings lock poisoned".into(),
        })?;
        let json = serde_json::to_vec_pretty(&*settings).map_err(|error| CommandError::Io {
            message: error.to_string(),
        })?;
        if let Some(parent) = self.settings_file.parent() {
            std::fs::create_dir_all(parent).map_err(|error| CommandError::Io {
                message: error.to_string(),
            })?;
        }
        std::fs::write(&self.settings_file, json).map_err(|error| CommandError::Io {
            message: error.to_string(),
        })
    }

    fn active_vault(&self) -> Result<Arc<Vault>, CommandError> {
        self.vault
            .lock()
            .map_err(|_| CommandError::Io {
                message: "vault state poisoned".into(),
            })?
            .clone()
            .ok_or_else(|| CommandError::NotFound {
                message: "No knowledge vault is open".into(),
            })
    }
}

fn open_vault_state(state: &DesktopState, path: String) -> Result<String, CommandError> {
    let root = PathBuf::from(&path)
        .canonicalize()
        .map_err(|error| CommandError::Io {
            message: error.to_string(),
        })?;
    let canonical = root.to_string_lossy().into_owned();
    let already_open = state
        .settings
        .lock()
        .map_err(|_| CommandError::Io {
            message: "settings lock poisoned".into(),
        })?
        .current_vault
        .as_deref()
        == Some(canonical.as_str());
    if already_open && state.vault.lock().ok().is_some_and(|vault| vault.is_some()) {
        return Ok(canonical);
    }
    let vault = Arc::new(Vault::open(VaultConfig::new(&root))?);
    *state.vault.lock().map_err(|_| CommandError::Io {
        message: "vault state poisoned".into(),
    })? = Some(vault);
    {
        let mut settings = state.settings.lock().map_err(|_| CommandError::Io {
            message: "settings lock poisoned".into(),
        })?;
        let path = root.to_string_lossy().into_owned();
        settings.recent_vaults.retain(|recent| recent != &path);
        settings.recent_vaults.insert(0, path.clone());
        settings.recent_vaults.truncate(12);
        settings.current_vault = Some(path.clone());
    }
    state.persist()?;
    Ok(root.to_string_lossy().into_owned())
}

fn restart_watcher(state: &DesktopState) -> Result<(), CommandError> {
    if let Some(previous) = state
        .watcher
        .lock()
        .map_err(|_| CommandError::Io {
            message: "file watcher lock poisoned".into(),
        })?
        .take()
    {
        drop(previous);
    }
    let root = state
        .settings
        .lock()
        .map_err(|_| CommandError::Io {
            message: "settings lock poisoned".into(),
        })?
        .current_vault
        .as_ref()
        .map(PathBuf::from)
        .ok_or_else(|| CommandError::NotFound {
            message: "No knowledge vault is open".into(),
        })?;
    let vault = state.active_vault()?;
    let handle = watcher::start(root, vault).map_err(|error| CommandError::Io {
        message: error.to_string(),
    })?;
    *state.watcher.lock().map_err(|_| CommandError::Io {
        message: "file watcher lock poisoned".into(),
    })? = Some(handle);
    Ok(())
}

fn restart_lock_retry(app: &tauri::AppHandle, state: &DesktopState) -> Result<(), CommandError> {
    if let Some(previous) = state
        .lock_retry_cancel
        .lock()
        .map_err(|_| CommandError::Io {
            message: "vault lock retry handle poisoned".into(),
        })?
        .take()
    {
        let _ = previous.send(true);
    }
    let vault = state.active_vault()?;
    let _ = app.emit(
        "vault://lock",
        serde_json::json!({"writable": vault.is_writable()}),
    );
    if vault.is_writable() {
        return Ok(());
    }
    let (cancel, mut cancelled) = tokio::sync::watch::channel(false);
    *state
        .lock_retry_cancel
        .lock()
        .map_err(|_| CommandError::Io {
            message: "vault lock retry handle poisoned".into(),
        })? = Some(cancel);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.tick().await;
        loop {
            tokio::select! {
                changed = cancelled.changed() => {
                    if changed.is_err() || *cancelled.borrow() { break; }
                }
                _ = interval.tick() => match vault.try_acquire_write() {
                    Ok(true) => {
                        let _ = app.emit("vault://lock", serde_json::json!({"writable": true}));
                        break;
                    }
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
    start_mcp_server(app.clone(), state)?;
    restart_watcher(state)?;
    restart_lock_retry(&app, state)?;
    Ok(result)
}

fn forward_vault_events(app: &tauri::AppHandle, state: &DesktopState) -> Result<(), CommandError> {
    let vault = state.active_vault()?;
    let events = vault.subscribe();
    let (cancel, mut cancelled) = tokio::sync::watch::channel(false);
    if let Some(previous) = state
        .event_cancel
        .lock()
        .map_err(|_| CommandError::Io {
            message: "event forwarding lock poisoned".into(),
        })?
        .replace(cancel)
    {
        let _ = previous.send(true);
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut events = events;
        loop {
            tokio::select! {
                changed = cancelled.changed() => {
                    if changed.is_err() || *cancelled.borrow() {
                        break;
                    }
                }
                event = events.recv() => match event {
                    Ok(event) => {
                        if let wiki_core::VaultEvent::Lock { writable } = event {
                            if app.emit("vault://lock", serde_json::json!({"writable": writable})).is_err() { break; }
                        } else if app.emit("vault://changed", &event).is_err() { break; }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    });
    Ok(())
}

fn start_mcp_server(app: tauri::AppHandle, state: &DesktopState) -> Result<(), CommandError> {
    if let Some(previous) = state
        .mcp_server
        .lock()
        .map_err(|_| CommandError::Io {
            message: "MCP server lock poisoned".into(),
        })?
        .take()
    {
        previous.shutdown();
    }
    let (port, allow_write, vault_path) = {
        let settings = state.settings.lock().map_err(|_| CommandError::Io {
            message: "settings lock poisoned".into(),
        })?;
        let vault_path = settings
            .current_vault
            .as_ref()
            .map(PathBuf::from)
            .ok_or_else(|| CommandError::NotFound {
                message: "No knowledge vault is open".into(),
            })?;
        (settings.mcp_port, settings.mcp_allow_write, vault_path)
    };
    let runtime_file = state.settings_file.with_file_name("runtime.json");
    let runtime = tauri::async_runtime::block_on(mcp_server::start(
        app,
        state.active_vault()?,
        port,
        allow_write,
        runtime_file,
        vault_path,
    ))
    .map_err(|message| CommandError::Io { message })?;
    let info = runtime
        .info()
        .map_err(|message| CommandError::Io { message })?;
    {
        let mut settings = state.settings.lock().map_err(|_| CommandError::Io {
            message: "settings lock poisoned".into(),
        })?;
        settings.mcp_port = Some(info.port);
    }
    state.persist()?;
    *state.mcp_server.lock().map_err(|_| CommandError::Io {
        message: "MCP server lock poisoned".into(),
    })? = Some(runtime);
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
    let path = path.into_path().map_err(|error| CommandError::Io {
        message: error.to_string(),
    })?;
    Ok(Some(activate_vault(
        app,
        &state,
        path.to_string_lossy().into_owned(),
    )?))
}

#[tauri::command]
fn vault_recent(state: State<'_, DesktopState>) -> Result<Vec<String>, CommandError> {
    Ok(state
        .settings
        .lock()
        .map_err(|_| CommandError::Io {
            message: "settings lock poisoned".into(),
        })?
        .recent_vaults
        .clone())
}

#[tauri::command]
fn vault_current(state: State<'_, DesktopState>) -> Result<Option<String>, CommandError> {
    Ok(state
        .settings
        .lock()
        .map_err(|_| CommandError::Io {
            message: "settings lock poisoned".into(),
        })?
        .current_vault
        .clone())
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

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let mut settings_file = app.path().app_data_dir()?;
            std::fs::create_dir_all(&settings_file)?;
            settings_file.push("settings.json");
            let settings = std::fs::read(&settings_file)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
                .unwrap_or_default();
            let state = DesktopState {
                vault: Mutex::new(None),
                settings: Mutex::new(settings),
                settings_file,
                event_cancel: Mutex::new(None),
                mcp_server: Mutex::new(None),
                watcher: Mutex::new(None),
                lock_retry_cancel: Mutex::new(None),
            };
            if let Some(current) = state
                .settings
                .lock()
                .ok()
                .and_then(|settings| settings.current_vault.clone())
            {
                if activate_vault(app.handle().clone(), &state, current).is_err() {
                    if let Ok(mut settings) = state.settings.lock() {
                        settings.current_vault = None;
                    }
                }
            }
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            vault_open,
            vault_pick,
            vault_recent,
            vault_current,
            vault_tree,
            vault_read,
            vault_search,
            vault_graph,
            vault_status,
            vault_is_writable,
            vault_apply,
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
            if matches!(event, tauri::RunEvent::Exit) {
                if let Some(state) = app.try_state::<DesktopState>()
                    && let Ok(mut runtime) = state.mcp_server.lock()
                    && let Some(runtime) = runtime.take()
                {
                    runtime.shutdown();
                }
            }
        });
}
