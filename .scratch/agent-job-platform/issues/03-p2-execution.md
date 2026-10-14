# 03 — P2 Python Plugin execution tracer

Type: task
Status: claimed
Baseline: ed2f383

Implement P2 from `docs/agent-job-platform-v1.5.1-integration/IMPLEMENTATION-PHASES.md`.

## Acceptance

- Job/Step persistence test seam selects one frozen, reviewed operation and reserves one durable attempt before container start.
- Rootless Linux worker runs the pinned Python 3.12/uv.lock image with fixed stdin/stdout JSONL, bounded time/output, `network=none`, least privilege and a scoped workspace.
- Validate input/output schemas, import checked artifacts with ownership/hash/quotas, commit Step/Job by CAS, and deny late replay.
- Classify application errors separately from package/runtime errors; timeout kills and reaps the worker, leaving unknown-outcome evidence.
- Run the real fixture and forced-kill qualification on a dedicated rootless Linux host. Record image digests and command output; run targeted, strict Clippy, full workspace and Standards/Spec review.

## Comments

2026-10-13: Implementation groundwork is in progress. The macOS host's Docker Engine reports `linux ["name=seccomp,profile=builtin","name=cgroupns"]`; `name=rootless` is absent, so the required real-container gate is NOT_RUN. The fixture test is explicitly ignored by normal workspace tests and must be run on a qualified host. Keep this ticket claimed until it passes; P1's rootless install/activate gate also remains open.

2026-10-13: Source gates passed after Standards/Spec review fixes: 748 workspace tests passed (12 ignored), strict Clippy, Rust formatting, Admin Web tests/typecheck/build. Worker saturation now waits to the Job deadline, and an active worker records deadline expiry distinctly from Step timeout. The first sandboxed full run could not bind loopback; the same full command passed outside the sandbox. Real rootless worker execution and the real pinned external Python dependency remain unqualified, so this is an interim implementation commit only.
