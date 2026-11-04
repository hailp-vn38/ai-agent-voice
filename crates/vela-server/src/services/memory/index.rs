//! Search Projection: one FTS5 table per scope, derived from `facts/` and safe to delete.
//!
//! The FTS column holds `normalize`d text, because `remove_diacritics` alone does not fold `đ`.
//! Errors stay as `sqlx::Error`; callers respond by discarding or rebuilding this file.

use super::{normalize::normalize, types::MemoryFact, vault};
use crate::database::memory_index::{self, IndexRow};
use std::{fs, io, path::Path};

const MAX_QUERY_TOKENS: usize = 16;

fn row_of(fact: &MemoryFact) -> IndexRow<'_> {
    IndexRow {
        fact_id: fact.id.as_str(),
        title: normalize(&fact.title),
        hook: normalize(&fact.hook),
        body: normalize(&fact.body),
    }
}

/// A missing index stays missing: creating it here would hold one fact and hide the rest from the
/// rebuild that the next `recall` runs when the file is absent.
pub(super) async fn upsert(path: &Path, fact: &MemoryFact) -> Result<(), sqlx::Error> {
    if !path.exists() {
        return Ok(());
    }
    memory_index::replace_row(path, &row_of(fact)).await
}

pub(super) async fn remove(path: &Path, id: &str) -> Result<(), sqlx::Error> {
    if !path.exists() {
        return Ok(());
    }
    memory_index::remove_row(path, id).await
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
    memory_index::search(path, &expression, limit).await
}

/// Builds beside the live file and renames, so a reader sees the old index or the whole new one.
pub(super) async fn rebuild(path: &Path, facts: &[&MemoryFact]) -> Result<(), sqlx::Error> {
    let parent = path.parent().ok_or(sqlx::Error::RowNotFound)?;
    vault::ensure_dir(parent)?;
    let temp = vault::temp_path(path)?;
    let rows: Vec<IndexRow<'_>> = facts.iter().map(|fact| row_of(fact)).collect();
    let built = memory_index::create(&temp, &rows).await;
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
