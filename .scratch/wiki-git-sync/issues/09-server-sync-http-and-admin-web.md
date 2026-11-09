# 09: Server HTTP sync + adapter Admin Web

Status: ready-for-agent
Blocked by: 05, 08
Spec: ../spec.md (user stories 12, 13, 16)
Thiết kế: docs/wiki-desktop/git-sync.md#http-admin-cần-token-admin-như-các-route-knowledge-khác

## Mục tiêu

Admin xem trạng thái và bấm "Sync now" trên trang Knowledge của Admin Web.

## Phạm vi

- `GET /api/admin/knowledge/sync`: `SyncStatus` + cấu hình đã che (remote, branch, interval; không lộ đường dẫn khóa). Sync tắt → `{ status: Disabled { NotConfigured } }`.
- `POST /api/admin/knowledge/sync`: `sync_now`, trả `SyncReport`; đang sync → `409 sync_in_progress`.
- Lỗi theo kiểu admin hiện có (`sync_auth`, `sync_network`, `sync_unsafe_remote`, `sync_disabled`, …), không chứa stderr thô.
- SSE `/knowledge/events` phát thêm event `sync_status`.
- Đăng ký route trong `app/admin/router.rs`, cùng guard admin với các route Knowledge.
- `apps/admin-web`: adapter `SyncBackend` HTTP (`configurable = false`) gắn vào `knowledgeBackend.ts`; nghe `sync_status` trên listener SSE sẵn có (`api/knowledge-updates.ts`); huy hiệu ở thanh công cụ trang Knowledge mở `KnowledgeSyncPanel`.
- Tài liệu API trong `docs/api/` và luồng Knowledge trong `docs/flows/`.

## Tiêu chí xong

- [ ] Test route: không có token admin → 401; sync tắt → trạng thái `NotConfigured`; `POST` song song → một cái `409`.
- [ ] Test SSE phát `sync_status`.
- [ ] Test admin-web: adapter map đúng route/lỗi; huy hiệu cập nhật khi nhận `sync_status`.
- [ ] `scripts/agent-test.sh` PASS cho target liên quan; typecheck và test `apps/admin-web` PASS.
