# Testing 05 — TTS module

## Cases

- segment text -> audio stream.
- provider PCM rate khác target -> resample đúng.
- PCM -> Opus frames đúng frame duration.
- decode Opus round-trip không lỗi.
- pacer spacing đúng với paused Tokio clock.
- prebuffer frames được gửi trước pacing loop.
- queue full tạo backpressure.
- cancel generation -> stale packets không send.
- cuối stream -> `tts:stop` sau frame cuối.
- hai segment hoàn tất lệch thứ tự -> `Drained` chỉ xuất hiện sau `FinishInput` và packet cuối.
- abort khi packet cũ đã vào outbound queue -> writer drop packet stale trước session-control `tts:stop`.

## Command

```bash
./scripts/test-module.sh tts
```
