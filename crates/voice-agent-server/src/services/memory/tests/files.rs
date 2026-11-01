//! The vault on disk: layout, isolation, tombstones, and edits made outside the system.

use super::*;

#[tokio::test]
async fn two_devices_do_not_see_each_other() {
    let f = fixture();
    let other = MemoryScope::for_device("device-b", DeviceKind::Physical);
    f.remember("Mã cổng A", "Mã cổng thiết bị một").await;
    remember_on(&f.core, &other, "Mã cổng B", "Mã cổng thiết bị hai").await;

    let mine = f.recall("cổng").await;
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].title, "Mã cổng A");
    let theirs = recall_on(&f.core, &other, "cổng").await;
    assert_eq!(theirs.len(), 1);
    assert_eq!(theirs[0].title, "Mã cổng B");
    assert!(!f.context_rows().await.contains("Mã cổng B"));
    assert!(!context_text(&f.core, &other).await.contains("Mã cổng A"));
    let error = f
        .core
        .apply(
            &other,
            MemoryActor::Admin,
            MemoryChange::Forget {
                id: mine[0].id.clone(),
                expected_revision: 1,
            },
        )
        .await
        .expect_err("a fact id does not cross scopes");
    assert_eq!(error.code(), "memory_not_found");
}

#[tokio::test]
async fn device_directories_are_hashes_never_the_client_supplied_id() {
    let f = fixture();
    f.remember("T", "Thân").await;
    assert!(
        f.root
            .join("devices")
            .join(DEVICE_A_DIR)
            .join("facts")
            .is_dir()
    );

    let hostile = "../../etc:passwd";
    let scope = MemoryScope::for_device(hostile, DeviceKind::Physical);
    remember_on(&f.core, &scope, "T", "Thân").await;
    // sha256("../../etc:passwd")[..16], computed with shasum.
    assert!(f.root.join("devices/553ef740d0e7ca3f/facts").is_dir());

    let mut names: Vec<String> = fs::read_dir(f.root.join("devices"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["553ef740d0e7ca3f", DEVICE_A_DIR]);
    let mut top: Vec<String> = fs::read_dir(&f.root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    top.sort();
    assert_eq!(top, ["_deployment", "devices"], "nothing escaped the vault");
    assert!(
        !format!("{scope:?}").contains("passwd"),
        "not even in debug output"
    );
}

#[tokio::test]
async fn the_virtual_device_keeps_a_fixed_directory_when_its_id_changes() {
    let f = fixture();
    let first = MemoryScope::for_device("web_assistant_aaa", DeviceKind::Virtual);
    let rotated = MemoryScope::for_device("web_assistant_bbb", DeviceKind::Virtual);
    remember_on(&f.core, &first, "Mã cổng", "Mã cổng bảy bảy").await;
    assert!(f.root.join("devices/web-assistant/facts").is_dir());
    assert_eq!(recall_on(&f.core, &rotated, "cổng").await.len(), 1);
}

#[tokio::test]
async fn tombstones_hold_only_a_fingerprint_and_a_time() {
    let f = fixture();
    let fact = f.remember("Tiêu đề bí mật", "Thân bí mật").await;
    f.forget(&fact).await.unwrap();

    let text = f.tombstones_file();
    for secret in ["Tiêu đề", "bí mật", "bi mat", "Thân", "than", "hook"] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }
    let entries: Vec<serde_json::Map<String, serde_json::Value>> =
        serde_json::from_str(&text).unwrap();
    assert_eq!(entries.len(), 1);
    let mut keys: Vec<&str> = entries[0].keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["at", "fp"]);
    assert_eq!(entries[0]["fp"].as_str().unwrap().len(), 64);
}

#[tokio::test]
async fn a_different_key_gives_a_different_fingerprint() {
    let (a, b) = (fixture(), fixture());
    let fingerprint = async |f: &Fixture| {
        let fact = f.remember("T", "Cùng một nội dung").await;
        f.forget(&fact).await.unwrap();
        serde_json::from_str::<serde_json::Value>(&f.tombstones_file()).unwrap()[0]["fp"].clone()
    };
    assert_ne!(fingerprint(&a).await, fingerprint(&b).await);
}

#[tokio::test]
async fn the_same_content_has_a_different_fingerprint_on_another_device() {
    let f = fixture();
    let other = MemoryScope::for_device("device-b", DeviceKind::Physical);
    let mine = f.remember("T", "Cùng một nội dung").await;
    let theirs = remember_on(&f.core, &other, "T", "Cùng một nội dung").await;
    f.forget(&mine).await.unwrap();
    assert_eq!(
        recall_on(&f.core, &other, "cùng").await.len(),
        1,
        "forgetting stays local"
    );
    f.core
        .apply(
            &other,
            MemoryActor::Admin,
            MemoryChange::Forget {
                id: theirs.fact_id.unwrap(),
                expected_revision: 1,
            },
        )
        .await
        .unwrap();
    let read = |dir: &str| {
        let text = fs::read_to_string(f.root.join("devices").join(dir).join("tombstones.json"));
        serde_json::from_str::<serde_json::Value>(&text.unwrap()).unwrap()[0]["fp"].clone()
    };
    let dirs: Vec<String> = fs::read_dir(f.root.join("devices"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != DEVICE_A_DIR)
        .collect();
    assert_eq!(dirs.len(), 1);
    assert_ne!(read(DEVICE_A_DIR), read(&dirs[0]));
}

#[tokio::test]
async fn the_tombstone_key_is_32_private_bytes_stable_across_opens() {
    let f = fixture();
    let path = f.root.join("_deployment/tombstone.key");
    let first = fs::read(&path).unwrap();
    assert_eq!(first.len(), 32);
    assert_eq!(mode_bits(&path), 0o600);

    f.reopen();
    assert_eq!(fs::read(&path).unwrap(), first);

    // A forget from before the restart still holds.
    let fact = f.remember("Mã cổng", "Mã cổng bảy bảy").await;
    f.forget(&fact).await.unwrap();
    assert!(recall_on(&f.reopen(), &f.scope, "cổng").await.is_empty());
}

#[test]
fn a_tombstone_key_of_the_wrong_length_is_an_error_and_is_kept() {
    let root = std::env::temp_dir().join(format!("memory-core-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("_deployment")).unwrap();
    fs::write(root.join("_deployment/tombstone.key"), b"short").unwrap();
    assert_eq!(
        MemoryCore::open(&config(&root))
            .err()
            .map(MemoryError::code),
        Some("memory_unavailable")
    );
    assert_eq!(
        fs::read(root.join("_deployment/tombstone.key")).unwrap(),
        b"short"
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn a_fact_file_has_the_specified_frontmatter() {
    let f = fixture();
    let receipt = f
        .core
        .apply(
            &f.scope,
            MemoryActor::Admin,
            MemoryChange::Remember {
                title: "Shop nghỉ\nChủ Nhật".into(),
                hook: "Không giao hàng".into(),
                body: "Nội dung.\n\nĐoạn hai.\n---\nĐoạn ba.".into(),
                kind: FactKind::Decision,
                source_ref: Some("device:12 turn:48".into()),
            },
        )
        .await
        .unwrap();

    let path = f.the_fact_path();
    assert_eq!(path.file_name().unwrap(), "shop-nghi-chu-nhat.md");
    let front = frontmatter(&path);
    let text = |key: &str| front[key].as_str().unwrap_or_default().to_string();
    assert_eq!(text("id"), receipt.fact_id.unwrap().as_str());
    assert_eq!(text("title"), "Shop nghỉ Chủ Nhật", "one line");
    assert_eq!(text("kind"), "decision");
    assert_eq!(text("provenance"), "user");
    assert_eq!(text("origin"), "admin");
    assert_eq!(text("status"), "confirmed");
    assert_eq!(text("source_ref"), "device:12 turn:48");
    assert_eq!(front["revision"].as_u64(), Some(1));
    assert_eq!(front["pinned"].as_bool(), Some(false));
    assert!(front["superseded_by"].is_null());
    assert_eq!(text("content_hash").len(), 64);
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .ends_with("\n---\nĐoạn ba.\n")
    );
    assert_eq!(
        f.only_fact().await.body,
        "Nội dung.\n\nĐoạn hai.\n---\nĐoạn ba."
    );
    let report = f.core.reconcile(&f.scope).await.unwrap();
    assert_eq!(report.quarantined, 0);
    assert_eq!(
        report.owner_edited, 0,
        "a fresh file hashes to its own content_hash"
    );
}

#[tokio::test]
async fn reconcile_records_an_outside_edit_once() {
    let f = fixture();
    f.remember("Giờ mở cửa", "Mở cửa lúc tám giờ").await;
    let path = f.the_fact_path();
    let text = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        text.replace("Mở cửa lúc tám giờ", "Mở cửa lúc chín giờ"),
    )
    .unwrap();

    let report = f.core.reconcile(&f.scope).await.unwrap();

    assert_eq!(report.owner_edited, 1);
    let fact = f.only_fact().await;
    assert_eq!(fact.revision, 2);
    assert_eq!(fact.origin, FactOrigin::OwnerEdited);
    assert_eq!(f.recall("chín").await.len(), 1);
    assert!(f.recall("tám").await.is_empty());

    let again = f.core.reconcile(&f.scope).await.unwrap();
    assert_eq!(again.owner_edited, 0);
    assert_eq!(f.only_fact().await.revision, 2);
}

#[tokio::test]
async fn reconcile_ignores_line_endings_and_trailing_blank_lines() {
    let f = fixture();
    f.remember("T", "Dòng một\nDòng hai").await;
    let path = f.the_fact_path();
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{}\r\n\r\n", text.replace('\n', "\r\n"))).unwrap();

    let report = f.core.reconcile(&f.scope).await.unwrap();

    assert_eq!(report.owner_edited, 0);
    assert_eq!(f.only_fact().await.revision, 1);
}

#[tokio::test]
async fn unknown_frontmatter_fields_survive_every_rewrite() {
    let f = fixture();
    let fact = f.remember("Giờ mở cửa", "Mở cửa lúc tám giờ").await;
    let id = fact.fact_id.clone().unwrap();
    let path = f.the_fact_path();
    let text = fs::read_to_string(&path).unwrap();
    let with_extra = text.replacen(
        "---\n",
        "---\ncssclass: wide\naliases:\n- Giờ làm việc\nobsidian: {pos: 3}\n",
        1,
    );
    fs::write(&path, with_extra).unwrap();
    let check = || {
        let front = frontmatter(&path);
        assert_eq!(front["cssclass"].as_str(), Some("wide"));
        assert_eq!(front["aliases"][0].as_str(), Some("Giờ làm việc"));
        assert_eq!(front["obsidian"]["pos"].as_u64(), Some(3));
    };

    f.core
        .apply(
            &f.scope,
            MemoryActor::Admin,
            MemoryChange::SetPinned {
                id: id.clone(),
                pinned: true,
            },
        )
        .await
        .unwrap();
    check();
    f.core
        .apply(
            &f.scope,
            MemoryActor::Admin,
            correct_change(&fact, "Mở cửa lúc chín giờ"),
        )
        .await
        .unwrap();
    check();
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("chín giờ", "mười giờ")).unwrap();
    f.core.reconcile(&f.scope).await.unwrap();
    check();
    assert_eq!(f.only_fact().await.revision, 3);
}

#[tokio::test]
async fn a_broken_fact_is_quarantined_and_harms_nothing_else() {
    let f = fixture();
    f.remember("Lành", "Thân lành").await;
    let broken_fact = f.remember("Hỏng", "zebrakhongdau").await;
    assert_eq!(f.recall("zebrakhongdau").await.len(), 1);
    let facts = f.device_dir().join("facts");
    let broken = facts.join("hong.md");
    fs::write(&broken, "---\nid: [unclosed\n---\nzebrakhongdau").unwrap();
    fs::write(facts.join("no-frontmatter.md"), "chỉ có zebrakhongdau").unwrap();
    fs::write(
        facts.join("missing-field.md"),
        "---\ntitle: x\n---\nzebrakhongdau",
    )
    .unwrap();
    fs::write(facts.join("binary.md"), [0xff, 0xfe, 0x00]).unwrap();

    let restarted = f.reopen();
    let overview = restarted
        .inspect(&f.scope, InspectFilter::default())
        .await
        .unwrap();
    assert_eq!(overview.quarantined, 4);
    assert!(overview.quarantined_files.contains(&"hong.md".to_string()));
    assert_eq!(overview.facts.len(), 1);
    assert!(
        !context_text(&restarted, &f.scope)
            .await
            .contains("zebrakhongdau")
    );
    assert!(!context_text(&restarted, &f.scope).await.contains("Hỏng"));
    assert!(
        recall_on(&restarted, &f.scope, "zebrakhongdau")
            .await
            .is_empty()
    );

    let error = restarted
        .apply(
            &f.scope,
            MemoryActor::Admin,
            MemoryChange::Forget {
                id: broken_fact.fact_id.unwrap(),
                expected_revision: 1,
            },
        )
        .await
        .expect_err("a quarantined fact cannot be operated on");
    assert_eq!(error.code(), "memory_not_found");

    let report = restarted.reconcile(&f.scope).await.unwrap();
    assert_eq!(report.quarantined, 4);
    assert!(broken.exists(), "quarantine never deletes the owner's file");
    assert!(
        recall_on(&restarted, &f.scope, "zebrakhongdau")
            .await
            .is_empty()
    );
    assert_eq!(recall_on(&restarted, &f.scope, "lành").await.len(), 1);
}

#[tokio::test]
async fn a_copied_file_with_a_repeated_id_is_quarantined() {
    let f = fixture();
    f.remember("Một", "Thân một").await;
    let path = f.the_fact_path();
    fs::copy(&path, path.with_file_name("sao-chep.md")).unwrap();

    let overview = f
        .core
        .inspect(&f.scope, InspectFilter::default())
        .await
        .unwrap();

    assert_eq!(overview.facts.len(), 1);
    assert_eq!(overview.quarantined, 1);
}

#[tokio::test]
async fn an_outside_edit_matching_a_tombstone_is_deleted_by_reconcile() {
    let f = fixture();
    let forgotten = f.remember("Mã cổng", "Mã cổng bảy bảy").await;
    f.forget(&forgotten).await.unwrap();
    let other = f.remember("Khác", "Nội dung khác").await;
    let path = f.the_fact_path();
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("Nội dung khác", "ma cong BAY BAY")).unwrap();

    assert!(
        f.recall("cổng").await.is_empty(),
        "already hidden before reconcile"
    );
    let report = f.core.reconcile(&f.scope).await.unwrap();

    assert_eq!(report.tombstones_applied, 1);
    assert!(f.fact_paths().is_empty());
    assert!(other.fact_id.is_some());
    assert!(
        f.tombstones_file().contains("\"fp\""),
        "the tombstone stays"
    );
}

#[tokio::test]
async fn reconcile_removes_interrupted_temp_files_but_nothing_else() {
    let f = fixture();
    f.remember("Một", "Thân một").await;
    let temp = f.device_dir().join("facts/.mot.md.abc.tmp");
    fs::write(&temp, "half a fact").unwrap();
    fs::write(f.device_dir().join(".MEMORY.md.abc.tmp"), "half").unwrap();
    fs::write(f.device_dir().join("notes.txt"), "the owner's").unwrap();

    let report = f.core.reconcile(&f.scope).await.unwrap();

    assert_eq!(report.temp_files_removed, 2);
    assert!(!temp.exists());
    assert!(f.device_dir().join("notes.txt").exists());
    assert_eq!(f.only_fact().await.title, "Một");
}

#[tokio::test]
async fn reconcile_regenerates_a_deleted_summary_file() {
    let f = fixture();
    f.remember("Một", "Thân một").await;
    fs::remove_file(f.device_dir().join("MEMORY.md")).unwrap();
    f.core.reconcile(&f.scope).await.unwrap();
    assert!(f.summary_file().contains("**Một**"));
}

#[tokio::test]
async fn reconcile_all_converges_known_scopes_and_only_reports_orphans() {
    let f = fixture();
    let orphan = MemoryScope::for_device("gone", DeviceKind::Physical);
    let empty = MemoryScope::for_device("never-used", DeviceKind::Physical);
    f.remember("Một", "Thân một").await;
    remember_on(&f.core, &orphan, "Mồ côi", "Thân mồ côi").await;
    let path = f.the_fact_path();
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("Thân một", "Thân đã sửa")).unwrap();
    let before: Vec<_> = fs::read_dir(f.root.join("devices")).unwrap().collect();
    assert_eq!(before.len(), 2);

    let restarted = f.reopen();
    let report = restarted
        .reconcile_all(&[f.scope.clone(), empty, MemoryScope::deployment()])
        .await;

    assert_eq!(report.reconciled, 2);
    assert_eq!(report.failed, 0);
    assert_eq!(report.orphans.len(), 1);
    assert_ne!(report.orphans[0], DEVICE_A_DIR);
    assert!(
        f.root
            .join("devices")
            .join(&report.orphans[0])
            .join("facts")
            .is_dir()
    );
    assert_eq!(
        fs::read_dir(f.root.join("devices")).unwrap().count(),
        2,
        "no vault was created"
    );
    let fact = f.only_fact().await;
    assert_eq!((fact.revision, fact.origin), (2, FactOrigin::OwnerEdited));
}
