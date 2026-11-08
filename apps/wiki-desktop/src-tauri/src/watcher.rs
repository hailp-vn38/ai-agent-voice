use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};
use wiki_core::{DocPath, Scope, Vault};

const DEBOUNCE: Duration = Duration::from_millis(300);
const POLL: Duration = Duration::from_millis(50);

pub struct WatchHandle {
    cancel: mpsc::Sender<()>,
}

impl WatchHandle {
    fn stop(&self) {
        let _ = self.cancel.send(());
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn start(root: PathBuf, vault: Arc<Vault>) -> Result<WatchHandle, String> {
    let (event_sender, event_receiver) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher =
        notify::recommended_watcher(event_sender).map_err(|error| error.to_string())?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|error| error.to_string())?;
    let (cancel, cancel_receiver) = mpsc::channel();

    thread::Builder::new()
        .name("wiki-vault-file-watcher".into())
        .spawn(move || {
            // Keep the platform watcher alive until the vault is switched or app exits.
            let _watcher: RecommendedWatcher = watcher;
            run_debounced(root, vault, event_receiver, cancel_receiver);
        })
        .map_err(|error| error.to_string())?;

    Ok(WatchHandle { cancel })
}

fn run_debounced(
    root: PathBuf,
    vault: Arc<Vault>,
    events: mpsc::Receiver<notify::Result<Event>>,
    cancel: mpsc::Receiver<()>,
) {
    let mut changed = HashSet::<DocPath>::new();
    let mut last_event = None::<Instant>;

    loop {
        if cancel.try_recv().is_ok() {
            break;
        }

        match events.recv_timeout(POLL) {
            Ok(Ok(event)) => {
                for path in event.paths {
                    if let Some(path) = watched_doc_path(&root, &path) {
                        changed.insert(path);
                    }
                }
                if !changed.is_empty() {
                    last_event = Some(Instant::now());
                }
            }
            Ok(Err(error)) => eprintln!("wiki watcher event error: {error}"),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if last_event.is_some_and(|last| last.elapsed() >= DEBOUNCE) {
            let paths = std::mem::take(&mut changed);
            last_event = None;
            if !paths.is_empty()
                && let Err(error) = vault.notice_external(paths)
            {
                eprintln!("wiki watcher index update failed: {error}");
            }
        }
    }
}

fn watched_doc_path(root: &Path, path: &Path) -> Option<DocPath> {
    if ignored_path(path) {
        return None;
    }
    let relative = path.strip_prefix(root).ok()?;
    let mut components = relative.components();
    let scope = match components.next()?.as_os_str().to_str()? {
        "sources" => Scope::Raw,
        "wiki" => Scope::Wiki,
        _ => return None,
    };
    let document = PathBuf::from(components.as_path());
    if scope == Scope::Wiki && matches!(document.to_str()?, "index.md" | "log.md") {
        return None;
    }
    DocPath::parse(scope, document).ok()
}

fn ignored_path(path: &Path) -> bool {
    let Some(file_name) = path.file_name() else {
        return false;
    };
    let name = file_name.to_string_lossy().to_ascii_lowercase();
    name == ".wiki-desktop.lock"
        || name.ends_with(".sqlite")
        || name.ends_with(".sqlite3")
        || name.ends_with(".db")
        || name.contains(".sqlite-")
        || name.ends_with("-journal")
        || name.ends_with(".tmp")
        || name.ends_with(".temp")
        || name.ends_with(".swp")
        || name.ends_with(".swo")
        || name.ends_with(".part")
        || name.ends_with('~')
        || name.starts_with(".#")
        || name.starts_with(".~")
}
