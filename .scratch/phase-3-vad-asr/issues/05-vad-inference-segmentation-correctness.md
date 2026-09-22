# 05: VAD inference và segmentation correctness remediation

**What to build:** Hoàn thiện contract Auto VAD để Silero inference, semantic segmentation và PCM retention cùng dùng contiguous sample timeline; không mất onset khi worker có bounded lag và không tiếp tục endpoint sau VAD input gap.

**Blocked by:** 03: Auto VAD, capacity và lifecycle session baseline.

**Status:** resolved

- [x] Silero VadSession giữ recurrent state và 64-sample context; mỗi ONNX inference ghép context với 512 current samples, `reset()` clear cả hai.
- [x] `VadProbability { probability, start_sample, end_sample }` giữ nguyên qua worker; gap, duplicate, out-of-order hoặc invalid range là VAD Stream Integrity Failure và affected Auto Voice Session fail closed.
- [x] VadSegmenter dùng hysteresis, candidate onset, `min_speech_ms` và `end_silence_ms` theo sample range; `SpeechStart { start_sample }` trỏ candidate onset đã xác nhận, không phải callback/cursor xác nhận.
- [x] Actor owns hard-bounded PCM retention. Khi SpeechStart, ASR nhận `[start_sample - pre_roll_samples, current_pcm_cursor)`; capacity bao gồm pre-roll, confirmation horizon, bounded VAD in-flight lag và frame/rechunk slack.
- [x] VAD queue full không silently drop canonical input rồi segmentation tiếp; bounded backpressure phải giữ continuity, nếu không fail closed affected Auto Voice Session.
- [x] Utterance re-arm reset Silero state/context, segmenter, retention và VAD cursor bookkeeping chỉ sau ResetDone.
- [x] Unit/worker/session tests cover context, hysteresis, sample ranges, bounded-lag pre-roll, queue-full integrity và reset boundary.

## Comments

- Implemented sample-timeline VAD remediation: Silero now supplies the required 64-sample context, worker preserves and validates probability ranges, the actor segments with configured hysteresis and keeps bounded onset-relative PCM. Queue pressure and any integrity failure close only the affected Auto Voice Session. Verified with `cargo fmt --check`, `cargo check --workspace`, and `cargo test --workspace`; the real-model reference-client smoke remains ignored because local model artifacts are unavailable.
