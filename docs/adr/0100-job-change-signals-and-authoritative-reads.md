# ADR 0100 — Ephemeral Job Change Signals and Authoritative Reads

> **Trạng thái hiện hành: cần đọc theo phạm vi (2026-11-01).** Các chi tiết JobEngine, Plugin Runtime và schema Job cũ bên dưới là lịch sử. Runtime hiện tại theo [ADR 0127](0127-orchestrator-flow-task-and-wiki-runtime.md); hợp đồng JSON Jobs còn được duy trì qua lớp projection.

Status: Accepted for implementation, 2026-10-13.

Rust's durable Job Engine continues to own all execution and state transitions. After a visible Job mutation commits to SQLite, a bounded in-process Job Change Signal announces only the Job ID; Admin SSE relays it to browsers, which fetch the authoritative Job Read View and durable per-Job Event timeline through existing authenticated REST endpoints. Delivery is best effort, and reconnect/visibility/fallback reads repair missed signals. This deliberately does not turn the event timeline into an event-sourced state machine.

HTTP polling every five seconds was rejected due to redundant requests and slow observation; WebSockets and Redis/broker infrastructure were rejected as unnecessary for one-way Admin observation on a single-process deployment. The implementation needs no additional plugin protocol, persistent SSE outbox, configuration or Agent grant, and must never publish before commit or claim that notification delivery is guaranteed.
