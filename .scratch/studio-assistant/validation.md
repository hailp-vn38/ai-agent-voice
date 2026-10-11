# Studio Assistant validation

Spec: [implementation guide](../../docs/studio-assistant-virtual-device-ws-implementation-guide.md).
Implemented on the current branch; no new runtime or WS route.

## Automated checks

- `cargo check -p voice-agent-server`: passed.
- `cargo test -p voice-agent-server --test web_assistant`: 3 passed. Covers authenticated/unconfigured settings; managed CRUD and kind isolation; physical bearer/ticket isolation; ticket replay; ACK; text and microphone → STT/LLM/TTS; terminal after writer stop; two-connection cap; disable closes live connections.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --workspace`: passed (645 passed, 6 ignored).
- `npm test` in `apps/admin-web`: 45 files, 172 tests passed, including protocol/client sequencing, stale socket generation and audio frame/capture tests.
- `npm run typecheck` and `npm run build`: passed.

## Browser evidence

Chromium headless, localhost secure context, mocked Admin API and shared Voice WS;
actual WebCodecs/AudioWorklet with the browser's fake microphone device.
Desktop light/dark and mobile panels inspected. Text produced one accepted user
bubble and one assistant bubble from multiple LLM segments. Collapse/route change
preserved the conversation and used exactly one ticket and socket. Escape collapsed
the focused panel. Native microphone emitted 9 canonical 16 kHz/60 ms raw Opus
packets; native raw Opus playback decoded without errors. No page errors.

- [Desktop light](validation/desktop-light.png)
- [Desktop dark](validation/desktop-dark.png)
- [Mobile](validation/mobile.png)
- [Browser checks](validation/browser.cjs)

Run the browser checks from repo root with a local Vite server on port 5198 and
Playwright available (`PLAYWRIGHT_MODULE` may point to an existing installation).
Browser shared libraries and Chromium must be installed; no repo dependency added.

## Standards

No documented-standard violations or actionable baseline smells found. Review caught
an Auto/Realtime ASR failure regression; the new branch is now restricted to
negotiated browser Manual mode, preserving the existing physical capture lifecycle.

## Spec

Review caught two browser lifecycle races, now corrected with regression tests:
Abort validates/discards in-flight audio and suppresses late playback controls until
terminal cancellation; delayed microphone/worklet failures cannot cancel a newer
turn. No significant scope creep found.

Review findings: Standards 1 corrected; Spec 2 corrected. No known open review findings.
The workspace suite also exposed an enrollment SELECT missing the new kind field;
that query is corrected. Disable preserves an unchanged binding after its Agent
is disabled, while changed/re-enabled bindings still require a valid graph.

## Remaining real-environment checks

Real ESP32 hardware, live ASR/LLM/TTS/MCP, physical microphone/speaker, other browser
engines, actual mobile keyboard and production proxy/CDN telemetry were not tested.
The supplied app/Nginx ticket-log policy is implemented; outer infrastructure must
apply the same policy. Native Opus support is checked at runtime; unsupported
browsers report errors. A lost tool result has unknown outcome and is never replayed.
