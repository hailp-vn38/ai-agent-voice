# 05: VAD Capture Cycle và prefix-safe retention

**What to build:** Auto và Realtime có VAD Capture Cycle độc lập lifetime worker, bảo toàn PCM onset/pre-roll qua bounded retention để một SpeechStart hợp lệ luôn có input trọn vẹn cho ASR turn mới.

**Blocked by:** 04 — Client AEC, Speaking và Realtime mode.

**Status:** resolved

- [x] Semantic VAD event mang cycle identity; event cycle cũ không thể mở ASR, còn cleanup acknowledgement stale vẫn được consume/quarantine đúng contract.
- [x] Retention dùng overwrite-oldest và capacity tính từ pre-roll, confirmation horizon, bounded VAD command lag, frame hiện hành và rechunk slack; thiếu range bắt buộc fail-closed.
- [x] Deterministic VAD/ASR tests chứng minh fixture onset giữ được `[start_sample - pre_roll, cursor)`, worker lag không làm mất prefix và Auto/Realtime re-arm đúng policy.

## Comments

- Hoàn tất 2026-09-23: `VadCaptureCycleId` tách semantic VAD event khỏi pinned worker lease. Actor chỉ mở ASR từ cycle active; reset acknowledgement chỉ promote cycle đang pending, trong khi worker vẫn observe stale cleanup acknowledgement/quarantine theo identity lease. Retention giữ overwrite-oldest và fail-closed nếu snapshot range thiếu.
- Xác minh: `cargo fmt --all -- --check`, `git diff --check`, `cargo check -p voice-agent-server`, các deterministic VAD/ASR tests, và `cargo test --workspace` pass.
