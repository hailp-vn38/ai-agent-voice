# 03: `VaultSync`: xử lý xung đột

Status: ready-for-agent
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

- [ ] Test: A và B cùng sửa một trang → B giữ bản của B, có file conflict chứa bản của A, `status` là `Conflicted`.
- [ ] Test: hai bên thêm dòng vào `log.md` → đủ dòng của cả hai, không có conflict marker.
- [ ] Test: `index.md` xung đột → được sinh lại, không có conflict marker.
- [ ] Test: A xóa trang, B sửa trang → trang còn, nội dung của B.
- [ ] Test: sau khi xóa file conflict và sync lại → `Idle`.
- [ ] `git status --porcelain` sạch sau mọi test.
- [ ] `scripts/agent-test.sh` cho `wiki-core` PASS.
