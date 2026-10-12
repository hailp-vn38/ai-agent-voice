# Voice Reminders and Direct TTS V1 contracts

Canonical design fixtures: [`voice_reminders_protocol_v1.json`](../crates/voice-agent-server/tests/fixtures/voice_reminders_protocol_v1.json).
These fixtures describe the opt-in extension; their presence does not enable or implement a caller.

The ordinary Voice/MCP audio path retains the admitted TTS profile, Active Turn permit,
Generation Gate, single writer and Opus 24 kHz mono 60 ms frames. LLM-origin segments
emit `llm` text. Direct speech, including tool results, uses `tts:sentence_start` text
following the first playable `tts:start`; it never labels a tool result as LLM output.
This corrects the previous tool-result text provenance while retaining MCP dispatch and audio.
A provider error before the first valid packet emits no playback start, sentence or stop.
After start, failure or abort invalidates the generation and lets the writer arbitrate one
terminal stop. Direct tool results stay out of the assistant transcript.

Both features default false and opt-in is capability negotiation, not authorization.
Legacy clients receive no Reminder frames or request/source/attempt metadata.
Every new application message carries the current `session_id`; authenticated connection
identity owns routing. Request payloads cannot choose a Device, Agent, provider or origin.
Request IDs correlate Direct TTS admission and start/stop, snapshot IDs correlate exactly
one transmitted cache window, and reminder/attempt IDs fence playback confirmation across
reconnects. Row revisions and per-Device schedule revisions have distinct meanings.

`accepted` proves actor admission only; `tts:stop` marks the stream end only.
`played` follows the actual audio sink drain; only the fenced persisted completion produces
`delivery_result:completed` and a new revision. An absent playback ACK leaves delivery
uncertain. The Device synchronizes and durably ACKs the terminal revision before sleeping.

Fixture rejection reasons are bounded machine-readable values. Server-side grants and
validation remain necessary even when the client advertises both capabilities. The new
Direct TTS and Reminder adapters must run these golden payloads through their public seams
as they are implemented; a fixture-only check is not production acceptance evidence.
