# 02: Vault: ghi trọn vẹn, Knowledge Revision và sự kiện

Status: ready-for-agent
Blocked by: 01
Spec: ../spec.md (user stories 8, 22–30, 41)

## Mục tiêu

Thêm hàm ghi duy nhất `apply(change)` / `apply_from(origin, change)` và `subscribe()`.

## Phạm vi

- Enum `Change` gồm `Write`, `Patch`, `Move`, `Delete`, `ImportRaw`, `IngestSource` (không dùng LLM), `Reindex`.
- Ghi trọn vẹn: ghi file tạm rồi rename, sau đó đồng bộ Search Projection và Graph Projection, cập nhật `index.md`/`log.md`, cuối cùng phát sự kiện. Nếu lỗi thì không để lại trạng thái dở dang nhìn thấy được.
- Write lock trong process. Đọc không chờ lock.
- Knowledge Revision là hash nội dung. `expect` không khớp thì trả `Conflict { current }` và file giữ nguyên.
- `Move` chặn hoặc báo lỗi khi làm gãy Wiki Link, giống hành vi hiện tại của server.
- `VaultEvent::{Changed { paths, origin }, Reindexed { stats }}` qua broadcast channel.
- Ghi lại tập "path vừa ghi kèm hash" để ticket 03 lọc sự kiện của watcher.

## Tiêu chí xong

- [ ] Mỗi loại `Change` có test: ghi xong thì `read`/`search` thấy kết quả và subscriber nhận đúng `Changed` với đúng `origin`.
- [ ] `expect` sai revision → `Conflict`, nội dung trên đĩa không đổi.
- [ ] Hai `apply` chạy đồng thời trên cùng file đều kết thúc nhất quán: một cái thành công, cái còn lại thành công hoặc trả `Conflict`.
- [ ] Mô phỏng lỗi giữa chừng (ví dụ đường dẫn đích không ghi được) không để lại file tạm hay index lệch.
- [ ] Các test ghi lấy từ test knowledge MCP hiện có pass trên `Vault`.
