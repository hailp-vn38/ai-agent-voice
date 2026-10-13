# P4 source work and deployment qualification handoff

The P4 ticket is `.scratch/agent-job-platform/issues/05-p4-deployment-qualification.md`. This source slice is an interim commit, not P4 acceptance. P1/P2 rootless gates and the P3 continuous Voice E2E gate also remain open on the target host.

## Source changes

- A package-attributable worker protocol/runtime failure persists `plugin_installations.status=error` before Step terminalization. The update matches plugin version, package and image, so an old worker cannot poison a new installation. Admin Test does not clear error; explicit Activate remains the reset path.
- On a configured Linux runner, startup reaps owned workers before clearing their UUID-scoped workspaces in yielding batches of 1,000. Invocation cleans its workspace on prepare failure and after output import. Unexpected workspace names fail closed.
- Terminal Jobs expire after 30 days in batches of 100; retry parents remain while referenced. SQLite deletion precedes orphan file reconciliation so a crash between them can be retried. A full batch triggers another pass after one second until backlog drains; idle maintenance repeats hourly.
- Structured tracing emits `background_job_committed` (`db_commit_ms`), `background_job_claimed` (`queue_wait_ms`), `background_provider_acquired` (`provider_wait_ms`), `background_provider_permit` (`capacity_wait_ms`), `background_container_started` (`container_start_ms`, CPU/memory/PID caps), and `background_container_cancel_reaped` (`reap_ms`). These are per-event observations, not load acceptance by themselves.
- Admin Plugin detail displays package/image digests and effective approved hosts. Approve/Uninstall use the shared confirmation dialog. Background displays scheduler readiness separately from the enabled switch.
- The Vieneu HTTP reconnect test now switches an accepted socket back to blocking mode before its timed reads. On this macOS host, accepted sockets inherited the listener's nonblocking mode and caused a full-suite-only `WouldBlock` failure.

Networked Plugin Activate/Run still returns `egress_policy_unavailable`. A host-enforced isolated-network/proxy/firewall implementation and its adversarial tests are required before enabling a crawler. Do not treat the declared `network_hosts` or a proxy environment variable as enforcement.

## Source gates

Run every check through `scripts/agent-test.sh`:

```sh
scripts/agent-test.sh targeted expired_terminal_job_releases_artifacts_but_keeps_recent_history
scripts/agent-test.sh targeted package_failure_persists_error_and_cannot_poison_new_image
scripts/agent-test.sh run -- cargo test --quiet -p voice-agent-server --lib services::background::tests
scripts/agent-test.sh run -- cargo test --quiet -p voice-agent-server --lib services::background::plugins::tests
scripts/agent-test.sh check
scripts/agent-test.sh run -- cargo fmt --all --check
scripts/agent-test.sh run -- cargo clippy --all-targets --all-features -- -D warnings
scripts/agent-test.sh full
scripts/agent-test.sh web test
scripts/agent-test.sh web typecheck
scripts/agent-test.sh web build
```

### Source evidence — 2026-10-13

| Gate | Result | Saved log basename |
| --- | --- | --- |
| Terminal retention targeted | PASS | `agent-test.targeted.oZ6j9a` |
| Package fail-stop targeted | PASS | `agent-test.targeted.P1AHrv` |
| Background source tests | PASS: 30 passed, 1 ignored | `agent-test.run.qvhCsj` |
| Plugin source tests | PASS: 4 passed | `agent-test.run.oBRUU2` |
| Vieneu HTTP regression after socket fix | PASS: 8 passed | `agent-test.run.9Mm1lw` |
| Rust package check | PASS | `agent-test.check.EDNkg4` |
| Rust format check | PASS | `agent-test.run.rClEfS` |
| Strict Clippy, all targets/features | PASS | `agent-test.run.LG1RAB` |
| Full Rust workspace | PASS: 764 passed, 0 failed, 13 ignored | `agent-test.full.N4DqWM` |
| Admin Web tests | PASS: 229 passed | `agent-test.web.EudYs4` |
| Admin Web typecheck/build | PASS | `agent-test.web.iFgwlq`, `agent-test.web.WaTAPb` |

After review, a 101-Job retention test and a 1,001-workspace cleanup test passed (`agent-test.targeted.DnQixI`, `agent-test.run.EkvTgs`). The final Background source gate passed 31 tests, 1 ignored (`agent-test.run.ltLxSu`); final format, strict Clippy and full workspace passed (`agent-test.run.ibbigA`, `agent-test.run.LGFXOM`, `agent-test.full.wk68j9`: 765 passed, 0 failed, 13 ignored). The first post-review full run (`agent-test.full.wcjyYa`) exposed test concurrency over a shared macOS temporary plugin directory; Linux-only startup cleanup and an isolated cleanup test fixed it. The failure log remains retained.

The two earlier full-suite failure logs (`agent-test.full.4O3P2K` and `agent-test.full.ADuYWA`) are retained. Both exposed the inherited-nonblocking Vieneu test socket; the corrected full run passed. Ignored tests are not rootless or real LLM proof.

## Target Linux host: NOT_RUN here

Record revision, Linux distribution/kernel, rootless Docker daemon/socket, immutable Python/uv/proxy image digests, server config limits, command, timestamp and bounded result for every row in the canonical 31-row matrix. Keep secrets and prompt/result payloads out of the report. Run P1 install/activation, P2 real fixture/forced kill, and P3 continuous Voice tracer first. Then exercise dual deployment-lock conflict, restart at cancelling/start_requested, Device and Plugin revoke versus actual start, malformed plugin response, explicit cancel/reaper, 30-day cleanup and concurrent Voice/Reminder/Background load. Export the structured latency events above and compare observed maxima with deployment limits. Re-run Reminder 0104, controls/ACK/Direct TTS and Web Reminders regression gates with the scheduler active.

The following register covers all normative rows in the [canonical matrix](docs/agent-job-plugin-platform-codebase-design.md). `NOT_RUN` means the complete acceptance scenario has not run on the deployment host, even if a narrower source test passed. For each row, the host operator replaces `pending` with the exact `scripts/agent-test.sh` command or manual procedure, Linux platform, immutable image digests and evidence path. The shared revision, rootless socket and server limits belong in the report header.

| Row | Acceptance scenario | Result | Host command/procedure | Platform/images/evidence |
| --- | --- | --- | --- | --- |
| 1 | Device/Agent incarnation revoke | NOT_RUN | pending | pending |
| 2 | Mutation versus actual container start | NOT_RUN | pending | pending |
| 3 | Crash after cancelling | NOT_RUN | pending | pending |
| 4 | Queued pins and running crash recovery | NOT_RUN | pending | pending |
| 5 | Frozen choices and grant revocation | NOT_RUN | pending | pending |
| 6 | Idempotency and queue capacity races | NOT_RUN | pending | pending |
| 7 | ZIP validation attacks | NOT_RUN | pending | pending |
| 8 | uv.lock, ABI and build isolation | NOT_RUN | pending | pending |
| 9 | Offline sandbox and network activation denial | NOT_RUN | pending | pending |
| 10 | Real Python and JSONL validation | NOT_RUN | pending | pending |
| 11 | Package health classification | NOT_RUN | pending | pending |
| 12 | Workspace and artifact safety | NOT_RUN | pending | pending |
| 13 | Budget and Voice reservation | NOT_RUN | pending | pending |
| 14 | DAG and replan validation | NOT_RUN | pending | pending |
| 15 | Required source and partial policy | NOT_RUN | pending | pending |
| 16 | Physical/Studio Voice Job ToolCall | NOT_RUN | pending | pending |
| 17 | Voice failure and stale completion | NOT_RUN | pending | pending |
| 18 | All Agent authorization | NOT_RUN | pending | pending |
| 19 | Reminder/Device/Direct TTS regressions under load | NOT_RUN | pending | pending |
| 20 | No completion push; Admin-only actions | NOT_RUN | pending | pending |
| 21 | Real rootless and egress preflight | NOT_RUN | pending | pending |
| 22 | Shared JobEngine handle to Voice | NOT_RUN | pending | pending |
| 23 | Silent/DirectTts ToolResult behavior | NOT_RUN | pending | pending |
| 24 | Async Tool Round fences | NOT_RUN | pending | pending |
| 25 | Lost wake and expiry | NOT_RUN | pending | pending |
| 26 | Bounded isolated installer | NOT_RUN | pending | pending |
| 27 | Pinned Python fixture and artifact | NOT_RUN | pending | pending |
| 28 | Continuous Voice-to-Web tracer | NOT_RUN | pending | pending |
| 29 | Concurrent load and latency bounds | NOT_RUN | pending | pending |
| 30 | Allowlisted crawler with bypass denial | NOT_RUN | pending | pending |
| 31 | Voice model without ToolCalls | NOT_RUN | pending | pending |

For networked crawlers, first provision and verify an isolated worker network whose only external route is a controlled egress proxy/firewall. Test allowed host, denied host, direct proxy bypass, metadata/private/link-local/localhost addresses, HTTP redirects, CONNECT and DNS rebinding using actual worker containers. Until that proof and the source egress integration exist, leave networked plugins disabled; record acceptance rows 21 and 30 as NOT_RUN/FAIL, never PASS.
