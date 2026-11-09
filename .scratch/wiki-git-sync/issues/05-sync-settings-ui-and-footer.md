# 05: UI sync dùng chung trong `knowledge-ui` + adapter Tauri

Status: resolved
Blocked by: 04
Spec: ../spec.md (user stories 1, 4, 5, 6, 8, 12, 13)
Thiết kế: docs/wiki-desktop/git-sync.md#ui-dùng-chung-cho-desktop-và-admin-web

## Mục tiêu

Một bộ UI sync trong `packages/knowledge-ui` dùng được cho cả desktop và Admin Web; ticket này nối nó vào desktop.

## Phạm vi

- `packages/knowledge-ui`: kiểu `SyncStatus`/`SyncReport`/lỗi, interface `SyncBackend` (`configurable`, `status`, `syncNow`, `configure?`, `disable?`, `onStatus`), thuộc tính tùy chọn `sync?` trên `KnowledgeBackend` và `AdminKnowledgeBackend`. Không có `sync` thì UI ẩn mọi phần sync.
- Component `KnowledgeSyncPanel` (hai chế độ theo `configurable`) và huy hiệu trạng thái; chuỗi qua `i18n.ts`.
- Lỗi có kiểu → thông điệp (`Auth` kèm gợi ý `gh auth setup-git`, `NoGit` kèm gợi ý cài Xcode CLT, `UnsafeRemote`, `Deferred`).
- `Conflicted`: danh sách bản conflict, bấm để mở trang.
- Adapter Tauri trong `apps/wiki-desktop/src/tauriBackend.ts` (command ticket 04, event `sync://status`); panel ở Settings, huy hiệu ở footer.
- Adapter in-memory cho test (giống `MemoryAdminKnowledgeBackend`).

## Tiêu chí xong

- [ ] Unit test: map `SyncStatus` → nhãn/huy hiệu, map lỗi → thông điệp, panel ẩn trường cấu hình khi `configurable = false`, không có `sync` thì không render gì.
- [ ] `package-boundary.test.ts` vẫn PASS (package không import gì từ app).
- [ ] Typecheck và test frontend của `packages/knowledge-ui` và `apps/wiki-desktop` PASS.
