# Cấu hình `[memory]` và module `MemoryCore`

Type: task
Status: ready-for-agent
Blocked by: 01

## Scope

Mốc 2 của [spec](../spec.md), bước 1.1 đến 1.5 trong [hướng dẫn](../../../docs/memory-core-implementation-guide.md). Chưa ai gọi module này.

- `[memory]` trong cấu hình, validation, `config.example.toml`, dependency `serde_yaml`.
- `services/memory/` với interface, kiểu dữ liệu, mã lỗi, thứ tự ghi, Forget Tombstone, `reconcile`, `reconcile_all`, Memory Summary theo spec. Gồm cả `intent.rs` (`classify_intent`) vì nó nằm trong interface công khai; đấu nối vào actor thuộc ticket 07.
- `#[cfg(test)] FailPoint` nội bộ để test crash.

User stories: 9 đến 13, 23 đến 25, 34 đến 38, 40, 41, 44, 45, 60, 63 đến 65, 67, 75 đến 77, 79, 80.

## Acceptance

- **Test đầu tiên:** `sqlx` tạo được bảng FTS5 với tokenizer đã chọn. Nếu không, dùng phương án dự phòng của spec.
- Mọi dòng mốc 2 trong ma trận test của spec đều có test qua và đi qua interface `MemoryCore`, trên thư mục tạm thật.
- Test trực tiếp các hàm thuần `normalize`, `fit_summary`, `guard`, `classify_intent`. Bảng cổng ý định của spec được phủ, kể cả `don't forget`.
- Interface chỉ lộ những gì spec liệt kê.
- `scripts/agent-test.sh targeted memory voice-agent-server` xanh. `[memory].enabled` mặc định `false`, Device hiện có không đổi hành vi.
