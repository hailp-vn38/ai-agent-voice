# Testing 05 — TTS module

## Cases

- segment text -> audio stream.
- provider PCM rate khác target -> resample đúng.
- `DownlinkPcmFrame` 1.440 samples -> Opus frame đúng 60 ms.
- encoder controls giữ explicit profile: VoIP, 32 kbps, VBR/constrained VBR bật, DTX/FEC tắt, packet-loss percent 0 và complexity 10.
- encoder luôn dùng scratch 4.000 bytes; lỗi encode, packet rỗng và output vượt `websocket.max_frame_bytes` trả `AudioCodecError`, không panic hoặc đóng WebSocket.
- decode Opus round-trip không lỗi.
- pacer spacing đúng với paused Tokio clock.
- với stream thông thường, chưa có `tts:start` khi worker còn đang tạo câu ngắn, kể cả đã có 5 Opus packet; worker hoàn tất thì `Started`, rồi tối đa 5 packet đầu được gửi liền trước pacing loop. Với segment dài, đủ 32 packet làm `Started` ngay cả khi worker còn active để hàng đợi không chặn inference.
- ZeroTTS stream đợi từng segment hoàn tất trước khi phát; vẫn nhận PCM qua ngưỡng 32 packet và refresh stall timeout. Regression `slow_native_segments_finish_before_playback_without_losing_progress` giữ đủ 52 packet trong 18 giây synthesis với stall budget 15 giây. `staged_audio_is_bounded_and_cancellation_discards_it` kiểm tra buffer bound và cancel.
- queue full tạo backpressure.
- cancel generation -> stale packets không send.
- cuối stream -> `tts:stop` sau frame cuối.
- hai segment hoàn tất lệch thứ tự -> `Drained` chỉ xuất hiện sau `FinishInput` và packet cuối.
- abort khi packet cũ đã vào outbound queue -> writer drop packet stale trước session-control `tts:stop`.
- real ZeroTTS model + fake streaming LLM: audio packet đầu tiên xuất hiện trước `Finished` của LLM.
- real ZeroTTS PCM đi qua resample 24 kHz, Opus và pacing; Reference Client decode được packet.
- mọi ZeroTTS segment ramp chunk 4 → 8 → 16 codec frames; segment sau vẫn giữ silence và không bỏ mẫu PCM. Real-model regression: `every_segment_ramps_small_chunks_without_dropping_codec_audio` (ignored nếu chưa cài model/runtime).
- mỗi utterance có codec history riêng; PCM của câu thứ hai/thứ ba khớp independent full decode, cả stream/file, và khoảng nghỉ là PCM zero không điều kiện hóa codec. `each_segment_matches_independent_codec_decode` dùng voice `huuduc` và câu Rust của regression.
- Floating Chat phát lại trace 135 packet từ model thật: buffer nối liên tục, đúng thứ tự, không mất PCM; starvation tích lại buffer và fade trước khi phát tiếp. Buffer vẫn giới hạn hai giây.
- terminal `completed` flush decoder và phát hết tail ngắn trước khi mở lượt mới; Auto không gửi microphone trong final playback drain. Interrupt, mute hoặc disconnect không để audio/callback cũ sang lượt mới.
- unexpected tool call sau audio đã deliver -> cancel audio còn lại, không MCP/retry và không stale packet sau generation invalidation.
- no worker slot hoặc command queue full -> fail-fast generation, không drop/skip segment.
- voice timeout tính từ worker accept segment rồi reset khi consumer nhận PCM không rỗng; segment vẫn phát đủ khi tổng synthesis vượt timeout nhưng có tiến triển. Không có PCM đầu tiên hoặc stall sau PCM vẫn timeout; PCM rỗng không kéo dài timer. Provider diagnostic giữ deadline tuyệt đối. Cleanup acknowledgement release slot, cleanup timeout quarantine worker.
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
