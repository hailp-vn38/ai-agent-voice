# ADR 0092 — Durable Jobs and server-owned Voice Job submission

## Status
Accepted architecture, 2026-10-13. Implementation is phased; acceptance is not execution readiness.

## Decision
A Job is owned by the process-owned, durable Rust Job Engine, not by the Voice Session, Reminder, Tool Round or browser tab. **Every admitted Agent may submit Background work**; no `agents.background_enabled` field, per-Agent grant, Admin endpoint or Web toggle is introduced. The only Background switch is deployment-wide; installed Plugin Permission Grants are global, Admin-reviewed and checked independently of Agent identity.

On WS admission for an enabled managed Device and enabled assigned Agent (physical or ticket-authenticated Studio Virtual Device), advertise a **builtin Tool Definition** named `server_create_job` in the immutable LLM Tool Catalog. Do **not** issue a tool_call at connect. A later model `ToolCall` runs through the existing sequential Tool Round and submits durably to `JobEngine::submit` using a typed bounded `SessionEvent::JobSubmitCompleted` matched to the originating session/turn/generation/tool-call; return a structured `created=true,job_id,status=queued` only after commit, or `created=false,isError=true,error.code` if rejected. No extra WS event type, Device MCP grant, voice get/cancel builtin, or synchronous Python execution is introduced. LLM resumes to tell the user whether *creation*, not execution, succeeded. `features.mcp=false` does not suppress this builtin.

**Job Requester Scope** is server-derived and immutable: `admin:deployment` or Voice exact Device+Agent DB incarnations. At submit and every new worker start, live Device incarnation must be enabled and assigned to the same existing enabled Agent; global Background and Plugin policies must still permit dispatch. Device disable/delete/reassignment, Agent disable/delete/reassignment and global disable coordinate with the worker-start publication gate. A confirmed worker start before revocation may have effects and must be stopped best effort; never claim that cancellation undoes the external work. Admin handles historical Job list/read/cancel/retry/artifacts; Voice V1 only creates.

## Trade-off / consequences
One builtin avoids duplicating the MCP transport and inventing Agent ACL or per-Agent settings. A WS connection alone creates zero Jobs. Creation receipt is a durable database fact; no progress push or final spoken notification is promised. If a tool result is lost after commit, server-generated session/turn/call idempotency prevents a duplicate submission, without rolling back a persistent Job. The actor must not attach a late completion to a subsequent turn. Existing Reminder and ordinary External MCP flows remain unchanged.

## Integration clarification v1.5.1
One shared JobEngine submit handle is supplied to admitted SessionActor through AppState/SessionRuntimes. Creation only occurs from the LLM ToolCall; a typed completion returns to the original Tool Round and cannot enter a later turn. The immediate ToolResult is a durable receipt, not Job completion; Administration reads Job state from SQLite-backed HTTP.

## P0 implementation boundary
P0 implements durable ownership, requester scope and submission. Voice builtin/catalog/async completion remain P3.
