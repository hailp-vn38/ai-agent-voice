# Knowledge Graph Query and Wiki Chat API

These routes are under `/api/admin`, share the existing authenticated Admin router, and return the standard `{ "error": { "code", "request_id" } }` envelope for HTTP errors. They do not expose unauthenticated public Wiki access. Conversation responses use `Cache-Control: no-store`.

## Query Graph

```http
GET /api/admin/knowledge/graph?scope=query&q=trading
```

`q` is required and must not be blank. The server searches Wiki and Source collections with a fixed result count and score threshold, then builds a graph from matching pages plus one hop of real Wikilinks. System pages are excluded. Each node includes its evidence tier (`retrieved` or `context`) and search `score`; the response also has the existing `nodes`, `edges`, `unresolved`, and `truncated` fields. Global and Local scopes remain available as before.

## Select the Chat Provider

Use the existing revision-guarded Background settings endpoint:

```http
GET /api/admin/background/settings
PUT /api/admin/background/settings
```

The response adds `wiki_chat_llm_provider` and `wiki_chat_llm_providers`. Supply `wiki_chat_llm_provider` in the update body to select an explicit catalog provider. The selected provider must have the Chat capability and support tools. The value is persisted in the existing Background settings file; no database migration is required. An unset field leaves Wiki Chat unconfigured.

## Ask the Wiki

```http
POST /api/admin/knowledge/ask
Accept: text/event-stream
Content-Type: application/json
```

Request body:

```json
{
  "conversation_id": "optional-existing-id",
  "question": "FVG liên hệ thế nào với Sharp Turn Entry?",
  "pinned_paths": ["wiki/concepts/trading/fair-value-gap.md"]
}
```

The browser supplies a question, not role-tagged history. The server builds prompt history from completed saved messages, capped at 12 messages and 16 KiB total. A new question is limited to 16 KiB. The route requires a configured chat provider with tool support. It admits at most two concurrent turns. A configured but unavailable provider returns a bounded provider error; an unset provider returns `503 wiki_chat_not_configured`; exhausted admission returns `429 wiki_chat_busy`.

The request body is `{ "conversation_id"?: string, "question": string, "pinned_paths"?: string[] }`. `pinned_paths` accepts up to four vault-relative `wiki/` or `sources/` paths. Duplicates are ignored; paths outside those folders or more than four paths return `400 invalid_input`. For a new conversation, the server creates an ID and a title from the first 80 characters of the question. The initial SSE event is `turn` with `conversation_id`, `user_message_id`, and `assistant_message_id`.

To retry the latest failed or interrupted answer, send `{ "conversation_id": "…", "retry_message_id": "…" }` instead of `question`. The retry reuses the preceding user message and its pinned paths. Other retries return `400 invalid_input`. A running conversation returns `409 wiki_conversation_busy`; a full conversation returns `409 wiki_conversation_full`. Before the model's first call, the server reads each pinned file through `wiki_get`, so the read appears as a `tool` event with a `pinned-<n>` call id, its result is bounded and untrusted like any other, and the file counts as evidence the answer may cite. These reads do not use up the model's six tool rounds. A missing file reaches the model as `not_found` and the turn continues.

Successful requests return `text/event-stream`, use `Cache-Control: no-store`, and send keep-alives every 15 seconds. A POST turn is never automatically retried by the browser. Closing the connection or pressing Stop cancels the active turn.

| Event | Data | Meaning |
| --- | --- | --- |
| `tool` | `{call_id,name,state,query?,pages[]}` | Tool started/completed/failed. `pages` is a bounded list of relative Wiki/Source paths for completed reads. |
| `graph` | Graph snapshot plus `revision` | Full graph snapshot after evidence changes and once at turn completion. Node tiers are `cited`, `read`, `retrieved`, or `context`. |
| `delta` | `{text}` | A streamed answer fragment. |
| `done` | `{citations,stop_reason,saved,content_truncated,graph_saved}` | Turn finished, stopped by a limit, or reached a terminal outcome. The save flags report which bounded answer artifacts were persisted. |
| `error` | `{code}` | Bounded public error code only. |

The server exposes only `wiki_query`, `wiki_get`, and `wiki_multi_get`; unknown or write tools receive `tool_not_allowed` and are not executed. The `filter` argument of `wiki_query` is not offered to the model. A rejected call returns `invalid_arguments` or `not_found` with a message to the model so it can correct the call; these codes stay between the server and the model, and the client sees only a `tool` event with state `failed`. Full tool metadata is used to record evidence before result text is bounded to 12,000 characters for the model. Answer Wikilinks become citations only when the normalized, resolved page is already in that evidence.

Each turn's system prompt holds the fixed read-only rules followed by the body of the `wiki-chat` Job Skill. It is read from `skills/wiki-chat/SKILL.md` for every turn, so an edit in Admin applies to the next question. A missing or invalid file falls back to the Built-in default. The skill guides searching, citing, and drawing, but it cannot add tools: the allowlist above is enforced in code.

An answer can include a fenced `mermaid` block. The browser leaves its source as text while the answer streams and draws it once the turn ends, with Mermaid's `strict` security level. Answers are untrusted because tool results can steer the model, so a diagram with images, CSS `url()`, or Mermaid config directives is not drawn: `strict` alone would still let them contact another host. In that case, or if the syntax is invalid or the pinned CDN cannot be reached, the source stays visible. The server does not parse or validate diagrams.

Initial limits are six tool rounds, 90 seconds per turn, 15 seconds per tool, 12,000 forwarded tool-result characters, 12 messages / 16 KiB, 60 graph context nodes, and two concurrent turns. These limits are defaults of the current implementation, not a provider latency guarantee.

## Error codes

`invalid_input`, `knowledge_unavailable`, `wiki_chat_not_configured`, `wiki_chat_busy`, `wiki_conversation_not_found`, `wiki_conversation_busy`, `wiki_conversation_full`, `database_unavailable`, `provider_failed`, `tool_not_allowed`, `step_limit`, and `timeout` are the relevant bounded codes. Internal provider messages, absolute vault paths, question text, and page contents are not included in error payloads.

## Saved conversations

Conversations are stored in the server's SQLite database. The server builds prompt history from saved messages; the browser does not submit conversation history. Each conversation holds at most 100 messages, and the database retains at most 200 conversations, evicting the oldest idle conversation when needed. Running conversations are not evicted. Answers are capped at 64 KiB, tool metadata at 64 KiB, and a serialized answer graph above 256 KiB is omitted. A conversation reaching its message limit rejects another question with `409 wiki_conversation_full`.

```http
GET    /api/admin/knowledge/conversations
GET    /api/admin/knowledge/conversations/{id}
PATCH  /api/admin/knowledge/conversations/{id}
DELETE /api/admin/knowledge/conversations/{id}
```

The list returns `{ "items": [...] }`, newest first. Each item has `id`, `title`, `updated_at_ms`, `message_count`, and `running`. Detail returns the conversation and messages, including each message's `status`, `error_code`, `pinned_paths`, `tools`, `citations`, and saved `graph` when available. Rename accepts `{ "title": "…" }` with a trimmed length of 1–120 characters. Delete cascades to messages, refuses a running conversation, and records an Admin audit event without the conversation title or question text. Missing IDs return `404 wiki_conversation_not_found`; a database outage returns `503 database_unavailable`.

On server startup, leftover `running` messages become `interrupted`. A `running` message older than 120 seconds no longer blocks the conversation: list and detail report it as not running and `interrupted`, and the next turn or delete rewrites it. Closing the tab or stopping a request records a partial answer as `stopped`; a server crash can lose the in-flight text because deltas are not written as they stream. Retry is limited to the latest failed or interrupted answer.

Config-only Database Snapshots exclude the conversation tables. Full SQLite Snapshots include them. The portable Database Backup Bundle does not include conversations, and import/replace therefore does not modify them.
