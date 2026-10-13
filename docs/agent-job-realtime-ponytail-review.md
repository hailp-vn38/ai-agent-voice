# PONYTAIL Review — Agent Job Realtime

Scope: eliminate unnecessary complexity in the proposal; correctness and security remain separate checks.

1. Previous design §Domain: **delete:** duplicate eight-term taxonomy and overlap with `CONTEXT.md`. Keep only `Job Change Signal` and `Job Read View` additions to `GLOSSARY.md`.
2. Previous design §Alternatives: **shrink:** repeat of broker vs SSE comparison. Keep the trade-off once in ADR 0100.
3. Previous design §Notifier: **yagni:** generic event interface/adapter factories. One bounded `tokio::sync::broadcast` sender owned by `Database`.
4. Previous design §Transport: **native:** custom streaming transport abstraction. Use Axum `Sse` / `KeepAlive` behind existing Admin Bearer middleware.
5. Previous design §Frontend: **delete:** global singleton, cross-tab broker and ref counting. Mount one `listenJobChanges` per Job route.
6. Previous design §Reconciliation: **shrink:** multiple competing algorithms. Keep dirty flag, serialized refresh, per-Job event cursor, 200 ms coalescing and 60-second fallback.
7. Previous design §Acceptance: **shrink:** overlapping phase tables and repeated test matrix. Use a compact code map and seven observable qualification gates.
8. Previous design §Worker Progress: **delete:** speculative streaming progress protocol. Rust and Python retain the same `jsonl-v1` single-response runner.
9. Previous design §Implementation notes: **reuse:** keep existing `database/jobs` writes, SQLite snapshot read and Admin REST endpoints instead of introducing a second persistence or subscription database.

**net: approximately -250 proposal lines relative to v1** (v2 document 209 lines vs v1 459 lines). This is a document simplification metric, not an assertion about net code changes.

A separate correctness pass must still validate lost notifications, SQLite snapshot consistency, retention foreign keys and duplicate ADR 0099 numbering. The design intentionally does not promise guaranteed delivery from an in-process broadcaster.
