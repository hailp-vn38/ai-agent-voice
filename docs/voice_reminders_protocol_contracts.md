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

## Admin Direct TTS implementation

`POST /api/admin/devices/{device_id}/speak` requires the Admin bearer token and a physical,
enabled Device. Its only body fields are `request_id` and `text`; unknown routing/provider
fields are rejected. Text is bounded to 1,024 Unicode characters/4,096 UTF-8 bytes with
control-character validation. The active Device session must negotiate `features.direct_tts`.
Direct text enters the same bounded segment queue without LLM JSON-artifact filtering or
punctuation rewriting; supplied JSON and punctuation reach the admitted TTS provider intact.
The request is ephemeral, with one admitted speech turn and a one-second request interval per
session. Offline, unsupported, busy, rate, capacity and validation errors use existing typed
Admin error envelopes. `202 {request_id,status:"accepted"}` follows actor admission, never just
mailbox enqueue. Admission expires after two seconds; the actor drops expired reply channels.

Current session registration occurs after ClientHello, profile/actor initialization and ServerHello
admission. A successful replacement cancels the old connection; unregister is fenced by session
identity, and retained old senders cannot remove the new handle. Device detail GET adds
`ws_online` and `direct_tts_supported` from this registry. Enabled permission remains separate.

Opted-in direct controls carry request ID and source (`admin_direct` or `client_direct`).
After the writer's terminal outcome, `tts` state `result` carries `outcome` (`completed`, `failed`
or `cancelled`), including failures before audio starts. `completed` here means the writer
finished the stream; it does not prove the Device speaker drained. Reminder playback completion
still requires its separate fenced `played` ACK. Legacy speech controls receive no new metadata.
