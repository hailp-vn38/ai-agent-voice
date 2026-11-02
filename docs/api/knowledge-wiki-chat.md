# Knowledge Graph Query and Wiki Chat API

These routes are under `/api/admin`, share the existing authenticated Admin router, and return the standard `{ "error": { "code", "request_id" } }` envelope for HTTP errors. They do not expose unauthenticated public Wiki access.

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
  "messages": [
    {"role":"user","content":"FVG liên hệ thế nào với Sharp Turn Entry?"}
  ]
}
```

Only `user` and `assistant` roles are accepted; the browser cannot supply a `system` message. At most 12 messages and 16 KiB total message content are accepted. The route requires a configured chat provider with tool support. It admits at most two concurrent turns. A configured but unavailable provider returns a bounded provider error; an unset provider returns `503 wiki_chat_not_configured`; exhausted admission returns `429 wiki_chat_busy`.

Successful requests return `text/event-stream`, use `Cache-Control: no-store`, and send keep-alives every 15 seconds. A POST turn is never automatically retried by the browser. Closing the connection or pressing Stop cancels the active turn.

| Event | Data | Meaning |
| --- | --- | --- |
| `tool` | `{call_id,name,state,query?,pages[]}` | Tool started/completed/failed. `pages` is a bounded list of relative Wiki/Source paths for completed reads. |
| `graph` | Graph snapshot plus `revision` | Full graph snapshot after evidence changes and once at turn completion. Node tiers are `cited`, `read`, `retrieved`, or `context`. |
| `delta` | `{text}` | A streamed answer fragment. |
| `done` | `{citations,stop_reason}` | Turn finished, stopped by a limit, or reached a terminal outcome. |
| `error` | `{code}` | Bounded public error code only. |

The server exposes only `wiki_query`, `wiki_get`, and `wiki_multi_get`; unknown or write tools receive `tool_not_allowed` and are not executed. The `filter` argument of `wiki_query` is not offered to the model. A rejected call returns `invalid_arguments` or `not_found` with a message to the model so it can correct the call; these codes stay between the server and the model, and the client sees only a `tool` event with state `failed`. Full tool metadata is used to record evidence before result text is bounded to 12,000 characters for the model. Answer Wikilinks become citations only when the normalized, resolved page is already in that evidence.

Initial limits are six tool rounds, 90 seconds per turn, 15 seconds per tool, 12,000 forwarded tool-result characters, 12 messages / 16 KiB, 60 graph context nodes, and two concurrent turns. These limits are defaults of the current implementation, not a provider latency guarantee.

## Error codes

`invalid_input`, `knowledge_unavailable`, `wiki_chat_not_configured`, `wiki_chat_busy`, `provider_failed`, `tool_not_allowed`, `step_limit`, and `timeout` are the relevant bounded codes. Internal provider messages, absolute vault paths, question text, and page contents are not included in error payloads.
