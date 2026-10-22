# 04 — Execute native ASR with Background capacity and acknowledged cleanup

Status: resolved
Type: task
Blocked by: 02, 03
Spec: [Core ASR](../spec.md)

## Work

Add the smallest managed ASR Background execution entrypoint; current `AsrWorkerRuntime::open` is Voice-only. Reuse initialized adapters/runtime and pin each stream to one worker across bounded push/finish. Give Background execution isolated event delivery so polling cannot consume Voice events. Do not use Diagnostic admission as production execution or create synthetic Voice Sessions.

Connect Core dispatch and output validation to the pinned `ResourceLease`, deadline and cancellation. Respect Voice reservation, bounded waiting and command backpressure. Normalize whitespace-only final text to empty; reject transcript output above 16,000 Unicode characters or existing result-byte budgets without silent truncation.

Audit success/error/cancel/timeout/task-drop paths. Retain exact native capacity and versioned provider lease until cleanup/reset acknowledgement. Quarantine unreaped work after grace; account for offline native decode that cannot hard-cancel. Reuse existing resource retention patterns, without changing Voice ASR contracts or adding automatic retries.

## Acceptance

- Deterministic success/silence returns final-only `{text}` with no new output artifact.
- Background cannot borrow Voice reservation; saturation and blocked pushes observe deadline/cancellation.
- Stream events and cleanup never cross into another Job or Voice Session.
- Terminal/reset acknowledgement permits release; unresolved cleanup retains resource/capacity and quarantines.
- Task drop, provider failure and native decode timeout cannot unload an active provider or spawn unbounded replacements.

## Verification

Run targeted deterministic ASR worker/Core execution tests through `scripts/agent-test.sh`. Include Voice ASR regressions and exact-worker cleanup assertions; real native cancellation remains separate runtime evidence.

## Completion

Implemented and verified through the shared native ASR path. See [verification](../verification.md), [review](../review.md), and [flow](../../../docs/flows/15-core-asr-v3.md). Real-provider runtime evidence is reported separately as NOT_RUN.
