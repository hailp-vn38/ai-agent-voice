# P4 source work and deployment qualification handoff

The P4 ticket is `.scratch/agent-job-platform/issues/05-p4-deployment-qualification.md`. Source code is implemented, but P4 acceptance is still open. P1/P2 rootless gates and the P3 continuous Voice E2E gate also remain open on the target host.

## Source changes

- A package-attributable worker protocol/runtime failure persists `plugin_installations.status=error` before Step terminalization. The update matches plugin version, package and image, so an old worker cannot poison a new installation. Admin Test does not clear error; explicit Activate remains the reset path.
- On a configured Linux runner, startup reaps owned workers before clearing their UUID-scoped workspaces in yielding batches of 1,000. Invocation cleans its workspace on prepare failure and after output import. Unexpected workspace names fail closed.
- Terminal Jobs expire after 30 days in batches of 100; retry parents remain while referenced. SQLite deletion precedes orphan file reconciliation so a crash between them can be retried. A full batch triggers another pass after one second until backlog drains; idle maintenance repeats hourly.
- Structured tracing emits `background_job_committed` (`db_commit_ms`), `background_job_claimed` (`queue_wait_ms`), `background_provider_acquired` (`provider_wait_ms`), `background_provider_permit` (`capacity_wait_ms`), `background_container_started` (`container_start_ms`, CPU/memory/PID caps), and `background_container_cancel_reaped` (`reap_ms`). These are per-event observations, not load acceptance by themselves.
- Admin Plugin detail displays package/image digests and effective approved hosts. Approve/Uninstall use the shared confirmation dialog. Background displays scheduler readiness separately from the enabled switch.
- The Vieneu HTTP reconnect test now switches an accepted socket back to blocking mode before its timed reads. On this macOS host, accepted sockets inherited the listener's nonblocking mode and caused a full-suite-only `WouldBlock` failure.

- Networked Plugin attempts now create an owned internal worker bridge, an external proxy bridge, and a bounded per-attempt HTTPS CONNECT proxy sidecar. Only the sidecar joins both networks. The worker receives proxy variables; the proxy checks exact approved hosts, port 443, public DNS answers, and matching TLS SNI before forwarding. It pins the resolved numeric address for the connection, limits concurrent tunnels, bytes and lifetime, and rejects plain HTTP. The sidecar and bridges are removed after the worker, with startup owner-label reconciliation after a crash.
- Activate/Test preflight provisions and removes this same topology. Networked activation also requires every approved host in `background.plugin_egress_qualified_hosts`; the default empty list fails closed with `egress_policy_unavailable`. Operators populate exact hosts only after target-host adversarial qualification. A missing local image ID or mismatched policy/base/source labels also fails closed. The image is built offline from the deployment's pinned Python 3.12 base using `scripts/build-agent-egress-proxy.sh`.

This source implementation is not host enforcement evidence. The target Linux host must prove the internal bridge cannot reach the internet, host bridge gateway, metadata/private addresses, or other container networks except through the proxy. TLS CONNECT filtering checks SNI; shared-IP domain fronting or a server that serves another HTTP authority under the allowed SNI is a residual risk that the host qualification must investigate. Keep `plugin_egress_qualified_hosts` empty until rows 21 and 30 pass on the deployed engine for each exact host. If the intended crawler can reach an unapproved authority, keep it disabled and strengthen the proxy policy.

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
scripts/agent-test.sh run -- python3 -m unittest discover -s deploy/agent-egress-proxy -p 'test_*.py'
scripts/agent-test.sh run -- bash -n scripts/build-agent-egress-proxy.sh
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

The repo-owned proxy source unit suite passed (`agent-test.run.nvk9Ww`), shell syntax passed (`agent-test.run.1ZoR7l`), Rust package check passed (`agent-test.check.0WlaYu`), focused Plugin tests passed (5 passed, `agent-test.run.P05YdK`), Rust format passed (`agent-test.run.jlkCiH`), strict Clippy passed (`agent-test.run.dmqIa0`), and the full Rust workspace passed (`agent-test.full.P3xH5X`). These are source checks, not live Docker or egress evidence.

After spec review, the empty-by-default qualification gate passed its targeted test (`agent-test.targeted.6Txl9O`: 1 passed). Strict Clippy and full workspace passed again (`agent-test.run.Q0Q4lo`, `agent-test.full.a7GBMC`). The review's two enforcement concerns remain explicit target-host acceptance criteria, not source PASS claims.

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

For networked crawlers on the dedicated rootless Linux engine, first load the pinned `plugin_python_image`, then build the local sidecar with `scripts/agent-test.sh run -- scripts/build-agent-egress-proxy.sh 'python:3.12-slim@sha256:<digest>'`. Put the returned `sha256:<image-id>` in `background.plugin_egress_image`. Record the image ID, Python base digest, proxy source SHA, rootless socket and server revision. With the qualification list still empty, use a disposable worker attached to the same internal/proxy topology to test allowed host, denied host, direct proxy bypass, host bridge gateway, metadata/private/link-local/localhost addresses, HTTP redirects, CONNECT, DNS rebinding and shared-IP authority/domain-fronting. Stop/kill the worker and restart the server to verify labelled proxy/container/network cleanup. Only after those probes and the complete rows 21/30 pass, put the exact verified hosts in `background.plugin_egress_qualified_hosts`, then use Admin Test and Activate on the reviewed crawler package. A failed authority or bypass probe means the host stays unqualified and the proxy policy must be strengthened; record rows 21 and 30 as NOT_RUN/FAIL, never PASS.
