# ADR 0075 — Chuẩn bị artifact và thư mục database trước runtime

## Status

Accepted — theo yêu cầu tự chuẩn bị model local và thư mục database của người dùng.

Phần về database của ADR này giữ nguyên hiệu lực. Phần về Model Preparation đã bị thay
thế bởi [ADR 0076](0076-provider-owned-model-assets.md): không còn chuẩn bị model lúc
startup, không còn offline mode, không còn `prepared://` source mapping và không còn
SHA-256 verification. Thư mục database vẫn được tạo cha và migrate trước bind như dưới đây.

## Decision

Database::connect parse SQLite URL, tạo thư mục cha nếu thiếu, rồi mở file và chạy
migration. Không tạo application rows ngầm; directory/file conflict hoặc lỗi ghi
vẫn fail trước bind. Mở lại database giữ nguyên dữ liệu và migration history.

Phần Model Preparation của ADR này đã bị thay thế. Sự thay đổi còn giữ lại: HTTP
acquisition stream qua buffer 64 KiB, connect timeout 15 giây, request timeout 900
giây, tối đa 3 attempts với backoff 1/2 giây cho lỗi transient, và không retry cho
HTTP 4xx thông thường hoặc lỗi ghi disk. Temporary files được dọn sau failure.
Không tự cài native runtime hoặc chạy code/converter từ nguồn model; ONNX Runtime và
G2P là deployment dependencies.

Thay thế bằng: mỗi provider tải model của nó khi Provider Runtime Manager materialize
nó, dùng `.part` + atomic rename thay cho checksum + atomic install. Không có
`prepared://` source mapping và không có Model License Acknowledgement.

## Validation

Deterministic tests dùng file-backed SQLite, local HTTP server và fixture asset:
missing directory/reopen, streaming retry, permanent HTTP failure, temporary-file
cleanup và provider asset download atomicity. Không cần model ONNX hoặc credential
thật.
