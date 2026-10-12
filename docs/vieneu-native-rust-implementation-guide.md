# VieNeu-TTS v3 Turbo Native Rust — Hướng dẫn thiết kế và triển khai

> Repository: https://github.com/hailp-vn38/ai-agent-voice \
> Upstream: https://github.com/pnnbao97/VieNeu-TTS \
> Phonemizer: https://github.com/pnnbao97/sea-g2p \
> Đối chiếu: `main` ngày 2026-10-12, commit `92b05c7d63cf1a634f5d1c4d8d6963aeb2189569`. \
> Trạng thái: **Đã triển khai native FP32 và kiểm tra worker/HTTP diagnostic; hiệu năng release và playback WebSocket chưa được qualification**. Xem [kết quả kiểm tra](../.scratch/vieneu-native/README.md). \
> Hướng triển khai: **Rust orchestration + ONNX Runtime CPU, không Python/sidecar/HTTP inference**.

## 1. Quyết định và mục tiêu

Triển khai **một local TTS Provider Adapter mới `vieneu_native`**, sử dụng **VieNeu-TTS v3 Turbo ONNX FP32**. Toàn bộ text preparation, autoregressive generation control, sampling, quản lý state, voice selection và streaming delivery được viết bằng Rust; ONNX Runtime là thư viện native C/C++ được gọi qua crate `ort`. Đây là “không Python” chứ **không phải pure-Rust neural kernels**.

Không thay thế `vieneu_http`. Không thay đổi giao thức WebSocket, `SpeechOutput`, `SessionActor`, Opus/pacing hay giao diện Providers ngoài khả năng render descriptor mới bằng UI generic sẵn có.

### Mục tiêu bắt buộc của bản đầu (MVP)

1. TTS offline trên CPU, không gọi mạng khi synthesize.
2. Dùng preset voice từ bộ VieNeu v3 Turbo đã pin, không yêu cầu audio tham chiếu trong mỗi lần nói.
3. Chuẩn hóa tiếng Việt và sinh phoneme bằng Rust core của `sea-g2p`, kèm parity với pipeline upstream.
4. Nạp model một lần theo vòng đời long-lived worker; sinh audio 48 kHz mono `f32`; streaming nhiều PCM chunks qua `TtsWorker::synthesize`.
5. Sử dụng Provider Asset Manager, typed config, registry, Provider Runtime Manager và admin provider test/diagnostic hiện có.
6. Giữ isolation và cancellation; không để âm thanh của operation đã bị huỷ đi vào generation mới.
7. Có test fixture parity và benchmark CPU thực tế trước khi bật production.

### Ngoài phạm vi MVP

- Voice cloning từ file WAV mới, speaker encoder, denoiser, codec encode.
- CUDA/PyTorch, CUDA Graphs, GPU continuous batching hoặc đổi inference backend.
- INT8 production ngay từ đầu, tự chọn precision theo request.
- Fine-tuning LoRA; podcast nhiều speakers; model v4 proprietary.
- Thay đổi `TtsProvider`/`TtsWorker` interface, thêm endpoint HTTP TTS, websocket message mới hoặc page web riêng.
- Tự tạo thêm worker scheduler/batch scheduler song song với `TtsWorkerRuntime`.

**Quyết định về phiên bản:** `vieneu_native` ở MVP tương ứng đúng một model identity `vieneu_v3_turbo_fp32`. INT8 là phase riêng: muốn expose precision, phải xác nhận/runtime-key hoá physical identity và asset set để không chia sẻ nhầm workers giữa FP32 và INT8. Không đưa `precision`, `model_path` hoặc `num_threads` tùy ý vào desired config ngay.

## 2. Codebase và ADR phải tuân theo

| Vị trí hiện có | Ý nghĩa khi triển khai |
| --- | --- |
| `src/providers/tts/mod.rs` | Interface `TtsProvider`, `TtsWorker`, `TtsBinding`, `TtsSynthesisRequest`, `TtsDiagnosticRequest`, `TtsError`; **giữ nguyên**. |
| `src/providers/tts/vieneu/*` | Adapter `vieneu_http`; **không sửa hành vi**, không xóa. |
| `src/providers/tts/zerotts/runtime/{codec,contract,synthesis}.rs` | Ví dụ graph ORT, cache, PCM, warmup; chỉ trích shared logic khi đã chứng minh cùng codec contract. |
| `src/providers/tts/zerotts/assets.rs` | Mẫu model assets pinned/atomic download và danh sách voices. |
| `src/providers/tts/kokoro_vi/descriptor.rs` | Mẫu local descriptor/inspector/voice metadata. |
| `src/providers/factory_registry.rs` | Compiled `TtsFactory`; `local_model_identity`; `effective_local_config`; `TTS_FACTORIES`. |
| `src/providers/registry.rs` | Compiled Provider Adapter Registry và capability registration. |
| `src/config/providers.rs` | `TtsInstanceConfig` enum, `adapter()`, `preload()`, typed validation. |
| `src/providers/local_runtime.rs` | Liên hệ local adapter với resource manager/estimated footprint và isolation. |
| `src/services/provider_runtime/*` | Versioned materialization, bounded loaders, resource leases, quotas và readiness. |
| `src/workers/tts/*` | Long-lived workers, cancellation, reset, queue/backpressure, acknowledgement. |
| `src/app/admin/provider_tests.rs` / `provider_test_drafts.rs` | Test provider đã lưu/chưa lưu qua cùng flow. |
| `docs/flows/05-tts.md` | `SpeechOutput` sở hữu PCM canonicalization, 24 kHz resample, Opus, pacing. |
| `apps/admin-web/src/components/providers/*` | Generic provider create/detail/diagnostic; ưu tiên cấu hình qua descriptor. |

Đường dẫn trong bảng là tương đối từ `crates/voice-agent-server`, trừ `docs` và `apps`.

ADR bắt buộc: [0076 — Provider-owned model assets](adr/0076-provider-owned-model-assets.md), [0071 — Versioned Provider Runtime Manager](adr/0071-versioned-provider-runtime-manager.md), [0062 — Credential-free Typed Provider Config](adr/0062-credential-free-typed-provider-config.md), [0085 — VieNeu HTTP Adapter](adr/0085-vieneu-http-streaming-provider.md), [0047 — TTS benchmark](adr/0047-tts-provider-benchmark-boundaries.md). Tuân theo các module hiện có; không tạo runtime manager, asset download flow hoặc compatibility fallback riêng.

## 3. Upstream chuẩn để đối chiếu

- VieNeu source pin tham chiếu: `pnnbao97/VieNeu-TTS@85344322b7258b4e25479b692e8e3396baf9db34`.
- SEA-G2P source pin tham chiếu: `pnnbao97/sea-g2p@e825173f235d08ea19315b2b279fb11153b44cea`.
- Model repo: https://huggingface.co/pnnbao-ump/VieNeu-TTS-v3-Turbo ; thư mục `onnx_update/` (FP32). **Trong implementation phải chốt full immutable Hugging Face commit SHA** trước khi tạo `MODEL_REVISION` và URL assets; không dùng `main` trong URL production.
- Codec: https://huggingface.co/OpenMOSS-Team/MOSS-Audio-Tokenizer-Nano-ONNX ; pin revision tương tự. Nếu quyết định dùng codec assets của ZeroTTS, phải chứng minh graph/metadata cùng version/contract trước, không chia sẻ theo tên file.
- Preset voice: `src/vieneu/assets/voices_v3_turbo.json` của VieNeu; có thể đối chiếu bản voice file trong model repository nếu có, nhưng phải pin cùng model và kiểm tra schema.
- G2P dictionary: `python/sea_g2p/sea_g2p.bin` trong upstream SEA-G2P; là input binary của `G2PEngine`, phải pin và cung cấp qua Provider Asset Manager.

Các file upstream trọng yếu:

| Upstream source | Port/trích hợp phần nào |
| --- | --- |
| `src/vieneu/_v3_turbo_engine/onnx_runtime_lite.py` | Graph I/O, embeddings, speaker anchor, sampling, prefill, KV cache, decoding frame, streaming. |
| `src/vieneu/_v3_turbo_engine/rep_history.py` | Sliding-window repetition penalty (mặc định 64 frames). |
| `src/vieneu/_v3_turbo_engine/configuration_v3_turbo.py` | Model config/tokens/shape constraints. |
| `src/vieneu/_v3_turbo_engine/prompt_v3_turbo.py` | Prompt token layout, reference rows. |
| `src/vieneu_utils/phonemize_text.py` | Chunking và emotion tags quanh SEA-G2P. |
| `src/vieneu_utils/core_utils.py` | Frame cap, text chunking, pause/crossfade và babble guard. |
| `src/vieneu/_v3_turbo_engine/speaker/*` | Phase cloning, không đưa vào MVP. |

### Assets bắt buộc cho MVP

`onnx_update/`:

~~~text
config.json
tokenizer.json
vieneu_prefill.onnx
vieneu_decode_step.onnx
vieneu_acoustic_cached.onnx
vieneu_backbone_shared.data
vieneu_v3_heads.npz
~~~

Codec:

~~~text
moss_audio_tokenizer_decode_step.onnx
moss_audio_tokenizer_decode_shared.data
codec_browser_onnx_meta.json
~~~

Nếu contract graph cần thêm file, bổ sung rõ trong `CORE_ASSETS`. Không yêu cầu `decode_full.onnx` cho MVP khi diagnostic dùng `decode_step`; thiếu streaming decoder phải fail readiness thay vì fallback. Voice preset JSON và SEA-G2P `sea_g2p.bin` là artifacts bắt buộc; license files/notice phải được giữ khi phân phối.

Lưu ý graph external data phải nằm đúng relative path mà ONNX protobuf tham chiếu. Không đổi tên `.data` tùy ý. Các model/codec files có thể lớn; bounded download/temp và memory reservation cần đo/điền theo thực tế.

## 4. Thiết kế module và seam

Khởi đầu với số file tối thiểu, chỉ tách module nội bộ khi phát sinh logic phức tạp cần quản lý riêng:

~~~text
crates/voice-agent-server/src/providers/tts/
├── vieneu/                    # HTTP hiện có, giữ nguyên
└── vieneu_native/
    ├── mod.rs
    ├── config.rs
    ├── descriptor.rs
    ├── assets.rs
    ├── provider.rs
    ├── runtime.rs             # ONNX, embeddings, sampling, KV cache
    ├── text.rs                # SEA-G2P và chunking
    └── codec.rs               # MOSS streaming state
~~~

**External seam duy nhất:** `TtsProvider::open_worker() -> Box<dyn TtsWorker>`. Caller chỉ biết đã chọn voice/language, nhận PCM callback hoặc error. Internals (G2P, graph paths, temperature, seeds, caches, codec) không lộ lên `SpeechOutput`.

**Module depth:** `vieneu_native` che tất cả khác biệt model sau interface TTS đã có. Không tạo hàng loạt trait “Model”, “Tokenizer”, “Codec”, “Sampler” chỉ để pass-through; chỉ tạo internal seam nếu cần fake/parity tests cụ thể hoặc khi đã có adapter thứ hai thực sự cần reuse.

**State ownership:**
- Provider/contract bất biến giữ resolved paths, tokenizer, config, voice registry/preset embeddings, ONNX metadata.
- Mỗi retained `VieneuNativeWorker` sở hữu các `ort::Session` và per-operation mutable states; không chia sẻ mutable session thiếu đảm bảo an toàn.
- Một operation có KV cache, repetition windows, codec streaming state, frame budget, generation-specific counters riêng; tạo lại hoặc reset triệt để mỗi operation.
- Voice selection đi cùng `request.selection.voice`, **không** mặc định lấy một voice từ shared physical provider và không mutate provider-wide voice.
- Có explicit readiness voice được xác minh. Không cho readiness binding `__readiness__` trở thành voice public.

Giữ MOSS decoder trong `vieneu_native/codec.rs` cho MVP. Không tạo shared codec abstraction với ZeroTTS khi chưa có duplication thực tế và contract tương thích.

## 5. Typed desired config, model identity và descriptor

Tên adapter: `vieneu_native`. Model ID cố định ban đầu: `vieneu_v3_turbo_fp32`.

Ví dụ desired provider JSON (API/Admin sau khi implement):

~~~json
{
  "adapter": "vieneu_native",
  "voice": "<valid-preset-voice-id>",
  "language": "vi-VN",
  "preload": true
}
~~~

`<valid-preset-voice-id>` là placeholder trong tài liệu, không phải voice ID có thể lưu; sử dụng ID thực tế từ catalog sau khi chốt preset mapping.

Không đưa vào desired config: `base_url`, `api_key`, `model`/`model_path`, `num_threads`, `precision`, arbitrary JSON options, tải artifacts theo URL người dùng cung cấp. `model` và `num_threads` là **effective local fields** được inject bằng `effective_local_config` giống ZeroTTS/Kokoro (nếu typed struct cần những trường này). Số threads lấy `runtime.onnx.threads_for("vieneu_native")`.

Thêm vào `src/config/providers.rs`:
- `pub use ...::vieneu_native::config::VieneuNativeConfig`.
- Variant `TtsInstanceConfig::VieneuNative` với serde adapter `vieneu_native`.
- Match `adapter()` và `preload()`; regression test serde/deny_unknown_fields/secret denylist.
- Giữ nguyên variants đang có.

Thêm vào `src/providers/factory_registry.rs`:
- `VieneuNativeFactory` implement `TtsFactory`, factory table `TTS_FACTORIES`, `local_model_identity`.
- `model_identity()` không trả `None` như remote adapter.
- `build()` validate typed selection/threads; resolve assets sau `ensure_assets()`; nạp immutable contract, **không tải model**; open worker khi resource manager tạo retained replicas.
- Gắn local runtime cost/replica classification ở `src/providers/local_runtime.rs` khi code hiện hành yêu cầu.

Thêm `descriptor.rs`:
- `ProviderType::Tts`, `offline: true`, `streaming: true`, `provider_output_sample_rates: [48000]`, `voice_delivery_sample_rates: [24000]`, `channels: [1]`, `languages: [vi-VN]`.
- `models` một lựa chọn `vieneu_v3_turbo_fp32`, `voices` từ pinned preset catalog, source là static/bootstrap local theo cơ chế hiện có; không remote discovery.
- `voice` và `language` là select từ capabilities; `preload` bool.
- Local `ProviderAdapterRegistration::local` kèm `assets::ASSETS` và bootstrap inspector giống ZeroTTS/Kokoro. Model select validation phải nhất quán với effective selection.
- Danh sách voice trong descriptor và voice registry phải đồng bộ; không có option selectable mà runtime không thể resolve.

Trong `src/providers/registry.rs` đăng ký descriptor/registration mới theo convention hiện hữu; không tạo route mới cho capability discovery. Generic UI phải hiển thị và test như các TTS provider khác; chỉ thêm test UI nếu generic renderer đang có assumptions về adapter names.

## 6. Rust SEA-G2P: normalizer và phonemizer

Dùng dependency `sea-g2p-rs` từ upstream đã pin revision, `default-features = false` để loại PyO3/Python. Phiên bản API cần được kiểm bằng compile ở thời điểm thực hiện:

~~~toml
sea-g2p-rs = { git = "https://github.com/pnnbao97/sea-g2p", rev = "e825173f235d08ea19315b2b279fb11153b44cea", default-features = false }
~~~

Cách dùng dự kiến: `sea_g2p_rs::lang::vi::Normalizer`, `sea_g2p_rs::g2p::G2PEngine` và `sea_g2p_rs::punc`. Đọc `src/lib.rs` và `src/g2p/mod.rs` của revision đã pin để xác nhận tên public methods, không tự suy đoán Python bindings là Rust API.

Pipeline chuẩn:

~~~text
text segment
  -> validate UTF-8/length/control characters
  -> preserve emotion cues (nếu model/checkpoint hỗ trợ)
  -> SEA normalizer (vi) + punc rules
  -> SEA G2P với pinned .bin
  -> split by phoneme / bounded model tokens
  -> tokenizer.json -> token IDs
~~~

Quan trọng: Python VieNeu gọi `SEAPipeline(lang="vi").run(text, punc_norm=True)`; bản Rust phải tương đương ở các trường hợp được hỗ trợ. Không dùng bộ normalize của ZeroTTS thay thế: hai model có text contracts khác nhau.

Chia chunk theo kết quả chuẩn hóa/phoneme, không chia thô theo số ký tự trước normalize vì số/ngày giờ có thể phình độ dài. Không cắt trong `<en>...</en>` hoặc `<|emotion_N|>`. Giữ semantic pause sau dấu câu, newline, và merge ngắn ở mức đủ giống upstream; không tạo silence chunks lớn nếu không cần.

Golden corpus khởi đầu với **20–30 câu đại diện**, bao phủ số/ngày/giờ, tiền/đơn vị, email/URL, viết tắt, code-switch, dấu tiếng Việt, văn bản dài và emotion cues. So sánh **phonemes/token IDs** với oracle Python upstream tạo offline trong môi trường dev; Python không thuộc runtime. Mở rộng regression fixtures từ những sai lệch thực tế, không chấp nhận sai lệch chưa được đánh giá.

## 7. Nạp graph và xác minh contract

Dùng `ort`, `tokenizers`, `ndarray`, `ndarray-npy` (đã có trong workspace). Trước khi tạo sessions:

1. Parse model `config.json` và validate `n_vq`, vocab sizes, token IDs, hidden dims, number of layers/heads và model version theo pinned profile.
2. Load `vieneu_v3_heads.npz`, đọc `text_emb`, `audio_emb`, `xvec_w`/`xvec_b`/`xvec_ln_w`/`xvec_ln_b`/`xvec_ln_eps`; xác minh shape, dtype và finite.
3. Load `tokenizer.json` và xác minh special IDs/known phoneme tokens.
4. Khởi tạo ORT qua `initialize_ort(runtime.onnx.library)`, tạo sessions từ paths của assets.
5. Kiểm tra graph input/output names, rank, dtype và dynamic dimension contract cho prefill/decode_step/acoustic_cached/codec_decode_step; fail loud nếu thiếu external data hoặc metadata không tương thích.
6. Chỉ load decoder streaming khi có codec metadata hợp lệ; cache tensors phải khởi tạo đúng, nhất là `cached_positions = -1`.
7. Nạp preset voice registry và map stable ID -> `speaker_emb` + `codes`; validate `speaker_emb` 192 chiều khi model yêu cầu, ref codes matrix đúng `n_vq`, IDs trong codebook.

Không hardcode dims từ bản minh họa thành tính năng generic. Các giá trị hiện upstream phổ biến: `n_vq = 16`, sample rate `48000`, một codec frame khoảng `3840` samples. Chúng vẫn phải được assert với version được pin.

Ràng buộc thread: cùng một worker sở hữu ONNX sessions theo configured ORT thread count; tránh để nhiều sessions phụ spawn thread pools quá mức; đo startup, memory và CPU saturation khi `max_workers > 1`.

## 8. Synthesis algorithm và state isolation

Pseudo-flow (diễn tả thuật toán, **không** phải code copy/paste):

~~~text
synthesize(request, cancelled, on_pcm):
  require language == "vi-VN"
  voice = preset_registry.get(request.selection.voice)  // fail explicit if absent
  chunks = normalize_phonemize_and_chunk(request.text)
  for each chunk in chunks in original order:
    if cancelled: abort
    rows = build_prompt_rows(style_id, phoneme_ids, voice.ref_codes)
    speaker_anchor = xvec_proj_and_layernorm(voice.speaker_emb)
    embeddings = compose_text_and_audio_embeddings(rows, speaker_anchor)
    (hidden, backbone_kv) = prefill(embeddings)
    acoustic_history = new bounded per-codebook histories
    codec_state = new_streaming_codec_state()
    max_frames = min(config.max_new_frames, expected_frames(chunk))
    for t in 0..max_frames:
      check_cancelled()
      (codes[0..n_vq], is_eos) = acoustic_decode_and_sample(hidden, acoustic_history)
      validate all code indices, dimensions and finite float tensors
      emit/update codec when streaming target reached:
          pcm48 = codec_decode_step(frames, codec_state)
          validate finite, non-empty, mono/48000
          on_pcm(pcm48)   // synchronous backpressure
      if is_eos: break
      (hidden, backbone_kv) = backbone_decode_step(codes, anchor, position, backbone_kv)
    flush pending codec frames
    if frame_limit reached without EOS: classify as capped/babble, no infinite loop
  reset operation state before success/failure worker reuse
~~~

Không dùng output waveform từ full `decode_full` để gọi “streaming”; callback đầu tiên phải đến trước khi toàn bộ đoạn hoàn tất đối với đoạn đủ dài.

Các yêu cầu parity quan trọng:

- **Prompt:** style token + text prompt start + tokenized phonemes + text end; reference rows có ref-slot trong cột text và 16 audio codes theo mỗi row; pad đúng. Khi đổi model/config phải test token IDs.
- **Speaker anchor:** nhân `speaker_emb` với projection weights rồi LayerNorm theo đúng công thức; cộng anchor vào input embeddings. Không đổi độ chính xác/fusion tuỳ ý trước parity test.
- **Acoustic:** sampling codebook đầu và các codebooks tiếp theo đúng thứ tự local decoder ONNX; phân biệt EOS text logits với mã âm thanh.
- **Sampling:** mặc định upstream tham chiếu `temperature=0.8`, `top_k=25`, `top_p=0.95`, `repetition_penalty=1.2`, sliding window `64`. Port chính xác xử lý giá trị âm/dương trong penalty và trường hợp temperature <= 0. Seed cố định ở parity tests; đối chiếu logits/top-k/accepted token, không yêu cầu toàn waveform giống nhau khi stochastic path khác PRNG.
- **Frame cap:** lấy từ logic `max_expected_frames(phonemes)` để tránh nói lặp khi model bỏ lỡ EOS; không chỉ dùng trần `max_new_frames=300` cho mọi chunk.
- **Codec:** `audio_codes` int32, `audio_code_lengths` int32, state offsets/caches theo metadata; mean stereo sang mono giống upstream, clip [-1,1] sau finite check.
- **Audio:** output `PcmF32Mono::new(samples, 48_000)`. Existing `SpeechOutput` canonicalizes 24 kHz và Opus; adapter không encode Opus.
- **Streaming target:** upstream có lead-in 4 frames và adaptive frames/lead. MVP có thể port adaptive strategy sau parity của audio framing; không được batch hết utterance trước khi trả chunk đầu.

Giữ state codec xuyên các chunk trong **một utterance/phoneme chunk**, không reset ở mỗi lần `on_pcm`. Reset giữa independent chunks/operations theo reference algorithm; xử lý silence/transition không tạo tiếng click. Khi callback lỗi: dừng inference, không tiếp tục sinh rồi discard.

## 9. Cancellation, timeout, backpressure, warmup

Tôn trọng `TtsWorkerRuntime` và `SpeechOutput`:
- `cancelled` kiểm tra trước prefill, mỗi outer acoustic frame, trước graph/callback và sau output.
- ONNX call có thể không interrupt giữa chừng; cancellation safe-point ở bước tiếp theo, sau đó reset. GenerationGate ở downstream đảm bảo stale audio không phát.
- `on_pcm` đồng bộ có thể block do bounded queue: không chạy inference tiếp theo cho tới khi callback trả về; không tạo unbounded vec/channels.
- Không thêm timeout wall-clock riêng cạnh tranh với worker timeout. Provider diagnostic theo existing absolute deadline, voice streaming timeout refresh khi non-empty PCM tiến triển.
- Không retry logical synthesis khi báo lỗi hoặc timeout. Nếu ONNX session corrupted, trả error và để worker quarantine; không âm thầm tái sử dụng worker lỗi.
- `warmup()` trên retained worker dùng một câu rất ngắn và bounded frame budget, đi qua **thực tế** prefill/acoustic/codec; xác minh 48 kHz mono finite non-empty rồi reset. Không phát warmup PCM cho user; readiness không decode cả utterance dài.
- `reset()` tái khởi tạo/cắt đứt per-operation mutable state và error nếu cleanup không thành công; không giữ K/V hay codec cache của turn trước.
- Theo dõi số physical workers, sessions, estimated/RSS memory và peak initialization; giới hạn concurrency bằng cơ chế hiện hữu. Nếu overcommit RAM, fail capacity/preload theo resource manager, không OOM crash.

## 10. Voice selection, diagnostics, admin web

Preset voice mapping nên có machine-safe stable `voice.id` riêng với display name, đồng nhất trong descriptor, JSON loader và tests. Không dựa vào thứ tự map và không tùy chọn voice ngoài pinned catalog.

- `TtsBinding` là nguồn sự thật cho mỗi work request; hai Agent Templates có thể chọn hai voices khác nhau trên cùng physical model khi runtime policy cho phép reuse.
- `TtsDiagnosticRequest` được validate bằng `validate_diagnostic` và xử lý `synthesize_diagnostic` để test override voice/language tạm thời. Override không mutate model/desired DB record và không thay worker mặc định.
- Test create provider (draft, chưa lưu DB) phải chạy native provider bằng flow hiện có; test detail provider giống nhau. Chi phí prepare/model load phải bị bounded bởi runtime manager.
- Nếu admin TTS diagnostic path hiện chỉ gọi provider trực tiếp, bảo đảm nó chạy qua retained worker/isolated diagnostic mechanism phù hợp native; không tạo model instance/ONNX sessions tạm thời cho mỗi HTTP test.
- Generic web `ProviderCreateDrawer`, `ProviderConfigEditor`, `ProviderTestPanel`, `ProviderVoiceDiscovery` có thể dùng descriptor; không redesign UI. Chỉ sửa assumptions hoặc tests nếu các flow chưa hỗ trợ local adapter này.

Cần cập nhật provider registry tests, model/voice discovery, draft-test, config validation, agent-template provider binding, runtime manager status, provider materialization và system diagnosis. Không thêm migration DB nếu typed config/adapter registry hiện tại đã hỗ trợ giá trị mới.

## 11. Kế hoạch triển khai theo tracer-bullet

### Bước A — Asset + type + catalog (không infer)

File chính:
- Tạo `tts/vieneu_native/{mod,config,assets,descriptor}.rs`.
- Sửa `tts/mod.rs`, `config/providers.rs`, `factory_registry.rs`, `providers/registry.rs`, `providers/local_runtime.rs` khi cần.
- Bổ sung crate `sea-g2p-rs` pinned, frozen lockfile, assets tải theo pinned HF commit và model license notices.

**Gate A:** unit tests config/descriptor/asset fixture; catalog thấy `vieneu_native`; preset voices trùng registry; fail khi thiếu asset hoặc mismatch pinned graph. Không cần có PCM.

### Bước B — Rust phoneme + ONNX contract

File chính: `text.rs` và `runtime.rs`; thêm golden fixtures. Dùng fake/in-memory artifacts cho unit tests không cần tải model. Kiểm tra special tokens, tensor layouts, NaNs, model revision và asset paths.

**Gate B:** golden G2P/token IDs pass; ONNX contract validate được real assets; không Python runtime.

### Bước C — Inference parity qua diagnostic hiện có

File chính: `runtime.rs`, `codec.rs` và `provider.rs`. Port prefill -> autoregressive acoustic -> backbone decode_step -> MOSS decode_step. Tạo retained worker tối thiểu sử dụng cùng `TtsWorker` seam để chạy `ProviderDiagnosticService::execute_tts`; diagnostic hiện có trả WAV, nên không thêm runner/WAV exporter hoặc đường `decode_full` test-only.

**Gate C:** sinh được toàn bộ một câu qua decoder streaming (có thể gom các chunk PCM trong diagnostic); audio finite và có tiếng; seed-based logits/tokens parity, EOS/frame-cap bounded và nghe đối chiếu 10–20 câu với upstream. Chưa yêu cầu phát PCM progressive ra WebSocket.

### Bước D — Stateful streaming + worker integration

Hoàn thiện retained `TtsWorker` với progressive PCM, cancellation/reset/warmup và bounded backpressure. Tránh bọc one-shot rồi chia PCM sau synthesize.

**Gate D:** callback PCM đầu khi inference còn chạy; decoded audio total đúng thứ tự/không mất đoạn; sạch state sau cancel; reset/warmup không leak. Kết nối `SpeechOutput` -> Opus -> WebSocket bằng reference client.

### Bước E — Admin integration + benchmarks + qualification

Bổ sung test draft/detail provider, local capabilities, Voice Template selection, `provider-bench` / native worker benchmark đúng seam, update tài liệu, memory/CPU metrics. So sánh FP32 với remote VieNeu trên cùng máy và cùng corpus, ghi hardware/thread config rõ.

**Gate E:** provider có thể tạo/test/assign trong web; production flow hoạt động; regression suites pass; không phá `vieneu_http` hoặc ZeroTTS.

## 12. Test matrix bắt buộc

| Test | Gate | Điều kiện đạt |
| --- | --- | --- |
| Typed config/unknown keys | Unit | Invalid adapter/voice/language/secret/model path bị từ chối. |
| Asset pinned + atomic | Unit/integration | No download in build; missing/corrupt/incompatible fail; không partial published. |
| G2P parity | Fixture | 20–30 cases ban đầu, mở rộng theo regression; normalized phonemes/token IDs khớp oracle. |
| Embeddings parity | Fixture | Shape và giá trị gần tham chiếu trong tolerance được đo. |
| Acoustic sampling | Fixture | Seed/greedy deterministic case khớp logits/top-k/chosen codes. |
| Prefill/KV/positions | Fixture | Shapes, layer order, state update chính xác qua nhiều frames. |
| Codec step vs full | Fixture | Sample length và waveform close trong tolerance; không clicks/chunk gaps. |
| EOS/frame cap/babble | Unit/integration | Không loop vô hạn, không empty-success, timeout bound. |
| PCM contract | Unit | 48 kHz mono finite, các chunk không rỗng; callback error propagates. |
| Streaming semantics | Integration | First PCM trước terminal synth cho câu dài; audio order đúng. |
| Cancel/cleanup | Integration | Không stale audio; worker reset hoặc quarantine. |
| Multi-voice/isolation | Integration | Hai bindings khác voice không lẫn reference/cache. |
| Readiness | Integration | Bounded hot-path, clean state, no user audio from warmup. |
| Admin draft/detail test | Integration | Test khi chưa lưu và khi đã lưu; không đổi config. |
| Provider Runtime Manager | Integration | Resource key đúng revision, quotas, preload, unload, bounded workers. |
| Full voice WS | Real-model gate | Client nhận start/audio/stop đúng thứ tự; Opus decode. |
| Benchmark | Real-model gate | Có TTFA, RTF, RSS/peak và concurrency theo hardware. |

Không định nghĩa “Rust nhanh hơn Python” trước khi đo. Số liệu upstream CPU TTFA FP32 ~260–400 ms, RTF ~0.55–0.61 chỉ là tham khảo; không đặt thành ngưỡng bắt buộc cho mọi máy. Ghi rõ hardware, số threads và baseline tại Gate E.

Đề xuất chạy (từ root repo, sau khi implement):

~~~bash
cargo fmt --all -- --check
cargo test -p voice-agent-server vieneu_native
./scripts/test-module.sh tts
cargo clippy -p voice-agent-server --all-targets -- -D warnings
npm --prefix apps/admin-web run test
npm --prefix apps/admin-web run build
~~~

Các bài test real model/benchmark phải gắn nhãn opt-in, không tự tải hàng trăm MB trong unit test hoặc CI mặc định. Dùng runtime `VOICE_ONNX_RUNTIME_LIB` và scripts benchmark mà repo đang hỗ trợ; đừng tạo CLI thứ hai chỉ để duplicate command có sẵn.

## 13. Metrics và kiểm tra hiệu năng

Tái sử dụng `benchmark/provider/tts.rs` (TTFA, RTF, provider/delivery), Provider Runtime Manager và telemetry có sẵn. **Không** tạo bộ `tts_native_*` metrics song song hoặc benchmark CLI thứ hai. Chỉ bổ sung chỉ số khi các module hiện tại không thể đo được (ví dụ graph load/peak RSS của retained worker), kèm lý do trong PR.

Đo hội thoại ngắn, câu dài và nhiều voice sequential, sau đó concurrency 1 -> 2+ trên cùng cấu hình phần cứng/threads. Phân biệt provider TTFA với first-playable packet sau `SpeechOutput`. Không log user text, phonemes, reference binary hoặc credential; memory reservation dựa trên RSS/peak đo được, không suy từ kích thước file ONNX.

## 14. Tham chiếu chính

- https://github.com/pnnbao97/VieNeu-TTS/blob/main/src/vieneu/_v3_turbo_engine/onnx_runtime_lite.py
- https://github.com/pnnbao97/VieNeu-TTS/blob/main/src/vieneu_utils/phonemize_text.py
- https://github.com/pnnbao97/sea-g2p
- https://huggingface.co/pnnbao-ump/VieNeu-TTS-v3-Turbo/tree/main/onnx_update
- https://huggingface.co/OpenMOSS-Team/MOSS-Audio-Tokenizer-Nano-ONNX
- https://github.com/hailp-vn38/ai-agent-voice/blob/main/docs/flows/05-tts.md

---
**Tài liệu này là hướng dẫn triển khai, không phải bằng chứng bản native provider đã được implement hoặc benchmark.**
