# 06: Acoustic Barge-in trong Voice Session

**What to build:** Với Auto đã arm hoặc Realtime, client AEC đã được server trust có thể nói chen lúc assistant Speaking: utterance B giữ triggering PCM/pre-roll, hủy response A đúng một lần, rồi hoàn tất ASR → LLM → TTS B trên cùng Voice Session.

**Blocked by:** 05 — VAD Capture Cycle và prefix-safe retention.

**Status:** ready-for-agent

- [x] SpeechStart snapshot retention trước reset, rồi invalidate/cancel A, urgent-stop nếu A đã Started, mở ASR B và route PCM tiếp theo vào B; không cho Barge-in trong Manual, no-AEC hoặc Processing.
- [x] Không JSON/audio A được writer admit sau gate boundary; old Drained/LLM/TTS/VAD event không tạo output, history commit hoặc capacity release lần hai.
- [ ] Deterministic public application E2E chứng minh một Socket sống qua A và B, exact-one stop, no stale output, B ASR final và B delivery; disconnect giữa transition giữ worker cleanup/quarantine đúng.

## Comments

- Tiến độ 2026-09-23: `SpeechStart` hợp lệ trong `Speaking` snapshot retention trước khi dùng primitive interruption hiện hữu để invalidate GenerationGate, cancel A và gửi urgent `tts:stop`; sau đó actor cấp generation mới và mở ASR B trong cùng VAD Capture Cycle. Ingress chỉ watch PCM khi policy AEC trusted hợp lệ và `Speaking`, nên Manual/no-AEC/untrusted/Processing không tự interrupt. `cancel_asr` detach semantic stream và giữ cleanup obligation theo identity để acknowledgement cũ vẫn được consume, không thể tái dùng/release capacity hai lần.
- Xác minh hiện có: `cargo fmt --all -- --check`, `git diff --check`, `cargo check -p voice-agent-server`, test E2E public `trusted_aec_auto_barge_in_stops_a_once_and_delivers_b_on_the_same_socket`, các gate worker cleanup/quarantine, và `cargo test --workspace -q` pass. Cần thêm E2E Realtime và disconnect ngay trong A→B trước khi resolve ticket; Real-model Reference Client và firmware/HIL vẫn là completion gate riêng của ticket 07.
