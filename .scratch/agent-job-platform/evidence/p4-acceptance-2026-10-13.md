# P4 acceptance — 2026-10-13

Result: **NOT ACCEPTED**. Ticket `05-p4-deployment-qualification.md` remains `claimed`.
Reviewed HEAD: `0817a373dc68f202d01ad570189b4224a12c9df9`.
Comparison: `git diff a954d0f...HEAD`; baseline is the last P3 source commit before P4. Scope is P4 commits `65a98e4`, `63efc6f` and their connected code; later P3 qualification evidence is contextual evidence.
Platform for this review: Linux; local Docker reports rootless/seccomp/cgroupns. This read-only daemon observation does not qualify the P4 worker topology or target deployment.

## Standards

One documented structure violation: `crates/voice-agent-server/src/services/background/plugins/egress.rs:220` embeds module tests inline. `CONTEXT.md:28` requires module tests in a separate test file. Move to `egress/tests.rs`. No additional smell finding warranted.

The concurrent retention data-loss issue below also conflicts with ADR 0095 staging/publication and orphan repair safety; it is recorded once under Spec.

## Spec

### P1 — Live retention deletes in-flight artifacts

`artifacts.rs:219-221` commits expired Job deletion, then calls global `reconcile`. The scheduler executes this maintenance concurrently with Job tasks (`scheduler.rs:34-39`). Reconciliation deletes any file without SQLite metadata (`artifacts.rs:163-180`), including artifacts belonging to running Jobs. Publication writes staging files, renames them and only subsequently commits metadata (`execution.rs:187-227`). Maintenance during this window can delete staging/final files; publication can fail, or commit metadata pointing to a missing file. The deployment lock excludes other processes, not tasks in this process.

Violates ticket acceptance: “Startup and terminal cleanup are bounded and preserve uncertainty, retry lineage and artifact safety.” The canonical design requires atomic rename followed by SQLite metadata and safe bounded orphan cleanup (section 9).

Smallest correction: restrict live filesystem cleanup to the expired Job directories; retain whole-store reconciliation for startup with no active publishers, or synchronize it with publication. Add a regression test covering publication before metadata commit.

A deterministic reproduction uses an expired Job plus a running Job whose artifact has been renamed but whose metadata is not committed. It invokes the actual `expire_terminal` function and asserts the running artifact survives. Test source: `p4-retention-repro-2026-10-13.rs`; failure log preserved alongside it as `p4-retention-repro-2026-10-13.log`. The assertion failed: `retention deleted an artifact of a running Job before metadata commit`. The test was temporarily appended to the existing recovery test module and restored byte-for-byte afterward.

### P2 — Failed orphan cleanup can be skipped on retry

Deletion commits before reconciliation. If reconciliation fails, maintenance schedules a retry after 60 seconds. When the next expired-Job batch is empty, `artifacts.rs:220` skips reconciliation, so already orphaned files remain until restart or another actual expiry. Keep pending cleanup eligible independently of finding new expired rows, without rescanning active publishers. This is established by control-flow review, not a separate fault-injection test.

### Open deployment gates

The canonical 31-row P4 register still records all scenarios as `NOT_RUN`, with commands/platform/images/evidence `pending`. In particular, rows 21/30 require deployed egress isolation/bypass/private-IP/redirect/DNS-rebinding/authority probes, and row 29 requires concurrent Voice/Reminder/Background load and measured latency bounds. Source tests and tracing events cannot satisfy those gates. Keep the qualified-host list empty until the actual topology and exact hosts pass qualification.

P3 evidence `p3-qualification-2026-10-13.json` records PASS for the isolated continuous E2E and rootless execution. All seven recorded source hashes match current files. `P4-IMPLEMENTATION.md:3` still says those prior gates remain open; reconcile that stale statement with the evidence without upgrading P4 rows automatically.

## Checks executed

All checks used `scripts/agent-test.sh`; targeted source tests preceded the workspace suite. Logs are under `/tmp/ai-agent-voice-test-logs/`.

| Check | Result | Log |
| --- | --- | --- |
| Background source | PASS: 35, ignored 1 | `agent-test.run.BqdAUW` |
| Plugin source | PASS: 5 | `agent-test.run.CWMwiD` |
| Proxy Python unit suite | PASS: 3 | `agent-test.run.sBVIFp` |
| Proxy build script syntax | PASS | `agent-test.run.OjThWP` |
| Rust format | PASS | `agent-test.run.VFnDEe` |
| Strict Clippy, all targets/features | PASS | `agent-test.run.eYInPG` |
| Admin Web tests | PASS: 229 / 55 files | `agent-test.web.qqlEs2` |
| Admin Web typecheck | PASS | `agent-test.web.6iAx8l` |
| Admin Web build | PASS | `agent-test.web.41adxA` |
| Rust full workspace | FAIL: two Vieneu timeout tests | `agent-test.full.kFwgLC` |
| Retention regression reproduction | FAIL: active artifact deleted | `agent-test.targeted.qlykj3` |
| Vieneu focused rerun | PASS: 8 | `agent-test.run.RJ9ONc` |

The workspace failure is `Elapsed(())` waiting for first audio in `cancellation_and_total_timeout_stop_a_stalled_stream` (`vieneu_http.rs:301`) and `a_consumer_that_blocks_past_the_deadline_cannot_report_success` (`vieneu_http.rs:341`). Its log is retained; this is not the previously documented nonblocking accepted-socket failure. The focused rerun passed all 8 tests. No attribution to P4 or definitive explanation for the workspace failure is established; the full workspace gate remains FAIL for this run.

## Acceptance disposition

Standards: one documented violation. Spec: two source findings, worst is active artifact deletion; deployment qualification remains incomplete. No source fixes or deployment configuration changes were made. Do not mark P4 resolved or deployment-ready. No live P4 adversarial egress, load, race/crash or complete 31-row qualification was executed during this review.
