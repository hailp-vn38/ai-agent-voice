
use super::*;
#[test]
fn expected_revision_is_fail_closed() {
    assert!(matches!(
        next_revision(None, None),
        Err(Error::RevisionConflict)
    ));
    assert_eq!(next_revision(None, Some(0)).unwrap(), 1);
    assert!(matches!(
        next_revision(None, Some(1)),
        Err(Error::RevisionConflict)
    ));
}

#[tokio::test]
async fn isolated_indexes_and_canonical_revision_cas() {
    let root = std::env::temp_dir().join(format!("knowledge_v3_{}", Uuid::new_v4()));
    let core = Runtime::open(root.clone()).await.unwrap();
    let wiki = core
        .knowledge
        .put_draft(WikiInput {
            id: None,
            title: "Agent Session".into(),
            body: "A durable agent session".into(),
            expected_revision: Some(0),
            source_ref: Some("src_1:1".into()),
        })
        .await
        .unwrap();
    assert_eq!(wiki.revision, 1);
    assert_eq!(
        core.knowledge
            .admin_search("durable", 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        core.knowledge
            .search("durable", 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        core.knowledge.read_verified(&wiki.id, 1, 20).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        core.knowledge
            .put_draft(WikiInput {
                id: Some(wiki.id.clone()),
                title: "conflict".into(),
                body: "bad update".into(),
                expected_revision: Some(0),
                source_ref: None
            })
            .await,
        Err(Error::RevisionConflict)
    ));

    let evidence = core
        .knowledge
        .import_source(SourceInput {
            name: "evidence.md".into(),
            content: "# Original durable fact".into(),
            provenance: "admin:test".into(),
        })
        .await
        .unwrap();
    let published = core
        .knowledge
        .publish(&wiki.id, wiki.revision, &format!("{}:1", evidence.id))
        .await
        .unwrap();
    assert_eq!(published.status, "verified");
    assert_eq!(core.knowledge.search("durable", 10).await.unwrap().len(), 1);
    let fact = core
        .memory
        .put_fact(FactInput {
            id: None,
            title: "Language".into(),
            body: "The user prefers Vietnamese".into(),
            expected_revision: Some(0),
            source_ref: Some("admin".into()),
            confirmed: true,
        })
        .await
        .unwrap();
    assert_eq!(core.memory.recall("Vietnamese", 5).await.unwrap().len(), 1);
    assert_eq!(
        core.knowledge.search("Vietnamese", 5).await.unwrap().len(),
        0
    );
    assert!(core.memory.summary().await.unwrap().contains("Vietnamese"));
    core.memory.forget(&fact.id, fact.revision).await.unwrap();
    assert!(matches!(
        core.memory
            .put_fact(FactInput {
                id: Some(fact.id.clone()),
                title: "Resurrected".into(),
                body: "Must not revive forgotten memory".into(),
                expected_revision: Some(fact.revision + 1),
                source_ref: Some("admin".into()),
                confirmed: true,
            })
            .await,
        Err(Error::InvalidInput)
    ));
    assert!(
        core.memory
            .recall("Vietnamese", 5)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(!core.memory.summary().await.unwrap().contains("Vietnamese"));
    assert_eq!(core.knowledge.health().await.unwrap().document_count, 1);
    assert_eq!(core.memory.health().await.unwrap().document_count, 1);
    drop(core);
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn admin_lists_draft_wiki_while_job_read_and_search_refuse_it() {
    let root = std::env::temp_dir().join(format!("wiki_v3_list_{}", Uuid::new_v4()));
    let core = Runtime::open(root.clone()).await.unwrap();
    let draft = core
        .knowledge
        .put_draft(WikiInput {
            id: None,
            title: "Unpublished".into(),
            body: "do not disclose draft".into(),
            expected_revision: Some(0),
            source_ref: None,
        })
        .await
        .unwrap();
    let pages = core.knowledge.admin_list(25).await.unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].id, draft.id);
    assert_eq!(pages[0].status, "draft");
    assert!(
        core.knowledge
            .search("disclose", 5)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        core.knowledge.read_verified(&draft.id, 1, 10).await,
        Err(Error::NotFound)
    ));
    drop(core);
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn admin_listing_includes_unconfirmed_but_job_recall_does_not() {
    let root = std::env::temp_dir().join(format!("memory_v3_list_{}", Uuid::new_v4()));
    let core = Runtime::open(root.clone()).await.unwrap();
    let draft = core
        .memory
        .put_fact(FactInput {
            id: None,
            title: "Pending preference".into(),
            body: "Private unconfirmed phrase".into(),
            expected_revision: Some(0),
            source_ref: Some("admin:test".into()),
            confirmed: false,
        })
        .await
        .unwrap();
    let items = core.memory.admin_list(25).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].status, "needs_review");
    assert_eq!(items[0].id, draft.id);
    assert!(
        core.memory
            .recall("unconfirmed", 10)
            .await
            .unwrap()
            .is_empty()
    );
    core.memory.forget(&draft.id, draft.revision).await.unwrap();
    let items = core.memory.admin_list(25).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].status, "forgotten");
    assert!(items[0].excerpt.is_empty());
    drop(core);
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn forced_rebuild_repairs_missing_fts_row_without_rewriting_markdown() {
    let root = std::env::temp_dir().join(format!("knowledge_v3_{}", Uuid::new_v4()));
    let core = Runtime::open(root.clone()).await.unwrap();
    let page = core
        .knowledge
        .put_draft(WikiInput {
            id: None,
            title: "Recovery".into(),
            body: "uniqueindexprobe".into(),
            expected_revision: Some(0),
            source_ref: None,
        })
        .await
        .unwrap();
    let canonical = tokio::fs::read(root.join("knowledge/wiki").join(format!("{}.md", page.id)))
        .await
        .unwrap();
    let uri = format!(
        "sqlite://{}",
        root.join("knowledge/knowledge_index.db").display()
    );
    let index = sqlx::SqlitePool::connect(&uri).await.unwrap();
    sqlx::query("DELETE FROM kb_fts WHERE stable_id=?")
        .bind(&page.id)
        .execute(&index)
        .await
        .unwrap();
    assert!(
        core.knowledge
            .admin_search("uniqueindexprobe", 10)
            .await
            .unwrap()
            .is_empty()
    );
    core.knowledge.rebuild().await.unwrap();
    assert_eq!(
        core.knowledge
            .admin_search("uniqueindexprobe", 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        canonical,
        tokio::fs::read(root.join("knowledge/wiki").join(format!("{}.md", page.id)))
            .await
            .unwrap()
    );
    drop(index);
    drop(core);
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn restart_with_missing_canonical_wiki_root_fails_without_pruning_index() {
    let root = std::env::temp_dir().join(format!("wiki_missing_restart_{}", Uuid::new_v4()));
    let runtime = Runtime::open(root.clone()).await.unwrap();
    let wiki = runtime
        .knowledge
        .put_draft(WikiInput {
            id: None,
            title: "Preserve".into(),
            body: "recovery evidence".into(),
            expected_revision: Some(0),
            source_ref: None,
        })
        .await
        .unwrap();
    drop(runtime);
    tokio::fs::rename(
        root.join("knowledge/wiki"),
        root.join("knowledge/wiki-unmounted"),
    )
    .await
    .unwrap();
    assert!(matches!(
        Runtime::open(root.clone()).await,
        Err(Error::Unavailable)
    ));
    let sqlite = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        root.join("knowledge/knowledge_index.db").display()
    ))
    .await
    .unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_documents WHERE stable_id=?")
        .bind(&wiki.id)
        .fetch_one(&sqlite)
        .await
        .unwrap();
    assert_eq!(rows, 1);
    sqlite.close().await;
    tokio::fs::rename(
        root.join("knowledge/wiki-unmounted"),
        root.join("knowledge/wiki"),
    )
    .await
    .unwrap();
    assert!(Runtime::open(root.clone()).await.is_ok());
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn published_wiki_can_be_revised_without_exposing_pending_draft() {
    let root = std::env::temp_dir().join(format!("wiki_edit_{}", Uuid::new_v4()));
    let core = Runtime::open(root.clone()).await.unwrap();
    let evidence = core
        .knowledge
        .import_source(SourceInput {
            name: "proof.md".into(),
            content: "# Source".into(),
            provenance: "admin:test".into(),
        })
        .await
        .unwrap();
    let reference = format!("{}:1", evidence.id);
    let first = core
        .knowledge
        .put_draft(WikiInput {
            id: None,
            title: "Article".into(),
            body: "originalworld".into(),
            expected_revision: Some(0),
            source_ref: Some(reference.clone()),
        })
        .await
        .unwrap();
    let accepted = core
        .knowledge
        .publish(&first.id, first.revision, &reference)
        .await
        .unwrap();
    let pending = core
        .knowledge
        .put_draft(WikiInput {
            id: Some(first.id.clone()),
            title: "Article revised".into(),
            body: "newworld".into(),
            expected_revision: Some(accepted.revision),
            source_ref: Some(reference.clone()),
        })
        .await
        .unwrap();
    assert_eq!(pending.status, "draft");
    assert_eq!(pending.draft_base_revision, Some(accepted.revision));
    assert_eq!(
        core.knowledge.read_full(&first.id).await.unwrap().body,
        "newworld"
    );
    assert_eq!(
        core.knowledge
            .read_verified(&first.id, 1, 100)
            .await
            .unwrap()
            .body,
        "originalworld"
    );
    assert_eq!(
        core.knowledge
            .search("originalworld", 5)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        core.knowledge
            .search("newworld", 5)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        core.knowledge
            .publish(&first.id, accepted.revision, &reference)
            .await,
        Err(Error::RevisionConflict)
    ));
    let accepted2 = core
        .knowledge
        .publish(&first.id, pending.revision, &reference)
        .await
        .unwrap();
    assert_eq!(accepted2.revision, accepted.revision + 1);
    assert_eq!(
        core.knowledge
            .read_verified(&first.id, 1, 100)
            .await
            .unwrap()
            .body,
        "newworld"
    );
    assert!(
        core.knowledge
            .search("originalworld", 5)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(core.knowledge.search("newworld", 5).await.unwrap().len(), 1);
    assert!(
        tokio::fs::metadata(
            root.join("knowledge/revisions")
                .join(&first.id)
                .join(format!("{}.md", accepted.revision))
        )
        .await
        .is_ok()
    );
    drop(core);
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn staged_revision_survives_restart_while_agent_uses_canonical() {
    let root = std::env::temp_dir().join(format!("wiki_stage_restart_{}", Uuid::new_v4()));
    let core = Runtime::open(root.clone()).await.unwrap();
    let proof = core
        .knowledge
        .import_source(SourceInput {
            name: "proof.md".into(),
            content: "Evidence".into(),
            provenance: "admin".into(),
        })
        .await
        .unwrap();
    let reference = format!("{}:1", proof.id);
    let draft = core
        .knowledge
        .put_draft(WikiInput {
            id: None,
            title: "Old".into(),
            body: "theorig".into(),
            expected_revision: Some(0),
            source_ref: Some(reference.clone()),
        })
        .await
        .unwrap();
    let accepted = core
        .knowledge
        .publish(&draft.id, draft.revision, &reference)
        .await
        .unwrap();
    let pending = core
        .knowledge
        .put_draft(WikiInput {
            id: Some(accepted.id.clone()),
            title: "New".into(),
            body: "thepending".into(),
            expected_revision: Some(accepted.revision),
            source_ref: Some(reference),
        })
        .await
        .unwrap();
    drop(core);
    let reopened = Runtime::open(root.clone()).await.unwrap();
    assert_eq!(
        reopened
            .knowledge
            .read_full(&accepted.id)
            .await
            .unwrap()
            .revision,
        pending.revision
    );
    assert_eq!(
        reopened
            .knowledge
            .read_verified(&accepted.id, 1, 10)
            .await
            .unwrap()
            .body,
        "theorig"
    );
    assert!(
        reopened
            .knowledge
            .search("thepending", 10)
            .await
            .unwrap()
            .is_empty()
    );
    drop(reopened);
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn tampered_evidence_invalidates_verified_index_and_rebuild() {
    let root = std::env::temp_dir().join(format!("wiki_provenance_{}", Uuid::new_v4()));
    let core = Runtime::open(root.clone()).await.unwrap();
    let proof = core
        .knowledge
        .import_source(SourceInput {
            name: "proof.md".into(),
            content: "trusted source".into(),
            provenance: "admin".into(),
        })
        .await
        .unwrap();
    let reference = format!("{}:1", proof.id);
    let draft = core
        .knowledge
        .put_draft(WikiInput {
            id: None,
            title: "Article".into(),
            body: "citedphrase".into(),
            expected_revision: Some(0),
            source_ref: Some(reference.clone()),
        })
        .await
        .unwrap();
    core.knowledge
        .publish(&draft.id, draft.revision, &reference)
        .await
        .unwrap();
    let hits = core.knowledge.search("citedphrase", 5).await.unwrap();
    assert_eq!(hits[0].source_ref.as_deref(), Some(reference.as_str()));
    tokio::fs::write(
        root.join("knowledge/raw")
            .join(&proof.id)
            .join("1/original.md"),
        b"tampered source",
    )
    .await
    .unwrap();
    assert!(matches!(
        core.knowledge.read_verified(&draft.id, 1, 10).await,
        Err(Error::IndexStale)
    ));
    assert_eq!(core.knowledge.health().await.unwrap().health, "stale");
    assert!(matches!(
        core.knowledge.rebuild().await,
        Err(Error::IndexStale)
    ));
    drop(core);
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn admin_full_read_never_truncates_long_wiki() {
    let root = std::env::temp_dir().join(format!("wiki_fullread_{}", Uuid::new_v4()));
    let runtime = Runtime::open(root.clone()).await.unwrap();
    let body = "x\n".repeat(10_050);
    let wiki = runtime
        .knowledge
        .put_draft(WikiInput {
            id: None,
            title: "Long".into(),
            body: body.clone(),
            expected_revision: Some(0),
            source_ref: None,
        })
        .await
        .unwrap();
    assert_eq!(
        runtime.knowledge.read_full(&wiki.id).await.unwrap().body,
        body
    );
    assert_ne!(
        runtime
            .knowledge
            .read(&wiki.id, 1, 10_000)
            .await
            .unwrap()
            .body,
        body
    );
    drop(runtime);
    let _ = tokio::fs::remove_dir_all(root).await;
}
#[tokio::test]
async fn missing_vault_root_fails_closed_without_mass_delete() {
    let root = std::env::temp_dir().join(format!("knowledge_v3_{}", Uuid::new_v4()));
    let core = Runtime::open(root.clone()).await.unwrap();
    let page = core
        .knowledge
        .put_draft(WikiInput {
            id: None,
            title: "Record".into(),
            body: "nonempty".into(),
            expected_revision: Some(0),
            source_ref: None,
        })
        .await
        .unwrap();
    tokio::fs::rename(
        root.join("knowledge/wiki"),
        root.join("knowledge/wiki-offline"),
    )
    .await
    .unwrap();
    assert!(core.knowledge.rebuild().await.is_err());
    assert!(matches!(
        core.knowledge.search("nonempty", 10).await,
        Err(Error::IndexStale)
    ));
    assert_eq!(core.knowledge.health().await.unwrap().document_count, 1);
    tokio::fs::rename(
        root.join("knowledge/wiki-offline"),
        root.join("knowledge/wiki"),
    )
    .await
    .unwrap();
    core.knowledge.rebuild().await.unwrap();
    assert_eq!(
        core.knowledge.read(&page.id, 1, 10).await.unwrap().body,
        "nonempty"
    );
    drop(core);
    let _ = tokio::fs::remove_dir_all(root).await;
}
#[tokio::test]
async fn memory_root_missing_at_restart_never_erases_existing_index() {
    let root = std::env::temp_dir().join(format!("memory_restart_{}", Uuid::new_v4()));
    let core = Runtime::open(root.clone()).await.unwrap();
    core.memory
        .put_fact(FactInput {
            id: None,
            title: "Language".into(),
            body: "use Vietnamese".into(),
            expected_revision: Some(0),
            source_ref: Some("admin".into()),
            confirmed: true,
        })
        .await
        .unwrap();
    drop(core);
    tokio::fs::rename(root.join("memory/facts"), root.join("memory/facts-offline"))
        .await
        .unwrap();
    assert!(matches!(
        Runtime::open(root.clone()).await,
        Err(Error::Unavailable)
    ));
    tokio::fs::rename(root.join("memory/facts-offline"), root.join("memory/facts"))
        .await
        .unwrap();
    let recovered = Runtime::open(root.clone()).await.unwrap();
    assert_eq!(
        recovered
            .memory
            .recall("Vietnamese", 5)
            .await
            .unwrap()
            .len(),
        1
    );
    drop(recovered);
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn source_tamper_is_detected_when_reopening_indexes() {
    let root = std::env::temp_dir().join(format!("provenance_restart_{}", Uuid::new_v4()));
    let core = Runtime::open(root.clone()).await.unwrap();
    let source = core
        .knowledge
        .import_source(SourceInput {
            name: "evidence.md".into(),
            content: "unmodified".into(),
            provenance: "admin".into(),
        })
        .await
        .unwrap();
    let reference = format!("{}:1", source.id);
    let draft = core
        .knowledge
        .put_draft(WikiInput {
            id: None,
            title: "Source".into(),
            body: "trustedmaterial".into(),
            expected_revision: Some(0),
            source_ref: Some(reference.clone()),
        })
        .await
        .unwrap();
    core.knowledge
        .publish(&draft.id, draft.revision, &reference)
        .await
        .unwrap();
    drop(core);
    tokio::fs::write(
        root.join("knowledge/raw")
            .join(&source.id)
            .join("1/original.md"),
        "tampered",
    )
    .await
    .unwrap();
    let reopened = Runtime::open(root.clone()).await.unwrap();
    assert_eq!(reopened.knowledge.health().await.unwrap().health, "stale");
    assert!(matches!(
        reopened.knowledge.search("trustedmaterial", 5).await,
        Err(Error::IndexStale)
    ));
    assert_eq!(reopened.memory.health().await.unwrap().health, "ready");
    drop(reopened);
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[test]
fn memory_summary_only_confirmed_active() {
    let rec = Record {
        id: "mem_1".into(),
        kind: "memory".into(),
        title: "Pref".into(),
        body: "Vietnamese".into(),
        revision: 1,
        status: "forgotten".into(),
        scope: Some("owner".into()),
        confirmed: false,
        source_ref: Some("admin".into()),
        draft_base_revision: None,
    };
    let scanned = vault::Scanned {
        record: rec,
        relative_path: String::new(),
        content_sha256: String::new(),
        mtime_ns: 0,
        size_bytes: 0,
    };
    assert_eq!(MemoryCore::generate(&[scanned]).unwrap(), "# Memory\n\n");
}
