//! Two physically separate SQLx/SQLite FTS5 projections using one implementation.
//! Canonical Markdown always outranks derived rows. A failed scan preserves old
//! rows but marks search unavailable, never silently claims freshness.
use crate::services::knowledge::{
    Error,
    vault::{MarkdownVault, Scanned},
};
use serde::Serialize;
use sqlx::{
    SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::Duration,
};
static KNOWLEDGE_MIGRATOR: Migrator = sqlx::migrate!("./migrations/knowledge");
static MEMORY_MIGRATOR: Migrator = sqlx::migrate!("./migrations/memory");
const INDEXER_VERSION: i64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    Knowledge,
    Memory,
}
impl Domain {
    fn file(self) -> &'static str {
        match self {
            Self::Knowledge => "knowledge_index.db",
            Self::Memory => "memory_index.db",
        }
    }
    fn doc_table(self) -> &'static str {
        match self {
            Self::Knowledge => "kb_documents",
            Self::Memory => "mem_documents",
        }
    }
    fn fts_table(self) -> &'static str {
        match self {
            Self::Knowledge => "kb_fts",
            Self::Memory => "mem_fts",
        }
    }
    fn health_table(self) -> &'static str {
        match self {
            Self::Knowledge => "kb_index_state",
            Self::Memory => "mem_index_state",
        }
    }
    fn id_col(self) -> &'static str {
        match self {
            Self::Knowledge => "stable_id",
            Self::Memory => "fact_id",
        }
    }
}

#[derive(Clone, Serialize)]
pub struct SearchHit {
    pub id: String,
    pub title: String,
    pub revision: i64,
    pub excerpt: String,
    pub source_ref: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct Health {
    pub health: String,
    pub document_count: i64,
    pub last_complete_scan_at: Option<i64>,
}

pub struct SearchIndex {
    pool: SqlitePool,
    domain: Domain,
}
impl SearchIndex {
    pub async fn open(root: &Path, domain: Domain) -> Result<Self, Error> {
        let file = root.join(domain.file());
        let opts = SqliteConnectOptions::new()
            .filename(&file)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_millis(3000));
        let pool = SqlitePoolOptions::new()
            .max_connections(3)
            .connect_with(opts)
            .await?;
        let migrator = match domain {
            Domain::Knowledge => &KNOWLEDGE_MIGRATOR,
            Domain::Memory => &MEMORY_MIGRATOR,
        };
        migrator.run(&pool).await.map_err(|_| Error::Unavailable)?;
        Ok(Self { pool, domain })
    }
    pub async fn mark_stale(&self) -> Result<(), Error> {
        let sql = format!(
            "UPDATE {} SET health='stale' WHERE singleton=1",
            self.domain.health_table()
        );
        sqlx::query(&sql).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn health(&self) -> Result<Health, Error> {
        let sql = format!(
            "SELECT health,last_complete_scan_at FROM {} WHERE singleton=1",
            self.domain.health_table()
        );
        use sqlx::Row;
        let row = sqlx::query(&sql).fetch_one(&self.pool).await?;
        let count: i64 =
            sqlx::query_scalar(&format!("SELECT count(*) FROM {}", self.domain.doc_table()))
                .fetch_one(&self.pool)
                .await?;
        Ok(Health {
            health: row.try_get("health")?,
            last_complete_scan_at: row.try_get("last_complete_scan_at")?,
            document_count: count,
        })
    }
    /// Each scan is staged in memory first. One SQLite transaction publishes all
    /// changed rows AND the ready marker; failure never mass-retires documents.
    pub async fn reconcile(&self, vault: &MarkdownVault) -> Result<usize, Error> {
        self.reconcile_with(vault, false).await
    }
    /// Force-repair the complete FTS projection even if the file hashes have not changed.
    pub async fn rebuild(&self, vault: &MarkdownVault) -> Result<usize, Error> {
        self.reconcile_with(vault, true).await
    }
    async fn reconcile_with(&self, vault: &MarkdownVault, force: bool) -> Result<usize, Error> {
        self.mark_stale().await?;
        let scanned = vault.scan().await?;
        for doc in &scanned {
            match self.domain {
                Domain::Knowledge if doc.record.kind == "wiki" => {}
                Domain::Memory
                    if doc.record.kind == "memory"
                        && doc.record.scope.as_deref() == Some("owner") => {}
                _ => return Err(Error::InvalidInput),
            }
        }
        let mut tx = self.pool.begin().await?;
        if force {
            // Explicit rebuild removes orphaned FTS rows as well as repairing missing
            // matching rows. A rollback restores the old index atomically.
            let sql = format!("DELETE FROM {}", self.domain.fts_table());
            sqlx::query(&sql).execute(&mut *tx).await?;
        }
        let existing_sql = format!(
            "SELECT {},content_sha256,relative_path,revision,indexer_version FROM {}",
            self.domain.id_col(),
            self.domain.doc_table()
        );
        let rows = sqlx::query(&existing_sql).fetch_all(&mut *tx).await?;
        use sqlx::Row;
        let mut existing = HashMap::<String, (String, String, i64, i64)>::new();
        for row in rows {
            existing.insert(
                row.try_get::<String, _>(self.domain.id_col())?,
                (
                    row.try_get("content_sha256")?,
                    row.try_get("relative_path")?,
                    row.try_get("revision")?,
                    row.try_get("indexer_version")?,
                ),
            );
        }
        let mut seen = HashSet::new();
        for doc in &scanned {
            if !seen.insert(doc.record.id.clone()) {
                return Err(Error::InvalidInput);
            }
            let unchanged =
                existing
                    .get(&doc.record.id)
                    .is_some_and(|(hash, path, rev, version)| {
                        hash == &doc.content_sha256
                            && path == &doc.relative_path
                            && *rev == doc.record.revision
                            && *version == INDEXER_VERSION
                    });
            if unchanged && !force {
                continue;
            }
            self.upsert(&mut tx, doc).await?;
        }
        for id in existing.keys().filter(|id| !seen.contains(*id)) {
            let sql = format!(
                "DELETE FROM {} WHERE {}=?",
                self.domain.fts_table(),
                self.domain.id_col()
            );
            sqlx::query(&sql).bind(id).execute(&mut *tx).await?;
            let sql = format!(
                "DELETE FROM {} WHERE {}=?",
                self.domain.doc_table(),
                self.domain.id_col()
            );
            sqlx::query(&sql).bind(id).execute(&mut *tx).await?;
        }
        sqlx::query(&format!("UPDATE {} SET health='ready',indexer_version=?,last_complete_scan_at=strftime('%s','now'),last_error=NULL WHERE singleton=1",
            self.domain.health_table())).bind(INDEXER_VERSION).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(scanned.len())
    }
    async fn upsert(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        doc: &Scanned,
    ) -> Result<(), Error> {
        let record = &doc.record;
        match self.domain {
            Domain::Knowledge => {
                sqlx::query("INSERT INTO kb_documents (stable_id,kind,relative_path,title,revision,content_sha256,mtime_ns,size_bytes,state,indexer_version)
                    VALUES(?,?,?,?,?,?,?,?,?,?)
                    ON CONFLICT(stable_id) DO UPDATE SET kind=excluded.kind,relative_path=excluded.relative_path,
                      title=excluded.title,revision=excluded.revision,content_sha256=excluded.content_sha256,
                      mtime_ns=excluded.mtime_ns,size_bytes=excluded.size_bytes,state=excluded.state,indexer_version=excluded.indexer_version")
                    .bind(&record.id).bind(&record.kind).bind(&doc.relative_path).bind(&record.title).bind(record.revision)
                    .bind(&doc.content_sha256).bind(doc.mtime_ns).bind(doc.size_bytes).bind(&record.status).bind(INDEXER_VERSION)
                    .execute(&mut **tx).await?;
                sqlx::query("DELETE FROM kb_fts WHERE stable_id=?")
                    .bind(&record.id)
                    .execute(&mut **tx)
                    .await?;
                sqlx::query("INSERT INTO kb_fts(stable_id,title,body) VALUES(?,?,?)")
                    .bind(&record.id)
                    .bind(&record.title)
                    .bind(&record.body)
                    .execute(&mut **tx)
                    .await?;
            }
            Domain::Memory => {
                sqlx::query("INSERT INTO mem_documents (fact_id,relative_path,title,owner_scope,revision,status,confirmed,content_sha256,mtime_ns,size_bytes,indexer_version)
                    VALUES(?,?,?,?,?,?,?,?,?,?,?)
                    ON CONFLICT(fact_id) DO UPDATE SET relative_path=excluded.relative_path,title=excluded.title,
                    owner_scope=excluded.owner_scope,revision=excluded.revision,status=excluded.status,
                    confirmed=excluded.confirmed,content_sha256=excluded.content_sha256,
                    mtime_ns=excluded.mtime_ns,size_bytes=excluded.size_bytes,indexer_version=excluded.indexer_version")
                    .bind(&record.id).bind(&doc.relative_path).bind(&record.title)
                    .bind(record.scope.as_deref().unwrap_or("owner")).bind(record.revision).bind(&record.status)
                    .bind(i64::from(record.confirmed)).bind(&doc.content_sha256)
                    .bind(doc.mtime_ns).bind(doc.size_bytes).bind(INDEXER_VERSION)
                    .execute(&mut **tx).await?;
                sqlx::query("DELETE FROM mem_fts WHERE fact_id=?")
                    .bind(&record.id)
                    .execute(&mut **tx)
                    .await?;
                sqlx::query("INSERT INTO mem_fts(fact_id,title,body) VALUES(?,?,?)")
                    .bind(&record.id)
                    .bind(&record.title)
                    .bind(&record.body)
                    .execute(&mut **tx)
                    .await?;
            }
        }
        Ok(())
    }
    pub async fn search(&self, query: &str, limit: i64) -> Result<Vec<SearchHit>, Error> {
        self.search_with(query, limit, false).await
    }
    pub async fn admin_search(&self, query: &str, limit: i64) -> Result<Vec<SearchHit>, Error> {
        self.search_with(query, limit, true).await
    }
    async fn search_with(
        &self,
        query: &str,
        limit: i64,
        include_drafts: bool,
    ) -> Result<Vec<SearchHit>, Error> {
        use sqlx::Row;
        if query.trim().is_empty() || query.len() > 256 || !(1..=50).contains(&limit) {
            return Err(Error::InvalidInput);
        }
        if self.health().await?.health != "ready" {
            return Err(Error::IndexStale);
        }
        // Entire user query is a literal FTS5 phrase: never evaluate query syntax
        // or allow model input to create SQL expressions.
        let phrase = format!("\"{}\"", query.trim().replace('"', "\"\""));
        let sql=match self.domain {
            Domain::Knowledge if include_drafts => "SELECT d.stable_id AS id,d.title,d.revision,substr(f.body,1,600) AS excerpt
                FROM kb_fts f JOIN kb_documents d ON d.stable_id=f.stable_id
                WHERE kb_fts MATCH ? AND d.kind='wiki' AND d.state IN ('draft','verified')
                ORDER BY bm25(kb_fts) LIMIT ?",
            Domain::Knowledge => "SELECT d.stable_id AS id,d.title,d.revision,substr(f.body,1,600) AS excerpt
                FROM kb_fts f JOIN kb_documents d ON d.stable_id=f.stable_id
                WHERE kb_fts MATCH ? AND d.kind='wiki' AND d.state='verified'
                ORDER BY bm25(kb_fts) LIMIT ?",
            Domain::Memory=>"SELECT d.fact_id AS id,d.title,d.revision,substr(f.body,1,600) AS excerpt
                FROM mem_fts f JOIN mem_documents d ON d.fact_id=f.fact_id
                WHERE mem_fts MATCH ? AND d.owner_scope='owner' AND d.status='active' AND d.confirmed=1
                ORDER BY bm25(mem_fts) LIMIT ?"
        };
        let rows = sqlx::query(sql)
            .bind(phrase)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        let mut hits = Vec::new();
        for row in rows {
            hits.push(SearchHit {
                id: row.try_get("id")?,
                title: row.try_get("title")?,
                revision: row.try_get("revision")?,
                excerpt: row.try_get("excerpt")?,
                source_ref: None, // KnowledgeCore enriches verified hits from canonical Markdown.
            });
        }
        Ok(hits)
    }
}
