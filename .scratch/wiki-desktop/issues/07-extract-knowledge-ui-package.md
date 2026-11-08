# 07: Tách package knowledge-ui dùng chung

Status: ready-for-agent
Blocked by: 06
Spec: ../spec.md (user stories 10–17, 49)

## Mục tiêu

Chuyển UI Knowledge thành một workspace package mà cả Admin Web và app desktop cùng dùng.

## Phạm vi

- Bật workspaces cho frontend, theo package manager đang dùng.
- Chuyển vào package: các component Knowledge, `useKnowledgeWorkspace`, `useDragReorder`, renderer Markdown/Mermaid/chart, CSS reader, interface `KnowledgeBackend`, `MemoryBackend` và các key i18n `knowledge.*`.
- Package không phụ thuộc vào router, shell hay store của admin. Trạng thái "trang đang mở" nằm trong workspace composable, mỗi app tự đồng bộ với URL hoặc window title.
- `HttpBackend` ở lại Admin Web.

## Tiêu chí xong

- [ ] Admin Web build và chạy được với package mới. Vitest và E2E Knowledge pass.
- [ ] Package có test riêng chạy trên `MemoryBackend`.
- [ ] Không còn import nào từ package trỏ ngược vào code của Admin Web (có kiểm tra bằng lint rule hoặc test).
