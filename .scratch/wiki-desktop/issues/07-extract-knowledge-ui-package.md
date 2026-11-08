# 07: Tách package knowledge-ui dùng chung

Status: resolved
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

- [x] Admin Web build và chạy được với package mới. Vitest và E2E Knowledge pass.
- [x] Package có test riêng chạy trên `MemoryBackend`.
- [x] Không còn import nào từ package trỏ ngược vào code của Admin Web (có kiểm tra bằng lint rule hoặc test).

## Comments

- 2026-11-08: Tạo npm workspace `@vela/knowledge-ui`; chuyển Markdown, Knowledge Graph/reader/editor/tabs, workspace composable, drag reorder, Mermaid/chart renderer và CSS dùng chung. Admin Web tiếp tục sở hữu HTTP adapter và cung cấp backend, i18n, theme, chart qua injection.
- 2026-11-08: Catalog `knowledgeV3.*` và `knowledgeEditor.*` chuyển vào package; contract test của `MemoryKnowledgeBackend` và boundary test của package được thêm.
- 2026-11-08: `scripts/agent-test.sh web typecheck`, `web test` (97 files/552 tests), `web build`, `web e2e` đều PASS.
