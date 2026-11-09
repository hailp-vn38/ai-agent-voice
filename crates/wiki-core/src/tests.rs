use super::{
    Change, DocPath, Origin, RawFile, Revision, Scope, TextEdit, Vault, VaultConfig, VaultEvent,
};
use llm_wiki::{CollectionScope, WikiEngine};
use std::fs;
use tempfile::tempdir;

#[test]
fn vault_events_serialize_as_lowercase_tagged_payloads() {
    let event = VaultEvent::Changed {
        paths: vec!["wiki/note.md".into()],
        origin: Origin::Ui,
    };
    assert_eq!(
        serde_json::to_value(event).unwrap(),
        serde_json::json!({
            "type": "changed",
            "paths": ["wiki/note.md"],
            "origin": "ui"
        })
    );
}

#[test]
fn opening_an_empty_vault_creates_the_standard_tree() {
    let temp = tempdir().unwrap();
    let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();

    assert!(temp.path().join("sources").is_dir());
    assert!(temp.path().join("wiki").is_dir());
    assert_eq!(vault.tree(Scope::All).unwrap().name, ".");
}

#[test]
fn opening_existing_documents_supports_tree_read_search_and_graph() {
    let temp = tempdir().unwrap();
    let engine = WikiEngine::open(temp.path()).unwrap();
    engine
        .write_file(
            CollectionScope::Wiki,
            "concepts/tri-thuc.md",
            "# Tri thức\n\nHệ thống quản lý tri thức cá nhân.",
        )
        .unwrap();
    engine
        .write_file(
            CollectionScope::Wiki,
            "concepts/agent.md",
            "# Agent\n\nXem [[tri-thuc]].",
        )
        .unwrap();
    drop(engine);

    let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
    let tree = vault.tree(Scope::Wiki).unwrap();
    assert_eq!(tree.name, "wiki");
    assert!(tree.children.iter().any(|node| node.name == "concepts"));

    let path = DocPath::parse(Scope::Wiki, "concepts/tri-thuc.md").unwrap();
    let page = vault.read(&path, Default::default()).unwrap();
    assert!(page.content.contains("Hệ thống quản lý tri thức"));

    let hits = vault.search("tri thuc").unwrap();
    assert!(
        hits.iter()
            .any(|hit| hit.path == "wiki/concepts/tri-thuc.md")
    );

    let graph = vault.graph(None).unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.path == "wiki/concepts/tri-thuc.md")
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.source == "wiki/concepts/agent.md")
    );
    let focused = vault.graph(Some(&path)).unwrap();
    assert_eq!(focused.nodes.len(), 2);
    assert_eq!(focused.edges.len(), 1);
}

#[test]
fn document_paths_reject_traversal_absolute_paths_and_escaping_symlinks() {
    let temp = tempdir().unwrap();
    let outside = tempdir().unwrap();
    fs::write(outside.path().join("secret.md"), "private").unwrap();
    let vault_root = temp.path().join("vault");
    let vault = Vault::open(VaultConfig::new(&vault_root)).unwrap();
    std::os::unix::fs::symlink(outside.path(), vault_root.join("wiki/external")).unwrap();

    assert!(DocPath::parse(Scope::Wiki, "../secret.md").is_err());
    assert!(DocPath::parse(Scope::Wiki, "/etc/passwd").is_err());
    let escaping = DocPath::parse(Scope::Wiki, "external/secret.md").unwrap();
    assert!(vault.read(&escaping, Default::default()).is_err());
    assert!(
        serde_json::from_str::<DocPath>(r#"{"scope":"wiki","relative":"../secret.md"}"#).is_err()
    );
}

#[test]
fn writing_a_page_updates_search_and_publishes_one_change() {
    let temp = tempdir().unwrap();
    let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
    let mut events = vault.subscribe();
    let path = DocPath::parse(Scope::Wiki, "concepts/locking.md").unwrap();

    vault
        .apply_from(
            Origin::Ui,
            Change::Write {
                path: path.clone(),
                content: "# Locking\n\nSingle writer lease behavior.".into(),
                expect: None,
            },
        )
        .unwrap();

    assert!(
        vault
            .read(&path, Default::default())
            .unwrap()
            .content
            .contains("Single writer")
    );
    assert!(
        vault
            .search("single writer")
            .unwrap()
            .iter()
            .any(|hit| hit.path == "wiki/concepts/locking.md")
    );
    let event = events.try_recv().unwrap();
    assert!(matches!(
        event,
        super::VaultEvent::Changed {
            origin: Origin::Ui,
            ..
        }
    ));
}

#[test]
fn writing_with_a_stale_revision_leaves_the_page_unchanged() {
    let temp = tempdir().unwrap();
    let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
    let path = DocPath::parse(Scope::Wiki, "concepts/revision.md").unwrap();
    vault
        .apply(Change::Write {
            path: path.clone(),
            content: "# Current\n\nKeep this content.".into(),
            expect: None,
        })
        .unwrap();

    let result = vault.apply(Change::Write {
        path: path.clone(),
        content: "# Stale\n\nMust not overwrite.".into(),
        expect: Some(Revision("stale-revision".into())),
    });

    assert!(matches!(result, Err(super::VaultError::Conflict { .. })));
    assert!(
        vault
            .read(&path, Default::default())
            .unwrap()
            .content
            .contains("Keep this content")
    );
}

#[test]
fn patch_move_delete_import_ingest_and_reindex_work_through_apply() {
    let temp = tempdir().unwrap();
    let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
    let source = DocPath::parse(Scope::Raw, "notes/first.md").unwrap();
    let page = DocPath::parse(Scope::Wiki, "concepts/first.md").unwrap();
    let moved = DocPath::parse(Scope::Wiki, "concepts/renamed.md").unwrap();

    vault
        .apply(Change::Write {
            path: page.clone(),
            content: "# First\n\nOld body.".into(),
            expect: None,
        })
        .unwrap();
    let revision = vault.read(&page, Default::default()).unwrap().revision;
    vault
        .apply(Change::Patch {
            path: page.clone(),
            edits: vec![TextEdit {
                start_line: 2,
                delete_lines: 1,
                insert: "New body.".into(),
            }],
            expect: Some(revision),
        })
        .unwrap();
    assert!(
        vault
            .read(&page, Default::default())
            .unwrap()
            .content
            .contains("New body")
    );

    vault
        .apply(Change::Move {
            from: page.clone(),
            to: moved.clone(),
        })
        .unwrap();
    assert!(vault.read(&moved, Default::default()).is_ok());
    assert!(matches!(
        vault.read(&page, Default::default()),
        Err(super::VaultError::NotFound(_))
    ));

    vault
        .apply(Change::ImportRaw {
            files: vec![RawFile {
                path: source.clone(),
                content: "# Source\n\nImported raw note for the wiki.".into(),
            }],
        })
        .unwrap();
    vault
        .apply(Change::IngestSource {
            source: source.clone(),
            details: None,
        })
        .unwrap();
    assert!(
        vault
            .search("Imported raw note")
            .unwrap()
            .iter()
            .any(|hit| hit.path.starts_with("wiki/"))
    );

    vault.apply(Change::Reindex { force: true }).unwrap();
    vault
        .apply(Change::Delete {
            path: moved.clone(),
        })
        .unwrap();
    assert!(matches!(
        vault.read(&moved, Default::default()),
        Err(super::VaultError::NotFound(_))
    ));
}

#[test]
fn concurrent_writes_with_the_same_revision_do_not_lose_an_update() {
    let temp = tempdir().unwrap();
    let vault = std::sync::Arc::new(Vault::open(VaultConfig::new(temp.path())).unwrap());
    let path = DocPath::parse(Scope::Wiki, "concepts/shared.md").unwrap();
    vault
        .apply(Change::Write {
            path: path.clone(),
            content: "# Original".into(),
            expect: None,
        })
        .unwrap();
    let revision = vault.read(&path, Default::default()).unwrap().revision;
    let mut workers = Vec::new();
    for body in ["# First", "# Second"] {
        let vault = vault.clone();
        let path = path.clone();
        let revision = revision.clone();
        workers.push(std::thread::spawn(move || {
            vault.apply(Change::Write {
                path,
                content: body.into(),
                expect: Some(revision),
            })
        }));
    }
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(super::VaultError::Conflict { .. })))
            .count(),
        1
    );
}

#[test]
fn failed_write_restores_files_and_does_not_publish_change() {
    let temp = tempdir().unwrap();
    let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
    let blocker = DocPath::parse(Scope::Wiki, "blocked.md").unwrap();
    vault
        .apply(Change::Write {
            path: blocker,
            content: "not a directory".into(),
            expect: None,
        })
        .unwrap();
    let mut events = vault.subscribe();
    let nested = DocPath::parse(Scope::Wiki, "blocked.md/child.md").unwrap();

    assert!(
        vault
            .apply(Change::Write {
                path: nested,
                content: "# Never committed".into(),
                expect: None,
            })
            .is_err()
    );
    assert!(events.try_recv().is_err());
    assert!(!temp.path().join("wiki/blocked.md/child.md").exists());
    assert_eq!(
        std::fs::read_dir(temp.path().join("wiki"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("tmp"))
            .count(),
        0
    );

    let first = DocPath::parse(Scope::Raw, "batch/first.md").unwrap();
    let second = DocPath::parse(Scope::Raw, "blocked.md/second.md").unwrap();
    fs::write(temp.path().join("sources/blocked.md"), "not a directory").unwrap();
    assert!(
        vault
            .apply(Change::ImportRaw {
                files: vec![
                    RawFile {
                        path: first.clone(),
                        content: "# First batch file".into(),
                    },
                    RawFile {
                        path: second,
                        content: "# Second batch file".into(),
                    },
                ],
            })
            .is_err()
    );
    assert!(!temp.path().join(first.vault_relative()).exists());
    assert!(!temp.path().join("sources/batch").exists());
    assert!(vault.search("First batch file").unwrap().is_empty());
    assert!(events.try_recv().is_err());
}

#[test]
fn moving_a_page_with_real_incoming_wiki_links_is_rejected() {
    let temp = tempdir().unwrap();
    let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
    let page = DocPath::parse(Scope::Wiki, "concepts/linked.md").unwrap();
    let referrer = DocPath::parse(Scope::Wiki, "concepts/referrer.md").unwrap();
    let moved = DocPath::parse(Scope::Wiki, "concepts/renamed.md").unwrap();
    for (path, content) in [
        (page.clone(), "# Linked page"),
        (referrer, "# Referrer\n\nSee [[linked]]."),
    ] {
        vault
            .apply(Change::Write {
                path,
                content: content.into(),
                expect: None,
            })
            .unwrap();
    }

    assert!(matches!(
        vault.apply(Change::Move {
            from: page,
            to: moved
        }),
        Err(super::VaultError::InvalidPath(_))
    ));
}

#[test]
fn a_second_vault_is_read_only_until_the_current_writer_releases_the_lock() {
    let temp = tempdir().unwrap();
    let first = Vault::open(VaultConfig::new(temp.path())).unwrap();
    let second = Vault::open(VaultConfig::new(temp.path())).unwrap();
    let path = DocPath::parse(Scope::Wiki, "lock/read-only.md").unwrap();
    let mut events = second.subscribe();

    assert!(first.is_writable());
    assert!(!second.is_writable());
    assert!(matches!(
        second.apply(Change::Write {
            path,
            content: "# Must not write".into(),
            expect: None,
        }),
        Err(super::VaultError::ReadOnly)
    ));
    drop(first);
    assert!(second.try_acquire_write().unwrap());
    assert!(second.is_writable());
    assert!(matches!(
        events.try_recv().unwrap(),
        super::VaultEvent::Lock { writable: true }
    ));
}

#[test]
fn a_reader_vault_leaves_the_shared_index_and_scaffold_untouched() {
    let temp = tempdir().unwrap();
    let writer = Vault::open(VaultConfig::new(temp.path())).unwrap();
    let path = DocPath::parse(Scope::Wiki, "concepts/shared.md").unwrap();
    writer
        .apply(Change::Write {
            path: path.clone(),
            content: "# Shared\n\nVisible to readers.".into(),
            expect: None,
        })
        .unwrap();
    let watched = [
        "wiki/index.md",
        "wiki/log.md",
        "SCHEMA.md",
        ".wiki/index.sqlite",
    ];
    let before: Vec<_> = watched
        .iter()
        .map(|file| {
            fs::metadata(temp.path().join(file))
                .unwrap()
                .modified()
                .unwrap()
        })
        .collect();
    fs::write(temp.path().join("wiki/concepts/on-disk.md"), "# On disk\n").unwrap();

    let reader = Vault::open(VaultConfig::new(temp.path())).unwrap();

    assert!(!reader.is_writable());
    assert!(
        reader
            .read(&path, Default::default())
            .unwrap()
            .content
            .contains("Visible to readers.")
    );
    let after: Vec<_> = watched
        .iter()
        .map(|file| {
            fs::metadata(temp.path().join(file))
                .unwrap()
                .modified()
                .unwrap()
        })
        .collect();
    assert_eq!(before, after);
    drop(writer);
    assert!(reader.try_acquire_write().unwrap());
    reader
        .apply(Change::Write {
            path: DocPath::parse(Scope::Wiki, "concepts/after-handoff.md").unwrap(),
            content: "# After handoff\n".into(),
            expect: None,
        })
        .unwrap();
}

#[test]
fn a_reader_refuses_an_uninitialized_locked_vault() {
    let temp = tempdir().unwrap();
    let _lock = Vault::open(VaultConfig::new(temp.path())).unwrap();
    fs::remove_file(temp.path().join("SCHEMA.md")).unwrap();

    assert!(Vault::open(VaultConfig::new(temp.path())).is_err());
    assert!(!temp.path().join("SCHEMA.md").exists());
}

#[test]
fn move_refuses_to_overwrite_an_existing_page() {
    let temp = tempdir().unwrap();
    let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
    let from = DocPath::parse(Scope::Wiki, "concepts/from.md").unwrap();
    let to = DocPath::parse(Scope::Wiki, "concepts/to.md").unwrap();
    for (path, content) in [(&from, "# From\n"), (&to, "# To\n")] {
        vault
            .apply(Change::Write {
                path: path.clone(),
                content: content.into(),
                expect: None,
            })
            .unwrap();
    }

    assert!(matches!(
        vault.apply(Change::Move {
            from,
            to: to.clone()
        }),
        Err(super::VaultError::InvalidPath(_))
    ));
    assert_eq!(
        fs::read_to_string(temp.path().join("wiki/concepts/to.md")).unwrap(),
        "# To\n"
    );
}

#[test]
fn subprocess_vault_lock_holder() {
    let Some(root) = std::env::var_os("WIKI_CORE_LOCK_CHILD_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let vault = Vault::open(VaultConfig::new(&root)).unwrap();
    assert!(vault.is_writable());
    fs::write(root.join("child-ready"), "ready").unwrap();
    for _ in 0..500 {
        if root.join("child-release").exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("parent did not release the subprocess lock holder");
}

#[test]
fn a_separate_process_holding_the_lock_forces_read_only_mode() {
    let temp = tempdir().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("tests::subprocess_vault_lock_holder")
        .arg("--nocapture")
        .env("WIKI_CORE_LOCK_CHILD_ROOT", temp.path())
        .spawn()
        .unwrap();
    let ready = temp.path().join("child-ready");
    for _ in 0..500 {
        if ready.exists() {
            break;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "lock holder exited before acquiring lock"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(ready.exists(), "subprocess never acquired the vault lock");
    let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
    assert!(!vault.is_writable());
    fs::write(temp.path().join("child-release"), "release").unwrap();
    assert!(child.wait().unwrap().success());
    assert!(vault.try_acquire_write().unwrap());
    assert!(vault.is_writable());
}

#[test]
fn external_changes_refresh_search_and_app_writes_are_suppressed() {
    let temp = tempdir().unwrap();
    let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
    let path = DocPath::parse(Scope::Wiki, "concepts/external.md").unwrap();
    let mut events = vault.subscribe();
    vault
        .apply(Change::Write {
            path: path.clone(),
            content: "# Original\n\nWritten by this vault.".into(),
            expect: None,
        })
        .unwrap();
    let _ = events.try_recv().unwrap();
    assert!(!vault.notice_external([path.clone()]).unwrap());
    assert!(!vault.notice_external([path.clone()]).unwrap());
    assert!(events.try_recv().is_err());

    fs::write(
        temp.path().join(path.vault_relative()),
        "# External\n\nChanged by another application.",
    )
    .unwrap();
    assert!(vault.notice_external([path]).unwrap());
    assert!(
        vault
            .search("another application")
            .unwrap()
            .iter()
            .any(|hit| hit.path == "wiki/concepts/external.md")
    );
    assert!(matches!(
        events.try_recv().unwrap(),
        super::VaultEvent::External { .. }
    ));
}

#[test]
fn a_read_only_vault_reports_external_changes_without_rewriting_the_index() {
    let temp = tempdir().unwrap();
    let writer = Vault::open(VaultConfig::new(temp.path())).unwrap();
    let reader = Vault::open(VaultConfig::new(temp.path())).unwrap();
    assert!(!reader.is_writable());
    let mut events = reader.subscribe();
    let index_md = temp.path().join("wiki/index.md");
    let index_before = fs::read(&index_md).ok();

    let path = DocPath::parse(Scope::Wiki, "concepts/edited-elsewhere.md").unwrap();
    let file = temp.path().join(path.vault_relative());
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(
        &file,
        "# Elsewhere\n\nEdited while another process holds the lock.",
    )
    .unwrap();

    assert!(reader.notice_external([path]).unwrap());
    assert!(matches!(
        events.try_recv().unwrap(),
        super::VaultEvent::External { .. }
    ));
    assert!(
        reader
            .search("another process holds")
            .unwrap()
            .iter()
            .any(|hit| hit.path == "wiki/concepts/edited-elsewhere.md")
    );
    assert_eq!(fs::read(&index_md).ok(), index_before);
    drop(writer);
}

#[test]
fn exclusive_external_refreshes_documents_and_publishes_one_event() {
    let temp = tempdir().unwrap();
    let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
    let mut events = vault.subscribe();

    let paths = vault
        .exclusive_external(|root| {
            fs::create_dir_all(root.join("wiki/concepts")).unwrap();
            fs::write(
                root.join("wiki/concepts/first-sync.md"),
                "# First sync\n\nQuartz knowledge entry.",
            )
            .unwrap();
            fs::write(
                root.join("wiki/concepts/second-sync.md"),
                "# Second sync\n\nObsidian knowledge entry.",
            )
            .unwrap();
            Ok(vec![
                "wiki/concepts/first-sync.md".into(),
                "wiki/concepts/second-sync.md".into(),
                "README.md".into(),
            ])
        })
        .unwrap();

    assert_eq!(
        paths
            .iter()
            .map(|path| path.vault_relative().display().to_string())
            .collect::<Vec<_>>(),
        [
            "wiki/concepts/first-sync.md",
            "wiki/concepts/second-sync.md"
        ]
    );
    for path in [
        DocPath::parse(Scope::Wiki, "concepts/first-sync.md").unwrap(),
        DocPath::parse(Scope::Wiki, "concepts/second-sync.md").unwrap(),
    ] {
        assert!(vault.read(&path, Default::default()).is_ok());
    }
    assert!(
        vault
            .search("quartz knowledge")
            .unwrap()
            .iter()
            .any(|hit| hit.path == "wiki/concepts/first-sync.md")
    );
    assert!(
        vault
            .search("obsidian knowledge")
            .unwrap()
            .iter()
            .any(|hit| hit.path == "wiki/concepts/second-sync.md")
    );
    assert!(matches!(
        events.try_recv().unwrap(),
        VaultEvent::External { paths }
            if paths == [
                "wiki/concepts/first-sync.md",
                "wiki/concepts/second-sync.md"
            ]
    ));
    assert!(events.try_recv().is_err());
}

#[test]
fn exclusive_external_rejects_a_reader_without_running_the_closure() {
    let temp = tempdir().unwrap();
    let _writer = Vault::open(VaultConfig::new(temp.path())).unwrap();
    let reader = Vault::open(VaultConfig::new(temp.path())).unwrap();
    let closure_ran = std::sync::atomic::AtomicBool::new(false);

    let result = reader.exclusive_external(|_| {
        closure_ran.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(Vec::new())
    });

    assert!(matches!(result, Err(super::VaultError::ReadOnly)));
    assert!(!closure_ran.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn exclusive_external_refreshes_the_index_even_when_the_closure_fails() {
    let temp = tempdir().unwrap();
    let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
    let result = vault.exclusive_external(|root| {
        fs::create_dir_all(root.join("wiki/concepts")).unwrap();
        fs::write(
            root.join("wiki/concepts/partial-sync.md"),
            "# Partial sync\n\nIndex this partial update.",
        )
        .unwrap();
        Err(super::VaultError::Io("simulated sync failure".into()))
    });

    assert!(
        matches!(result, Err(super::VaultError::Io(message)) if message == "simulated sync failure")
    );
    assert!(
        vault
            .search("partial update")
            .unwrap()
            .iter()
            .any(|hit| hit.path == "wiki/concepts/partial-sync.md")
    );
}

#[test]
fn apply_waits_until_exclusive_external_finishes() {
    use std::sync::mpsc;
    use std::time::Duration;

    let temp = tempdir().unwrap();
    let vault = std::sync::Arc::new(Vault::open(VaultConfig::new(temp.path())).unwrap());
    let path = DocPath::parse(Scope::Wiki, "concepts/after-sync.md").unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (apply_done_tx, apply_done_rx) = mpsc::channel();

    let sync_vault = std::sync::Arc::clone(&vault);
    let sync_thread = std::thread::spawn(move || {
        sync_vault
            .exclusive_external(|root| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                fs::create_dir_all(root.join("wiki/concepts")).unwrap();
                fs::write(root.join("wiki/concepts/external.md"), "# External").unwrap();
                Ok(vec!["wiki/concepts/external.md".into()])
            })
            .unwrap();
    });
    entered_rx.recv().unwrap();

    let apply_vault = std::sync::Arc::clone(&vault);
    let apply_thread = std::thread::spawn(move || {
        let result = apply_vault.apply(Change::Write {
            path,
            content: "# After sync".into(),
            expect: None,
        });
        apply_done_tx.send(result.is_ok()).unwrap();
    });

    assert!(
        apply_done_rx
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );
    release_tx.send(()).unwrap();
    sync_thread.join().unwrap();
    assert!(apply_done_rx.recv_timeout(Duration::from_secs(2)).unwrap());
    apply_thread.join().unwrap();
}
