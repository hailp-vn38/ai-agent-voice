# 08: Server: cấu hình `[wiki.sync]`, lịch sync và hoãn khi ingest

Status: ready-for-agent
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

- [ ] Test config: thiếu mục `[wiki.sync]` vẫn đọc được; URL có credential, thiếu khóa, khóa quyền 0644 đều bị từ chối.
- [ ] Test: có job ingest đang chạy → `Deferred`, job xong → lần sau sync chạy.
- [ ] Test với bare repo: ghi qua route admin → sau debounce có commit trên bare repo.
- [ ] `scripts/agent-test.sh` PASS cho target liên quan, rồi workspace.
