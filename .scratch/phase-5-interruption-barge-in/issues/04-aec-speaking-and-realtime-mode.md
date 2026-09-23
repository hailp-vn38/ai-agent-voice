# 04: Client AEC, Speaking và Realtime mode

**What to build:** Voice Protocol Client có thể khai báo AEC tương thích, server opt-in rõ ràng, và Realtime dùng VAD/ASR runtime xuyên Processing/Speaking; đây là nền state/policy an toàn trước khi tự động interruption.

**Blocked by:** 03 — TurnContext và listen arm semantics.

**Status:** resolved

- [x] Client Hello nhận missing/false/true AEC assertion và ignore feature không biết; cả config Barge-in mặc định false.
- [x] `Speaking` chỉ bắt đầu từ playable TTS output; Manual/no-AEC/untrusted AEC microphone trong Speaking không thay đổi turn.
- [x] Auto chỉ đủ điều kiện watch khi cycle đã arm; Realtime chạy VAD/ASR thay vì về Ready và giữ policy capture qua Processing/Speaking, nhưng chưa mở acoustic interrupt.

## Comments

- Hoàn tất 2026-09-23: thêm `ClientHello.features.aec` tương thích unknown fields, `[barge_in]` hai cờ opt-in mặc định false, predicate policy đủ điều kiện Auto/Realtime, `SessionPhase::Speaking` tại playable TTS `Started`, Auto armed-VAD ingress trong Speaking và Realtime VAD/ASR cycle qua Processing/Speaking. Acoustic interruption vẫn chưa mở theo phạm vi ticket.
- Xác minh: `cargo fmt --all -- --check`, `git diff --check`, `cargo check -p voice-agent-server`, và full `cargo test -p voice-agent-server --no-fail-fast` pass.
