# Testing 05 — TTS module

## Cases

- segment text -> audio stream.
- provider PCM rate khác target -> resample đúng.
- `DownlinkPcmFrame` 1.440 samples -> Opus frame đúng 60 ms.
- encoder controls giữ explicit profile: VoIP, 32 kbps, VBR/constrained VBR bật, DTX/FEC tắt, packet-loss percent 0 và complexity 10.
- encoder luôn dùng scratch 4.000 bytes; lỗi encode, packet rỗng và output vượt `websocket.max_frame_bytes` trả `AudioCodecError`, không panic hoặc đóng WebSocket.
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
