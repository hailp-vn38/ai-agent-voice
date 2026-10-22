# 02 — P1 executable Plugin installation

Type: task
Status: claimed
Baseline: 74aaf19

Implement P1.

## Acceptance

- Privileged ZIP and curated package ingress, bounded validator and isolated Python 3.12/uv.lock image build.
- Immutable package/image digests, explicit permission review, `disabled/ready/error`, restart persistence, Admin Plugin API.
- Ready reviewed operation choices frozen at Job submit; nonterminal Job pins installation.
- Real fixture install and activation on a qualified rootless Linux runner, then restart and submit tracer.
- Rejections for zip-slip, oversize, stale lock, build timeout and unqualified runtime/egress.
- Targeted, workspace, strict Clippy and Admin Web gates through `scripts/agent-test.sh`; Standards and Spec review, then scoped commit.

## Comments

2026-10-13: Work is in progress. Docker Desktop on this macOS host reports Linux Engine without `name=rootless`; the server correctly fails its Linux/rootless preflight. Python 3.12 and uv image digests were pulled; a diagnostic fixture image builds offline. Real Admin install/activate/restart gate requires a dedicated rootless Linux Engine. Keep Status claimed and do not create a P1 phase commit until that gate passes.

2026-10-13: User explicitly requested a commit of the current work. Commit the validated implementation as an interim P1 commit while retaining `claimed`; this does not satisfy the rootless exit gate or close this ticket.
