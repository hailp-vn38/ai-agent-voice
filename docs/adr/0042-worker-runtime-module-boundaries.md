# ADR 0042 — Worker runtime và module boundary cho local inference

## Status

Accepted

## Context

Phase 3 cần pinned VAD/ASR streams, acknowledgement-driven cleanup, generation-safe event routing và timeout/quarantine. Provider interface đồng bộ trực tiếp từ `SessionActor` không biểu đạt được ownership hoặc acknowledgement của mutable native runtime, nên không thể chứng minh slot chỉ reusable sau cleanup.

## Decision

Tách local inference thành ba boundary:

- `providers` chứa ba trait tối thiểu, error, `ProviderSet` (`vad` và `asr`) và adapter concrete theo loại; startup loader chỉ dùng `match` theo typed config. Không có registry, capability negotiation hoặc TTS placeholder ở Phase 3. Adapter không biết WebSocket, Voice Session, worker pool hoặc generation.
- `workers` là application-owned bounded runtime. Mỗi worker sở hữu provider session/stream mutable và nhận command, trả typed event có session, generation và opaque lease/stream identity. `WorkerSupervisor` là consumer duy nhất của worker event ingress, route event vào mailbox của đúng Voice Session và chạy timeout/quarantine độc lập lifetime socket. Cleanup acknowledgement là điều kiện release slot.
- `session` chứa actor, event và turn state. Actor chỉ drain mailbox của chính Voice Session, owns generation và quyết định outbound V1; không trực tiếp gọi mutable provider session hoặc poll global worker receiver.

Phase 3 materialize `audio::vad_segmenter`, VAD/ASR provider modules, VAD/ASR workers và session event/turn modules. TTS chỉ được đưa vào khi Phase 4 bắt đầu; không reserve module hoặc dependency trong Phase 3.

## Consequences

- Ticket Auto VAD phải bắt đầu sau worker-runtime foundation; không còn cố vá acknowledgement/quarantine vào synchronous provider API.
- Unit test có thể fake provider tại provider trait boundary và quan sát worker command/event, còn session/WebSocket test quan sát phase, STT ordering, event routing theo session, disconnect cleanup và close code.
- Module layout có thêm indirection, đổi lại provider implementation không sở hữu session lifecycle và worker failure có thể được cô lập theo Voice Session.
