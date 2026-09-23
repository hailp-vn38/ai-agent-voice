# Testing 02 — VAD module

## Unit cases

- silence 2 giây -> zero utterance.
- noise ngắn dưới minimum speech -> zero utterance.
- 1 giây speech + 200 ms pause + speech -> một utterance.
- speech + end silence threshold -> emit đúng một `SpeechEnded`.
- probability `0.82, 0.74, 0.43, 0.38, 0.71` không endpoint khi vẫn ở/qua hysteresis policy.
- `SpeechStarted { start_sample }` dùng candidate onset thay vì cursor xác nhận; duration tính bằng contiguous `[start_sample, end_sample)`, không callback count hay paused worker clock.
- gap, duplicate hoặc out-of-order `VadProbability` range fail closed Auto session; VAD queue full không được silently drop input rồi tiếp tục segmentation.
- Silero inference ghép 64-sample previous context với 512 current samples, update context đúng và reset clear recurrent state lẫn context.
- actor retention feed PCM từ `start_sample - pre_roll_samples` tới cursor hiện thời; bounded VAD lag không làm mất onset và retention không vượt capacity đã tính.
- max utterance limit được enforce.
- packet Opus rỗng, vượt 4.000 bytes, decode lỗi hoặc khác 960 samples bị drop; capture và Voice Session vẫn tiếp tục với PCM hợp lệ trước/sau packet lỗi. Packet 4.000 bytes malformed có thể tới decoder; packet 4.001 bytes phải trả `PacketTooLarge` trước decoder.
- binary 65.536 bytes được transport nhận rồi codec drop `PacketTooLarge`, session vẫn sống; binary 65.537 bytes bị transport close 1009 trước codec.
- actor xử lý frame theo ingress order nhưng không expose PCM buffer; test xác nhận qua `CaptureOutcome`, không đọc storage nội bộ.
- `listen:start`/`listen:stop`/`abort` reset Manual Capture nhưng không recreate/reset UplinkOpusDecoder; frame hợp lệ của capture kế tiếp vẫn decode được.
- repeated `listen:start` và `abort` khi capture không rỗng discard toàn bộ PCM cũ, không tạo utterance/outcome xuống ASR và không phát wire payload mới.
- utterance re-arm trong một Auto Listening cycle clear Silero state/context, segmenter, retention ring và VAD cursor bookkeeping trước input utterance kế tiếp.
- với `workers.vad.max_workers = 1`, `tts:stop` rồi `listen:start(auto)` phải reuse cùng VAD capacity trên cùng WebSocket và turn sau vẫn hoàn tất; không được close `1013`.
- `listen:start(auto)` lặp lại khi Reset pending là idempotent; `abort` trong Auto reset/re-arm lease hiện hữu và không Close/reacquire nó.

## Manual mode

- start -> collect.
- stop -> flush.
- stop với empty buffer -> không gọi ASR.
- runtime Phase 2 finalize non-empty utterance chỉ trace duration/frame count rồi giải phóng nó và về Ready; không tạo Processing, Active Turn permit hay consumer/callback tạm.

## Barge-in

- phase Speaking + one noisy frame -> chưa cancel.
- phase Speaking + confirmed SpeechStarted -> không cancel current generation trong V1; chỉ `abort` hoặc `listen:start` mới hủy output.

## Command

```bash
./scripts/test-module.sh vad
```
