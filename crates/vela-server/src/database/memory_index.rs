//! SQL of the Memory Search Projection: one FTS5 file per Memory Scope, outside the control database.
//! `services::memory::index` is the only caller and decides what text goes in.

use sqlx::{
    ConnectOptions, Connection, Row, SqliteConnection,
    sqlite::{SqliteConnectOptions, SqliteJournalMode},
};
use std::path::Path;

/// Text columns are stored as given, already normalized by the caller.
pub(crate) struct IndexRow<'a> {
    pub(crate) fact_id: &'a str,
    pub(crate) title: String,
    pub(crate) hook: String,
    pub(crate) body: String,
}

async fn connect(path: &Path, create: bool) -> Result<SqliteConnection, sqlx::Error> {
    SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(create)
        .journal_mode(SqliteJournalMode::Delete)
        // Forgotten text must not linger in free pages.
        .pragma("secure_delete", "ON")
        .disable_statement_logging()
        .connect()
        .await
}

async fn insert(conn: &mut SqliteConnection, row: &IndexRow<'_>) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO facts_fts(fact_id,title,hook,body) VALUES (?,?,?,?)")
        .bind(row.fact_id)
        .bind(&row.title)
        .bind(&row.hook)
        .bind(&row.body)
        .execute(conn)
        .await?;
    Ok(())
}

/// Writes a new index file holding `rows`. `path` must not exist yet.
pub(crate) async fn create(path: &Path, rows: &[IndexRow<'_>]) -> Result<(), sqlx::Error> {
    let mut conn = connect(path, true).await?;
    let mut tx = conn.begin().await?;
    sqlx::query(
        "CREATE VIRTUAL TABLE facts_fts USING fts5(\
            fact_id UNINDEXED, title, hook, body, \
            tokenize = 'unicode61 remove_diacritics 2')",
    )
    .execute(&mut *tx)
    .await?;
    for row in rows {
        insert(&mut tx, row).await?;
    }
    tx.commit().await?;
    conn.close().await
}

pub(crate) async fn replace_row(path: &Path, row: &IndexRow<'_>) -> Result<(), sqlx::Error> {
    let mut conn = connect(path, false).await?;
    let mut tx = conn.begin().await?;
    sqlx::query("DELETE FROM facts_fts WHERE fact_id=?")
        .bind(row.fact_id)
        .execute(&mut *tx)
        .await?;
    insert(&mut tx, row).await?;
    tx.commit().await?;
    conn.close().await
}

pub(crate) async fn remove_row(path: &Path, fact_id: &str) -> Result<(), sqlx::Error> {
    let mut conn = connect(path, false).await?;
    sqlx::query("DELETE FROM facts_fts WHERE fact_id=?")
        .bind(fact_id)
        .execute(&mut conn)
        .await?;
    // FTS5 only marks deleted terms until segments merge; merge now so the terms really leave.
    sqlx::query("INSERT INTO facts_fts(facts_fts) VALUES ('optimize')")
        .execute(&mut conn)
        .await?;
    conn.close().await
}

/// Fact ids matching an FTS5 `expression`, best match first.
pub(crate) async fn search(
    path: &Path,
    expression: &str,
    limit: usize,
) -> Result<Vec<String>, sqlx::Error> {
    let mut conn = connect(path, false).await?;
    let rows = sqlx::query(
        "SELECT fact_id FROM facts_fts WHERE facts_fts MATCH ? ORDER BY bm25(facts_fts) LIMIT ?",
    )
    .bind(expression)
    .bind(i64::try_from(limit).unwrap_or(i64::MAX))
    .fetch_all(&mut conn)
    .await?;
    conn.close().await?;
    rows.iter().map(|row| row.try_get(0)).collect()
}
