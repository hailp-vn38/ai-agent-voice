# 03: Auto VAD, capacity và lifecycle session an toàn

**What to build:** Voice Protocol Client ở Auto mode có thể nói hands-free: server xác định boundary, giữ pre-roll, tạo final STT V1 đúng thứ tự và re-arm an toàn. Overload, cancellation và VAD runtime failure không được làm lộ transcript stale hoặc thay đổi interaction mode im lặng.

**Blocked by:** 01: Nền tảng local VAD/ASR và Manual STT; 02: Worker runtime VAD/ASR có acknowledgement.

**Status:** resolved

- [x] Auto cycle SpeechStart/SpeechEnd giữ pre-roll, mở ASR stream pinned đúng lúc, chuyển sang Processing tại endpoint và chỉ re-arm sau ResetDone; Manual không chiếm VAD worker.
- [x] Active Turn và ASR stream capacity giới hạn recognition; denied/ASR overload kết thúc không STT, còn VAD queue pressure chỉ drop frame kèm observability privacy-safe và giữ sample timeline.
- [x] Replacement/Abort invalidate generation; stale final không sinh STT. VAD inference/reset/close failure hoặc cleanup timeout đóng duy nhất affected Voice Session bằng WebSocket 1011, cancel dependent ASR và không fallback sang Manual.

## Comments

- Implemented Auto VAD cycle, global Active Turn limiter and fatal VAD session handling. Verified with `cargo check --workspace` and `cargo test --workspace`; the real-model smoke remains ignored because local model artifacts are not present.
