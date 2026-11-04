use super::*;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn publication_covers_commit_and_invalidation_but_not_a_rolled_back_revocation() {
    for reject_audit in [false, true] {
        let path =
            std::env::temp_dir().join(format!("mcp-publication-{}.db", uuid::Uuid::new_v4()));
        let db = Database::connect(&crate::config::DatabaseConfig {
            url: format!("sqlite://{}", path.display()),
            max_connections: 1,
            ..Default::default()
        })
        .await
        .unwrap();
        db.create_mcp_server(
            McpInput {
                key: "test_mcp",
                name: "Test",
                url: "https://example.com/mcp",
                auth_type: "none",
                auth_header_name: None,
                credential: None,
                connect_timeout_ms: 5000,
                request_timeout_ms: 30000,
            },
            "create",
        )
        .await
        .unwrap();
        if reject_audit {
            sqlx::query("CREATE TRIGGER reject_update_audit BEFORE INSERT ON admin_audit_events WHEN NEW.action='update' BEGIN SELECT RAISE(ABORT,'audit unavailable'); END")
                .execute(&db.pool).await.unwrap();
        }
        let old = mcp_by(&db, "test_mcp").await.unwrap();
        let close = db.tool_security.register(1, vec![old.id]);
        let security = db.tool_security.clone();
        // Hold the sole pool connection: the mutation must hold publication protection while
        // waiting for SQLite, and a concurrent admission reader must remain outside it.
        let mut connection = db.pool.acquire().await.unwrap();
        let writer_db = db.clone();
        let writer = tokio::spawn(async move {
            writer_db
                .update_mcp_server(
                    &old,
                    1,
                    McpChanges {
                        name: &old.name,
                        url: &old.url,
                        auth_type: &old.auth_type,
                        auth_header_name: old.auth_header_name.as_deref(),
                        credential: old.credential_json.as_deref(),
                        connect_timeout: old.connect_timeout_ms,
                        request_timeout: old.request_timeout_ms,
                        enabled: 0,
                    },
                    "update",
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while security.publication.try_read().is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("writer must acquire publication before waiting for the pool");
        let observed: (i64, i64) =
            sqlx::query_as("SELECT enabled,revision FROM mcp_servers WHERE key='test_mcp'")
                .fetch_one(&mut *connection)
                .await
                .unwrap();
        assert_eq!(observed, (1, 1));
        assert!(!close.is_cancelled());
        let reader_db = db.clone();
        let reader_security = security.clone();
        let reader_close = close.clone();
        let (attempted, attempt) = tokio::sync::oneshot::channel();
        let reader = tokio::spawn(async move {
            let acquisition = reader_security.publication.read();
            tokio::pin!(acquisition);
            tokio::select! {
                biased;
                _ = &mut acquisition => panic!("admission crossed an unpublished mutation"),
                _ = std::future::ready(()) => {}
            }
            attempted.send(()).unwrap();
            let _publication = acquisition.await;
            let cancelled = reader_close.is_cancelled();
            let current = mcp_by(&reader_db, "test_mcp").await.unwrap();
            (current.enabled, current.revision, cancelled)
        });
        attempt.await.unwrap();
        assert!(!reader.is_finished());
        drop(connection);
        let result = tokio::time::timeout(Duration::from_secs(5), writer)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.is_err(), reject_audit);
        let observed = tokio::time::timeout(Duration::from_secs(5), reader)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            observed,
            if reject_audit {
                (1, 1, false)
            } else {
                (0, 2, true)
            }
        );
        db.pool.close().await;
        let _ = tokio::fs::remove_file(path).await;
    }
}
