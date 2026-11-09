# 01: Seam `Vault::exclusive_external`

Status: ready-for-agent
Spec: ../spec.md (user stories 2, 3, 10)
Thiết kế: docs/wiki-desktop/git-sync.md#seam-mới-trên-vault

## Mục tiêu

Cho code trong `wiki-core` sửa nhiều file trên đĩa trong lúc giữ write lock, rồi báo các path đó như thay đổi bên ngoài trong một bước, để `apply` của UI/MCP không chen vào và watcher không báo lặp.

## Phạm vi

- Thêm `pub(crate) fn exclusive_external(&self, f) -> Result<Vec<DocPath>, VaultError>` vào `crates/wiki-core/src/vault/events.rs` (hoặc module phù hợp).
- Trả `ReadOnly` nếu vault không giữ flock.
- Sau khi `f` chạy xong: lọc path về `DocPath` (chỉ `sources/`, `wiki/`), ghi revision mới vào `self_writes`, sync index, phát `VaultEvent::External`.
- Nếu `f` trả lỗi: vẫn sync index cho các file có thể đã đổi, rồi trả lỗi.
- Interface công khai của `Vault` không đổi.

## Tiêu chí xong

- [ ] Test: closure ghi hai file → `read`/`search` thấy nội dung mới, `subscribe` nhận một `External` chứa đúng hai path.
- [ ] Test: vault reader gọi → `ReadOnly`, closure không chạy.
- [ ] Test: `apply` gọi từ thread khác trong lúc closure chạy phải chờ tới khi closure xong.
- [ ] `scripts/agent-test.sh` cho `wiki-core` PASS.
