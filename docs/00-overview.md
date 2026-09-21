# 00 — Tổng quan hệ thống

## 1. Mục tiêu sản phẩm

`voice-agent-server` là một voice-agent server cá nhân, chạy như **một binary Rust duy nhất**, dùng OTA discovery + WebSocket protocol v1.

Hệ thống phải đạt được chuỗi hành vi:

```text
Voice Protocol Client boot
  -> OTA discovery
  -> WebSocket connect
  -> hello / hello
  -> microphone Opus uplink
  -> VAD / ASR
  -> STT display
  -> streaming LLM
  -> incremental TTS
  -> Opus downlink
  -> interrupt / new turn
  -> optional Device MCP tool call
```

## 2. Nguyên tắc thiết kế

### 2.1 Protocol-compatible, implementation-independent

Mọi Voice Protocol Client chỉ phụ thuộc wire protocol. Rust server không cần sao chép kiến trúc reference hay phụ thuộc một loại phần cứng cụ thể.

### 2.2 Một state owner duy nhất

Mỗi kết nối có một `SessionActor`. Chỉ actor này được phép thay đổi state của phiên.

Không cho VAD, ASR, LLM, TTS hoặc WebSocket task sửa trực tiếp cùng một struct state.

### 2.3 Provider thay thế được

ASR/LLM/TTS/VAD được đặt sau trait. Core không biết provider là OpenAI, local HTTP, Ollama, Whisper server hay một dịch vụ khác.

### 2.4 Cancellation là first-class concept

Mỗi lượt hội thoại có `generation_id` và `CancellationToken` riêng. Output thuộc generation cũ phải bị drop.

### 2.5 Bounded queues

Mọi queue audio phải bounded để tránh tăng RAM khi downstream chậm.

### 2.6 Single WebSocket writer

Chỉ một task ghi WebSocket. Các module khác gửi `OutboundMessage` vào channel.

### 2.7 Compatibility baseline

V1 cam kết Compatibility Profile `voice-ws-v1-baseline`: WebSocket v1 raw Opus và MCP `type:"mcp"` bọc JSON-RPC 2.0. Reference source là `78/xiaozhi-esp32` v2.5.0 tại commit `ac6deed3d8e75348475364bf40ad953c6cd48054`; source này chỉ cung cấp provenance và fixture. HIL Reference Profile là `bread-compact-wifi`.

### 2.8 Ranh giới vận hành V1

V1 là personal trusted-LAN deployment. `auth.token = ""` tắt auth; token không rỗng yêu cầu `Authorization: Bearer <token>`. `Device-Id` và `Client-Id` chỉ là metadata. OTA trả static token khi auth bật nên không phải security boundary; Internet không nằm trong supported V1 profile. Mỗi Device ID có đúng một Voice Session hoạt động; reconnect thay thế session cũ và reset dialogue RAM.

## 3. Phạm vi V1

| Khả năng | V1 |
|---|---|
| OTA trả WebSocket URL | Có |
| WebSocket protocol v1 | Có |
| JSON control messages | Có |
| Opus raw frame | Có |
| Manual listen | Có |
| Server-side VAD | Có |
| ASR final result | Có |
| Streaming LLM | Có |
| Incremental TTS | Có |
| Audio pacing | Có |
| Abort / barge-in | Có |
| Short-term dialogue | Có |
| Device MCP | Có |
| WS protocol v2/v3 | Sau V1 |
| Server AEC | Sau V1 |
| Vision | Sau V1 |
| Persistent memory | Sau V1 |
| Server MCP | Sau V1 |

V1 không hỗ trợ acoustic barge-in khi Speaking: raw microphone audio và VAD không được hủy TTS. Manual mode chỉ hủy qua `abort` hoặc `listen:start`; auto VAD chỉ endpoint speech khi Listening. Device MCP dùng server-side allowlist, default deny; LLM chỉ thấy tool đã qua policy filter.

WS writer dùng hai queue bounded cho control và audio; control hợp lệ ưu tiên audio. Audio luôn mang generation ID, `tts:start` phải đứng trước Opus đầu tiên của turn và không Opus nào của turn được tới WebSocket sau `tts:stop`.

Sau `tts:stop`, manual mode vào `Ready` (giữ WS/session nhưng không nhận mic), còn auto mode vào `Listening`. Active Turn chỉ acquire sau utterance hoàn tất, ngay trước ASR; không có permit thì từ chối turn ngay. Tool-capable LLM round buffer toàn bộ và chỉ TTS final no-tool round.

Audio V1 dùng Canonical Audio Profile cố định: uplink raw Opus/16 kHz/mono/60 ms và downlink Opus/24 kHz/mono/60 ms; mismatch đóng WS 1002 trước Ready. Provider operation không automatic retry. Telemetry chỉ chứa metadata vận hành không nhạy cảm qua Trace Session ID ngẫu nhiên; dialogue chỉ tồn tại trong RAM của Voice Session.

History giới hạn theo message count và prompt token budget, eviction theo Exchange Atom cũ nhất. V1 pin ba adapter OpenAI-compatible concrete: transcription WAV multipart, Chat Completions SSE/function tools và speech WAV; tool-level failure đã sanitize quay lại LLM, còn session/cancellation failure là terminal. Config immutable theo process; shutdown gửi cleanup control và close bounded trước khi force terminate.

Device tool batch chạy tuần tự theo LLM order. ASR text rỗng là `CompletedSilent`; final LLM text rỗng hoặc TTS không tạo audio hợp lệ là `Failed`. `tts:start` chỉ được gửi khi AudioPacket hợp lệ đầu tiên đã sẵn sàng, nên không cần `tts:stop` cho TTS không từng phát được audio.

Protocol V1 fail closed trong `AwaitHello`, fail soft cho từng application message sau handshake, đóng 1009 cho frame quá lớn và không invent custom wire error. WebSocket framing/UTF-8 fault là trách nhiệm transport layer.

## 4. Definition of Done V1

V1 chỉ hoàn thành khi cả ba gate sau pass:

1. **Gate A — Protocol Conformance:** unit, integration, protocol fixture, Reference Client, cancellation, queue/backpressure và MCP mock test đều pass.
2. **Gate B — Independent Client Interoperability:** một Voice Protocol Client độc lập hoàn tất OTA, WebSocket connect (kèm Bearer auth khi `auth.token` được bật), `hello`, raw payload uplink/downlink, `tts:start`/`tts:stop`, `abort`, MCP discovery/call và reconnect.
3. **Gate C — Real Voice Pipeline:** ít nhất một chuỗi real ASR → streaming LLM → real TTS → một Voice Protocol Client pass, gồm multi-turn context, cancellation, provider timeout và reconnect.

Reference Client độc lập cung cấp bằng chứng Gate A/B theo phạm vi tương ứng. Firmware từ reference source trên HIL Reference Profile là **Reference Hardware Compatibility Test** có giá trị bổ sung, không phải định nghĩa duy nhất của protocol compatibility.
