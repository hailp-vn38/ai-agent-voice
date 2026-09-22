# ADR 0027 — Provider concrete Phase 4: OpenAI LLM qua crate `llm` và ZeroTTS local

## Status
Accepted

V1 chọn hai adapter compile-time cho Phase 4:

- `providers.llm.type = "openai"` build client qua crate Rust `llm`, pin `=1.3.8` với `default-features = false` và features `openai`, `rustls-tls`. Factory chỉ map typed configuration sang `LLMBackend::OpenAI` và `LLMBuilder`, rồi bridge typed stream của crate thành `LlmEvent`; `SessionActor` không import type hay protocol của crate `llm`.
- `providers.tts.adapter = "zerotts_onnx"` dùng native Rust + ONNX. Typed configuration chọn Logical Model Identity; Model Preparation resolve và inject `ResolvedModel` vào `TtsFactory`. `ZeroTtsProvider` chỉ infer và trả typed PCM với sample rate thực tế.

`ProviderRegistry` fixed mở rộng với `LlmFactory` và `TtsFactory`; loader validate/build đủ VAD, ASR, LLM và TTS trước bind socket. Registry không discovery code runtime, `dlopen` hoặc hot-load. LLM remote không cần `ResolvedModel`, nhưng startup phải validate typed config và API key. API key được phép nằm trong TOML Phase 4 nhưng không được log, debug, telemetry hoặc gửi về client.

Không dùng `openai_speech_v1`, custom OpenAI HTTP/SSE, Python sidecar, ZeroTTS HTTP service hoặc HTTP fallback. `SpeechOutput`, không phải ZeroTTS, sở hữu normalize/resample, 24 kHz mono Opus 60 ms, pacing, generation và `tts:start`/`tts:stop`. ASR vẫn dùng local streaming baseline theo ADR-0039.

Phase 4 luôn gọi `chat_stream_with_tools(messages, None)`. Tool call xuất hiện bất thường làm generation fail `llm_unexpected_tool_call`: không MCP, không retry, cancel LLM/SpeechOutput và không submit segment mới. Audio đã delivered trước event không thể thu hồi; GenerationGate loại mọi audio stale hoặc còn queue. Phase 6 mới truyền tool definitions và chạy tool-result loop.

`LlmRuntime` application-owned chỉ giữ `Arc<dyn llm::LLMProvider>`, global `llm_concurrency` semaphore, runtime config và bounded event routing. Mỗi request là Tokio task mới mang Voice Session/generation và CancellationToken, không sticky session hoặc OS worker. Permit/timeout bắt đầu khi runtime accept request, kết thúc khi `Finished`/`Failed`/`Cancelled`; text delta không reset timeout. Cancel/timeout dừng polling, drop stream và chỉ release permit khi task đã terminal. Không route được terminal event là controlled failure/cancellation, không silent drop.

OpenAI startup chỉ validate API key/base URL/model, timeout và `LLMBuilder` build local; không gọi network health probe, model list hay dummy completion trước bind. DNS/TLS/auth/quota/model/5xx/stream failures thuộc LLM Operation hiện tại, fail generation thay vì server-wide unavailability.

ZeroTTS chạy trong `TtsWorkerRuntime` application-owned, bounded; worker sở hữu native mutable inference, timeout, cancel/cleanup acknowledgement và quarantine. `SpeechOutput` giữ `Submit`/`FinishInput`/`Cancel` cùng event semantic. Một ONNX call không preempt được chỉ có thể nhận cancel ở safe point kế tiếp, nhưng output stale không được vượt GenerationGate và cleanup acknowledgement mới release worker slot.

Mỗi generation chỉ có tối đa một segment đang synthesize; pending ordinal liên tục và bounded. Segment N+1 đợi `SegmentFinished(N)`, nhưng inference N+1 có thể overlap AudioPacer của N khi output queue vẫn bounded và playback order giữ nguyên. `Drained` đòi `FinishInput`, pending rỗng, không active synthesis và audio ordinal cuối đã paced. Sau `tts:start`, bất kỳ failure nào invalidate generation trước, cancel pipeline, drop queued audio rồi gửi chính xác một `tts:stop`; trước `Started` không gửi cặp control. Failure không commit Delivered Assistant Response.

`speech_output.pending_segments` default 8 là hard semantic bound riêng của worker command queue. Pending full làm generation fail `speech_output_backpressure`, cancel LLM và không poll/accept thêm delta; không drop/overwrite segment. `limits.tts_concurrency` là application admission capacity, `workers.tts.max_workers` là native capacity; Phase 4 bắt buộc hai giá trị bằng nhau và chỉ admission semaphore cấp permit trước worker lease.

`zerotts_default` pin roles `config`, `tokenizer`, `null_voice`, `voices_index`, `voice`, `text_encoder`, `prefix_step`, `local_frame_decode`, `codec_decode_full`, `codec_decode_step`, `codec_shared_data`, `codec_metadata`, `codec_license`; default voice là `maichi`. Model-level acknowledgement phải biểu diễn cả ZeroTTS MIT và bundled MOSS codec Apache-2.0. Startup/warmup reject missing external codec data, tokenizer/config/metadata mismatch, sai graph I/O hoặc sai voice dimensions. Provider trả PCM float mono 48 kHz; SpeechOutput riêng resample sang downlink 24 kHz.

Phase 4 giữ manifest license model-level: `license = "MIT; bundled-codec=Apache-2.0"`, acknowledgement phải khớp chính xác identity, revision và chuỗi này; `codec_license` vẫn là artifact bắt buộc. Không migrate sang artifact-level licensing trừ khi policy deployment phải quyết định độc lập theo component.

Sau Model Preparation, ZeroTtsFactory chạy deterministic non-user warmup trước bind với voice `maichi`. Warmup phải exercise tokenizer, voice latent, ba TTS graph, codec và external data; success chỉ khi completion tạo PCM mono 48 kHz không rỗng, finite. Nó không tạo SessionActor/SpeechOutput/GenerationGate/Opus/WS và không giữ mutable inference state làm user operation. Bất kỳ artifact/shape/codec/PCM failure nào đều fail startup.
