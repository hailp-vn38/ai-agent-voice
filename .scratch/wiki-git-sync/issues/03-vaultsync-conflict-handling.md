# 03: `VaultSync`: xử lý xung đột

Status: resolved
Blocked by: 02
Spec: ../spec.md (user story 6)
Thiết kế: docs/wiki-desktop/git-sync.md#xung-đột

## Mục tiêu

Merge xung đột không bao giờ để repo ở trạng thái dở và không làm mất nội dung.

## Phạm vi

- `wiki/log.md`: dựa vào `merge=union` trong `.gitattributes`.
- `wiki/index.md`: lấy bản local, sau merge chạy `Change::Reindex { force: false }` để sinh lại.
- Trang khác: giữ bản local, ghi bản remote thành `<tên> (conflict <YYYY-MM-DD> <host>).md` cạnh file gốc.
- Một bên xóa, một bên sửa: giữ bản đã sửa.
- Commit merge, trả `SyncStatus::Conflicted { copies }`. Lần sync sau không còn file conflict nào thì về `Idle`.
- Nếu bất kỳ bước nào lỗi giữa chừng: `git merge --abort`, trả lỗi, repo sạch.

## Tiêu chí xong

- [x] Test: A và B cùng sửa một trang → B giữ bản của B, có file conflict chứa bản của A, `status` là `Conflicted`.
- [x] Test: hai bên thêm dòng vào `log.md` → đủ dòng của cả hai, không có conflict marker.
- [x] Test: `index.md` xung đột → được sinh lại, không có conflict marker.
- [x] Test: A xóa trang, B sửa trang → trang còn, nội dung của B.
- [x] Test: sau khi xóa file conflict và sync lại → `Idle`.
- [x] `git status --porcelain` sạch sau mọi test.
- [x] `scripts/agent-test.sh` cho `wiki-core` PASS.

Kiểm tra: `scripts/agent-test.sh targeted conflict wiki-core` PASS (4 test); `scripts/agent-test.sh package wiki-core` PASS (31 test; lượt đầu có một lỗi lock test không tái hiện ở lượt hai); `cargo fmt --manifest-path crates/wiki-core/Cargo.toml -- --check` và `git diff --check` PASS qua wrapper.
