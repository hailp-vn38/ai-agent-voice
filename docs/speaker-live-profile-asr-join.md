# Live Speaker profile -> LLM after ASR/Speaker join

## Purpose

For each WebSocket utterance, run ASR and CAM++ speaker extraction concurrently and join their terminal outcomes before starting the LLM. Only the speaker verified on this exact turn contributes a name and optional description. Unknown, unavailable and insufficient audio remain non-authorizing states; External MCP and Device tools keep their independent policies.

## Flow

1. At WS admission, load the enabled Agent speaker candidates in the built-in embedding space. Read each candidate's `speaker_id`, `name`, `description`, and embedding into the session-scoped Observe plan.
2. Accumulate at most six seconds of canonical 16 kHz PCM. On manual or VAD terminal boundary, start speaker scoring concurrently with ASR finish.
3. ASR Final may complete before or after Speaker Observe. The existing per-turn identification join waits for both outcomes. It is bounded by the new join timeout to avoid indefinite LLM latency.
4. A verified Speaker result contributes one sanitized, bounded system message with only that person's name and description, after the active Template system prompt and before the Dialogue History messages. No other person's profile is included.
5. A nonmatch, extraction failure, insufficient audio, skipped scoring job, or join timeout sends plain ASR transcript to LLM without speaker metadata. Late/stale results never attach to another turn; text Detect remains unaffected.
6. Speaker profile never enters the transcript archive, dialogue history, client wire frames or authorization decisions.

## Configuration

```toml
[speaker_recognition.observe]
min_clip_ms = 1000
min_speech_ms = 800
max_window_ms = 6000
join_timeout_ms = 10000
```

These are initial development defaults and must be calibrated against recorded voice turns. A full second of input is required by the resident extractor. The Observe input buffer is capped at six seconds; the live min-clip/min-speech thresholds are independent of strict Enrollment quality requirements.

## Diagnostic explanation

For 38,400 canonical samples at 16,000 Hz the total received audio is 2.4 seconds. Before this change, live Observe borrowed Enrollment's default five-second minimum, so the extractor could be skipped with `state=insufficient_audio` and `inference_ms=0`. With the new live thresholds, such a clip clears the duration gate; it must still pass the actual energy-based speech gate and similarity threshold.

## Comparison with Xiaozhi voiceprint-api

The reference implementation `xinnan-tech/voiceprint-api` performs `len(audio_bytes) < 1000` as a file-size check in both register and identify paths. It has no configurable minimum clip/speech time for `/voiceprint/identify`. Its utility `validate_audio_file()` has a 0.5–30 second bound but is not used by that endpoint.

Sources:

- https://github.com/xinnan-tech/voiceprint-api/blob/main/app/services/voiceprint_service.py
- https://github.com/xinnan-tech/voiceprint-api/blob/main/app/api/v1/voiceprint.py
- https://github.com/xinnan-tech/voiceprint-api/blob/main/app/utils/audio_utils.py

## Verification checklist

- Config parses explicit values and fails on invalid Observe bounds.
- 2.4s valid-speech PCM reaches CAM++ inference instead of being rejected solely by enrollment duration.
- ASR-first and Speaker-first order produce the same LLM context.
- Unknown, unavailable, empty ASR, skipped Observe and timed-out Speaker never inject stale profile data.
- Only verified speaker description appears, not the full candidate catalog.
- Name and description are escaped and bounded; tool authorization remains unaffected.
- Existing speaker status frames remain `type` + `state`, without profile/score/identity.
- Check Rust formatting and targeted tests; validate with a real ESP32 WS audio sample before production rollout.
