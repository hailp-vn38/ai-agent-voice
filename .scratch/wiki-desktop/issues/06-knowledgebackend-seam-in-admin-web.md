# 06: Seam KnowledgeBackend trong Admin Web

Status: resolved
Blocked by: none
Spec: ../spec.md (user stories 49)

## Mục tiêu

Gom mọi lệnh gọi dữ liệu của trang Knowledge về sau một interface `KnowledgeBackend`, inject qua provide/inject. Không đổi giao diện. Ticket này làm song song được với 01–05.

## Phạm vi

- Interface `KnowledgeBackend`: `tree`, `read`, `search`, `graph`, `status`, `apply`, `onChange`, `capabilities { chat }`. `Change`/`VaultError` dùng cùng vốn từ với `Vault`.
- `HttpBackend`: bọc các route `/api/admin/knowledge/*` hiện có. `onChange` dùng cơ chế notification sẵn có hoặc polling.
- `MemoryBackend`: dùng cho test.
- `useKnowledgeWorkspace` và các component Knowledge lấy backend qua `inject`, không gọi `fetch` trực tiếp nữa.
- Tab Wiki Chat ẩn khi `capabilities.chat = false`. Admin Web luôn đặt `true`.

## Tiêu chí xong

- [x] Test Knowledge workspace (`KnowledgePage`, `KnowledgeGraph`, `RightPanelTabs`, …) dùng `MemoryBackend` cho tree/read/search/graph và hành vi workspace. HTTP mocks chỉ còn ở kiểm tra Admin-specific như chat/SSE, hội thoại, CRUD/import/upload và lỗi route.
- [x] Một bộ contract test chạy cho cả `MemoryBackend` và `HttpBackend` (với server giả) cho thấy hai adapter hành xử giống nhau.
- [x] E2E trang Knowledge pass, giao diện không đổi.

## Cập nhật tiến độ

- 2026-11-08: Hợp đồng common dùng `Scope`, validated `DocPath { scope, relative }`, `ReadWindow`, cây lồng nhau, `Page { content, revision }`, `Hit[]`, `VaultStatus`, các `VaultChange` của wiki-core và `Applied`. HTTP DTO được chuyển đổi ở adapter; DTO/routes riêng của Admin nằm trong `AdminKnowledgeBackend` extension.
- 2026-11-08: `knowledgeBackend.vault.test.ts` chạy contract chung trên Memory và Http (server giả) cho tree, read window, search, status, graph và write. `knowledgeBackend.admin.test.ts` giữ contract route/DTO Admin. `KnowledgePage.backend.test.ts` kiểm tra mở Wiki, Raw, search, sửa Wiki và chạy Page không có Admin extension.
- E2E Chromium chạy qua Vite và mock Admin API xác nhận đọc Wiki Page, chuyển Knowledge/File, mở Raw Working Note và quay lại cây Knowledge.
- 2026-11-08: Chuyển thêm Recent, khôi phục tab/pane, Graph navigation, chuyển nguồn, path bar và panel layout trong `KnowledgePage.test.ts` sang Memory. Suite mục tiêu 57/57; các test HTTP còn lại giữ lại cho API Admin-specific.
- 2026-11-08: Xác minh lại toàn bộ frontend: Vitest 95 files/550 tests, typecheck, build và Playwright Chromium E2E đều PASS.

## Comments

- 2026-11-08: `KnowledgeGraph.test.ts` dùng MemoryKnowledgeBackend cho global/focused graph; query graph còn đi qua Admin extension. `RightPanelTabs.test.ts` không gọi backend; `useWikiChat.test.ts` kiểm tra API/SSE Admin nên vẫn giữ server-specific.
- 2026-11-08: Phần E2E Chromium chạy qua Vite với Admin API mock, xác nhận đọc Wiki Page, chuyển Knowledge/File, mở Raw Working Note và quay lại cây Knowledge.
