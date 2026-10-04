# Provider config và voice preparation: contract sau refactor

> **Ghi chú:** tài liệu này ghi lại trạng thái tại thời điểm nó được viết. Cơ chế Model
> Preparation, `ResolvedModel`, `prepare_immutable()` và `deployment.models.*` đã bị thay thế
> bởi [ADR 0076](adr/0076-provider-owned-model-assets.md). Đọc ADR đó cho kiến trúc hiện hành.

## Phạm vi đã chốt

Triển khai theo `Provider Descriptor, Runtime Config & Voice Model Preparation Refactor Guide.md`.
Theo xác nhận ngày 2026-10-03, Gipformer chỉ có model từ
`g-group-ai-lab/gipformer1.5-68M-rnnt`; giữ identity `gipformer15_vi_int8`
và revision hiện tại, không thêm hai identity FP32/INT8 của phase 4 trong guide.
Vì chỉ có một model, descriptor/JSON Gipformer cũng không chứa `model`; server tự resolve identity.

## Cấu hình server

`runtime.onnx.threads` là map theo adapter, authoritative cho tất cả local factories
và Runtime Resource Key. Ví dụ:

```toml
[runtime.onnx.threads]
silero_onnx = 1
zipformer_sherpa = 2
gipformer_sherpa_offline = 4
zerotts_onnx = 2
kokoro_vi_onnx = 2

[runtime.chillaudio]
# Bỏ ws_url để dùng endpoint mặc định, hoặc cấu hình endpoint deployment.
# ws_url = "wss://your-deployment-chillaudio-endpoint.example/ws"
timeout_ms = 12000
```

Threads chỉ hợp lệ trong 1..=128. Map thiếu một adapter dùng default của adapter đó;
key không phải local adapter bị reject. ChillAudio yêu cầu WSS và timeout 1..=120000 ms.
Debug redact URL vì query có thể chứa credential.

Các provider structs còn nhận field TOML legacy để tương thích deployment cũ;
factory luôn thay threads, fixed model và ChillAudio endpoint/timeout bằng giá trị
server-owned. Các field này không serialize vào deployment snapshot. Nên chuyển
các giá trị deployment cũ vào section runtime trước khi nâng binary.

Admin POST/PATCH reject từng field internal với `400 provider_config_invalid`,
đúng ADR-0062. JSON mới:

- Silero: `{}`.
- Zipformer: `{"decoding_method":"greedy_search"}`.
- Gipformer: `{"decoding_method":"modified_beam_search","max_active_paths":4}`.
- ZeroTTS: `{"voice":"maichi","language":"vi-VN","delivery_mode":"stream"}`.
- Kokoro: `{"voice":"duc_an","language":"vi-VN","speed_percent":100}`.
- ChillAudio: `{"voice":"BV421_vivn_streaming","language":"vi"}`.

Migration 0006 loại đúng field internal đã biết, giữ user selection và credential reference,
tăng desired revision khi thay đổi. Field không hợp lệ khác vẫn bị validator từ chối.
Không normalize tùy tiện JSON mới gửi qua API.

## Artifact contract

ZeroTTS: revision `c2bfbd67dc648cac455077333f7cf5c18a2e3bb4`, đủ tám role `voice_<id>`.
Factory resolve đúng selected role; thiếu artifact hoặc voice lạ đều fail, không fallback.
Diagnostic chỉ chấp nhận voice của runtime đã materialize.

Kokoro: revision `9f210d622209fcc216fe2ac6159fed2ff381cb8a`, đủ 14 role `voicepack_<id>`.
Source `.pt` và output `.bin` đều có checksum SHA-256 riêng trong manifest.
`kokoro_voicepack_v1` chỉ đọc ZIP theo layout pinned: metadata tensor 510×1×256
float32 contiguous, offset 0, little endian, stride 256/256/1. Metadata `data.pkl`
được so byte-for-byte với fixture upstream (SHA-256
`645dea33b46e50fdda6c8a43b39ec4b7adcf28e4643408c02d74fef10ff9d712`).
Không interpret pickle, gọi PyTorch hay shell-out. Archive, entry count, entry sizes,
layout và finite values được kiểm tra trước khi xuất `KOVI_VOICEPACK_V1`.
Một layout upstream khác cần version transform mới.

Startup, Runtime Manager preparation và materialization dùng ModelPreparation chung.
FactoryMaterializer build tuân theo `deployment.models.offline`; offline fail khi artifact
thiếu/corrupt. Preparation giữ locking, source/output verification, temporary files và atomic
install. Không có downloader/converter trong synthesis hoặc capability inspector.
Static voice catalogs được kiểm tra đối chiếu với manifest.

## Verification

Automated gates: Admin create/PATCH và migration readback; descriptor/catalog consistency;
ModelPreparation reuse/acquire/corruption/offline/transform failure/output checksum/atomicity;
selected voice thiếu artifact không fallback; resource key phân biệt server threads.

Các gate offline opt-in được chạy với file thật:

```sh
hf download contextboxai/Kokoro-Vietnamese --revision 9f210d622209fcc216fe2ac6159fed2ff381cb8a --include 'voicepacks/*' --local-dir /tmp/provider-kokoro-source
hf download zeroweight-ai/ZeroTTS --revision c2bfbd67dc648cac455077333f7cf5c18a2e3bb4 --include 'voices/*/voice.npz' --local-dir /tmp/provider-zerotts-source

KOKORO_PREPARATION_FIXTURE_DIR=/tmp/provider-kokoro-source PROVIDER_QUALIFICATION_CONFIG="$PWD/config.toml" cargo test --test model_preparation pinned_kokoro_sources_prepare -- --ignored
ZEROTTS_PREPARATION_FIXTURE_DIR=/tmp/provider-zerotts-source PROVIDER_QUALIFICATION_CONFIG="$PWD/config.toml" cargo test --test zerotts_artifact_preflight every_pinned_zerotts_voice -- --ignored
PROVIDER_QUALIFICATION_CONFIG="$PWD/config.toml" cargo test --test provider_registry native_transducers_accept -- --ignored
```

Gate TTS kiểm preparation/checksum và factory contract của từng voice, không phải kiểm âm thanh
synthesis của từng voice. Gate ASR tạo recognizer thật và decode silence với cả hai enum modes.
Không chạy live ChillAudio, Voice WS E2E hoặc hardware gate.

Nếu deployment có `provider_runtime.measured_manifest_sha256`, manifest mới làm receipt cũ
không còn hợp lệ. Cần đo lại qualification và tạo receipt mới trước khi chạy Runtime Manager;
không sửa checksum receipt chỉ để vượt gate.

Kết quả validation: `cargo fmt --check`, `git diff --check`,
`cargo check --workspace --all-targets --all-features` pass;
`cargo test --workspace`: 507 passed, 5 ignored, 0 failed.
Ba gate offline opt-in ở trên đã được chạy riêng và pass.
