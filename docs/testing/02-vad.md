# Testing 02 — VAD module

## Unit cases

- silence 2 giây -> zero utterance.
- noise ngắn dưới minimum speech -> zero utterance.
- 1 giây speech + 200 ms pause + speech -> một utterance.
- speech + end silence threshold -> emit đúng một `SpeechEnded`.
- pre-roll được prepend khi speech bắt đầu.
- max utterance limit được enforce.

## Manual mode

- start -> collect.
- stop -> flush.
- stop với empty buffer -> không gọi ASR.

## Barge-in

- phase Speaking + one noisy frame -> chưa cancel.
- phase Speaking + confirmed SpeechStarted -> không cancel current generation trong V1; chỉ `abort` hoặc `listen:start` mới hủy output.

## Command

```bash
./scripts/test-module.sh vad
```
