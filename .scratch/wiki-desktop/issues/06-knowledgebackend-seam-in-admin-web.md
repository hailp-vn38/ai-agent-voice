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
- [ ] Một bộ contract test chạy cho cả `MemoryBackend` và `HttpBackend` (với server giả) cho thấy hai adapter hành xử giống nhau.
- [ ] E2E trang Knowledge pass, giao diện không đổi.
