//! Search Projection: one FTS5 table per scope, derived from `facts/` and safe to delete.
//!
//! The FTS column holds `normalize`d text, because `remove_diacritics` alone does not fold `đ`.
//! Errors stay as `sqlx::Error`; callers respond by discarding or rebuilding this file.

use super::{normalize::normalize, types::MemoryFact, vault};
use sqlx::{
    ConnectOptions, Connection, Row, SqliteConnection,
    sqlite::{SqliteConnectOptions, SqliteJournalMode},
};
use std::{fs, io, path::Path};

const MAX_QUERY_TOKENS: usize = 16;

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

async fn insert(conn: &mut SqliteConnection, fact: &MemoryFact) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO facts_fts(fact_id,title,hook,body) VALUES (?,?,?,?)")
        .bind(fact.id.as_str())
        .bind(normalize(&fact.title))
        .bind(normalize(&fact.hook))
        .bind(normalize(&fact.body))
        .execute(conn)
        .await?;
    Ok(())
}

/// A missing index stays missing: creating it here would hold one fact and hide the rest from the
/// rebuild that the next `recall` runs when the file is absent.
pub(super) async fn upsert(path: &Path, fact: &MemoryFact) -> Result<(), sqlx::Error> {
    if !path.exists() {
        return Ok(());
    }
    let mut conn = connect(path, false).await?;
    let mut tx = conn.begin().await?;
    sqlx::query("DELETE FROM facts_fts WHERE fact_id=?")
        .bind(fact.id.as_str())
        .execute(&mut *tx)
        .await?;
    insert(&mut tx, fact).await?;
    tx.commit().await?;
    conn.close().await
}

pub(super) async fn remove(path: &Path, id: &str) -> Result<(), sqlx::Error> {
    if !path.exists() {
        return Ok(());
    }
    let mut conn = connect(path, false).await?;
    sqlx::query("DELETE FROM facts_fts WHERE fact_id=?")
        .bind(id)
        .execute(&mut conn)
        .await?;
    // FTS5 only marks deleted terms until segments merge; merge now so the terms really leave.
    sqlx::query("INSERT INTO facts_fts(facts_fts) VALUES ('optimize')")
        .execute(&mut conn)
        .await?;
    conn.close().await
}

/// The FTS5 query for `text`, or `None` when it has no searchable token. Tokens are alphanumeric
/// and quoted, so user text cannot use FTS5 operators.
fn match_expression(text: &str) -> Option<String> {
    let normalized = normalize(text);
    let tokens: Vec<String> = normalized
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .take(MAX_QUERY_TOKENS)
        .map(|token| format!("\"{token}\""))
        .collect();
    (!tokens.is_empty()).then(|| tokens.join(" OR "))
}

/// Fact ids, best match first.
pub(super) async fn search(
    path: &Path,
    text: &str,
    limit: usize,
) -> Result<Vec<String>, sqlx::Error> {
    let Some(expression) = match_expression(text) else {
        return Ok(Vec::new());
    };
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

async fn build(path: &Path, facts: &[&MemoryFact]) -> Result<(), sqlx::Error> {
    let mut conn = connect(path, true).await?;
    let mut tx = conn.begin().await?;
    sqlx::query(
        "CREATE VIRTUAL TABLE facts_fts USING fts5(\
            fact_id UNINDEXED, title, hook, body, \
            tokenize = 'unicode61 remove_diacritics 2')",
    )
    .execute(&mut *tx)
    .await?;
    for fact in facts {
        insert(&mut tx, fact).await?;
    }
    tx.commit().await?;
    conn.close().await
}

/// Builds beside the live file and renames, so a reader sees the old index or the whole new one.
pub(super) async fn rebuild(path: &Path, facts: &[&MemoryFact]) -> Result<(), sqlx::Error> {
    let parent = path.parent().ok_or(sqlx::Error::RowNotFound)?;
    vault::ensure_dir(parent)?;
    let temp = vault::temp_path(path)?;
    let built = build(&temp, facts).await;
    let moved = match built {
        Ok(()) => {
            remove_journal(path)?;
            fs::rename(&temp, path).map_err(sqlx::Error::from)
        }
        Err(error) => Err(error),
    };
    if moved.is_err() {
        let _ = fs::remove_file(&temp);
    }
    moved
}

fn remove_journal(path: &Path) -> io::Result<()> {
    let mut journal = path.as_os_str().to_owned();
    journal.push("-journal");
    vault::remove_durable(Path::new(&journal))
}

/// What a failed update falls back to: no index at all, which `recall` rebuilds.
pub(super) fn discard(path: &Path) -> io::Result<()> {
    remove_journal(path)?;
    vault::remove_durable(path)
}

#[cfg(test)]
mod tests {
    use super::match_expression;
    use sqlx::{Connection, Row, SqliteConnection, sqlite::SqliteConnectOptions};

    /// Gate for the whole index design: the spec falls back to `rusqlite` if this ever fails.
    #[tokio::test]
    async fn memory_index_sqlx_creates_fts5_table_with_chosen_tokenizer() {
        let path =
            std::env::temp_dir().join(format!("memory-fts5-{}.sqlite", uuid::Uuid::new_v4()));
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let mut conn = SqliteConnection::connect_with(&options).await.unwrap();
        sqlx::query(
            "CREATE VIRTUAL TABLE facts_fts USING fts5(fact_id UNINDEXED, title, hook, body, tokenize = 'unicode61 remove_diacritics 2')",
        )
        .execute(&mut conn)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO facts_fts(fact_id,title,hook,body) VALUES ('fact_a','dat lich','','goi lai')",
        )
        .execute(&mut conn)
        .await
        .unwrap();
        let row = sqlx::query(
            "SELECT fact_id FROM facts_fts WHERE facts_fts MATCH '\"lich\"' ORDER BY bm25(facts_fts)",
        )
        .fetch_one(&mut conn)
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>(0), "fact_a");
        conn.close().await.unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn memory_index_query_quotes_tokens_and_caps_them_at_16() {
        assert_eq!(
            match_expression("Đặt lịch, \"NEAR\" OR*"),
            Some(r#""dat" OR "lich" OR "near" OR "or""#.to_string())
        );
        assert_eq!(match_expression(" ?! "), None);
        let many = (0..40)
            .map(|i| format!("t{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let expression = match_expression(&many).unwrap();
        assert_eq!(expression.matches(" OR ").count(), 15);
    }
}
