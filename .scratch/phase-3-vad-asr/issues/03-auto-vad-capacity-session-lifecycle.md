# 03: Auto VAD, capacity và lifecycle session an toàn

**What to build:** Voice Protocol Client ở Auto mode có thể nói hands-free: server xác định boundary, giữ pre-roll, tạo final STT V1 đúng thứ tự và re-arm an toàn. Overload, cancellation và VAD runtime failure không được làm lộ transcript stale hoặc thay đổi interaction mode im lặng.

**Blocked by:** 01: Nền tảng local VAD/ASR và Manual STT; 02: Worker runtime VAD/ASR có acknowledgement.

**Status:** resolved

- [x] Auto cycle SpeechStart/SpeechEnd giữ pre-roll, mở ASR stream pinned đúng lúc, chuyển sang Processing tại endpoint và chỉ re-arm sau ResetDone; Manual không chiếm VAD worker.
- [x] Active Turn và ASR stream capacity giới hạn recognition; denied/ASR overload kết thúc không STT. VAD queue-pressure behavior trong baseline được remediation bởi ticket 05 để không tạo sample-timeline gap.
- [x] Replacement/Abort invalidate generation; stale final không sinh STT. VAD inference/reset/close failure hoặc cleanup timeout đóng duy nhất affected Voice Session bằng WebSocket 1011, cancel dependent ASR và không fallback sang Manual.

## Comments

- Implemented Auto VAD cycle, global Active Turn limiter and fatal VAD session handling. Verified with `cargo check --workspace` and `cargo test --workspace`; the real-model smoke remains ignored because local model artifacts are not present.
- Phase 3 remains open: ticket này là baseline implementation, không chứng minh Silero 64-sample context, sample-timeline segmentation, bounded retention hoặc VAD queue-full integrity. Các acceptance criteria đó thuộc remediation 05; Model Preparation/registry thuộc 06/07; ticket 04 là final E2E gate.
- 2026-09-22: Sửa regression lifecycle: `listen:start(auto)` lặp lại và `abort` trong Auto reset/re-arm `VadWorkerLease` đang pin, không `Close`/reacquire. Test deterministic với `max_workers = 1` xác nhận một lease duy nhất, duplicate khi Reset pending idempotent và Auto quay lại Listening sau abort. `cargo fmt --check`, `cargo test --workspace` và `git diff --check` pass. `cargo clippy --workspace --all-targets -- -D warnings` hiện fail ở thay đổi Ticket 07 có sẵn trong `session/speech_output.rs` (`clippy::collapsible_if`), không thuộc thay đổi VAD này.
- 2026-09-22: Regression sau TTS được tái hiện ở seam Auto thực: sau `complete_recognition()` gửi `VadCommand::Reset`, supervisor consume `ResetDone` nhưng slot vẫn là `Resetting`; microphone frame kế tiếp bị `UnknownLease`, actor fail-closed và WS đóng 1011. Test `auto_cycle_opens_asr_after_speech_start_and_rearms_only_after_reset_done` đã đỏ trước fix và xanh sau fix. `VadWorkerRuntime::observe(ResetDone)` nay chuyển slot về `Active`; state chỉ đổi sang Resetting/Cleaning sau khi command thực sự vào mailbox.
- 2026-09-22: Live validation dùng `reference/xiaozhi-esp32-server-reference/main/digital-human` qua OTA `http://127.0.0.1:8000/voice/ota/`: client nhận `tts:stop` và vẫn hiển thị `已连接`; gửi tiếp một text turn trên cùng WebSocket thành công. `cargo test --workspace` pass. Clippy còn một lint `collapsible_if` ở thay đổi Ticket 07 trong `session/speech_output.rs`, không thuộc VAD fix.
