# 09: Tauri shell: soạn, xung đột và sự kiện

Status: ready-for-human
Blocked by: 02, 08
Spec: ../spec.md (user stories 8, 22–30)

## Mục tiêu

Cho phép ghi từ app qua `vault_apply`, và UI cập nhật theo sự kiện của `Vault`.

## Phạm vi

- Command `vault_apply(change)` gọi `Vault::apply_from(Origin::Ui, …)`.
- Chuyển `VaultEvent` thành event `vault://changed`. `TauriBackend.onChange` lắng nghe event này.
- UI: tạo trang `⌘N`, Compose/lưu, kéo thả để di chuyển hoặc đổi tên, xóa có xác nhận, import nhiều file vào `raw`, `IngestSource`, nút Rebuild index.
- Xung đột: lưu với `expect` = revision lúc mở trang. Gặp `Conflict` thì hiện hộp thoại "Giữ bản của tôi / Tải bản mới".
- Mở hai cửa sổ cùng một trang: lưu ở cửa sổ này thì cửa sổ kia tự cập nhật.

## Tiêu chí xong

- [ ] Test của package (trên `MemoryBackend`) cho luồng xung đột: lưu bị `Conflict` thì hiện lựa chọn, và chọn mỗi phương án cho ra đúng kết quả.
- [ ] Smoke test thủ công cho từng loại `Change` trên app thật.

## Cập nhật tiến độ

- Đã nối `vault_apply`/`Origin::Ui`, event relay, tạo/sửa trang, xung đột với hai lựa chọn, xóa, ingest, rebuild và kéo thả.
- PASS: test conflict/keep/reload và tạo Wiki Page qua MemoryBackend; wiki-core 17 tests; desktop `cargo check`/format/build.
- NOT_RUN: smoke test thủ công từng loại `Change` và đồng bộ giữa hai cửa sổ trên app thật.

## Comments

### Review 2026-11-08 (`/code-review`, diff `3298b791...HEAD`). Các mục còn mở cần sửa trước khi nghiệm thu thủ công.

- [x] Cửa sổ phụ không nhận sự kiện: ⌘⇧N tạo cửa sổ label `wiki-N` (`App.vue:144-145`) nhưng capability chỉ áp cho `"windows": ["main"]`. `listen('vault://changed')` sẽ bị từ chối, nên tiêu chí "mở hai cửa sổ… cửa sổ kia tự cập nhật" nhiều khả năng fail. Đổi thành `["main", "wiki-*"]` và thêm kiểm tra.
- [x] Docs: `desktop-shell.md` viết `origin = Mcp`, còn `types.rs` serialize thành `"mcp"`. Đồng bộ lại.

### Sửa sau review (2026-11-08)

- ⌘⇧N gọi command `window_new`, tạo cửa sổ `wiki-<uuid>`; capability áp cho `wiki-*` nên cửa sổ phụ nghe được `vault://changed`/`vault://lock`.
- `desktop-shell.md` ghi `origin` là `"mcp"`.
- NOT_RUN: smoke hai cửa sổ cùng một trang trên app thật.
