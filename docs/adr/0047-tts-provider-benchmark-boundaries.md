# ADR 0047 — TTS Provider Benchmark tách cold, steady và canonical delivery boundary

## Status
Accepted

`provider-bench tts provider|delivery` là developer CLI chỉ benchmark selected Typed Provider Configuration. V1 chỉ có TTS: không enumerate Provider Registry, không hỗ trợ multi-provider configuration, LLM hoặc ASR. Cả hai mode dùng cùng fixed, versioned Vietnamese workload và cùng run policy; text không được override bằng CLI và report chỉ công bố workload version.

Runner resolve config theo thứ tự `--config`, `VOICE_AGENT_CONFIG`, rồi `config.toml`. Nó chạy Model Preparation hoặc explicit installed-model verification, build provider đúng một lần (bao gồm startup readiness), workload warmup mặc định một lần, rồi năm measured run trên cùng provider instance. Warmup không thuộc statistics. Cold metrics gồm `model_preparation_ms` và `provider_build_and_readiness_ms`; steady metrics gồm per-run processing, provider audio duration, RTF, cùng `ttfa_ms` của provider mode hoặc `first_packet_ms` của delivery mode. Aggregate báo min, median, max và chỉ báo p95 từ 20 samples trở lên.

Mode `provider` gọi `TtsProvider::synthesize_stream()` và kết thúc khi nhận terminal PCM provider-facing hợp lệ. Mode `delivery` dùng đúng PCM callback stream đó, sau đó chạy component audio deterministic dùng chung với production: fade-in đầu stream, stateful 48 kHz-to-24 kHz resample, float-to-i16 conversion, canonical 1.440-sample framing, Opus profile và tail fade-out/zero-padding. Nó kết thúc khi canonical Opus packet cuối cùng sẵn sàng; không bao gồm worker runtime, queue, prebuffer, pacing, WebSocket hay client playback. Extraction không được tự sửa behavior production, kể cả fade duration; sửa behavior phải thay đồng thời production component và regression tests.

Một lỗi ở warmup, synthesis, PCM validation hoặc delivery pipeline dừng mode hiện tại, tạo final failure report privacy-safe và exit 1. JSON là opt-in qua `--output`; file tồn tại cần `--overwrite`, và write dùng temporary file rồi atomic rename. JSON/stdout không chứa workload text, audio, filesystem path, secret, deployment endpoint hoặc raw error string; failure chỉ dùng error category ổn định. `comparison_qualified` chỉ true khi config offline hoặc caller yêu cầu local-model verification, không suy đoán download từ trạng thái cache.

## Consequences

TTS benchmark đo đúng deployment-selected adapter, model identity và inference configuration mà không mở rộng Provider Registry hay refactor TtsFactory trước khi có adapter thứ hai. Canonical downlink conversion phải được trích từ `SpeechOutput` thành component audio reusable; `SpeechOutput` giữ ownership của worker/session, queue, prebuffer, pacing và WebSocket-facing lifecycle. Benchmark có deterministic unit tests cho workload version-to-bytes, provider build một lần, warmup exclusion, metric boundaries, tail fidelity, error category/privacy, atomic output behavior và qualification semantics; real-model benchmark vẫn là gate riêng phụ thuộc model/runtime local.
