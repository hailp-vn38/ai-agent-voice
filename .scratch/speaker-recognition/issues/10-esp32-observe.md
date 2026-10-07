# 10: Observe giọng ESP32 qua Voice WS

**What to build:** Agent Observe chấm giọng từ PCM ESP32/Opus thật theo active Template và báo diagnostic mà không thay accept behavior hoặc cấp quyền theo giọng.

**Blocked by:** 07: Validate holdout và finalize Voiceprint, 09: Cấu hình Agent policy và Template grants.

**Status:** ready-for-agent

- [ ] Resolve selected exact runtime/lease, capture contiguous window bounded, min/window quality theo pinned preprocessing; Manual/Auto/Realtime endpoint không thay.
- [ ] Observe best-effort, bounded operations/queues, runtime unavailable không chặn core path; Off không speaker inference/PCM overhead.
- [ ] Result identity session/turn/generation/operation/provider/space/catalog đúng; stale/abort/disconnect xử lý cleanup-only và không release native permit sớm.
- [ ] Speaker status opt-in riêng với pipeline status, bounded/no speaker identity/score trên WS; diagnostic timing gồm queue/quality/inference và gate wait, ASR/barrier tách.
- [ ] Reference Client/production WS deterministic gate và real ESP32 smoke evidence PASS/FAIL/NOT_RUN; chưa claim accuracy hay Required qualification.
