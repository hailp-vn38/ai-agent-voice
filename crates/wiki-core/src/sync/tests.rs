use super::git::{ssh_command, ssh_environment};
use super::*;
use crate::{Change, Origin, ReadWindow};
use std::io::Write;
use std::{
    path::Path,
    process::{Command, Output},
};

fn git(root: &Path, args: &[&str]) -> Output {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git is installed for this test");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn config(remote: String) -> SyncConfig {
    SyncConfig {
        remote_url: Some(remote),
        author: Some(GitAuthor {
            name: "Wiki Test".into(),
            email: "wiki-test@example.invalid".into(),
        }),
        ..SyncConfig::default()
    }
}

fn write_page(vault: &Vault, path: &str, content: &str) {
    vault
        .apply_from(
            Origin::Ui,
            Change::Write {
                path: DocPath::parse(Scope::Wiki, path).unwrap(),
                content: content.into(),
                expect: None,
            },
        )
        .unwrap();
}

fn new_bare_remote(root: &Path) -> PathBuf {
    let remote = root.join("remote.git");
    fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "--bare", "-b", "main"]);
    remote
}

fn skip_without_git() -> bool {
    if Command::new("git").arg("--version").output().is_ok() {
        return false;
    }
    println!("SKIPPED: git is not installed");
    true
}

fn assert_clean(root: &Path) {
    let status = git(root, &["status", "--porcelain"]);
    assert!(
        status.stdout.is_empty(),
        "repository should be clean after sync: {}",
        String::from_utf8_lossy(&status.stdout)
    );
}

#[test]
fn two_vaults_exchange_markdown_and_publish_external_event() {
    if skip_without_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let remote = new_bare_remote(temp.path());
    let remote_url = remote.to_string_lossy().into_owned();
    let vault_a =
        Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault-a"))).unwrap());
    let sync_a = VaultSync::configure(Arc::clone(&vault_a), config(remote_url.clone())).unwrap();
    sync_a.sync_now().unwrap();

    let b_root = temp.path().join("vault-b");
    fs::create_dir_all(&b_root).unwrap();
    git(&b_root, &["clone", &remote_url, "."]);
    let vault_b = Arc::new(Vault::open(crate::VaultConfig::new(&b_root)).unwrap());
    let sync_b = VaultSync::configure(Arc::clone(&vault_b), config(remote_url)).unwrap();
    let mut statuses = sync_b.subscribe();
    let mut events = vault_b.subscribe();
    write_page(
        &vault_a,
        "remote-page.md",
        "# Remote Page\nneedle from another computer\n",
    );
    sync_a.sync_now().unwrap();
    let message =
        String::from_utf8(git(vault_a.root_path(), &["log", "-1", "--format=%B"]).stdout).unwrap();
    assert!(message.contains("Origin: ui"));

    let report = sync_b.sync_now().unwrap();
    assert!(matches!(statuses.try_recv(), Ok(SyncStatus::Syncing)));
    assert!(matches!(statuses.try_recv(), Ok(SyncStatus::Idle { .. })));
    let expected = DocPath::parse(Scope::Wiki, "remote-page.md").unwrap();
    assert!(report.pulled.contains(&expected));
    assert_eq!(
        vault_b
            .read(&expected, ReadWindow::default())
            .unwrap()
            .content,
        "# Remote Page\nneedle from another computer"
    );
    assert!(
        vault_b
            .search("needle")
            .unwrap()
            .iter()
            .any(|hit| hit.path == "wiki/remote-page.md")
    );
    let external = loop {
        match events.try_recv() {
            Ok(VaultEvent::External { paths }) => break paths,
            Ok(_) => continue,
            Err(error) => panic!("expected External event, got {error:?}"),
        }
    };
    assert!(external.iter().any(|path| path == "wiki/remote-page.md"));
}

#[test]
fn concurrent_page_edits_preserve_local_and_create_remote_conflict_copy() {
    if skip_without_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let remote = new_bare_remote(temp.path());
    let remote_url = remote.to_string_lossy().into_owned();
    let vault_a =
        Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault-a"))).unwrap());
    let sync_a = VaultSync::configure(Arc::clone(&vault_a), config(remote_url.clone())).unwrap();
    write_page(&vault_a, "shared.md", "# Shared\nbase\n");
    sync_a.sync_now().unwrap();

    let b_root = temp.path().join("vault-b");
    fs::create_dir_all(&b_root).unwrap();
    git(&b_root, &["clone", &remote_url, "."]);
    let vault_b = Arc::new(Vault::open(crate::VaultConfig::new(&b_root)).unwrap());
    let sync_b = VaultSync::configure(Arc::clone(&vault_b), config(remote_url)).unwrap();

    write_page(&vault_a, "shared.md", "# Shared\nchange from A\n");
    write_page(&vault_b, "shared.md", "# Shared\nchange from B\n");
    sync_a.sync_now().unwrap();

    let report = sync_b.sync_now().unwrap();
    let shared = DocPath::parse(Scope::Wiki, "shared.md").unwrap();
    assert_eq!(
        vault_b
            .read(&shared, ReadWindow::default())
            .unwrap()
            .content,
        "# Shared\nchange from B"
    );
    assert!(!report.conflicts.is_empty());
    let status = sync_b.status();
    assert!(matches!(status, SyncStatus::Conflicted { .. }));

    let remote_copy = fs::read_dir(vault_b.root_path().join("wiki"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .contains("shared (conflict ")
        })
        .expect("remote version should be saved as a conflict copy");
    assert_eq!(
        fs::read_to_string(remote_copy.path()).unwrap(),
        "# Shared\nchange from A\n"
    );

    fs::remove_file(remote_copy.path()).unwrap();
    sync_b.sync_now().unwrap();
    assert!(matches!(sync_b.status(), SyncStatus::Idle { .. }));
    assert_clean(vault_a.root_path());
    assert_clean(vault_b.root_path());
}

#[test]
fn concurrent_log_appends_are_unioned_without_conflict_markers() {
    if skip_without_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let remote = new_bare_remote(temp.path());
    let remote_url = remote.to_string_lossy().into_owned();
    let vault_a =
        Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault-a"))).unwrap());
    let sync_a = VaultSync::configure(Arc::clone(&vault_a), config(remote_url.clone())).unwrap();
    fs::write(vault_a.root_path().join("wiki/log.md"), "# Log\nbase\n").unwrap();
    sync_a.sync_now().unwrap();

    let b_root = temp.path().join("vault-b");
    fs::create_dir_all(&b_root).unwrap();
    git(&b_root, &["clone", &remote_url, "."]);
    let vault_b = Arc::new(Vault::open(crate::VaultConfig::new(&b_root)).unwrap());
    let sync_b = VaultSync::configure(Arc::clone(&vault_b), config(remote_url)).unwrap();

    fs::OpenOptions::new()
        .append(true)
        .open(vault_a.root_path().join("wiki/log.md"))
        .unwrap()
        .write_all(b"from A\n")
        .unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(vault_b.root_path().join("wiki/log.md"))
        .unwrap()
        .write_all(b"from B\n")
        .unwrap();
    sync_a.sync_now().unwrap();
    sync_b.sync_now().unwrap();

    let log = fs::read_to_string(vault_b.root_path().join("wiki/log.md")).unwrap();
    assert!(log.contains("from A"));
    assert!(log.contains("from B"));
    assert!(!log.contains("<<<<<<<"));
    assert!(!log.contains(">>>>>>>"));
    assert!(matches!(sync_b.status(), SyncStatus::Idle { .. }));
    assert_clean(vault_a.root_path());
    assert_clean(vault_b.root_path());
}

#[test]
fn index_conflict_is_rebuilt_after_remote_pages_merge() {
    if skip_without_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let remote = new_bare_remote(temp.path());
    let remote_url = remote.to_string_lossy().into_owned();
    let vault_a =
        Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault-a"))).unwrap());
    let sync_a = VaultSync::configure(Arc::clone(&vault_a), config(remote_url.clone())).unwrap();
    sync_a.sync_now().unwrap();

    let b_root = temp.path().join("vault-b");
    fs::create_dir_all(&b_root).unwrap();
    git(&b_root, &["clone", &remote_url, "."]);
    let vault_b = Arc::new(Vault::open(crate::VaultConfig::new(&b_root)).unwrap());
    let sync_b = VaultSync::configure(Arc::clone(&vault_b), config(remote_url)).unwrap();

    write_page(&vault_a, "from-a.md", "# From A\nA content\n");
    write_page(&vault_b, "from-b.md", "# From B\nB content\n");
    sync_a.sync_now().unwrap();
    sync_b.sync_now().unwrap();

    let index = fs::read_to_string(vault_b.root_path().join("wiki/index.md")).unwrap();
    assert!(!index.contains("<<<<<<<"));
    assert!(!index.contains("======="));
    assert!(!index.contains(">>>>>>>"));
    assert!(index.contains("From A"));
    assert!(index.contains("From B"));
    assert_clean(vault_a.root_path());
    assert_clean(vault_b.root_path());
}

#[test]
fn delete_modify_conflict_keeps_the_modified_page() {
    if skip_without_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let remote = new_bare_remote(temp.path());
    let remote_url = remote.to_string_lossy().into_owned();
    let vault_a =
        Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault-a"))).unwrap());
    let sync_a = VaultSync::configure(Arc::clone(&vault_a), config(remote_url.clone())).unwrap();
    write_page(&vault_a, "delete-modify.md", "# Keep Edited\nbase\n");
    sync_a.sync_now().unwrap();

    let b_root = temp.path().join("vault-b");
    fs::create_dir_all(&b_root).unwrap();
    git(&b_root, &["clone", &remote_url, "."]);
    let vault_b = Arc::new(Vault::open(crate::VaultConfig::new(&b_root)).unwrap());
    let sync_b = VaultSync::configure(Arc::clone(&vault_b), config(remote_url)).unwrap();

    vault_a
        .apply(Change::Delete {
            path: DocPath::parse(Scope::Wiki, "delete-modify.md").unwrap(),
        })
        .unwrap();
    write_page(&vault_b, "delete-modify.md", "# Keep Edited\nB edit\n");
    sync_a.sync_now().unwrap();
    sync_b.sync_now().unwrap();

    let page = DocPath::parse(Scope::Wiki, "delete-modify.md").unwrap();
    assert_eq!(
        vault_b.read(&page, ReadWindow::default()).unwrap().content,
        "# Keep Edited\nB edit"
    );
    assert!(matches!(sync_b.status(), SyncStatus::Idle { .. }));
    assert_clean(vault_a.root_path());
    assert_clean(vault_b.root_path());
}

#[test]
fn sync_ignores_search_projection_and_lock_file() {
    if skip_without_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let remote = new_bare_remote(temp.path());
    let vault = Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault"))).unwrap());
    let sync = VaultSync::configure(
        Arc::clone(&vault),
        config(remote.to_string_lossy().into_owned()),
    )
    .unwrap();
    sync.sync_now().unwrap();
    let files = String::from_utf8(git(vault.root_path(), &["ls-files"]).stdout).unwrap();
    assert!(
        !files
            .lines()
            .any(|path| path == ".wiki/index.sqlite" || path == ".wiki-desktop.lock")
    );
}

#[test]
fn oversized_markdown_is_rejected_before_creating_a_commit() {
    if skip_without_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let remote = new_bare_remote(temp.path());
    let vault = Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault"))).unwrap());
    let mut cfg = config(remote.to_string_lossy().into_owned());
    cfg.max_file_bytes = 64 * 1024;
    let sync = VaultSync::configure(Arc::clone(&vault), cfg).unwrap();
    sync.sync_now().unwrap();
    let before = String::from_utf8(git(vault.root_path(), &["rev-parse", "HEAD"]).stdout).unwrap();
    write_page(
        &vault,
        "too-large.md",
        &format!("# Big\n{}", "x".repeat(70 * 1024)),
    );
    let error = sync.sync_now().unwrap_err();
    assert!(
        matches!(error, SyncError::FileTooLarge { .. }),
        "unexpected error: {error:?}"
    );
    let after = String::from_utf8(git(vault.root_path(), &["rev-parse", "HEAD"]).stdout).unwrap();
    assert_eq!(before, after);
}

#[test]
fn unchanged_oversized_tracked_file_does_not_block_small_change_sync() {
    if skip_without_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let remote = new_bare_remote(temp.path());
    let vault = Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault"))).unwrap());
    let mut cfg = config(remote.to_string_lossy().into_owned());
    cfg.max_file_bytes = 64 * 1024;
    let sync = VaultSync::configure(Arc::clone(&vault), cfg).unwrap();
    sync.sync_now().unwrap();

    let oversized = vault.root_path().join("wiki/legacy-large.md");
    fs::write(
        &oversized,
        format!("# Existing large file\n{}", "x".repeat(70 * 1024)),
    )
    .unwrap();
    git(vault.root_path(), &["add", "wiki/legacy-large.md"]);
    git(
        vault.root_path(),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "existing large file",
        ],
    );
    write_page(&vault, "small-change.md", "# Small change\n");

    let report = sync.sync_now().unwrap();
    assert!(report.committed >= 1);
    assert!(report.pushed);
}

#[test]
fn unsafe_remote_is_rejected_and_reader_cannot_sync() {
    if skip_without_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let writer = Arc::new(Vault::open(crate::VaultConfig::new(&root)).unwrap());
    assert!(matches!(
        VaultSync::configure(Arc::clone(&writer), config("https://u:t@host/r.git".into())),
        Err(SyncError::UnsafeRemote(_))
    ));
    let reader = Arc::new(Vault::open(crate::VaultConfig::new(&root)).unwrap());
    let sync =
        VaultSync::configure(reader, config("https://example.invalid/r.git".into())).unwrap();
    assert!(matches!(
        sync.status(),
        SyncStatus::Disabled {
            reason: DisabledReason::ReadOnly
        }
    ));
    assert!(matches!(sync.sync_now(), Err(SyncError::ReadOnly)));
}

#[test]
fn initial_auth_failure_keeps_a_retryable_status_bearing_sync() {
    if skip_without_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let fake_bin = temp.path().join("fake-bin");
    fs::create_dir_all(&fake_bin).unwrap();
    let fake_git = fake_bin.join("git");
    fs::write(
            &fake_git,
            "#!/bin/sh\nfor arg in \"$@\"; do\n  if [ \"$arg\" = \"ls-remote\" ]; then echo 'fatal: Authentication failed' >&2; exit 128; fi\ndone\nexec /usr/bin/git \"$@\"\n",
        )
        .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o755)).unwrap();
    let vault = Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault"))).unwrap());
    let sync = VaultSync::configure_with_path(
        vault,
        config("https://github.com/owner/wiki.git".into()),
        Some(fake_bin.as_os_str()),
    )
    .expect("initial auth failure should preserve the configured synchronizer");
    assert!(matches!(
        sync.status(),
        SyncStatus::Error {
            kind: SyncErrorKind::Auth,
            ..
        }
    ));
}

#[test]
fn no_git_disables_sync_and_ssh_arguments_are_headless() {
    let temp = tempfile::tempdir().unwrap();
    let empty_path = temp.path().join("empty-bin");
    fs::create_dir(&empty_path).unwrap();
    let vault =
        Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("no-git-vault"))).unwrap());
    let sync = VaultSync::configure_with_path(
        Arc::clone(&vault),
        config("file:///tmp/unused.git".into()),
        Some(empty_path.as_os_str()),
    )
    .unwrap();
    assert!(matches!(
        sync.status(),
        SyncStatus::Disabled {
            reason: DisabledReason::NoGit
        }
    ));
    assert!(matches!(sync.sync_now(), Err(SyncError::GitMissing)));

    let ssh = ssh_command(
        Path::new("/keys/deploy key"),
        Path::new("/keys/known hosts"),
    );
    assert!(ssh.contains("-i '/keys/deploy key'"));
    assert!(ssh.contains("-o IdentitiesOnly=yes"));
    assert!(ssh.contains("-o UserKnownHostsFile='/keys/known hosts'"));
    assert!(ssh.contains("-o StrictHostKeyChecking=yes"));
    let ssh_config = SyncConfig {
        auth: GitAuth::SshKey {
            key: PathBuf::from("/keys/deploy"),
            known_hosts: PathBuf::from("/keys/known_hosts"),
        },
        ..SyncConfig::default()
    };
    let (ssh_command, global_config, no_system_config) = ssh_environment(&ssh_config.auth).unwrap();
    assert!(ssh_command.contains("-i '/keys/deploy'"));
    assert_eq!(global_config, "/dev/null");
    assert_eq!(no_system_config, "1");
    let git_binary = find_git(std::env::var_os("PATH").as_deref());
    let version = run_git(
        temp.path(),
        &["--version"],
        &ssh_config,
        git_binary.as_deref(),
        false,
    )
    .unwrap();
    assert!(version.stdout_string().unwrap().starts_with("git version "));
}

#[test]
fn current_branch_is_available_to_restore_desktop_sync_settings() {
    if skip_without_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-b", "custom-wiki"]);
    let vault = Vault::open(crate::VaultConfig::new(&root)).unwrap();

    assert_eq!(
        VaultSync::current_branch(&vault).unwrap().as_deref(),
        Some("custom-wiki")
    );
}

#[test]
fn initial_sync_uses_and_pushes_the_selected_custom_branch() {
    if skip_without_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let remote = new_bare_remote(temp.path());
    let root = temp.path().join("vault");
    let vault = Arc::new(Vault::open(crate::VaultConfig::new(&root)).unwrap());
    let mut sync_config = config(remote.to_string_lossy().into_owned());
    sync_config.branch = "custom-wiki".into();
    let sync = VaultSync::configure(Arc::clone(&vault), sync_config).unwrap();

    assert_eq!(
        String::from_utf8(git(&root, &["branch", "--show-current"]).stdout)
            .unwrap()
            .trim(),
        "custom-wiki"
    );
    write_page(&vault, "custom.md", "# Custom branch\n");
    assert!(sync.sync_now().unwrap().pushed);
    assert!(
        git(&remote, &["show-ref", "--verify", "refs/heads/custom-wiki"])
            .status
            .success()
    );
}

#[test]
fn refresh_status_recovers_when_a_reader_acquires_the_write_lock() {
    if skip_without_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let remote = new_bare_remote(temp.path());
    let root = temp.path().join("vault");
    let writer = Arc::new(Vault::open(crate::VaultConfig::new(&root)).unwrap());
    VaultSync::configure(
        Arc::clone(&writer),
        config(remote.to_string_lossy().into_owned()),
    )
    .unwrap();
    let reader = Arc::new(Vault::open(crate::VaultConfig::new(&root)).unwrap());
    let sync = VaultSync::configure(
        Arc::clone(&reader),
        config(remote.to_string_lossy().into_owned()),
    )
    .unwrap();
    assert!(matches!(
        sync.status(),
        SyncStatus::Disabled {
            reason: DisabledReason::ReadOnly
        }
    ));

    drop(writer);
    assert!(reader.try_acquire_write().unwrap());
    assert!(matches!(
        sync.refresh_status().unwrap(),
        SyncStatus::Idle { .. }
    ));
}

#[test]
fn ssh_user_without_secret_is_a_safe_remote() {
    use super::git::is_unsafe_remote;
    assert!(!is_unsafe_remote("ssh://git@github.com/owner/repo.git"));
    assert!(!is_unsafe_remote("git@github.com:owner/repo.git"));
    assert!(is_unsafe_remote("ssh://git:secret@github.com/owner/repo.git"));
    assert!(is_unsafe_remote("https://git@github.com/owner/repo.git"));
    assert!(is_unsafe_remote("https://u:t@host/r.git"));
}

#[test]
fn second_machine_with_its_own_history_merges_into_existing_remote() {
    if skip_without_git() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let remote = new_bare_remote(temp.path());
    let remote_url = remote.to_string_lossy().into_owned();
    let vault_a =
        Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault-a"))).unwrap());
    let sync_a = VaultSync::configure(Arc::clone(&vault_a), config(remote_url.clone())).unwrap();
    write_page(&vault_a, "from-a.md", "# A\n");
    sync_a.sync_now().unwrap();

    // Vault B was initialised independently, so it shares no history with the remote.
    let vault_b =
        Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault-b"))).unwrap());
    let sync_b = VaultSync::configure(Arc::clone(&vault_b), config(remote_url)).unwrap();
    write_page(&vault_b, "from-b.md", "# B\n");
    sync_b.sync_now().unwrap();

    for name in ["from-a.md", "from-b.md"] {
        let path = DocPath::parse(Scope::Wiki, name).unwrap();
        assert!(vault_b.read(&path, ReadWindow::default()).is_ok(), "{name}");
    }
    assert_clean(vault_b.root_path());
}
