# SQLx domain modules implementation

Status: implemented and reviewed; verification complete with baseline limitations

Spec: `docs/sqlx-domain-modules-refactor-guide.md`.
Baseline: `30b69e33f5be0c5393b60b15e065518e4961beff`.

## Changes and preserved contracts

The SQL inventory contains 158 production query/construction sites across 90
functions, including existing database modules. Production sites outside database
have moved into domain-owned methods; dead `patch_agent_enabled` and its duplicate
queries were removed. No production query remains outside database. The inventory
records source functions, tables, transaction scope, outward errors, side effects,
destinations and regression suites.

New owners: Agents, Providers, Templates/relationships, Devices, Speakers with
policy/captures/observations, History queries, MCP servers, shared audit and guarded
deletion. Enrollment Claim and tool review/dispatch extend existing database
modules. All callers, storage row types and relevant exports moved with their SQL.
The architecture record is `docs/sqlx-domain-modules.md`.

One pool, SQLite startup/readiness, WAL/foreign keys/migrations, encrypted credentials,
HTTP routes/bodies/statuses/ETags, revision comparisons, relationship constraints,
audit atomicity and post-commit effects are preserved. Enrollment Claim retains
BEGIN IMMEDIATE. MCP publication guards cover commit through invalidation. Dispatch
permission stays live and fail-closed. Capture promotion/replacement retains one
transaction and idempotent consumed-capture responses. No SQL was introduced into
inference, audio turns or Dialogue History; archive handoff and retention are intact.

The database interfaces return records, mutation outcomes or errors, not Axum types
or SQLx transactions/pools. Internal shared SQLx audit helpers are private to the
database subtree. No generic repositories, speculative traits, dependencies, schema
migrations or web changes were introduced.

Domain-modeling: existing CONTEXT vocabulary and ADRs 0070, 0073 and 0083 agree with
implemented behavior. No new domain term was resolved; storage-module names are
implementation details, so no glossary/ADR entry was needed.

## Verification

- Baseline cargo check: PASS.
- Baseline admin_api: PASS (31 tests); device_admission: PASS (10).
- P1/P2/History: PASS: admin_api (31), device_admission (10), device_enrollment_ws (4),
  provider_load_plan (8), provider_admission_snapshot (4), transcript_archive (15).
- Speaker/MCP: PASS: admin_api (31), external_mcp_admission (13),
  speaker_identification_builtin (3), speaker_observe (7).
- Added HTTP audit-failure rollback across Agent, Template, Provider, MCP and Speaker:
  PASS after correcting the test's incomplete OpenAI configuration.
- Added real-SQLite MCP publication contention and rollback test: PASS, including
  an independent queued reader observing cancellation immediately after acquiring
  publication protection, before its database read.
- SQL placement check and five regression tests: PASS; CI runs both.
- Final `cargo fmt --all -- --check` and `git diff --check`: PASS.
- Full server suite: FAIL/TIMEOUT (360 seconds including compilation), reaching
  tool_round_executor with four failures and three tests that did not terminate.
- `cargo clippy --all-targets --all-features -- -D warnings`: UNAVAILABLE due to an
  unchanged refutable pattern in external_mcp_admission.rs:447: enabling qualification
  adds QualificationVad, but the test destructures only SileroOnnx.
- `cargo clippy -p voice-agent-server --all-targets -- -D warnings`: PASS.
- Workspace continuation: FAIL (578 passed, 5 failed, 4 ignored, 3 filtered out);
  all five assertion failures independently reproduced on pristine baseline.
  The three non-terminating tool tests were explicitly skipped only in this continuation.
- Final `cargo check -p voice-agent-server`: PASS (also repeated after removing a
  redundant checked Database lookup in provider capabilities).

## Baseline failures and unavailable checks

The workspace run reports four failures in tool_round_executor:

- a_refused_external_call_is_typed_and_does_not_stop_the_round
- a_mixed_round_runs_its_calls_in_model_order_and_pairs_every_result
- a_session_local_action_runs_in_the_same_order_as_the_mcp_calls_around_it
- the_peer_really_is_a_device_mcp_server_and_the_external_catalog_really_resolves

All four were reproduced from a pristine archive of the recorded baseline, with
identical assertions (missing unreviewed external tools / bounded unknown_tool).
Do not weaken reviewed-tool authorization to satisfy these legacy expectations.

session_profile's
public_api_created_provider_is_used_by_new_ws_and_patch_keeps_old_session_version
also fails on pristine baseline at line 1785: prepare returns 202 while the test
expects 200 without waiting for asynchronous readiness. This is unchanged behavior.

The original full server attempt timed out after 360 seconds including compilation.
Three tool tests did not terminate during both targeted and full attempts:

- an_external_call_that_outlives_the_execution_budget_ends_the_turn
- an_interrupted_turn_drops_its_in_flight_call_and_never_continues
- two_sessions_calling_one_server_never_exceed_the_shared_per_server_bound

The continuation used `cargo test --workspace --no-fail-fast -- --skip <each name>`
to exercise every later suite while retaining all failing assertions. Their hang
behavior was not separately reproduced on baseline. Four existing ignored tests
require optional native/model qualification assets; they remain ignored.
All-features Clippy is blocked by the unchanged integration-fixture enum pattern
not covering QualificationVad. Default-feature Clippy passes. No UI or physical
hardware qualification was performed for this storage organization refactor.

## Final integration matrix

| Suite | Result | Counts |
| --- | --- | --- |
| `tests/admin_api.rs` | PASS | ok. 32 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.77s |
| `tests/audio.rs` | PASS | ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `tests/builtin_tools.rs` | PASS | ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `tests/config_audio.rs` | PASS | ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s |
| `tests/cors.rs` | PASS | ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.50s |
| `tests/database_bootstrap.rs` | PASS | ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.77s |
| `tests/device_admission.rs` | PASS | ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.87s |
| `tests/device_enrollment_ws.rs` | PASS | ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.51s |
| `tests/device_mcp.rs` | PASS | ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `tests/docker_config.rs` | PASS | ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s |
| `tests/exit_intent_integration.rs` | PASS | ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.59s |
| `tests/external_mcp_admission.rs` | PASS | ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.37s |
| `tests/lifecycle.rs` | PASS | ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.45s |
| `tests/llm_runtime.rs` | PASS | ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `tests/manual_stt.rs` | PASS | ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s |
| `tests/model_download.rs` | PASS | ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.10s |
| `tests/protocol_e2e.rs` | PASS | ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.76s |
| `tests/protocol_listen.rs` | PASS | ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `tests/provider_admission_snapshot.rs` | PASS | ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.25s |
| `tests/provider_assets.rs` | PASS | ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `tests/provider_load_plan.rs` | PASS | ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.83s |
| `tests/provider_materializer.rs` | PASS | ok. 18 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 10.17s |
| `tests/provider_registry.rs` | PASS | ok. 7 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `tests/provider_runtime_manager.rs` | PASS | ok. 23 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.01s |
| `tests/provider_template_snapshot.rs` | PASS | ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `tests/provider_worker_unload.rs` | PASS | ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `tests/session_profile.rs` | FAIL | FAILED. 25 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.09s |
| `tests/session_state.rs` | PASS | ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s |
| `tests/speaker_identification_builtin.rs` | PASS | ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.46s |
| `tests/speaker_observe.rs` | PASS | ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s |
| `tests/speechoutput_tracer.rs` | PASS | ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.73s |
| `tests/tool_round_executor.rs` | FAIL | FAILED. 5 passed; 4 failed; 0 ignored; 0 measured; 3 filtered out; finished in 2.48s |
| `tests/transcript_archive.rs` | PASS | ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.03s |
| `tests/tts_worker_runtime.rs` | PASS | ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s |
| `tests/vad_segmentation_correctness.rs` | PASS | ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| `tests/vision_api.rs` | PASS | ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.38s |
| `tests/vision_config.rs` | PASS | ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s |
| `tests/vision_provider.rs` | PASS | ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.15s |
| `tests/worker_runtime.rs` | PASS | ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s |
| `tests/worker_runtime_review.rs` | PASS | ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s |
| `tests/ws_protocol.rs` | PASS | ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.76s |
| `tests/zerotts_assets.rs` | PASS | ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s |
| `tests/zerotts_synthesis_core.rs` | PASS | ok. 4 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.16s |
| `tests/zerotts_warmup_contract.rs` | PASS | ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s |

## Review

### Standards

Final follow-up: 0 unresolved findings.

### Spec

Final follow-up: 0 remaining findings.


Two independent code-review axes inspected the complete diff and new modules.
Standards found shared production/test source exemptions, missing query*_with
constructors and overly wide SQLx audit visibility; all were corrected with tests.
The new module test was moved into its own file under the module per CONTEXT.
Spec requested publication-lock concurrency coverage; added a contention/rollback
test, then strengthened the reader to run independently. Neither review found SQL,
credential, capture or endpoint behavior drift.

## Delivery

Implementation stays on the current branch because the invoked implement skill
requires committing there. The user's existing `apps/admin-web/tsconfig.app.json`
edit, untracked guide and subsequent deletions of four scripts/test-*.sh files
are excluded from the implementation commit. No push or
merge is authorized or attempted.

## Changed file inventory

Before: 121 production query/construction sites outside database/. After: 0.
Tests retain intentional direct SQLx access.

- `.github/workflows/ci.yml`
- `.scratch/sqlx-domain-modules/implementation.md`
- `.scratch/sqlx-domain-modules/inventory.md`
- `crates/voice-agent-server/src/app/admin/agents.rs`
- `crates/voice-agent-server/src/app/admin/deletion.rs`
- `crates/voice-agent-server/src/app/admin/devices.rs`
- `crates/voice-agent-server/src/app/admin/enrollments.rs`
- `crates/voice-agent-server/src/app/admin/history.rs`
- `crates/voice-agent-server/src/app/admin/mcp_servers.rs`
- `crates/voice-agent-server/src/app/admin/mod.rs`
- `crates/voice-agent-server/src/app/admin/provider_adapters.rs`
- `crates/voice-agent-server/src/app/admin/providers.rs`
- `crates/voice-agent-server/src/app/admin/speaker_policy.rs`
- `crates/voice-agent-server/src/app/admin/speaker_quick.rs`
- `crates/voice-agent-server/src/app/admin/speakers.rs`
- `crates/voice-agent-server/src/app/admin/system.rs`
- `crates/voice-agent-server/src/app/admin/templates.rs`
- `crates/voice-agent-server/src/app/admin/templates/relationships.rs`
- `crates/voice-agent-server/src/app/admin/tool_allowlist.rs`
- `crates/voice-agent-server/src/app/mod.rs`
- `crates/voice-agent-server/src/app/websocket.rs`
- `crates/voice-agent-server/src/database/agents.rs`
- `crates/voice-agent-server/src/database/audit.rs`
- `crates/voice-agent-server/src/database/deletion.rs`
- `crates/voice-agent-server/src/database/device_enrollments.rs`
- `crates/voice-agent-server/src/database/devices.rs`
- `crates/voice-agent-server/src/database/history.rs`
- `crates/voice-agent-server/src/database/history/queries.rs`
- `crates/voice-agent-server/src/database/mcp_servers.rs`
- `crates/voice-agent-server/src/database/mcp_servers/tests.rs`
- `crates/voice-agent-server/src/database/mod.rs`
- `crates/voice-agent-server/src/database/providers.rs`
- `crates/voice-agent-server/src/database/speakers/captures.rs`
- `crates/voice-agent-server/src/database/speakers/mod.rs`
- `crates/voice-agent-server/src/database/speakers/observations.rs`
- `crates/voice-agent-server/src/database/speakers/policy.rs`
- `crates/voice-agent-server/src/database/templates/mod.rs`
- `crates/voice-agent-server/src/database/templates/relationships.rs`
- `crates/voice-agent-server/src/database/tool_allowlist.rs`
- `crates/voice-agent-server/src/database/tool_security.rs`
- `crates/voice-agent-server/src/database/writes.rs`
- `crates/voice-agent-server/src/services/provider_diagnostic.rs`
- `crates/voice-agent-server/src/services/provider_prewarm.rs`
- `crates/voice-agent-server/src/session/speaker_observe.rs`
- `crates/voice-agent-server/tests/admin_api.rs`
- `docs/sqlx-domain-modules.md`
- `scripts/check-sqlx-placement.py`
- `scripts/tests/test_sqlx_placement.py`
