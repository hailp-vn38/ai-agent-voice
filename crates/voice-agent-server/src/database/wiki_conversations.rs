//! Persistent Wiki Chat conversations.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Sqlite, Transaction};

use super::Database;

const MAX_CONVERSATIONS: i64 = 200;
const MAX_MESSAGES_PER_CONVERSATION: i64 = 100;
pub(crate) const MAX_STORED_ANSWER_BYTES: usize = 64 * 1024;
const MAX_STORED_TOOLS_BYTES: usize = 64 * 1024;
pub(crate) const MAX_STORED_GRAPH_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ToolStep {
    pub call_id: String,
    pub name: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    pub pages: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct HistoryMessage {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ConversationDetail {
    pub id: String,
    pub title: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub updated_at_ms: i64,
    pub message_count: i64,
    pub running: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ConversationMessage {
    pub id: String,
    pub seq: i64,
    pub role: String,
    pub content: String,
    pub content_truncated: bool,
    pub status: String,
    pub error_code: Option<String>,
    pub pinned_paths: Vec<String>,
    pub tools: Vec<ToolStep>,
    pub citations: Vec<String>,
    pub graph: Option<serde_json::Value>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct StartedTurn {
    pub conversation_id: String,
    pub user_message_id: String,
    pub assistant_message_id: String,
    pub user_seq: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct RetriedTurn {
    pub conversation_id: String,
    pub user_message_id: String,
    pub assistant_message_id: String,
    pub user_seq: i64,
    pub pinned_paths: Vec<String>,
}

#[derive(Debug)]
pub(crate) enum RetryTurnError {
    NotFound,
    InvalidInput,
    Busy,
    Sql,
}

#[derive(Debug)]
pub(crate) enum StartTurnError {
    NotFound,
    Full,
    Sql(sqlx::Error),
}

#[derive(Debug)]
pub(crate) enum DeleteConversationError {
    NotFound,
    Busy,
    Sql(sqlx::Error),
}

impl From<sqlx::Error> for DeleteConversationError {
    fn from(value: sqlx::Error) -> Self {
        Self::Sql(value)
    }
}

impl From<sqlx::Error> for StartTurnError {
    fn from(value: sqlx::Error) -> Self {
        Self::Sql(value)
    }
}

#[derive(FromRow)]
struct ConversationRow {
    id: String,
    title: String,
    created_at_ms: i64,
    updated_at_ms: i64,
}

#[derive(FromRow)]
struct ConversationSummaryRow {
    id: String,
    title: String,
    updated_at_ms: i64,
    message_count: i64,
    running: i64,
}

#[derive(FromRow)]
struct MessageRow {
    id: String,
    seq: i64,
    role: String,
    content: String,
    content_truncated: i64,
    status: String,
    error_code: Option<String>,
    pinned_paths_json: Option<String>,
    tools_json: Option<String>,
    citations_json: Option<String>,
    graph_json: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
}

pub(crate) async fn conversation_exists(
    database: &Database,
    id: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM wiki_conversations WHERE id = ?)")
        .bind(id)
        .fetch_one(database.pool())
        .await
}

pub(crate) async fn list_conversations(
    database: &Database,
) -> Result<Vec<ConversationSummary>, sqlx::Error> {
    let rows = sqlx::query_as::<_, ConversationSummaryRow>(
        "SELECT c.id, c.title, c.updated_at_ms, COUNT(m.id) AS message_count, EXISTS(SELECT 1 FROM wiki_conversation_messages running WHERE running.conversation_id = c.id AND running.status = 'running') AS running FROM wiki_conversations c LEFT JOIN wiki_conversation_messages m ON m.conversation_id = c.id GROUP BY c.id ORDER BY c.updated_at_ms DESC, c.created_at_ms DESC, c.id DESC LIMIT 200",
    )
    .fetch_all(database.pool())
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ConversationSummary {
            id: row.id,
            title: row.title,
            updated_at_ms: row.updated_at_ms,
            message_count: row.message_count,
            running: row.running != 0,
        })
        .collect())
}

pub(crate) async fn rename_conversation(
    database: &Database,
    id: &str,
    title: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("UPDATE wiki_conversations SET title = ? WHERE id = ?")
        .bind(title)
        .bind(id)
        .execute(database.pool())
        .await?;
    Ok(result.rows_affected() == 1)
}

pub(crate) async fn delete_conversation(
    database: &Database,
    id: &str,
    request_id: &str,
) -> Result<(), DeleteConversationError> {
    let mut transaction = database.pool().begin_with("BEGIN IMMEDIATE").await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM wiki_conversations WHERE id = ?)")
            .bind(id)
            .fetch_one(&mut *transaction)
            .await?;
    if !exists {
        transaction.rollback().await?;
        return Err(DeleteConversationError::NotFound);
    }
    let running: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM wiki_conversation_messages WHERE conversation_id = ? AND status = 'running')",
    )
    .bind(id)
    .fetch_one(&mut *transaction)
    .await?;
    if running {
        transaction.rollback().await?;
        return Err(DeleteConversationError::Busy);
    }
    let deleted = sqlx::query("DELETE FROM wiki_conversations WHERE id = ?")
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    if deleted.rows_affected() != 1 {
        transaction.rollback().await?;
        return Err(DeleteConversationError::NotFound);
    }
    crate::database::audit::audit(
        &mut *transaction,
        request_id,
        "wiki_conversation",
        None,
        "delete",
        None,
        None,
        crate::database::audit::AuditOutcome::Success,
        1,
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

/// A process restart cannot resume a stream, so every row left running is interrupted before
/// the HTTP router is exposed.
pub(crate) async fn mark_all_running_interrupted(
    database: &Database,
    now_ms: i64,
) -> Result<(), sqlx::Error> {
    let mut transaction = database.pool().begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query(
        "UPDATE wiki_conversations SET updated_at_ms = ? WHERE id IN (SELECT conversation_id FROM wiki_conversation_messages WHERE status = 'running')",
    )
    .bind(now_ms)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE wiki_conversation_messages SET status = 'interrupted', updated_at_ms = ? WHERE status = 'running'",
    )
    .bind(now_ms)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await
}

/// Mark only expired turns in the conversation currently being started. Kept separate so the
/// normal start transaction and the 100-message storage limit can call the same recovery step.
pub(crate) async fn mark_stale_running_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    conversation_id: &str,
    now_ms: i64,
) -> Result<(), sqlx::Error> {
    let cutoff_ms = now_ms.saturating_sub(120_000);
    let result = sqlx::query(
        "UPDATE wiki_conversation_messages SET status = 'interrupted', updated_at_ms = ? WHERE conversation_id = ? AND status = 'running' AND updated_at_ms < ?",
    )
    .bind(now_ms)
    .bind(conversation_id)
    .bind(cutoff_ms)
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() > 0 {
        sqlx::query("UPDATE wiki_conversations SET updated_at_ms = ? WHERE id = ?")
            .bind(now_ms)
            .bind(conversation_id)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

pub(crate) async fn begin_retry(
    database: &Database,
    conversation_id: &str,
    assistant_message_id: &str,
    now_ms: i64,
) -> Result<RetriedTurn, RetryTurnError> {
    let mut transaction = database
        .pool()
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|_| RetryTurnError::Sql)?;
    let conversation_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM wiki_conversations WHERE id = ?)")
            .bind(conversation_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|_| RetryTurnError::Sql)?;
    if !conversation_exists {
        return Err(RetryTurnError::NotFound);
    }
    mark_stale_running_in_transaction(&mut transaction, conversation_id, now_ms)
        .await
        .map_err(|_| RetryTurnError::Sql)?;
    let has_running_message: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM wiki_conversation_messages WHERE conversation_id = ? AND status = 'running')",
    )
    .bind(conversation_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(|_| RetryTurnError::Sql)?;
    if has_running_message {
        return Err(RetryTurnError::Busy);
    }

    let latest: Option<(String, String, String, i64)> = sqlx::query_as(
        "SELECT id, role, status, seq FROM wiki_conversation_messages WHERE conversation_id = ? ORDER BY seq DESC LIMIT 1",
    )
    .bind(conversation_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|_| RetryTurnError::Sql)?;
    let Some((latest_id, role, status, assistant_seq)) = latest else {
        return Err(RetryTurnError::InvalidInput);
    };
    if latest_id != assistant_message_id
        || role != "assistant"
        || !matches!(status.as_str(), "failed" | "interrupted")
    {
        return Err(RetryTurnError::InvalidInput);
    }

    let previous: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, pinned_paths_json FROM wiki_conversation_messages WHERE conversation_id = ? AND seq = ? AND role = 'user'",
    )
    .bind(conversation_id)
    .bind(assistant_seq - 1)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|_| RetryTurnError::Sql)?;
    let Some((user_message_id, pinned_paths_json)) = previous else {
        return Err(RetryTurnError::InvalidInput);
    };

    let updated = sqlx::query(
        "UPDATE wiki_conversation_messages SET content = '', content_truncated = 0, status = 'running', error_code = NULL, tools_json = NULL, citations_json = NULL, graph_json = NULL, updated_at_ms = ? WHERE conversation_id = ? AND id = ? AND role = 'assistant' AND status IN ('failed', 'interrupted')",
    )
    .bind(now_ms)
    .bind(conversation_id)
    .bind(assistant_message_id)
    .execute(&mut *transaction)
    .await
    .map_err(map_retry_update_error)?;
    if updated.rows_affected() != 1 {
        return Err(RetryTurnError::InvalidInput);
    }
    sqlx::query("UPDATE wiki_conversations SET updated_at_ms = ? WHERE id = ?")
        .bind(now_ms)
        .bind(conversation_id)
        .execute(&mut *transaction)
        .await
        .map_err(|_| RetryTurnError::Sql)?;
    transaction
        .commit()
        .await
        .map_err(|_| RetryTurnError::Sql)?;

    Ok(RetriedTurn {
        conversation_id: conversation_id.to_owned(),
        user_message_id,
        assistant_message_id: assistant_message_id.to_owned(),
        user_seq: assistant_seq - 1,
        pinned_paths: decode_json(pinned_paths_json.as_deref()),
    })
}

fn map_retry_update_error(error: sqlx::Error) -> RetryTurnError {
    if matches!(
        &error,
        sqlx::Error::Database(cause) if cause.code().as_deref() == Some("2067")
    ) {
        RetryTurnError::Busy
    } else {
        RetryTurnError::Sql
    }
}

pub(crate) async fn start_question(
    database: &Database,
    conversation_id: Option<&str>,
    new_conversation_id: &str,
    user_message_id: &str,
    assistant_message_id: &str,
    question: &str,
    pinned_paths: &[String],
    now_ms: i64,
) -> Result<StartedTurn, StartTurnError> {
    let mut transaction = database.pool().begin_with("BEGIN IMMEDIATE").await?;
    let conversation_id = if let Some(id) = conversation_id {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM wiki_conversations WHERE id = ?)")
                .bind(id)
                .fetch_one(&mut *transaction)
                .await?;
        if !exists {
            return Err(StartTurnError::NotFound);
        }
        mark_stale_running_in_transaction(&mut transaction, id, now_ms).await?;
        let message_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM wiki_conversation_messages WHERE conversation_id = ?",
        )
        .bind(id)
        .fetch_one(&mut *transaction)
        .await?;
        if message_count + 2 > MAX_MESSAGES_PER_CONVERSATION {
            return Err(StartTurnError::Full);
        }
        id
    } else {
        let conversation_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM wiki_conversations")
            .fetch_one(&mut *transaction)
            .await?;
        if conversation_count >= MAX_CONVERSATIONS {
            let oldest_idle: Option<String> = sqlx::query_scalar(
                "SELECT c.id FROM wiki_conversations c WHERE NOT EXISTS (SELECT 1 FROM wiki_conversation_messages m WHERE m.conversation_id = c.id AND m.status = 'running') ORDER BY c.updated_at_ms ASC, c.created_at_ms ASC, c.id ASC LIMIT 1",
            )
            .fetch_optional(&mut *transaction)
            .await?;
            let Some(oldest_idle) = oldest_idle else {
                return Err(StartTurnError::Full);
            };
            let deleted = sqlx::query("DELETE FROM wiki_conversations WHERE id = ?")
                .bind(oldest_idle)
                .execute(&mut *transaction)
                .await?;
            if deleted.rows_affected() == 0 {
                return Err(StartTurnError::Full);
            }
        }
        let title = question.chars().take(80).collect::<String>();
        sqlx::query(
            "INSERT INTO wiki_conversations (id, title, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?)",
        )
        .bind(new_conversation_id)
        .bind(title)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut *transaction)
        .await?;
        new_conversation_id
    };

    let next_seq: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM wiki_conversation_messages WHERE conversation_id = ?",
    )
    .bind(conversation_id)
    .fetch_one(&mut *transaction)
    .await?;
    let pinned_paths_json = serde_json::to_string(pinned_paths)
        .map_err(|_| StartTurnError::Sql(sqlx::Error::Protocol("serialize pinned paths".into())))?;
    sqlx::query(
        "INSERT INTO wiki_conversation_messages (id, conversation_id, seq, role, content, status, pinned_paths_json, created_at_ms, updated_at_ms) VALUES (?, ?, ?, 'user', ?, 'done', ?, ?, ?)",
    )
    .bind(user_message_id)
    .bind(conversation_id)
    .bind(next_seq)
    .bind(question)
    .bind(pinned_paths_json)
    .bind(now_ms)
    .bind(now_ms)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO wiki_conversation_messages (id, conversation_id, seq, role, content, status, created_at_ms, updated_at_ms) VALUES (?, ?, ?, 'assistant', '', 'running', ?, ?)",
    )
    .bind(assistant_message_id)
    .bind(conversation_id)
    .bind(next_seq + 1)
    .bind(now_ms)
    .bind(now_ms)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("UPDATE wiki_conversations SET updated_at_ms = ? WHERE id = ?")
        .bind(now_ms)
        .bind(conversation_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;

    Ok(StartedTurn {
        conversation_id: conversation_id.to_owned(),
        user_message_id: user_message_id.to_owned(),
        assistant_message_id: assistant_message_id.to_owned(),
        user_seq: next_seq,
    })
}

pub(crate) async fn history_before_question(
    database: &Database,
    conversation_id: &str,
    user_seq: i64,
) -> Result<Vec<HistoryMessage>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT role, content FROM wiki_conversation_messages WHERE conversation_id = ? AND seq <= ? AND content != '' AND (role = 'user' OR (role = 'assistant' AND status = 'done')) ORDER BY seq",
    )
    .bind(conversation_id)
    .bind(user_seq)
    .fetch_all(database.pool())
    .await?;

    let mut start = rows.len().saturating_sub(1);
    let mut bytes = rows.last().map(|(_, content)| content.len()).unwrap_or(0);
    while start > 0 && rows.len() - start < 12 {
        let next_bytes = rows[start - 1].1.len();
        if bytes + next_bytes > 16 * 1024 {
            break;
        }
        bytes += next_bytes;
        start -= 1;
    }
    while start < rows.len().saturating_sub(1) && rows[start].0 == "assistant" {
        start += 1;
    }
    Ok(rows[start..]
        .iter()
        .map(|(role, content)| HistoryMessage {
            role: role.clone(),
            content: content.clone(),
        })
        .collect())
}

pub(crate) async fn finish_answer(
    database: &Database,
    conversation_id: &str,
    assistant_message_id: &str,
    content: &str,
    status: &str,
    error_code: Option<&str>,
    tools: &[ToolStep],
    citations: &[String],
    graph: Option<&serde_json::Value>,
    now_ms: i64,
) -> Result<(), sqlx::Error> {
    let stored_content = truncate_at_char_boundary(content, MAX_STORED_ANSWER_BYTES);
    let content_truncated = stored_content.len() < content.len();
    let mut bounded_tools = tools.to_vec();
    let tools_json = loop {
        let serialized = serde_json::to_string(&bounded_tools).map_err(serialize_error)?;
        if serialized.len() <= MAX_STORED_TOOLS_BYTES || bounded_tools.is_empty() {
            break serialized;
        }
        bounded_tools.pop();
    };
    let citations = serde_json::to_string(citations).map_err(serialize_error)?;
    let graph = graph
        .map(serde_json::to_string)
        .transpose()
        .map_err(serialize_error)?
        .filter(|serialized| serialized.len() <= MAX_STORED_GRAPH_BYTES);
    let mut transaction = database.pool().begin().await?;
    let updated = sqlx::query(
        "UPDATE wiki_conversation_messages SET content = ?, content_truncated = ?, status = ?, error_code = ?, tools_json = ?, citations_json = ?, graph_json = ?, updated_at_ms = ? WHERE id = ? AND conversation_id = ? AND role = 'assistant' AND status = 'running'",
    )
    .bind(stored_content)
    .bind(content_truncated as i64)
    .bind(status)
    .bind(error_code)
    .bind(tools_json)
    .bind(citations)
    .bind(graph)
    .bind(now_ms)
    .bind(assistant_message_id)
    .bind(conversation_id)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(sqlx::Error::Protocol(
            "Wiki Conversation turn is no longer running".into(),
        ));
    }
    sqlx::query("UPDATE wiki_conversations SET updated_at_ms = ? WHERE id = ?")
        .bind(now_ms)
        .bind(conversation_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await
}

fn truncate_at_char_boundary(value: &str, max_bytes: usize) -> &str {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

pub(crate) async fn get_conversation(
    database: &Database,
    id: &str,
) -> Result<Option<ConversationDetail>, sqlx::Error> {
    let Some(conversation) = sqlx::query_as::<_, ConversationRow>(
        "SELECT id, title, created_at_ms, updated_at_ms FROM wiki_conversations WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(database.pool())
    .await?
    else {
        return Ok(None);
    };
    let rows = sqlx::query_as::<_, MessageRow>(
        "SELECT id, seq, role, content, content_truncated, status, error_code, pinned_paths_json, tools_json, citations_json, graph_json, created_at_ms, updated_at_ms FROM wiki_conversation_messages WHERE conversation_id = ? ORDER BY seq",
    )
    .bind(id)
    .fetch_all(database.pool())
    .await?;
    let messages = rows
        .into_iter()
        .map(|row| ConversationMessage {
            id: row.id,
            seq: row.seq,
            role: row.role,
            content: row.content,
            content_truncated: row.content_truncated != 0,
            status: row.status,
            error_code: row.error_code,
            pinned_paths: decode_json(row.pinned_paths_json.as_deref()),
            tools: decode_json(row.tools_json.as_deref()),
            citations: decode_json(row.citations_json.as_deref()),
            graph: row
                .graph_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok()),
            created_at_ms: row.created_at_ms,
            updated_at_ms: row.updated_at_ms,
        })
        .collect();
    Ok(Some(ConversationDetail {
        id: conversation.id,
        title: conversation.title,
        created_at_ms: conversation.created_at_ms,
        updated_at_ms: conversation.updated_at_ms,
        messages,
    }))
}

fn decode_json<T: for<'de> Deserialize<'de> + Default>(value: Option<&str>) -> T {
    value
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default()
}

fn serialize_error(_: serde_json::Error) -> sqlx::Error {
    sqlx::Error::Protocol("serialize Wiki Conversation data".into())
}
