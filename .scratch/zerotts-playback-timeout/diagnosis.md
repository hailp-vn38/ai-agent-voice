# ZeroTTS playback cut off at the synthesis deadline

## Confirmed cause

Voice TTS used an absolute 15-second segment deadline. A streaming provider could
continue producing PCM and still be cancelled at the next empty worker-event poll
after that deadline. SpeechOutput then failed delivery, and the actor advanced the
generation, discarding the rest of the audio.

The supplied trace submits at 10:15:03.403345Z, starts playback at
10:15:13.479618Z, and reports synthesis timeout at 10:15:18.404356Z. Cancellation
is acknowledged at 10:15:19.223759Z. These are the original trace's UTC times.

## Reproduction and verification

```bash
cargo test -p voice-agent-server --lib progressing_tts_does_not_cut_playback_at_total_timeout -- --nocapture
```

Before the fix: failed twice with
`progressing synthesis must not cut playback: Synthesis` (0.10–0.14 seconds).
After the fix: passed (0.12 seconds).

The test uses a controlled streaming provider through the production TTS worker,
SpeechOutput, PCM conversion and Opus encoding. It starts playback at the bounded
initial buffer, advances a paused clock past 15 seconds while supplying more PCM,
and asserts every expected packet is sent and the output drains normally.
It does not measure real ZeroTTS inference speed or physical-client sound quality.

## Fix

Voice TTS refreshes its deadline when the consumer receives non-empty PCM.
This accounts for pacing backpressure and allows long, progressing speech to
complete. No first PCM or a subsequent stall still times out. Empty PCM does not
refresh the deadline. Diagnostic operations retain their absolute deadline, and
cleanup grace is unchanged.

TTS deadline calculations now use Tokio Instant so regression tests can control
time without sleeping. Native worker duration measurements still use std Instant.

## Remaining acoustic evidence

The timeout explains the observed truncation. A pop at abrupt cancellation is
plausible but is not established by this log or the synthetic regression test.
Pops within completed speech need separate captured audio and timing evidence.
The capture procedure in docs/testing/05-tts.md compares provider PCM, resampled
PCM and decoded Opus; physical client output remains a separate observation.

Suggested commit message:
`fix(tts): avoid cutting progressing voice synthesis at the total segment deadline`

## Final checks

- Original SpeechOutput regression: passed after the fix.
- Library suite: 179 passed, 1 ignored before adding the final backpressure test.
- TTS timeout unit tests, including backpressure: 5 passed with paused time.
- TTS worker integration: 15 passed.
- Speech output / actor / WebSocket delivery: 14 passed.
- ZeroTTS core contracts: 4 passed, 1 real-model test ignored.
- ZeroTTS warmup PCM contract: 1 passed.
- Changed Rust files pass rustfmt; git diff --check is clean.
- No temporary debug instrumentation remains.

The repository's historical `scripts/test-module.sh tts` references a missing
`tts_stream` target. Validation used the current targets above instead.
