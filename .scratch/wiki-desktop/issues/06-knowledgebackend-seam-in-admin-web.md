# 06: Seam KnowledgeBackend trong Admin Web

Status: ready-for-agent
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

- [ ] Test Knowledge hiện có (`KnowledgePage`, `KnowledgeGraph`, `RightPanelTabs`, `useWikiChat`, …) chuyển từ mock `fetch` sang `MemoryBackend` và pass.
- [x] Một bộ contract test chạy cho cả `MemoryBackend` và `HttpBackend` (với server giả) cho thấy hai adapter hành xử giống nhau.
- [ ] E2E trang Knowledge pass, giao diện không đổi.

## Comments

- 2026-11-08: Tiêu chí chuyển test vẫn mở. `KnowledgeGraph.test.ts` và `KnowledgePage.backend.test.ts` inject `MemoryKnowledgeBackend`; Page memory suite bao phủ mở Wiki Page, Raw Working Note, tìm kiếm, sửa Wiki Page, và mount/browse chỉ với common interface khi không có Admin extension. `KnowledgePage.test.ts` còn là suite lớn dùng HTTP route mocks; phần chat/SSE và khẳng định endpoint là kiểm thử Admin/server-specific, còn các luồng Vault khác vẫn cần chuyển. `RightPanelTabs.test.ts` không gọi backend; `useWikiChat.test.ts` kiểm tra API/SSE phía Admin nên không chuyển qua MemoryBackend.
- 2026-11-08: Hợp đồng chung đã được chuẩn hóa theo `wiki-core::Vault`: `Scope`, validated `DocPath { scope, relative }`, `ReadWindow`, cây lồng nhau, `Page { content, revision }`, `Hit[]`, `VaultStatus`, `VaultChange` variants và `Applied`. `AdminKnowledgeBackend` là extension riêng cho DTO/route của Admin. `knowledgeBackend.vault.test.ts` chạy chung các ca cây, đọc theo window, tìm kiếm, status, graph và write trên Memory/Http (server giả); `knowledgeBackend.admin.test.ts` giữ contract route/DTO Admin. Ticket vẫn mở do test component cũ chưa chuyển hết và E2E chưa chạy.
