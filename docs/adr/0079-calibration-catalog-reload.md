# ADR 0079 — Explicit reload of deployment calibration catalog

## Status

Accepted design, 2026-10-07; chưa triển khai. Bổ sung security invalidation cho [ADR 0077](0077-speaker-v1-authority-and-calibration.md); không hot-reload Effective Session Profile, provider runtime hoặc toàn bộ TOML.

Operator gọi reload tường minh qua control path được xác thực. Reload chỉ đọc nguồn calibration deployment đã cấu hình, không nhận đường dẫn/URL tùy ý; Web không được sửa qualification. Validate toàn catalog trước publish: thất bại giữ catalog hiện hành và báo lỗi. Sửa file đơn thuần chưa có hiệu lực cho tới khi reload thành công. V1 dùng Admin Bearer hiện có, chưa thêm credential operator riêng. Web không có nút reload, sửa qualification hoặc bypass; người giữ Admin token vẫn tự gọi được reload API. Đây là giới hạn giao diện, không phải phân quyền operator. Route cụ thể và response contract cần chốt trước triển khai.

Publish catalog và security invalidation nhất quán trước khi báo thành công. Thu hồi qualification hoặc đổi contract chặn dispatch mới, đóng Required WS liên quan và không hạ policy xuống Observe; work đã dispatch không thể hoàn tác. Reload không tải lại model hoặc provider runtime. Chọn reload có phạm vi để thu hồi qualification trong process mà không mở một đường sửa runtime/config tổng quát; giữ nguyên catalog cũ khi reload lỗi tránh áp dụng deployment nửa chừng.
