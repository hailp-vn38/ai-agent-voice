# ADR 0139 — Durable Wiki Conversations

Status: Accepted, 2026-11-03. Amends the browser-owned history decision in [ADR 0138](0138-admin-read-only-wiki-chat-and-answer-graph.md). Live vault/provider acceptance remains pending in [ticket 07](../../.scratch/wiki-chat-conversations/issues/07-nghiem-thu-that-va-tai-lieu.md).

Browser-only history loses answers, citations, and Answer Graphs on reload, and those are the most expensive output of Wiki Chat. We store each exchange as a Wiki Conversation in SQLite, and the server reads prompt history from there instead of accepting it from the browser. A Wiki Conversation is stored data, not a runtime: between turns nothing runs on the server, and a turn keeps ADR 0138's contract of one bounded, read-only SSE request that is cancelled when the connection closes.

A turn is written as `running` before the stream opens and finalized by the spawned task that ran it, so a cancelled turn still records its partial answer. A partial unique index allows one running turn per conversation. Startup marks leftover running turns `interrupted`; text is not written per delta, so a crash loses the part being streamed. Each answer stores its final graph snapshot, which records the evidence at answer time and is only ever redisplayed; ADR 0124 still holds, and Wikilinks remain the sole source of edges.

Storing questions and answers changes ADR 0138's privacy position, where this content only passed through the server. Logs and error payloads still exclude it, Admin can delete a conversation, and storage is capped at 200 conversations of 100 messages, removing the oldest idle conversation automatically. Config-only snapshots explicitly delete these tables; Full SQLite snapshots include them. The portable Backup Bundle does not include conversations, and import/replace leaves them untouched.

## Considered Options

- **Keep browser-only history in IndexedDB.** Cheapest, with no migration, but history stays in one browser and the server still trusts client-supplied `assistant` text. Rejected because SQLite and migrations already exist, so the server option costs little more.
- **Reuse Agent Sessions or Tasks.** Rejected again for the reason in the Wiki Chat design: each question would become a durable Job, far too heavy for question and answer, and Job tables have immutability rules that a retried message would break.
- **Keep the turn running after the tab closes and let clients reattach.** Rejected for now. It would mean background execution, a replay buffer, and spending on answers nobody is reading, and nothing yet shows the need.
- **Support both stateless and stored requests.** Rejected. The endpoint has no external clients, so a second path would only add testing and let the browser keep supplying history.

## Consequences

The term "session" is deliberately avoided, since Voice Session, Agent Session, and Muse session all name things with a running lifecycle. Conversations belong to the deployment rather than a user, following the single Admin identity of ADR 0008, so per-user ownership would need a later migration. Retrying is limited to the newest answer, so a stored conversation can't hold a rewritten middle that later turns never saw.
