# 08: Server: cấu hình `[wiki.sync]`, lịch sync và hoãn khi ingest

Status: resolved
Blocked by: 02, 03, 07
Spec: ../spec.md (user stories 11, 15, 16)
Thiết kế: docs/wiki-desktop/git-sync.md#cấu-hình-configtoml

## Mục tiêu

Server tự đồng bộ vault theo cấu hình, không giẫm lên request ghi hay job AI ingest.

## Phạm vi

- `WikiSyncConfig` trong `config/orchestrator_wiki.rs` (`#[serde(default)]`, `enabled = false`), map sang `SyncConfig` với `GitAuth::SshKey`.
- `config/validation.rs`: URL không chứa credential; khi `enabled` thì khóa và `known_hosts` tồn tại; khóa không cho group/other đọc.
- Một service sync trong `AppState` bọc `VaultSync`: `sync_now` chạy trong write guard của `KnowledgeWriteGate`, chạy `spawn_blocking`.
- Hoãn khi có job ingest Knowledge đang chạy (`Deferred { IngestRunning }`, thử lại sau 30 s, tối đa 10 phút rồi vẫn sync và log cảnh báo). Tìm truy vấn job đang chạy sẵn có, không có thì thêm.
- Lịch: lúc khởi động, debounce `debounce_sec` sau lần ghi cuối, định kỳ `interval_min`; dừng gọn khi server tắt (theo cơ chế shutdown hiện có).
- Không giữ lock vault (reader) → `Disabled { ReadOnly }`, không chạy lịch.
- Cập nhật `config.example.toml` và `docker/config.prod.example.toml`.

## Tiêu chí xong

- [x] Test config: thiếu mục `[wiki.sync]` vẫn đọc được; URL có credential, thiếu khóa, khóa quyền 0644 đều bị từ chối.
- [x] Test: có job ingest đang chạy → `Deferred`, job xong → lần sau sync chạy.
- [x] Test với bare repo: ghi qua route Knowledge sau debounce có commit trên bare repo.
- [ ] `scripts/agent-test.sh` PASS cho target liên quan, rồi workspace. Target liên quan PASS; package gate dừng ở `vieneu_default` thiếu trong ignored `config.toml`, xem `Comments`.

## Comments

- Implemented config compatibility/validation, SSH `VaultSync` mapping, startup and periodic/debounced scheduler, the Knowledge write gate, bounded ingest deferral, read-only scheduler suppression, and example config sections.
- `scripts/agent-test.sh targeted wiki_sync vela-server`: PASS (5 tests); focused MCP read-only, MCP HTTP compatibility, and Knowledge Admin search regressions: PASS; `scripts/agent-test.sh check vela-server`: PASS; scoped rustfmt and `git diff --check`: PASS.
- `scripts/agent-test.sh package vela-server` ran for 481 seconds and failed only at `tests/vieneu.rs:66/72`: the ignored local `config.toml` lacks the expected `vieneu_default` entry. The failure is machine-config dependent and outside this ticket. The temporary config copy used to diagnose it was removed. The workspace gate also retains the unrelated protocol TTS failure recorded by ticket 07.
## Review follow-up

- Active-ingest query errors are treated as unknown/active and deferred; the existing ten-minute maximum wait still permits sync to proceed with a warning.
