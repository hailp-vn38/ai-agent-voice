# P2 Python Plugin execution — progress and evidence

Baseline: `ed2f383`, branch `integration/voice-reminders-direct-tts`, 2026-10-13.

## Implemented path

The internal Job/Step test seam persists a one-Step plan and one reserved attempt. `JobEngine::execute_plugin_step` passes a frozen choice to `PluginStore::invoke`, which checks package/image identity, live review and schemas, prepares a rootless Docker worker, records `start_requested` before `docker start`, exchanges one bounded JSONL request/response, reaps the container, checks artifact bytes, and publishes artifact metadata with terminal Step/Job CAS. Startup scans only labeled owned containers for orphan removal. Production Background admission remains disabled until P3 planner/scheduler readiness.

Worker saturation waits for a permit until the persisted Job deadline. The running worker is bounded by the earlier Step timeout or Job deadline; expiry is recorded as `deadline_exceeded` with an unknown outcome when the container may have run.

## Qualification

| Gate | Evidence | Result |
|---|---|---|
| JSONL one-line/request/schema bounds | `scripts/agent-test.sh targeted worker_response_requires_one_bounded_matching_schema_line` | PASS |
| Worker slot wait and deadline classification | `scripts/agent-test.sh targeted worker_saturation_waits_for_release_and_expires_at_job_deadline`, `scripts/agent-test.sh targeted worker_timeout_distinguishes_job_deadline_from_step_limit` | PASS |
| Attempt CAS and no late replay | `scripts/agent-test.sh targeted services::background::tests::execution` | PASS for DB test; container test ignored |
| Source typecheck | `scripts/agent-test.sh check` | PASS |
| Artifact reader refuses symlink and hardlink | `scripts/agent-test.sh targeted artifact_reader_rejects_symlink_and_hardlink` | PASS |
| Rootless Linux fixture, artifacts, timeout/forced reap | `scripts/agent-test.sh run -- cargo test -p voice-agent-server fixture_python_runs_through_job_step_and_checked_artifact -- --ignored` on dedicated rootless Linux host with `AGENT_P2_DOCKER_HOST`, `AGENT_P2_PYTHON_IMAGE`, `AGENT_P2_UV_IMAGE` | NOT_RUN on this macOS host |
| Host Docker security options | `docker info --format '{{.OSType}} {{json .SecurityOptions}}'` | Rootless prerequisite FAIL: Linux Engine has seccomp/cgroupns but no `name=rootless` |
| Rust formatting and whitespace | `scripts/agent-test.sh run -- cargo fmt --all --check`, `scripts/agent-test.sh run -- git diff --check` | PASS |
| Strict Clippy | `scripts/agent-test.sh run -- cargo clippy --all-targets --all-features -- -D warnings` | PASS |
| Full workspace | `scripts/agent-test.sh full` outside sandbox, log `agent-test.full.tI9uDm` | PASS, 748 passed / 12 ignored / 0 failed |
| Admin Web | `scripts/agent-test.sh web test`, `typecheck`, `build` | PASS, 227 tests |
| Standards and Spec review | Two-axis review against `ed2f383`; runtime, isolation and CAS findings addressed in source | PASS for reviewed source issues; rootless gate and pinned external dependency remain open |

The first full workspace run inside the sandbox failed only because loopback binds were denied (`Operation not permitted`); the same command passed outside the sandbox. The checked-in `plugins/fixture-research-1.0.0.zip` has a locked Python project with no third-party dependency; the separate P2 criterion for a real pinned external Python dependency remains open. A source build and ignored test do not establish container isolation or execution. P1 install/activation qualification is still pending on the same host requirement.
