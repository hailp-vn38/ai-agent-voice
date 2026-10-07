# ADR 0081 — Qualification covers exact candidate sets

## Status

Accepted design, 2026-10-07; chưa triển khai. Bổ sung phạm vi qualification cho [ADR 0077](0077-speaker-v1-authority-and-calibration.md).

Pilot qualification pin các candidate set thực tế của Agent/Template đã đánh giá: candidate identities, voiceprint revisions, embedding space, scoring/calibration contract và đường thu. Admission Required chỉ dùng đúng set được evidence bao phủ; subset không tự qualified vì bớt candidate có thể đổi top-2 và margin, làm một query bị reject trở thành accepted. Không dùng một roster chung toàn deployment để suy quyền cho mọi Agent/Template.

Thêm/thay người, re-enroll hoặc đổi grants làm thay candidate set cần evidence cập nhật; enrollment/Observe vẫn dùng được. Required ngoài phạm vi bị từ chối, không tự hạ mode. Thu hồi quyền có hiệu lực ngay dù khiến Required unavailable; phiên bị ảnh hưởng vẫn phải invalidated. Quyền Template được kiểm sau scoring trên candidate set theo domain contract; không loại người thiếu grant chỉ để cải thiện match.

Domain mutation hợp lệ được lưu dù set mới chưa có evidence, không trả 409 chỉ vì thiếu qualification. Required giữ mode; admission set mới từ chối và response/UI ghi “Đã lưu; Required chưa dùng được với candidate set này” cùng dependency thiếu. WS cũ tiếp tục nếu exact snapshot còn qualified và quyền chưa bị thu hồi; thêm quyền không sửa nóng snapshot. Re-enroll, giảm quyền và contract change vẫn invalidation. Chuyển policy sang Required tiếp tục kiểm qualification hợp lệ; quyết định lưu config không bypass policy-enable gate.

Catalog có thể giữ evidence cho nhiều exact sets; thêm set mới không tự xóa qualification set cũ. Catalog generation thay đổi không tự đổi scoring/calibration contract revision hoặc thu hồi evidence; removal/revocation của evidence và contract change phải invalidation những Required snapshots thực sự bị ảnh hưởng.
