# 12: Readiness, shutdown và operational degradation

**What to build:** Operator receives truthful liveness/readiness and controlled shutdown behavior across database-backed admission, RuntimeCatalog, optional External MCP and HistoryWriter degradation.

**Blocked by:** 07: Provider Load Plan và Effective Session Profile; 09: External MCP admission snapshot; 10: Shared Tool-round Executor; 11: Optional persistent transcript.

**Status:** done

- [x] `/health` remains liveness while `/ready` reports ability to accept new connections from startup/schema state, required RuntimeCatalog, admission resolver and DB dependency state only.
- [x] Readiness never runs full admission, Device resolution, External MCP discovery or provider/secret reload; optional MCP failure remains ready, while DB failure makes readiness fail when DB admission is required.
- [x] Shutdown first flips one application-owned admission/work gate that refuses new listener, DB admission and Tool-round work (including External limiter acquisition) before draining existing sessions up to configured grace deadline, then controlled-closes remaining sessions. It must not depend on an individual SessionActor observing shutdown before the gate is closed.
- [x] Application owns a session-drain registry (or equivalent registered completion handles) before a connection begins work, so shutdown can observe drain completion, issue controlled close to remaining sessions at the shared deadline, and never use task abort as the normal close mechanism.
- [x] HistoryWriter flush is best-effort only inside the same deadline; maintenance and database contention create bounded telemetry without retries or shutdown extension.
- [x] Lifecycle tests prove liveness/readiness distinction, optional MCP fail-soft readiness, DB-admission degradation, no expensive readiness probes, admission/tool rejection during shutdown and bounded drain completion.

## Comments

Implementation notes:

- `src/lifecycle.rs` owns `Readiness`, `AdmissionGate`, `SessionDrainRegistry`, `RuntimeLifecycle` and
  the ordered shutdown. `RuntimeLifecycle::shutdown()` is the single sequence: close gate → drain to
  one shared deadline → controlled close what is left → best-effort archival flush on the *same*
  deadline → hard stop. The flush runs after the closes are issued, not beside the drain, because a
  session still draining is still enqueueing records and a writer measured against a moving target
  settles by accident.
- `AppState` holds `Arc<RuntimeLifecycle>`. The gate is consulted by the WebSocket boundary, by
  `resolve_session_profile`, by `SessionActor::start_tool_batch`/`dispatch_next_tool` and by
  `ExternalMcpCallLimiter` — one decision, four asking points, none of them able to reopen it.
- `AppState::readiness()` reports `Ready | ShuttingDown | StartupIncomplete |
  RequiredRuntimeUnavailable | DatabaseUnreachable`. The database question is one `SELECT 1` through
  `Database::is_reachable()`, and only Database-backed Device Admission counts as needing it: the
  Admin API and the Persistent Transcript also use the database, but their failure is per-request
  and coarse for the caller rather than a reason to pull the Voice listener out of rotation.
- `CONTROLLED_CLOSE_SETTLE` in `main.rs` is not extra drain grace. It only bounds how long the
  protocol closes issued at the deadline may take to land; without it, aborting the listener task
  would be the *normal* close mechanism, which the ticket forbids.
- A pre-existing defect was found and fixed while proving drain completion: the socket writer's
  `select!` kept the watch branch pending forever, so `handle_socket` deadlocked at `writer.await`
  after any client disconnect. Because the drain registry counts a session until its socket task
  finishes, that deadlock would have made every drain hit the deadline. The fix releases the watch
  sender explicitly once the actor has finished, and puts the watch branch last so a queued terminal
  `Close(1001)` is still delivered before the writer stops.

