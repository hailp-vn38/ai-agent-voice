# Testing 03 — ASR module

## Streaming provider contract

Fake provider phải mô phỏng `open → push_pcm* → partial* → finish → final`, với PCM 16 kHz và session state không share giữa hai stream.

Cases:

- success -> normalized text.
- Vietnamese Unicode giữ nguyên.
- timeout -> timeout error.
- empty text -> actor không khởi tạo LLM turn.
- `finish()` drain recognizer trước final.
- partial thay đổi/coalesce chỉ là event nội bộ: không WebSocket, dialogue hoặc LLM.
- final current-generation, non-empty enqueue đúng một `type:"stt"` trước LLM.
- final empty/error/cancel/stale không gửi `stt`.

## Generation test

1. start generation 10.
2. gửi ASR request chậm.
3. actor chuyển sang generation 11.
4. ASR generation 10 trả về.
5. assert không gửi STT và không gọi LLM.

## Permit và lease test

1. `SpeechStart`/`listen:start` acquire `AsrStreamLease` rồi mở stream.
2. `SpeechEnd`/`listen:stop` không lấy được Active Turn permit.
3. assert stream bị cancel, lease được release, không có `finish()`, STT hay LLM.
4. final rỗng/lỗi/cancel release đúng các resource đang giữ.

## Command

```bash
./scripts/test-module.sh asr
```
