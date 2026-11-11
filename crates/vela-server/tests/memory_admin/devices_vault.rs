use super::harness::*;

#[tokio::test]
async fn memory_is_kept_apart_per_device_and_follows_a_device_to_a_new_agent() {
    let server = start().await;
    server.agent("home").await;
    server.agent("office").await;
    let kitchen = server.device("kitchen", "home").await;
    server.device("bedroom", "home").await;
    let id = server
        .remember("kitchen", "Bí mật bếp", "Chỉ bếp biết")
        .await;

    assert!(server.facts("bedroom").await.is_empty());
    let searched = ok(server.get("/memory/facts?device=bedroom&q=bep")).await;
    assert_eq!(searched["items"], json!([]));
    expect_error(
        server
            .get(&format!("/memory/facts/{id}?device=bedroom"))
            .send()
            .await
            .unwrap(),
        StatusCode::NOT_FOUND,
        "memory_not_found",
    )
    .await;

    let revision = kitchen["revision"].as_i64().unwrap();
    let moved = ok(server
        .client
        .patch(format!("{}/devices/kitchen", server.base))
        .bearer_auth(&server.token)
        .header("if-match", format!("\"{revision}\""))
        .json(&json!({"agent_key": "office"})))
    .await;
    assert_eq!(moved["agent_key"], "office");
    let items = server.facts("kitchen").await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], id.as_str());
    server.task.abort();
}

#[tokio::test]
async fn memory_of_the_virtual_device_lives_under_its_own_directory() {
    let server = start().await;
    server.agent("home").await;
    let settings = ok(server.put(
        "/system/assistant-device",
        json!({"enabled": true, "agent_key": "home", "template_key": null, "memory_mode": "read"}),
    ))
    .await;
    assert_eq!(settings["memory_mode"], "read");
    let device = settings["device_id"].as_str().unwrap();
    server
        .remember(device, "Trợ lý", "Ghi chú của trợ lý")
        .await;
    assert_eq!(server.device_dirs(), vec!["web-assistant".to_string()]);
    assert_eq!(server.facts(device).await.len(), 1);
    server.task.abort();
}

#[tokio::test]
async fn memory_vault_is_purged_only_after_the_device_delete_succeeds() {
    let server = start().await;
    server.agent("home").await;
    let doomed = server.device("doomed", "home").await;
    let keeper = server.device("keeper", "home").await;
    server
        .remember("doomed", "Của doomed", "Nội dung doomed")
        .await;
    server
        .remember("keeper", "Của keeper", "Nội dung keeper")
        .await;
    let before = server.device_dirs();
    assert_eq!(before.len(), 2);

    // A stale revision refuses the delete, so the vault stays.
    let stale = delete_device(&server, "doomed", 99).await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(server.device_dirs(), before);
    assert_eq!(server.facts("doomed").await.len(), 1);

    // So does `device_in_use`, which is what history on the Device means.
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        server.root.join("vela.db").display()
    ))
    .await
    .unwrap();
    let keeper_row = keeper["id"].as_i64().unwrap();
    sqlx::query(
        "INSERT INTO history_messages \
         (session_id, device_id, agent_id, template_id, sequence, turn_id, role, text, created_at) \
         SELECT 's', id, agent_id, NULL, 1, '1', 'user', 'x', 1 FROM devices WHERE id = ?",
    )
    .bind(keeper_row)
    .execute(&pool)
    .await
    .unwrap();
    let in_use = delete_device(&server, "keeper", keeper["revision"].as_i64().unwrap()).await;
    expect_error(in_use, StatusCode::CONFLICT, "device_in_use").await;
    assert_eq!(server.device_dirs(), before);
    assert_eq!(server.facts("keeper").await.len(), 1);

    // A successful delete takes the vault with it, and only that vault.
    let deleted = delete_device(&server, "doomed", doomed["revision"].as_i64().unwrap()).await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert_eq!(server.device_dirs().len(), 1);
    assert_eq!(server.facts("keeper").await.len(), 1);
    expect_error(
        server
            .get("/memory/facts?device=doomed")
            .send()
            .await
            .unwrap(),
        StatusCode::NOT_FOUND,
        "device_not_found",
    )
    .await;

    // Enrolling the same id again starts empty.
    server.device("doomed", "home").await;
    assert!(server.facts("doomed").await.is_empty());
    server.task.abort();
}

#[tokio::test]
async fn memory_managed_device_refuses_delete_and_keeps_its_vault() {
    let server = start().await;
    server.agent("home").await;
    let settings = ok(server.put(
        "/system/assistant-device",
        json!({"enabled": true, "agent_key": "home", "template_key": null}),
    ))
    .await;
    let device = settings["device_id"].as_str().unwrap();
    server
        .remember(device, "Trợ lý", "Ghi chú của trợ lý")
        .await;
    let refused = delete_device(&server, device, settings["revision"].as_i64().unwrap()).await;
    expect_error(
        refused,
        StatusCode::CONFLICT,
        "managed_device_use_system_settings",
    )
    .await;
    assert_eq!(server.facts(device).await.len(), 1);
    server.task.abort();
}

#[tokio::test]
async fn memory_startup_reconciles_in_the_background_and_only_reports_orphans() {
    let root = temp_root();
    let extra = memory_on(&root);
    let server = start_at(&root, &extra).await;
    server.agent("home").await;
    server.device("kitchen", "home").await;
    let id = server.remember("kitchen", "Lịch", "Nội dung gốc").await;
    let dir = server.device_dirs().remove(0);
    server.task.abort();

    // While the server is down: an owner edit, and a directory no Device owns.
    let facts_dir = root.join("vault/devices").join(&dir).join("facts");
    let fact_path = fs::read_dir(&facts_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let text = fs::read_to_string(&fact_path).unwrap();
    fs::write(&fact_path, text.replace("Nội dung gốc", "Nội dung đã sửa")).unwrap();
    let orphan = root.join("vault/devices/0000000000000000");
    fs::create_dir_all(orphan.join("facts")).unwrap();
    fs::write(orphan.join("facts/giu-lai.md"), "giữ lại").unwrap();

    let server = start_at(&root, &extra).await;
    // Convergence is visible without anyone asking for it.
    let path = format!("/memory/facts/{id}?device=kitchen");
    eventually(|| async { ok(server.get(&path)).await["revision"] == 2 }).await;
    let fact = ok(server.get(&path)).await;
    assert_eq!(fact["origin"], "owner-edited");
    assert_eq!(fact["body"], "Nội dung đã sửa");
    assert_eq!(
        fs::read_to_string(orphan.join("facts/giu-lai.md")).unwrap(),
        "giữ lại"
    );
    server.task.abort();
}
