# 10: Observe giọng ESP32 qua Voice WS

**What to build:** Agent Observe chấm giọng từ PCM ESP32/Opus thật theo active Template và báo diagnostic mà không thay accept behavior hoặc cấp quyền theo giọng.

**Blocked by:** 07: Validate holdout và finalize Voiceprint, 09: Cấu hình Agent policy và Template grants.

**Status:** resolved

- [x] Resolve selected exact runtime/lease, capture contiguous window bounded, min/window quality theo pinned preprocessing; Manual/Auto/Realtime endpoint không thay.
- [x] Observe best-effort, bounded operations/queues, runtime unavailable không chặn core path; Off không speaker inference/PCM overhead.
- [x] Result identity session/turn/generation/operation/provider/space/catalog đúng; stale/abort/disconnect xử lý cleanup-only và không release native permit sớm.
- [x] Speaker status opt-in riêng với pipeline status, bounded/no speaker identity/score trên WS; diagnostic timing gồm queue/quality/inference và gate wait, ASR/barrier tách.
- [x] Reference Client/production WS deterministic gate và real ESP32 smoke evidence PASS/FAIL/NOT_RUN; chưa claim accuracy hay Required qualification.

## Answer

Implemented on branch `speaker/10`.

- `src/session/speaker_observe.rs`: `ObservePlan` resolved once at session admission (Agent policy + active Template grants + published voiceprints), scoring against the exact selected `SpeakerRuntime` via `ResourceLease`. `SpeakerPolicyMode::{Off,Observe,Required}`; `Off` short-circuits so no inference and no PCM retention happen.
- `src/session/actor/observe.rs` + wiring in `construct.rs`/`ingress.rs`/`listening.rs`: bounded contiguous utterance PCM (`OBSERVE_MAX_SAMPLES`), one best-effort scoring task per utterance boundary (extra boundaries dropped while one runs), reset at each speech-segment start.
- Result identity carries session/turn/generation/operation/provider/space/catalog; stale/abort/disconnect results are dropped cleanup-only and never release the native permit early (reuses ticket 04's permit lifetime).
- Separate opt-in `speaker` status field, distinct from pipeline status: bounded state plus queue/quality/inference/gate-wait timing; ASR/barrier reported separately. **No** speaker identity, key or raw score on the wire.
- `min_window_samples()` is the shared source of truth so the quality gate and the extractor cannot disagree.
- Tests: `tests/speaker_observe.rs` (3) — Off emits no speaker frame and retains nothing; Observe emits bounded state without identity/score; unavailable runtime does not block the core path.
- Smoke evidence: `docs/speaker-observe-smoke.md` records **NOT_RUN** (no ESP32 attached), with the operator procedure and the deterministic coverage that stands in for it. No accuracy or `Required` qualification is claimed.

Remaining gap (documented): end-to-end device timing still needs the real ESP32 run; the smoke record stays `NOT_RUN` until then.
