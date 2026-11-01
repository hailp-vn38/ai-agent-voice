# ADR 0135, glossary và sửa tài liệu

Type: task
Status: resolved
Blocked by:

## Scope

Mốc 1 của [spec](../spec.md), bước GĐ 0 trong [hướng dẫn](../../../docs/memory-core-implementation-guide.md) mục 3. Chỉ có tài liệu, không có code.

- Viết `docs/adr/0135-memory-core-runtime.md`, gồm D1 đến D11 và mục "Đối chiếu ADR" của spec (lệch 0119, lệch tinh thần 0008, phạm vi ngoại lệ 0024, Memory Vault nằm ngoài backup theo 0131). Ghi cả hai bổ sung của spec so với hướng dẫn: `Correct` khớp tombstone thì gỡ tombstone; `don't forget` bị loại khỏi nhóm `forget`.
- Cập nhật `GLOSSARY.md` và rà `CONTEXT.md` qua skill `domain-modeling` theo mục "Glossary" của spec.
- Đánh dấu lỗi thời `docs/knowledge/README.md` và `docs/flows/13-knowledge-v3.md`.
- Sửa [thiết kế](../../../docs/memory-core-integration-design.md) theo mục 1 của hướng dẫn.

User story: 82.

## Acceptance

- ADR 0135 tồn tại, trạng thái được chấp nhận, nêu đủ các điểm trên.
- Không tài liệu nào trong phạm vi trên còn mô tả Memory Fact nằm ở `wiki/entities/`, hay Memory Summary lấy từ `INDEX.md`, như hiện trạng.
- Thiết kế không còn mâu thuẫn với D1 đến D11.

## Answer

Đã triển khai tại `05dbb1a9`: ADR 0135, `GLOSSARY.md`, `CONTEXT.md`, đánh dấu lỗi thời hai tài liệu Knowledge và sửa thiết kế theo D1 đến D11. Các lỗi code review được sửa tại `15556c6a`; lỗi 503 có từ trước của `admin_api` được sửa tại `7c808c32`.
