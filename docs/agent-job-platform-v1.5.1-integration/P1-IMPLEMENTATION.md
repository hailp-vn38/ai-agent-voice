# P1 executable Plugin installation — progress and evidence

Baseline: `74aaf19`, branch `integration/voice-reminders-direct-tts`, 2026-10-13.

## Current scope

The server has authenticated Plugin API routes, ZIP and lock validation, a one-slot bounded Docker build, immutable package/image identity, explicit approval and status transitions. A ready reviewed choice set is serialized inside the same Job submission transaction that pins the provider pair. Production Background remains disabled until P3.

The installer intentionally fails closed unless deployed on Linux against a dedicated rootless Docker Engine with pinned local Python 3.12 and uv images. Networked Plugin activation remains blocked because the enforced egress service is not implemented in P1. Offline fixture dependencies are resolved with `uv sync --locked --offline`; packages requiring a missing local wheel fail build.

## Qualification

| Gate | Evidence | Result |
|---|---|---|
| ZIP fixture manifest/lock validation; zip-slip, duplicate, oversize, stale lock | `scripts/agent-test.sh targeted services::background::plugins::archive::tests` | PASS, 4 tests |
| Admin invalid ZIP and unqualified host fail-closed | `scripts/agent-test.sh targeted services::background` | PASS |
| Ready reviewed choices frozen at submit; install pinned | `scripts/agent-test.sh targeted reviewed_plugin_choices_freeze_at_submit_and_pin_nonterminal_installation` | PASS |
| Offline Docker build of real fixture | `scripts/agent-test.sh run -- docker build --network=none --pull=false --no-cache -f /tmp/agent-p1-build.GTMYmt/.agent-plugin.Dockerfile -t agent-p1-fixture-diagnostic /tmp/agent-p1-build.GTMYmt` | PASS; image `sha256:da4d31b697321fad4584f11ae93e03022c4edc4b3247f42301e2c711c3abbf0b` |
| Rootless runtime preflight on this host | `scripts/agent-test.sh run -- docker info --format '{{json .SecurityOptions}}'` | FAIL for P1 gate: `name=rootless` absent; host is macOS with Docker Desktop |
| Real Admin fixture install, Activate, restart and submit | Dedicated rootless Linux Engine | NOT_RUN |
| Build timeout and permission/egress denial under real rootless Engine | Dedicated rootless Linux Engine | NOT_RUN |
| Strict Clippy, all targets/features | `scripts/agent-test.sh run -- cargo clippy --all-targets --all-features -- -D warnings` | PASS |
| Rust formatting | `scripts/agent-test.sh run -- cargo fmt --all --check` | PASS |
| Full workspace | `scripts/agent-test.sh full` | PASS, 742 passed / 11 ignored / 0 failed |
| Admin Web | `scripts/agent-test.sh web test`, `scripts/agent-test.sh web typecheck`, `scripts/agent-test.sh web build` | PASS, 227 tests, typecheck and build |
| Standards and Spec review | After the rootless acceptance gate | NOT_RUN |

Pinned source image digests recorded in `config.example.toml`; the local Docker Desktop engine holds those images for the diagnostic build. A successful diagnostic build is not evidence of rootless activation.

Source/test logs are retained by `scripts/agent-test.sh`, including the first red Admin route test and the ZIP test that exposed duplicate-name behavior. The final workspace log is `agent-test.full.av6uTk`; strict Clippy is `agent-test.run.my2wSd`. The user requested an interim implementation commit; it does not close P1 while the hard exit gate remains open.
