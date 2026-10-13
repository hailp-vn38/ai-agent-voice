# 01 — Durable Job foundation and canonical domain docs

Type: task
Status: resolved
Baseline: c6c8861

Implement P0 from `docs/agent-job-platform-v1.5.1-integration/IMPLEMENTATION-PHASES.md`; move ADR 0092–0097 into repository `docs/adr/`, preserve ADR 0087–0091 and migration 0104.

## Acceptance

- Forward migration 0105; global revisioned settings; no Agent Background flag.
- Durable submit, normalized payload/scope idempotency, immutable LLM/limits/plugin snapshots and SQLite queue capacity.
- One shared AppState runtime, exclusive companion lockfile, startup recovery, periodic scan fallback, absolute deadlines and revoked/configuration-changed terminalization.
- Production fail-closed until P3; P0 HTTP queue tracer through cfg(test) harness.
- Device/Agent mutation publication guards and non-Voice capacity isolation.
- Targeted tests first, Reminder/Direct Speech regressions, full workspace and strict Clippy via scripts/agent-test.sh.
- Standards + Spec review, then scoped commit on current branch.

## Comments

P0 plugin choices are frozen empty, as allowed by the phase cutline. Actual Plugin package/image eligibility, container-start revocation races, physical orphan reaping and Voice Job E2E require P1–P4 and are not claimed by the foundation tracer.

## Review

Fixed point: `c6c88618948d924c265ab2363e089f8d4a9bddad`; review command: `git diff --cached c6c88618948d924c265ab2363e089f8d4a9bddad` before commit.

### Standards

PASS. No documented violations or blocking smells. The duplicated error-code strings were corrected: `JobError::code()` is the single source for HTTP/DB/Display. Typed JobStatus remains optional P3 cleanup; no current state error was found. Modules/test placement and canonical ADR layout satisfy repo rules.

### Spec

Initial review found a submission-vs-shutdown race. A targeted regression reproduced it before fixing. Shutdown now closes admission and drains submission readers before the scheduler releases deployment ownership; database submission checks live readiness after locks and before commit. Re-review PASS, no outstanding findings in P0. Planner, Python runner, real orphan reaping and Voice tool injection remain later phases.

Counts after correction: Standards 0 blocking, Spec 0 outstanding.

Workspace first run: FAIL, 26 SQLite in-memory regressions caused by SQLx URL reconstruction while detecting the backing path. Fixed by querying the opened database with `pragma_database_list`; targeted System MCP 14 tests PASS. Spec re-review of backing-path change PASS. The saved failed gate is retained; workspace is rerun after the correction.

## Answer

P0 foundation completed. Production remains disabled/runtime_unavailable until P3 by contract. Canonical ADRs are in docs/adr; Job vocabulary is in CONTEXT.md. Evidence: docs/agent-job-platform-v1.5.1-integration/P0-IMPLEMENTATION.md.

Final gates: P0 15 tests PASS; production HTTP PASS; Reminder and Direct Speech targeted regressions PASS; System MCP 14 tests PASS after in-memory fix; workspace 735 passed / 11 ignored / 0 failed; strict Clippy all targets/all features PASS; Web 227 tests PASS and typecheck PASS; formatting PASS. Standards and Spec re-review PASS, no outstanding findings.

Independent untracked plugins/ files were excluded from this scoped commit.
