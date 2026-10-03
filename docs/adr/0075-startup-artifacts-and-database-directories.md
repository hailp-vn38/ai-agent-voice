# ADR 0075 — Chuẩn bị artifact và thư mục database trước runtime

## Status

Accepted — theo yêu cầu tự chuẩn bị model local và thư mục database của người dùng.

## Decision

Database::connect parse SQLite URL, tạo thư mục cha nếu thiếu, rồi mở file và chạy
migration. Không tạo application rows ngầm; directory/file conflict hoặc lỗi ghi
vẫn fail trước bind. Mở lại database giữ nguyên dữ liệu và migration history.

Production startup đọc Provider Load Plan và enabled Desired Provider rows, rồi
Model Preparation chuẩn bị artifact trước khi dựng bất kỳ inference runtime nào.
Các local instance trong TOML được chuẩn bị xuống đĩa; DB chỉ bao gồm provider
được enabled Template assignment sử dụng. Gộp theo adapter và Logical Model
Identity; required thắng optional. Server defaults, effective deployment bindings,
TTS preload và DB default Template bindings là required. Model required lỗi fail
trước bind; optional lỗi được log mà không nạp worker hay chặn startup.

Download không nằm trong deadline khởi tạo Provider Runtime Manager. Legacy loader
chỉ dùng artifact đã chuẩn bị (offline), tránh tải lại một artifact vừa thất bại.
Managed startup chuẩn bị cả immutable copies trước acquisition; policy hot runtime,
memory reservations và session isolation trong ADR-0071 vẫn giữ nguyên.

Model hợp lệ được kiểm tra SHA-256 và reuse không network. HTTP acquisition stream
qua buffer 64 KiB, connect timeout 15 giây, request timeout 900 giây, tối đa 3 attempts
với backoff 1/2 giây cho lỗi transient. HTTP 4xx thông thường, lỗi ghi disk và checksum
sai không retry. Source/output checksum và atomic install vẫn thuộc ADR-0044.
Temporary files được dọn sau failure; identity transform và checksum không buffer
toàn bộ model trong RAM. Startup có thể sửa immutable artifact corrupt trước khi
runtime tồn tại; hot acquisition tiếp tục từ chối thay thế artifact đã publish.

deployment.models.sources map prepared:// source sang HTTP(S) URL cho artifact đã
chuẩn bị ngoài server, như Kokoro voicepack. Manifest revision/checksum/license và
Model License Acknowledgement vẫn authoritative. Không tự cài native runtime hoặc
chạy code/converter từ nguồn model; ONNX Runtime và G2P là deployment dependencies.

## Validation

Deterministic tests dùng file-backed SQLite, local HTTP server và fixture artifact:
missing directory/reopen, streaming retry, permanent HTTP failure, checksum cleanup,
prepared source mapping, required/optional/unbound planning, offline failure và
startup-only immutable repair. Không cần model ONNX hoặc credential thật.
