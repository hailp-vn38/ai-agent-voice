# Đồng bộ vault lên GitHub (Wiki Desktop và Admin Web)

Status: ready-for-agent

Thiết kế chi tiết: `docs/wiki-desktop/git-sync.md`.

## Problem Statement

Vault của app Wiki Desktop chỉ nằm trên một máy, vault của `vela-server` chỉ nằm trong volume Docker. Nếu máy hỏng hoặc mất thì mất luôn kho tri thức, và người dùng không làm việc được trên máy thứ hai. Hiện người dùng phải tự `git add/commit/push` trong terminal. Làm vậy dễ đưa nhầm Search Projection (`.wiki/index.sqlite`) lên repo, dễ ghi đè lúc app hoặc agent MCP đang ghi, và dễ kẹt ở trạng thái merge dở.

## Solution

App tự đồng bộ vault với một repo GitHub bằng `git` CLI của hệ thống. Toàn bộ quy trình nằm trong module `wiki-core::sync::VaultSync`. Module này chỉ chạy khi giữ lock ghi của vault, và khi xung đột thì giữ cả hai bản. Tauri shell lên lịch chạy (khi mở vault, sau khi có thay đổi, định kỳ), còn UI có mục Sync trong Settings và một huy hiệu trạng thái ở footer.

`vela-server` dùng lại cùng module: cấu hình trong `[wiki.sync]` của `config.toml`, xác thực bằng GitHub deploy key, chặn ghi qua `KnowledgeWriteGate` trong lúc merge và hoãn sync khi job AI ingest đang chạy. Admin Web xem trạng thái, bấm "Sync now" và xem xung đột. UI sync nằm trong `packages/knowledge-ui` sau seam `SyncBackend`, dùng chung cho cả hai.

## User Stories

1. As a người dùng, I want nhập URL repo GitHub và branch trong Settings, so that vault được lưu trữ ngoài máy.
2. As a người dùng, I want app tự commit và push sau khi tôi hoặc agent sửa vault, so that tôi không phải nhớ đồng bộ.
3. As a người dùng có hai máy, I want app tự kéo thay đổi từ máy kia khi mở vault và định kỳ, so that hai máy luôn giống nhau.
4. As a người dùng, I want bấm "Sync now", so that tôi chủ động đồng bộ trước khi tắt máy.
5. As a người dùng, I want thấy trạng thái sync (lần cuối, ahead/behind, lỗi), so that tôi biết dữ liệu đã an toàn chưa.
6. As a người dùng, I want khi hai máy sửa cùng một trang thì cả hai bản đều được giữ, so that không mất nội dung.
7. As a người dùng, I want index SQLite và lock file không bị đưa lên repo, so that repo gọn và không xung đột vô nghĩa.
8. As a người dùng, I want app báo rõ khi thiếu git hoặc lỗi xác thực kèm cách sửa, so that tôi tự xử lý được.
9. As a người dùng, I want app từ chối URL có chứa token, so that token không bị lộ trong `.git/config`.
10. As a người dùng, I want sync tạm tắt khi process khác đang giữ vault, so that hai process không cùng chạy git.

### Admin Web / vela-server
11. As a Admin, I want bật sync vault của server bằng `[wiki.sync]` trong `config.toml` với một deploy key, so that kho tri thức của server được lưu trữ trên GitHub.
12. As a Admin, I want thấy trạng thái sync trên trang Knowledge của Admin Web, so that tôi biết vault server đã được lưu trữ.
13. As a Admin, I want bấm "Sync now" trên Admin Web, so that tôi đồng bộ ngay sau khi sửa nhiều trang.
14. As a Admin, I want sửa trang trên Admin Web trong lúc server đang sync mà không hỏng dữ liệu, so that tôi không phải canh thời điểm.
15. As a Admin, I want sync chờ job AI ingest xong rồi mới chạy, so that merge không giẫm lên file job đang ghi.
16. As a Admin, I want remote không sửa được từ giao diện web, so that người có quyền admin không trỏ vault sang repo khác để lấy dữ liệu.

## Ngoài phạm vi

Sửa cấu hình sync của server từ Admin Web, chặn từng lần ghi bên trong `orchestrator-flow`, Git LFS, PAT trong Keychain, giao diện gộp xung đột, lịch sử phiên bản. Xem `docs/wiki-desktop/git-sync.md#ngoài-phạm-vi-v1`.
