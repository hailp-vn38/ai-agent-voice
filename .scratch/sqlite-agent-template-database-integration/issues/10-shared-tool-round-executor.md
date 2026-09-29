# 10: Shared Tool-round Executor

**What to build:** LLM tool rounds execute Device MCP and External MCP calls in strict model order with one bounded semantic path for success, controlled failure, cancellation and continuation.

**Blocked by:** 08: Session-local Template switching; 09: External MCP admission snapshot.

**Status:** done

- [x] One Tool-round Executor executes calls sequentially across Device MCP, External MCP and existing session-local built-in actions (including `server.switch_template`), validates all calls in a round before call one and preserves matching call/result ordering for completed work.
- [x] Configured call-per-round, round-per-turn and total execution-budget caps fail startup when invalid and terminalize turn before new side effects when exceeded.
- [x] External call performs one outbound attempt only; timeout, unavailable, protocol/invalid response and remote 401/403 produce typed content-free ToolResult failures without retry, secret refresh, catalog mutation or session close.
- [x] Cancellation/shutdown/generation invalidation prevents subsequent calls, drops/cancels in-flight work where possible and discards late response without ToolResult, continuation, transcript or TTS.
- [x] Mixed Device/External/internal-action WebSocket tests prove ordering, tool budget, limiter wait behavior, cancellation and late-response discard. Ticket 11 owns proof that its archival writer never receives tool arguments or results.

## Answer

### What the executor is

`SessionActor` keeps one tool-round loop (`session/actor/tools.rs`) that now resolves three
origins through a single `ToolTarget` and dispatches them all through one step function. The pure
parts — the three caps, the turn-level failure classes, and the one function that turns an
External ToolCall outcome into ToolResult content — live in `tools/round.rs`, so they are testable
without a session.

Strict sequencing falls out of the structure rather than out of discipline: `dispatch_next_tool` is
re-entered only from a call's terminal outcome, so a second call cannot begin until the previous one
has produced its result. `record_tool_call` is the only writer of `completed_calls` and `results`
and pushes both, so `results[i]` belongs to `calls[i]` by construction. There is no `join_all`, no
`FuturesUnordered` and no spawned sibling.

### Bridging the asynchronous External transport

The actor is a `&mut self` state machine, and `ExternalMcpClient::call_tool` is an `async fn`. The
bridge is fire-and-correlate, exactly like the existing Device MCP path: the call leaves the process
on one task, and its completion comes back through the session's own bounded mailbox
(`EXTERNAL_CALL_CAPACITY`, drained in `drain_provider_events`). Cancellation is that task's own
`select!` arm, so an interrupted turn drops the in-flight request instead of waiting it out. The
`ExternalMcpCallLimiter` moved onto `ResolvedExternalMcp`, so a server handle cannot be reached
without also carrying the process-global bound its calls must acquire from.

### Configuration

`llm.max_tool_depth` is replaced by `[llm.tools]` — `max_calls_per_round` `1..=32` (8),
`max_rounds_per_turn` `1..=8` (4), `execution_budget_ms` `1..=120_000` (30 000) — validated in
`config/validation.rs` before a listener binds, and never re-checked by the actor. ADR 0029 records
the supersession explicitly rather than being rewritten.

`max_rounds_per_turn` keeps ADR 0029's counting rule (tool rounds, not individual calls) so the
existing loop bound does not silently change meaning. `Tool Execution Budget` starts at the turn's
first ToolCall and bounds every call to `min(per-call timeout, remaining)`, which is new for Device
MCP too.

### One behavioural fix outside the executor

`interrupt_active_turn` did not cancel the tool round, so a fail-closed or a shutdown could leave a
batch alive into the next generation — where the old generation guard finished it and started an
LLM request on a closing session. The round is now cancelled there, which is also the single seam
the abort and barge-in paths use, and `dispatch_next_tool` treats a generation mismatch as "stop the
round" rather than "continue it".

### Test seams

- `tests/tool_round_executor.rs` — 10 tests over a real WebSocket session with a real Device MCP
  peer and a real External MCP server: mixed-origin ordering and result pairing, a session-local
  action in the same order, the call cap executing zero calls, the round cap, the execution budget
  spent by the External origin *and* by the Device origin, the shared per-server bound across two
  sessions, interruption, and a typed refusal. A scripted LLM records every request, so a test
  asserts what the continuation was actually handed rather than that something was sent.
- Two actor-level tests in `session::actor::tools` own what a socket cannot order. Over a socket, a
  posted response and a sent abort race inside a 1 ms tick, so which one the executor sees first is
  not a black-box test's to choose. They are sleep-free: a gate holds the answer, the test ends the
  turn through the real seam, and the gate opens. One drives the turn's own cancellation and proves
  `external_tool_call_cancelled_total` with no ToolResult; the other leaves the round gone and proves
  `external_tool_late_response_discarded_total` with no ToolResult, no history and no round.

### Review outcome

A two-axis review (standards / spec) ran over the diff and every substantive finding was fixed:

- The execution budget was only enforced on the External origin, so a Device MCP call could consume
  it and still continue the turn. The check now lives at the one seam both origins reach —
  `begin_tool_continuation` — and both origins have a WebSocket test against it.
- A completion the bounded mailbox could not accept was dropped in silence; it is now counted as the
  late response it is.
- The name-to-origin decision was re-derived per call site; `ToolBatchState` now carries the
  delivery decision resolved once, where the round begins.
- `external_telemetry` scanned the catalog; `SessionExternalMcp::server` uses the index the catalog
  already keeps.
- New log events now carry `generation` and `turn_id`, per `docs/05-development-workflow.md`.
- ADR 0026 and `docs/04a-prompt-composition.md` still named the removed `tool_depth_exceeded`;
  ADR 0026 now records that its "one final no-tools round" claim was never what the code did, and
  0029 records the rename.

Accepted, with reasons recorded in the diff: `deny_unknown_fields` on `LlmConfig` matches every
sibling config struct and fails a stale key loudly at startup; `ToolRoundLimits::from_config` keeps
one source of truth for the defaults, matching `config::validation`, which already depends on
`tools::device_mcp`.

### Verification

`cargo fmt --check`, `cargo clippy --all-targets --all-features` at the pre-change warning count
(19, none in touched files), and `cargo test --workspace` green across 48 test binaries. The live
`config.toml` was checked to load and validate against the new section.
