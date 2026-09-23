# Testing 05 — TTS module

## Cases

- segment text -> audio stream.
- provider PCM rate khác target -> resample đúng.
- `DownlinkPcmFrame` 1.440 samples -> Opus frame đúng 60 ms.
- encoder controls giữ explicit profile: VoIP, 32 kbps, VBR/constrained VBR bật, DTX/FEC tắt, packet-loss percent 0 và complexity 10.
- encoder luôn dùng scratch 4.000 bytes; lỗi encode, packet rỗng và output vượt `websocket.max_frame_bytes` trả `AudioCodecError`, không panic hoặc đóng WebSocket.
- decode Opus round-trip không lỗi.
- pacer spacing đúng với paused Tokio clock.
- chưa có `tts:start` khi worker stream còn đang tạo câu ngắn, kể cả đã có 5 Opus packet; worker hoàn tất thì `Started`, rồi tối đa 5 packet đầu được gửi liền trước pacing loop. Với segment dài, đủ 32 packet làm `Started` ngay cả khi worker còn active để hàng đợi không chặn inference.
- queue full tạo backpressure.
- cancel generation -> stale packets không send.
- cuối stream -> `tts:stop` sau frame cuối.
- hai segment hoàn tất lệch thứ tự -> `Drained` chỉ xuất hiện sau `FinishInput` và packet cuối.
- abort khi packet cũ đã vào outbound queue -> writer drop packet stale trước session-control `tts:stop`.
- real ZeroTTS model + fake streaming LLM: audio packet đầu tiên xuất hiện trước `Finished` của LLM.
- real ZeroTTS PCM đi qua resample 24 kHz, Opus và pacing; Reference Client decode được packet.
- unexpected tool call sau audio đã deliver -> cancel audio còn lại, không MCP/retry và không stale packet sau generation invalidation.
- no worker slot hoặc command queue full -> fail-fast generation, không drop/skip segment.
- timeout tính từ worker accept segment, không reset bởi PCM chunk; cleanup acknowledgement release slot, cleanup timeout quarantine worker.
- segment ordinal liên tục: chỉ một active synthesis, N+1 đợi `SegmentFinished(N)` nhưng có thể overlap pacer N; pending full fail generation.
- failure trước `Started` không gửi start/stop; failure sau `Started` gửi đúng một stop và không audio packet cùng generation nào tới wire sau stop.
- `pending_segments` full -> `speech_output_backpressure`, cancel LLM và không accept thêm delta; no dropped/overwritten segment.
- config reject `limits.tts_concurrency != workers.tts.max_workers`.
- deterministic non-user ZeroTTS warmup validates all pinned artifacts and returns finite, non-empty mono PCM at 48 kHz; mutable warmup state is not reused by a session.

## Command

```bash
./scripts/test-module.sh tts
```

Với model/ONNX Runtime đã cài, chạy `scripts/test-phase4-reference-gate.sh` với
`VOICE_ONNX_RUNTIME_LIB`. Để đối chiếu tiếng nhiễu ở packet đầu, đặt thêm
`ZEROTTS_DIAGNOSTIC_TEXT` bằng đúng câu gặp lỗi và
`ZEROTTS_FIRST_PACKET_CAPTURE_DIR` tới thư mục đầu ra. Gate sẽ ghi
`A-provider-48k.wav` (PCM thô), `B-resampled-24k.wav` (sau fade/FIR) và
`C-opus-decoded-24k.wav` (Opus giải mã lại), mỗi file chứa 60 ms đầu.
Nghe cả ba file trước khi quy nguồn tạp âm; gate tự động chỉ xác nhận tính hợp lệ
của tín hiệu, không xác nhận chất lượng nghe trên thiết bị vật lý.
Capture mặc định dùng đường `file`/`decode_full`; đặt
`ZEROTTS_DIAGNOSTIC_MODE=stream` để lấy bản so sánh từ `decode_step`.
Đặt `ZEROTTS_DIAGNOSTIC_FULL_WAV` nếu cần lưu toàn bộ câu 48 kHz để nghe.
