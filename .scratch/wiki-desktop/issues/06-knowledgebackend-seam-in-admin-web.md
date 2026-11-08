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

## Cập nhật tiến độ

- 2026-11-08: Hợp đồng common dùng `Scope`, validated `DocPath { scope, relative }`, `ReadWindow`, cây lồng nhau, `Page { content, revision }`, `Hit[]`, `VaultStatus`, các `VaultChange` của wiki-core và `Applied`. HTTP DTO được chuyển đổi ở adapter; DTO/routes riêng của Admin nằm trong `AdminKnowledgeBackend` extension.
- 2026-11-08: `knowledgeBackend.vault.test.ts` chạy contract chung trên Memory và Http (server giả) cho tree, read window, search, status, graph và write. `knowledgeBackend.admin.test.ts` giữ contract route/DTO Admin. `KnowledgePage.backend.test.ts` kiểm tra mở Wiki, Raw, search, sửa Wiki và chạy Page không có Admin extension.
- Chưa hoàn tất: chuyển hết các luồng Vault trong `KnowledgePage.test.ts` khỏi HTTP route mocks, đồng thời chạy E2E Knowledge. Ticket vẫn `ready-for-agent`.

## Comments

- 2026-11-08: `KnowledgeGraph.test.ts` dùng MemoryKnowledgeBackend cho global/focused graph; query graph còn đi qua Admin extension. `RightPanelTabs.test.ts` không gọi backend; `useWikiChat.test.ts` kiểm tra API/SSE Admin nên vẫn giữ server-specific.
