# 03: Vault: một writer giữa các process và thay đổi bên ngoài

Status: ready-for-agent
Blocked by: 02
Spec: ../spec.md (user stories 31–33, 44)

## Mục tiêu

Bảo đảm chỉ một process ghi kho tri thức, và `Vault` nhận được thay đổi do công cụ khác tạo ra.

## Phạm vi

- `open` lấy advisory `flock` trên lock file trong kho:
  - lấy được lock thì có quyền ghi;
  - không lấy được thì mở chỉ đọc, và `apply` trả `VaultError::ReadOnly`.
- `Vault` có cách thử lấy lại quyền ghi (ví dụ `try_acquire_write()`), và phát sự kiện khi trạng thái ghi thay đổi.
- `notice_external(paths)`: đồng bộ index cho các path đó và phát `VaultEvent::External`. Bỏ qua path nằm trong tập "vừa ghi kèm hash" nếu hash vẫn khớp.

## Tiêu chí xong

- [ ] Hai `Vault` trên cùng thư mục (mô phỏng hai process): cái thứ hai bị `ReadOnly` khi ghi. Cái thứ nhất đóng thì cái thứ hai lấy được quyền ghi.
- [ ] Test có spawn process con giữ lock để chứng minh `flock` hoạt động giữa các process.
- [ ] Sửa file trực tiếp trên đĩa rồi gọi `notice_external` → `search` thấy nội dung mới và nhận được `External`.
- [ ] Path vừa được `apply` ghi xong rồi gọi `notice_external` → không phát `External`.
