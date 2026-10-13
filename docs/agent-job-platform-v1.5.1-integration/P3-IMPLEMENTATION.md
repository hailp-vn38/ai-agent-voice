# P3 source implementation and deferred live qualification

Source baseline: `64bbb59`, `integration/voice-reminders-direct-tts`, 2026-10-13.
The operator explicitly assigned real rootless/LLM/browser qualification to another host. Source completion and live phase acceptance are recorded separately below.

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

Two-axis Standards and Spec reviews completed; material findings were fixed before these final gates. The full-suite streaming regression was fixed by making the Job builtin affect streaming policy only when the Voice model supports ToolCalls; the original failed log remains retained. Source implementation is complete for this handoff. Ignored tests are not runtime proof: Linux rootless runner, real LLM A/B and browser continuous E2E remain `NOT_RUN`, and the P3 ticket stays claimed pending that live acceptance.

## Live host prerequisites and commands — NOT_RUN here

Use an isolated deployment with SQLite, managed provider runtime, an enabled enrolled Device/Agent (or authenticated Studio Virtual Device), real tool-capable Voice LLM, global LLM A/B, and at least two slots per shared LLM capacity domain. Provision Python 3.12 and uv images locally by immutable digest and configure the dedicated Linux rootless Docker socket. Run the existing P1/P2 install/execution and isolation gates first. The bundled fixture has no external dependency; the separate P2 real pinned external dependency gate still needs its own qualified package/cache.

The ignored live tracer installs and activates the checked-in network-free fixture, enables Background, connects a real Voice WS, submits a Detect user intent, waits for real LLM ToolCall/queued acknowledgement, disconnects and reconnects, observes Python plus Worker terminal success, and downloads every checked artifact through authenticated Admin HTTP. Run on a fresh qualification database; package upgrade is correctly blocked by prior nonterminal Jobs.

Provide secrets through the host's environment; never commit them or place them in the report:

- `AGENT_P3_BASE_URL`: HTTP origin of the running server.
- `AGENT_P3_WS_URL`: physical `/voice/v1/` URL, or a freshly issued Studio ticket URL. Studio also supplies `AGENT_P3_RECONNECT_WS_URL` with a second fresh single-use ticket.
- `AGENT_P3_ADMIN_TOKEN`, `AGENT_P3_DEVICE_ID`; optional `AGENT_P3_WS_TOKEN` and `AGENT_P3_WS_ORIGIN` for that admission profile.
- `AGENT_P3_ORCHESTRATOR_PROVIDER`, `AGENT_P3_WORKER_PROVIDER`: canonical refs such as `config:llm:planner` or database resource keys.
- `AGENT_P3_MCP_FEATURE`: `true` or `false`.
- `AGENT_P3_REPORT`: new report file path; contains safe identities/digests/counts, never prompts/tokens/results/audio.

```sh
scripts/agent-test.sh run -- cargo test -p voice-agent-server --test background_p3_live live_voice_to_python_worker_and_admin_artifact -- --ignored --exact
```

Repeat with physical and Studio admissions, both MCP feature values, and Device result delivery Silent/DirectTts. Provide a second fresh Studio ticket URL for reconnect and verify its origin-bound lifecycle in the browser. Confirm no Job at hello, one durable Job after intent, no reconnect replay, and ordinary Voice acknowledgement only after committed ToolResult.

Open Admin Web `/jobs` on that same deployment: observe the marker Job reaching succeeded without manual refresh, inspect plan/Step result/unknown-outcome lineage, download artifacts and compare SHA-256, then exercise failure/cancel/manual retry as a new Job. Record browser result separately; the CLI report intentionally leaves `browser_gate=NOT_RUN`.

Run failures from the normative matrix: provider absent/non-tool-capable, Background disabled, queue/capacity exhausted, stale or revoked Device/Agent, abort/disconnect before/after commit, failed Plugin/Worker schema, lost wake, mailbox saturation, restart/reap, deadline and cancellation. Record command, revision, image digest, platform and PASS/FAIL/NOT_RUN. Feed failures back for fixes; do not claim deployment acceptance from local source tests.
