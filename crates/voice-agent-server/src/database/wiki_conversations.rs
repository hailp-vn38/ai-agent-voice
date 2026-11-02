//! Persistent Wiki Chat conversations.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use super::Database;

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

#[derive(Debug)]
pub(crate) enum StartTurnError {
    NotFound,
    Sql(sqlx::Error),
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
    let mut transaction = database.pool().begin().await?;
    let conversation_id = if let Some(id) = conversation_id {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM wiki_conversations WHERE id = ?)")
                .bind(id)
                .fetch_one(&mut *transaction)
                .await?;
        if !exists {
            return Err(StartTurnError::NotFound);
        }
        id
    } else {
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
    let tools = serde_json::to_string(tools).map_err(serialize_error)?;
    let citations = serde_json::to_string(citations).map_err(serialize_error)?;
    let graph = graph
        .map(serde_json::to_string)
        .transpose()
        .map_err(serialize_error)?;
    let mut transaction = database.pool().begin().await?;
    sqlx::query(
        "UPDATE wiki_conversation_messages SET content = ?, status = ?, error_code = ?, tools_json = ?, citations_json = ?, graph_json = ?, updated_at_ms = ? WHERE id = ? AND conversation_id = ? AND role = 'assistant'",
    )
    .bind(content)
    .bind(status)
    .bind(error_code)
    .bind(tools)
    .bind(citations)
    .bind(graph)
    .bind(now_ms)
    .bind(assistant_message_id)
    .bind(conversation_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("UPDATE wiki_conversations SET updated_at_ms = ? WHERE id = ?")
        .bind(now_ms)
        .bind(conversation_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await
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
