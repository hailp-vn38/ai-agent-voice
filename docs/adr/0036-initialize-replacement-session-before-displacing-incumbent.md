# ADR 0036 — Khởi tạo runtime session mới trước khi thay session đang khỏe

Khi một connection mới có cùng Device ID, server chỉ atomically thay Voice Session cũ sau khi ClientHello mới đã validate và audio runtime (`UplinkOpusDecoder`, `ManualCapture`) khởi tạo thành công. Lỗi init đóng connection mới với 1011, không ServerHello và không ảnh hưởng session cũ; điều này tránh reconnect lỗi làm mất dịch vụ đang khỏe.
