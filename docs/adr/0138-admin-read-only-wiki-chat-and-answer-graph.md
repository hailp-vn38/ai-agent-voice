# ADR 0138 — Admin Read-only Wiki Chat and Answer Graph

Status: Accepted, 2026-11-02. Extends [ADR 0124](0124-wiki-wikilinks-graph-projection.md).

Wiki questions run as authenticated Admin requests through a dedicated SSE endpoint and a separately selected chat provider stored in Background settings. The server grants only `wiki_query`, `wiki_get`, and `wiki_multi_get`; it records retrieved/read evidence before bounding tool text, derives every answer graph from the canonical graph projection, and promotes a citation only when its resolved page is in that evidence. This keeps chat independent of Knowledge MCP and preserves ADR 0124: Markdown Wikilinks remain the sole source of graph edges; the model cannot add inferred nodes or relationships.

The browser owns transient conversation history and cancels an active request when stopped or disconnected. A query graph can be used without a provider; an answer graph is a bounded view of the pages consulted for one answer, not a second knowledge store or a published Wiki Page. The provider is explicit, must support chat and tools, and is never chosen implicitly.

The initial operational bounds are six tool rounds, 90 seconds per answer, 15 seconds per tool, 12,000 characters forwarded per tool result, 12 history messages / 16 KiB, 60 graph nodes, and two concurrent answers. Live-provider acceptance remains separate from local source checks; unrun provider/vault scenarios must stay marked `NOT_RUN`.
