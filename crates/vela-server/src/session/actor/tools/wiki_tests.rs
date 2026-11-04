use super::*;
use crate::{
    config::McpResultDelivery,
    tools::{
        builtin::{WIKI_READ_TOOL_NAME, WIKI_SEARCH_TOOL_NAME, WIKI_TOOL_NAMES},
        knowledge_mcp::{KnowledgeMcp, KnowledgeMcpAccess, KnowledgeToolContext},
    },
};
use llm_wiki::WikiEngine;
use std::{path::PathBuf, sync::Arc};

struct Vault {
    root: PathBuf,
    wiki: Arc<KnowledgeMcp>,
}

impl Drop for Vault {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Vault {
    fn open() -> Self {
        let root = std::env::temp_dir().join(format!("voice-wiki-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let engine = Arc::new(WikiEngine::open(&root).unwrap());
        Self {
            root,
            wiki: Arc::new(KnowledgeMcp::new(engine, None, true, 12_000)),
        }
    }

    async fn write_page(&self, rel_path: &str, content: &str) {
        let context = KnowledgeToolContext {
            access: KnowledgeMcpAccess::ReadWrite,
            cancellation: tokio_util::sync::CancellationToken::new(),
            deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(10),
        };
        self.wiki
            .call_tool(
                &context,
                "wiki_write_file",
                serde_json::json!({"scope": "wiki", "rel_path": rel_path, "content": content}),
            )
            .await
            .unwrap();
    }
}

fn actor_with(wiki: Option<Arc<KnowledgeMcp>>) -> SessionActor {
    let mut actor = tests::session_with_external_mcp(Default::default())
        .with_session_wiki(wiki)
        .with_tool_status(true);
    actor.mcp.result_delivery = McpResultDelivery::Silent;
    actor
}

fn wiki_tool_names(actor: &SessionActor) -> Vec<String> {
    actor
        .available_llm_tools()
        .into_iter()
        .map(|tool| tool.name)
        .filter(|name| name.starts_with("server_wiki_"))
        .collect()
}

async fn call(
    actor: &mut SessionActor,
    name: &str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    actor.turn = None;
    actor.begin_active_turn().unwrap();
    actor.commit_user_text("tra wiki giúp tôi".into()).unwrap();
    actor.start_tool_batch(vec![ToolCall {
        id: "wiki-call".into(),
        name: name.into(),
        arguments,
    }]);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while actor.tool_batch.is_some() {
            actor.drain_async_call_completions();
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    actor
        .llm_messages
        .iter()
        .rev()
        .find_map(|message| match message {
            ChatMessage::ToolResult { content, .. } => Some(serde_json::from_str(content).unwrap()),
            _ => None,
        })
        .expect("the call has a result")
}

#[test]
fn a_session_without_the_wiki_is_offered_no_wiki_tool() {
    assert!(wiki_tool_names(&actor_with(None)).is_empty());
}

#[test]
fn a_session_with_the_wiki_is_offered_only_the_read_tools() {
    let vault = Vault::open();
    let actor = actor_with(Some(Arc::clone(&vault.wiki)));
    assert_eq!(wiki_tool_names(&actor), WIKI_TOOL_NAMES);
}

#[tokio::test]
async fn search_finds_a_page_and_marks_it_untrusted() {
    let vault = Vault::open();
    vault
        .write_page(
            "entities/trading.md",
            "---\ntitle: Trading\n---\n\n# Trading\n\nwombatmarket là chiến lược giao dịch.\n",
        )
        .await;
    let mut actor = actor_with(Some(Arc::clone(&vault.wiki)));

    let envelope = call(
        &mut actor,
        WIKI_SEARCH_TOOL_NAME,
        serde_json::json!({"query": "wombatmarket"}),
    )
    .await;

    assert_eq!(envelope["ok"], true, "{envelope}");
    let content = envelope["content"].as_str().unwrap();
    assert!(content.contains("entities/trading.md"), "{content}");
    assert!(content.contains("wombatmarket"), "{content}");
    let payload: serde_json::Value = serde_json::from_str(content).unwrap();
    assert_eq!(payload["untrusted_data"], true);
}

#[tokio::test]
async fn a_search_over_long_pages_stays_bounded_and_still_names_them() {
    let vault = Vault::open();
    let body = "wombatmarket là chiến lược giao dịch dài.\n".repeat(400);
    for index in 0..5 {
        vault
            .write_page(
                &format!("entities/trading-{index}.md"),
                &format!("---\ntitle: Trading {index}\n---\n\n# Trading {index}\n\n{body}"),
            )
            .await;
    }
    let mut actor = actor_with(Some(Arc::clone(&vault.wiki)));

    let envelope = call(
        &mut actor,
        WIKI_SEARCH_TOOL_NAME,
        serde_json::json!({"query": "wombatmarket"}),
    )
    .await;

    assert_eq!(envelope["ok"], true, "{envelope}");
    let content = envelope["content"].as_str().unwrap();
    assert!(content.len() <= 6_100, "{}", content.len());
    assert!(content.contains("entities/trading-"), "{content}");
}

#[tokio::test]
async fn read_returns_the_page_a_search_named() {
    let vault = Vault::open();
    vault
        .write_page(
            "entities/trading.md",
            "---\ntitle: Trading\n---\n\n# Trading\n\nwombatmarket là chiến lược giao dịch.\n",
        )
        .await;
    let mut actor = actor_with(Some(Arc::clone(&vault.wiki)));

    let envelope = call(
        &mut actor,
        WIKI_READ_TOOL_NAME,
        serde_json::json!({"file": "wiki/entities/trading.md"}),
    )
    .await;

    assert_eq!(envelope["ok"], true, "{envelope}");
    assert!(
        envelope["content"]
            .as_str()
            .unwrap()
            .contains("wombatmarket")
    );
}

#[tokio::test]
async fn a_failed_read_returns_a_code_without_the_vault_path() {
    let vault = Vault::open();
    let mut actor = actor_with(Some(Arc::clone(&vault.wiki)));

    for file in ["wiki/entities/missing.md", "../../etc/passwd"] {
        let envelope = call(
            &mut actor,
            WIKI_READ_TOOL_NAME,
            serde_json::json!({"file": file}),
        )
        .await;
        assert_eq!(envelope["ok"], false, "{envelope}");
        assert!(!envelope.to_string().contains(vault.root.to_str().unwrap()));
    }
}

#[tokio::test]
async fn the_read_only_context_cannot_reach_a_write_tool() {
    let vault = Vault::open();
    let context = KnowledgeToolContext {
        access: KnowledgeMcpAccess::ReadOnly,
        cancellation: tokio_util::sync::CancellationToken::new(),
        deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(10),
    };
    let denied = vault
        .wiki
        .call_tool(
            &context,
            "wiki_write_file",
            serde_json::json!({"scope": "wiki", "rel_path": "entities/x.md", "content": "x"}),
        )
        .await;
    assert!(denied.is_err());
}

#[tokio::test]
async fn invalid_arguments_and_a_disabled_wiki_are_refused() {
    let vault = Vault::open();
    let mut actor = actor_with(Some(Arc::clone(&vault.wiki)));
    let envelope = call(
        &mut actor,
        WIKI_SEARCH_TOOL_NAME,
        serde_json::json!({"query": "  "}),
    )
    .await;
    assert_eq!(envelope["code"], "invalid_arguments", "{envelope}");

    let mut disabled = actor_with(None);
    let envelope = call(
        &mut disabled,
        WIKI_SEARCH_TOOL_NAME,
        serde_json::json!({"query": "trading"}),
    )
    .await;
    assert_eq!(envelope["code"], "wiki_disabled", "{envelope}");
}

#[tokio::test]
async fn wiki_results_always_go_back_through_the_model() {
    let vault = Vault::open();
    let actor = actor_with(Some(Arc::clone(&vault.wiki)));
    let delivery = actor.round_result_delivery(&[ToolCall {
        id: "wiki-call".into(),
        name: WIKI_SEARCH_TOOL_NAME.into(),
        arguments: serde_json::json!({"query": "trading"}),
    }]);
    assert_eq!(delivery, McpResultDelivery::LlmThenTts);
}
