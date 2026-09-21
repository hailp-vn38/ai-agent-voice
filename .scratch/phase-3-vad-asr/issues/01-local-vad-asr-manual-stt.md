# 01: Nền tảng local VAD/ASR và Manual STT

**What to build:** Voice Protocol Client có thể thực hiện một lượt Manual hoàn chỉnh: server chỉ nhận traffic sau khi local provider và model đã sẵn sàng; audio Opus được nhận dạng thành final STT V1, lưu user utterance trong lịch sử hội thoại có giới hạn và quay về trạng thái sẵn sàng. Test có thể thay local inference bằng provider giả tại cùng boundary ứng dụng.

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] Khởi động fail trước khi bind nếu config, artifact hoặc warmup provider local không hợp lệ; application boundary hỗ trợ deterministic fake VAD/ASR cho test.
- [x] Manual `listen:start` → audio → `listen:stop` tạo tối đa một `type:"stt"` cho final khác rỗng, sau commit Dialogue History bounded; empty hoặc failed final không gửi STT và không commit. Stale worker events thuộc ticket pool/generation kế tiếp; ticket này không tạo worker event bất đồng bộ.
- [x] Partial ASR không được gửi, lưu hoặc dùng làm dialogue input; các kiểm thử contract và session xác nhận final ordering và terminal Ready.

## Comments

- Implemented local Silero/Zipformer construction and stream warmup in `ProviderSet::load`; `application` constructs it before `TcpListener::bind`.
- Added deterministic `AsrProvider` injection for SessionActor tests. Manual opens/pushes a streaming ASR session, commits only a non-empty final into bounded RAM-only Dialogue History, then enqueues one existing `stt` payload.
- Verification: `cargo test --workspace`, `cargo clippy -p voice-agent-server --tests -- -D warnings`, and `git diff --check` passed. The real-model WAV smoke remains ignored because model artifacts are absent locally.
