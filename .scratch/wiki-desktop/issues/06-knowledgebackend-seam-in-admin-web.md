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

- Đã thêm `KnowledgeBackend` cùng `HttpKnowledgeBackend` và `MemoryKnowledgeBackend`; Admin Web inject backend ở cấp ứng dụng. Các luồng cây, đọc, tìm kiếm, đồ thị, trạng thái, thay đổi và khả năng chat đi qua seam này.
- Contract test hiện chạy cùng kịch bản đọc, tìm kiếm không dấu và ghi có revision cho cả hai adapter; test HTTP dùng server giả.
- Đã thêm test KnowledgePage qua `MemoryKnowledgeBackend`, gồm Wiki Page, Raw Working Note và tìm kiếm, đồng thời xác nhận các luồng này không gọi `fetch`.
- Còn thiếu: chuyển bộ `KnowledgePage.test.ts` hiện hữu khỏi mock `requestJson` sang `MemoryKnowledgeBackend` và xác nhận toàn bộ suite đó pass. Chưa có gate E2E trang Knowledge để chạy và ghi nhận kết quả.
- Vì vậy ticket vẫn `ready-for-agent`; tiêu chí chuyển test hiện hữu và E2E chưa hoàn tất.
