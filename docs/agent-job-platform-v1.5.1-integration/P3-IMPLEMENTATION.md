# P3 implementation and qualification

Source baseline: `64bbb59`, `integration/voice-reminders-direct-tts`, 2026-10-13.
The original source handoff deferred real rootless/LLM/browser qualification. Source evidence, the repeatable live recipe, and subsequent qualification results are recorded separately below.

## Implemented path

The process owns one Background scheduler. Its bounded scan and Notify claim queued Jobs by SQLite CAS within active limits; managed ProviderRuntimeManager resolves the exact source-qualified LLM A/B snapshot. Each request admits Background capacity while preserving Voice reservation, and reserves durable budget counters before dispatch. Uncertain LLM completion retains the exact permit/lease until process restart. Confirmed provider-open rejection releases the permit.

Orchestrator returns a bounded JSON DAG. Rust validates schemas, frozen Plugin choices, unique keys, cycles, dependencies and input refs before publication. Worker executes JSON-only LLM Steps; Plugin Steps keep their immutable arguments and receive bounded dependency results in an optional `inputs` JSONL field. Migration `0106_job_step_contract.sql` stores instructions and prevents contract edits. Plugin attempts persist once; completed attempts never replay. Replan can append unique Steps only at an acknowledged quiescent boundary, preserving every old Step and prohibiting repeated Plugin operations. The initial correction and later replan share the same persisted planner budget.

Rust derives terminal status from required Steps, structured Plugin provenance, immutable required sources/capabilities and partial policy. Without explicit source/capability requirements it reports `goal_only_unverified`. Partial success requires usable persisted artifacts and disclosed unsatisfied requirements. Failure skips dependency descendants. Cancel/revoke/shutdown signal bounded active work; success publication checks live Device/Agent/global authority and deadline. Restart uses existing fail-without-replay recovery and owned-container reaping.

Admitted managed physical and Studio Devices receive the shared `VoiceJobSubmit` through AppState → SocketRuntimes → SessionRuntimes → SessionActor. The builtin is reserved independently of Device MCP and forces LLM receipt continuation for Silent/DirectTts. Its model arguments cannot supply identity or idempotency. A bounded async submit produces `JobSubmitCompleted` tagged with session, turn, generation and call ID; only the current call can record it. A committed Job survives disconnect or lost receipt. A model without declared ToolCall support returns `llm_tool_calls_unavailable`.

Admin Job detail exposes persisted plan/steps/results/budget/uncertainty/lineage. Retry requires explicit safe-read review and creates a new Admin Job with current snapshots and a payload/lineage-bound replay key. Artifact downloads verify the same open inode's size and SHA-256 before streaming it in bounded chunks. Web polls without overlapping requests, retains mutation replay keys, uses ConfirmDialog for retry, and authenticates downloads through the shared API client.

## Repeatable source gates

```sh
scripts/agent-test.sh run -- cargo test --quiet -p voice-agent-server --lib services::background::tests
scripts/agent-test.sh run -- cargo test --quiet -p voice-agent-server --lib session::actor::tools::job_tests
scripts/agent-test.sh run -- cargo fmt --all --check
scripts/agent-test.sh run -- cargo clippy --all-targets --all-features -- -D warnings
scripts/agent-test.sh full
scripts/agent-test.sh web test
scripts/agent-test.sh web typecheck
scripts/agent-test.sh web build
```

### Final source evidence — 2026-10-13

All commands above ran through `scripts/agent-test.sh`. Logs are retained in the host's `ai-agent-voice-test-logs` directory:

| Gate | Result | Log basename |
| --- | --- | --- |
| Background targeted tests | PASS: 29 passed, 1 ignored | `agent-test.run.h9usT5` |
| Voice Job Tool Round tests | PASS: 4 passed | `agent-test.run.iKUELn` |
| Ordinary sentence streaming regression | PASS | `agent-test.run.NMzkwf` |
| Rust format | PASS | `agent-test.run.yEIAeA` |
| Strict Clippy, all targets/features | PASS | `agent-test.run.vYkga2` |
| Full Rust workspace | PASS: 761 passed, 0 failed, 13 ignored | `agent-test.full.QpkZ12` |
| Admin Web tests | PASS: 229 passed | `agent-test.web.mAl5em` |
| Admin Web typecheck | PASS | `agent-test.web.8UhEpm` |
| Admin Web production build | PASS | `agent-test.web.dshNdB` |
| Diff whitespace check | PASS | `agent-test.run.CEYYnC` |

Two-axis Standards and Spec reviews completed; material findings were fixed before these final gates. The full-suite streaming regression was fixed by making the Job builtin affect streaming policy only when the Voice model supports ToolCalls; the original failed log remains retained. Source implementation is complete for this handoff. At this original source handoff, ignored tests were not runtime proof: Linux rootless runner, real LLM A/B and browser continuous E2E were `NOT_RUN`, and the P3 ticket stayed claimed pending live acceptance. Later results are recorded separately below.

## Live host prerequisites and repeatable commands

Use an isolated deployment with SQLite, managed provider runtime, an enabled enrolled Device/Agent (or authenticated Studio Virtual Device), real tool-capable Voice LLM, global LLM A/B, and at least two slots per shared LLM capacity domain. Provision Python 3.12 and uv images locally by immutable digest and configure the dedicated Linux rootless Docker socket. Run the existing P1/P2 install/execution and isolation gates first. The bundled fixture has no external dependency; the separate P2 real pinned external dependency gate still needs its own qualified package/cache.

The ignored live tracer installs and activates the checked-in network-free fixture, enables Background, connects a real Voice WS, submits a Detect user intent, waits for real LLM ToolCall/queued acknowledgement, disconnects and reconnects, observes Python plus Worker terminal success, and downloads every checked artifact through authenticated Admin HTTP. Run on a fresh qualification database; package upgrade is correctly blocked by prior nonterminal Jobs.

Provide secrets through the host's environment; never commit them or place them in the report:

- `AGENT_P3_BASE_URL`: HTTP origin of the running server.
- `AGENT_P3_WS_URL`: physical `/voice/v1/` URL. For automated Studio qualification, set `AGENT_P3_STUDIO=true` and supply the ticket-free URL plus `AGENT_P3_WS_ORIGIN`; the tracer issues each ticket immediately before its handshake. This avoids the 30-second ticket expiring during fixture installation or the first turn.
- External-ticket Studio mode remains available: supply a freshly issued ticket URL, `AGENT_P3_CLIENT_ID`, and `AGENT_P3_RECONNECT_WS_URL` plus `AGENT_P3_RECONNECT_CLIENT_ID` matching the second single-use ticket.
- `AGENT_P3_ADMIN_TOKEN`, `AGENT_P3_DEVICE_ID`; optional `AGENT_P3_WS_TOKEN` and `AGENT_P3_WS_ORIGIN` for that admission profile.
- `AGENT_P3_ORCHESTRATOR_PROVIDER`, `AGENT_P3_WORKER_PROVIDER`: canonical refs such as `config:llm:planner` or database resource keys.
- `AGENT_P3_MCP_FEATURE`: `true` or `false`.
- `AGENT_P3_REPORT`: new report file path; contains safe identities/digests/counts, never prompts/tokens/results/audio.

```sh
scripts/agent-test.sh run -- cargo test -p voice-agent-server --test background_p3_live live_voice_to_python_worker_and_admin_artifact -- --ignored --exact
```

Repeat with physical and Studio admissions, both MCP feature values, and Device result delivery Silent/DirectTts. Use automated fresh ticket issuance or provide the second ticket URL and matching Client ID for reconnect; verify its origin-bound lifecycle in the browser. The tracer negotiates existing `tool_status`, fences `server_create_job` start/completion to the admitted session and turn, verifies the committed Job on completion, and rejects queued acknowledgement before that completion. It compares the isolated deployment Job count before/after hello, then verifies one durable marker Job and no reconnect replay.

Open Admin Web `/jobs` on that same deployment: observe the marker Job reaching succeeded without manual refresh, inspect plan/Step result/unknown-outcome lineage, download artifacts and compare SHA-256, then exercise failure/cancel/manual retry as a new Job. Record browser result separately; the CLI report intentionally leaves `browser_gate=NOT_RUN`.

Run failures from the normative matrix: provider absent/non-tool-capable, Background disabled, queue/capacity exhausted, stale or revoked Device/Agent, abort/disconnect before/after commit, failed Plugin/Worker schema, lost wake, mailbox saturation, restart/reap, deadline and cancellation. Record command, revision, image digest, platform and PASS/FAIL/NOT_RUN. Feed failures back for fixes; do not claim deployment acceptance from local source tests.


## P3 acceptance — isolated Linux rootless deployment, 2026-10-13

P3 first-functional-E2E is **PASS**. The qualified source is based on `1b32a24` plus the source file hashes in [the qualification report](../../.scratch/agent-job-platform/evidence/p3-qualification-2026-10-13.json). The report pins the running production process and live tracer binary hashes, provider snapshots, Python/uv images and each installed fixture image. No prompts, credentials, results or audio are included.

The three review findings are fixed: terminal publication retains its execution owner and retries only SQLite, with a five-second attempt bound and restart recovery during shutdown; an LLM EOF without `Finished` fails through the existing permit/lease quarantine; the live tracer verifies the original ToolResult before queued acknowledgement. Studio tickets and their Client IDs are issued immediately before each handshake. Regression tests first failed in `agent-test.run.vU26Ba`, then passed. Both Standards and Spec reviews passed after the counted-dispatch and blocked-publication shutdown regressions were added.

A dedicated rootless daemon with separate storage and a private socket mount namespace isolated qualification from the existing deployment. The same host passed the Python fixture install/execution/artifact and timeout/reap prerequisite. All checks used `scripts/agent-test.sh`. While Cargo ran workspace checks, live cases invoked the already-compiled, hash-pinned `background_p3_live` test executable through that script; the Cargo command in the recipe above remains the repeatable build-and-run equivalent.

| Gate | Result | Log basename |
| --- | --- | --- |
| Background targeted | PASS: 35 passed, 1 ignored | `agent-test.run.18x5nA` |
| Voice Job Tool Round | PASS: 4 passed | `agent-test.run.B2W6DK` |
| Terminal persistence/replay/cancel/shutdown regressions | PASS: 4 passed | `agent-test.run.oRvMUn` |
| Live receipt-order matcher | PASS: 2 passed; live test separately qualified | `agent-test.run.s2hxsx` |
| Rootless Python fixture, checked artifacts, timeout/reap | PASS | `agent-test.run.mzpanA` |
| Rust format / strict all-target/all-feature Clippy | PASS | `agent-test.run.p5kRod`, `agent-test.run.kOJgr2` |
| Full Rust workspace | PASS: 773 passed, 0 failed, 13 ignored | `agent-test.full.V0ghHo` |
| Admin Web tests / typecheck / production build | PASS: 229 tests | `agent-test.web.UoIf3A`, `agent-test.web.SbcbbT`, `agent-test.web.7h4ETQ` |
| Physical and Studio × MCP false/true × Silent | PASS: 4 continuous real LLM/Python cases | `agent-test.run.DM091O` |
| Physical and Studio × MCP false/true × DirectTts | PASS: 4 continuous real LLM/Python cases | `agent-test.run.eNVZPm` |
| Additional Voice tracer observed concurrently by Chromium | PASS | `agent-test.run.HGYFQK` |
| Chromium Admin Web and Studio ticket lifecycle | PASS | `agent-test.run.MLG78Z` |
| Live Admin rejection/idempotency cases | PASS: 6 cases, zero extra Jobs | `agent-test.run.vBEXMw` |

Each of the eight matrix cases verifies zero Jobs at hello, matched ToolCall/committed receipt before acknowledgement, Python plus Worker terminal success, disconnect/reconnect without replay, and authenticated checked artifact downloads. Both Background roles used the same real managed LLM Provider, permitted by ADR 0096 with two shared capacity slots and the Voice reservation intact.

Chromium followed an additional Voice Job from nonterminal state to `succeeded` through automatic polling, inspected that Job's plan/results and verified both downloaded artifact hashes. It also created an Admin Job and observed terminal polling, exercised a definite Plugin application failure with skipped descendants, confirmed manual retry as a different lineage-linked Job, and cancelled active work. Native browser WebSockets proved fresh Studio ticket admission, single-use rejection, wrong-origin rejection and fresh-ticket reconnect. Browser page errors: zero. CLI reports continue to record their own `browser_gate=NOT_RUN`; the separate browser report supplies the browser proof.

Live HTTP checks rejected disabled Background, missing required capability, forged identity and unauthenticated Admin reads without inserting Jobs. A committed receipt replayed after global disable; the same key with changed payload returned an idempotency conflict.

Original failed regression logs and setup/harness failures remain retained. The isolated launcher was corrected to preserve UID for Docker's build plugin and to use this repo's actual Cargo target directory; its obsolete isolated database was replaced before live qualification. Browser/HTTP assertions were corrected to the public normalized error contract before the successful reruns. These failed attempts are not counted as PASS.

P3's first-functional exit gate is accepted. P4's complete deployment, egress, crash/race and cross-feature load matrix remains open. The separate P2 external pinned Python dependency gate is outside this network-free fixture qualification.

Cleanup passed: the isolated server, Admin Web and dedicated daemon were stopped; zero owned worker containers remained (`agent-test.run.nTTte0`), and temporary configuration/credentials were removed. Server shutdown log: `agent-test.run.CNqDmH`.
