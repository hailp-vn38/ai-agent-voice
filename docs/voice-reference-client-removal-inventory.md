# Kiểm kê và kết quả xoá `voice-reference-client`

Ngày kiểm kê và thực hiện: 2026-10-03. Phạm vi kiểm kê là toàn bộ file **đã được Git theo dõi**; truy vấn dùng cả `voice-reference-client`, `voice_reference_client`, `reference client` và `reference-client` (không phân biệt hoa/thường).

**Trạng thái: đã xoá.** Crate không còn là workspace member hoặc dependency; `Cargo.lock` đã được Cargo tái tạo. Các fixture Phase 5 được chuyển vào `crates/voice-agent-server/tests/fixtures/`; gate Phase 5/6 phụ thuộc toàn bộ client crate đã retire.

## Tóm tắt

- Có **24 file thuộc chính crate**: 17 file cấu hình/mã nguồn, 2 integration test Rust và 5 fixture Opus.
- Có **10 file ngoài crate có phụ thuộc cứng**: workspace/lockfile, dev-dependency của server, 5 server gate/test, preflight Phase 5 và shell gate Phase 4.
- Có **39 file ngoài crate chỉ có tham chiếu ngữ nghĩa/tài liệu**: 2 file gốc, 4 server test/source chỉ dùng tên `reference-client` hoặc mô tả, 17 tài liệu và 16 ticket/spec lịch sử dưới `.scratch/`.
- Tổng cộng có **73 file tracked** khớp truy vấn (24 + 49). Con số này là inventory tham chiếu, không phải số file nên xoá; các tài liệu lịch sử cần quyết định lưu/đánh dấu obsolete thay vì xoá mặc định.

## Các phụ thuộc cứng đã xử lý khi xoá crate

| File | Phụ thuộc | Hướng xử lý khi xoá |
| --- | --- | --- |
| `Cargo.toml` | Workspace member `crates/voice-reference-client` | Bỏ member. |
| `Cargo.lock` | Package và dependency edge của crate | Để Cargo tái sinh sau khi đã bỏ các dependency khác. Không sửa tay. |
| `crates/voice-agent-server/Cargo.toml` | `dev-dependency` path tới crate | Bỏ dependency hoặc thay bằng test helper độc lập. |
| `crates/voice-agent-server/tests/phase4_reference_gate.rs` | `decode_canonical_downlink_opus_packet` | Chuyển decoder assertion vào test helper/server test hoặc thay bằng một client độc lập khác. |
| `crates/voice-agent-server/tests/phase5_reference_gate.rs` | `run_barge_in`, kiểu Barge-in, và 4 fixture Opus | Tách client scenario và fixture ra ngoài crate trước; nếu bỏ gate thì cập nhật rõ acceptance evidence Phase 5. |
| `crates/voice-agent-server/tests/phase6_reference_gate.rs` | `ReferenceClient` và `McpSessionOptions` | Thay thế/bỏ deterministic Device MCP completion gate; đây là thay đổi acceptance contract Phase 6. |
| `crates/voice-agent-server/tests/speechoutput_tracer.rs` | `run_text_turn`, request/config text turn | Thay test public-boundary bằng helper/client khác hoặc viết lại assertion ở transport boundary. |
| `crates/voice-agent-server/tests/vision_api.rs` | `run_vision_request`, `VisionRequestOptions` | Đã thay bằng multipart HTTP trực tiếp trong test API hiện có. |
| `crates/voice-agent-server/src/bin/phase5-offline-preflight.rs` | 5 đường dẫn fixture dưới crate | Di chuyển fixture tới vị trí thuộc server/assets hoặc bỏ kiểm tra fixture. |
| `scripts/test-phase4-reference-gate.sh` | Binary `decode-downlink-opus` | Thay bằng decoder/check mới hoặc bỏ bước độc lập khỏi script. |

Ghi chú: `phase4_reference_gate` từng được feature-gate `real-model-gate`; `phase5_reference_gate` và `phase6_reference_gate` đã retire cùng client crate. `speechoutput_tracer` và `vision_api` vẫn là test thường, không còn import client crate.

## Nội dung đã bị xoá cùng crate (24 file)

### Cấu hình và mã nguồn (17)

- `crates/voice-reference-client/Cargo.toml`
- `crates/voice-reference-client/src/lib.rs`
- `crates/voice-reference-client/src/main.rs`
- `crates/voice-reference-client/src/chillaudio.rs`
- `crates/voice-reference-client/src/scenario.rs`
- `crates/voice-reference-client/src/admin/mod.rs`
- `crates/voice-reference-client/src/admin/models.rs`
- `crates/voice-reference-client/src/bin/audio-turn.rs`
- `crates/voice-reference-client/src/bin/chillaudio-tts.rs`
- `crates/voice-reference-client/src/bin/decode-downlink-opus.rs`
- `crates/voice-reference-client/src/bin/generate-phase5-uplink-fixtures.rs`
- `crates/voice-reference-client/src/bin/vision.rs`
- `crates/voice-reference-client/src/bin/zerotts.rs`
- `crates/voice-reference-client/src/bin/zerotts/codec.rs`
- `crates/voice-reference-client/src/bin/zerotts/contract.rs`
- `crates/voice-reference-client/src/bin/zerotts/synthesis.rs`
- `crates/voice-reference-client/src/bin/zerotts/text.rs`

### Test và fixture (7)

- `crates/voice-reference-client/tests/admin_contract.rs`
- `crates/voice-reference-client/tests/scenario_contract.rs`
- `crates/voice-reference-client/tests/fixtures/phase5-uplink-01-silence.opus`
- `crates/voice-reference-client/tests/fixtures/phase5-uplink-02-speech-a.opus`
- `crates/voice-reference-client/tests/fixtures/phase5-uplink-03-silence.opus`
- `crates/voice-reference-client/tests/fixtures/phase5-uplink-04-speech-b.opus`
- `crates/voice-reference-client/tests/fixtures/phase5-uplink-05-silence.opus`

## Tham chiếu mã nguồn không tạo dependency Cargo (4)

Những file dưới đây không import crate; chúng dùng device ID `reference-client-01` hoặc câu mô tả. Chỉ đổi khi muốn xoá luôn thuật ngữ/reference profile, không cần thiết để Cargo build lại.

- `crates/voice-agent-server/src/session/speech_output/tests.rs`
- `crates/voice-agent-server/tests/protocol_e2e.rs`
- `crates/voice-agent-server/tests/support/mod.rs`
- `crates/voice-agent-server/tests/ws_protocol.rs`

## Tài liệu vận hành và kiến trúc (17 file dưới `docs/`, cộng `README.md` và `CONTEXT.md`)

### Cần sửa/xoá liên kết hoặc lệnh chạy

- `README.md` — lệnh `cargo run -p voice-reference-client` và link tới tài liệu crate.
- `docs/voice-reference-client.md` — tài liệu vận hành chuyên biệt của crate; ứng viên xoá.
- `docs/testing/01-ws.md` — link tới `voice-reference-client.md`.
- `docs/testing/README.md` — link tới `voice-reference-client.md`.
- `docs/PHASE6_DEVICE_MCP_IMPLEMENTATION_GUIDE.md` — mô tả/lệnh crate là Device MCP server.
- `docs/rust-reference-client-database-integration-guide.md` — design guide cho việc mở rộng crate, bao gồm path và wire DTO.
- `docs/performance_tester_rust_guide.md` — chỉ dẫn/path dự kiến cho binary trong crate.

### Cần rà soát ngữ nghĩa gate/contract (không nên xoá máy móc)

- `CONTEXT.md` — định nghĩa `Reference Client`, wire type và MCP gate.
- `docs/00-overview.md`
- `docs/04a-prompt-composition.md`
- `docs/06-implementation-plan.md`
- `docs/PHASE5_XIAOZHI_BARGE_IN_RUST_UPDATE_GUIDE.md`
- `docs/adr/0012-v1-release-gates.md`
- `docs/adr/0035-uplink-decoder-lifetime-is-the-voice-session.md`
- `docs/adr/0045-client-aec-acoustic-barge-in.md`
- `docs/provider-runtime-manager-three-phase-guide.md`
- `docs/testing/00-test-strategy.md`
- `docs/testing/05-tts.md`
- `docs/testing/06-e2e.md`

Các mục nhóm thứ hai ghi nhận quyết định/gate đã có. Nếu chỉ bỏ implementation crate nhưng vẫn giữ yêu cầu về Voice Protocol Client độc lập, hãy đổi tên công cụ/gate thay vì xoá nội dung lịch sử.

## Ticket/spec lịch sử dưới `.scratch/` (16)

Không có file nào trong nhóm này là input build. Giữ chúng làm lịch sử, rồi thêm ghi chú superseded/obsolete nếu quyết định xoá làm thay đổi gate đã chốt.

- `.scratch/phase-3-vad-asr/spec.md`
- `.scratch/phase-3-vad-asr/issues/02-worker-runtime-vad-asr-acknowledgement.md`
- `.scratch/phase-3-vad-asr/issues/04-protocol-compatibility-e2e-verification.md`
- `.scratch/phase-3-vad-asr/issues/05-vad-inference-segmentation-correctness.md`
- `.scratch/phase-3-vad-asr/issues/06-model-preparation-lifecycle.md`
- `.scratch/phase-3-vad-asr/issues/07-compile-time-provider-registry.md`
- `.scratch/phase-4-llm-zerotts/spec.md`
- `.scratch/phase-4-llm-zerotts/issues/01-typed-provider-foundation.md`
- `.scratch/phase-4-llm-zerotts/issues/06-zerotts-synthesis-core.md`
- `.scratch/phase-4-llm-zerotts/issues/08-reference-client-completion-gate.md`
- `.scratch/phase-4-llm-zerotts/issues/09-pyxiaozhi-text-turn-compatibility.md`
- `.scratch/phase-5-interruption-barge-in/spec.md`
- `.scratch/phase-5-interruption-barge-in/issues/01-offline-model-preflight-and-opus-fixtures.md`
- `.scratch/phase-5-interruption-barge-in/issues/06-acoustic-barge-in-session-flow.md`
- `.scratch/phase-5-interruption-barge-in/issues/07-rust-reference-client-barge-in-completion-gate.md`
- `.scratch/sqlite-agent-template-database-integration/spec.md`

## Thứ tự xoá đã thực hiện

1. Chốt việc thay hay bỏ bốn public qualification flows: Phase 4 downlink decode, Phase 5 barge-in, Phase 6 Device MCP và Vision/text-turn public client.
2. Di chuyển hoặc retire 5 fixture Phase 5, sau đó sửa preflight và các test/gate phụ thuộc.
3. Bỏ `voice-reference-client` khỏi `crates/voice-agent-server` dev-dependencies; sửa/chuyển các test để `cargo test --workspace` không còn import `voice_reference_client`.
4. Bỏ workspace member, xoá thư mục crate, rồi để Cargo cập nhật `Cargo.lock`.
5. Cập nhật README/tài liệu vận hành; đánh dấu các ADR, plan và ticket lịch sử là superseded khi contract qualification tương ứng thực sự bị thay.
6. Xác minh tối thiểu: `cargo fmt --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, và `git diff --check`.

## Giới hạn kiểm kê

- Không quét file untracked, ignored hoặc artifact build tại thời điểm kiểm kê ban đầu.
- Sau khi xoá, truy vấn không còn trả về tên package trong mã và tài liệu hiện hành; report này cùng ticket/spec lịch sử được loại trừ vì chúng là bằng chứng lịch sử.
