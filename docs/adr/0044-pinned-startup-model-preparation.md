# ADR 0044 — Model Preparation đã pin tại startup

## Status
Superseded by [ADR 0076](0076-provider-owned-model-assets.md)

Decision này đã bị thay thế: provider không còn nhận artifact qua manifest hay
Resolved Model nữa. Provider tự khai báo URL và tự tải khi runtime manager materialize nó.
Xem ADR 0076 cho kiến trúc thay thế và các trade-off đã chấp nhận.

Lý do thay thế: checksum verification và content-addressed install tree phải hash
hàng gigabyte model ở mỗi materialization, trong khi integrity thật sự của model
chỉ được biết khi runtime nạp nó. Một server homelab không đổi được gì từ việc
xác minh lại file mà nó sở hữu, nhưng mất thời gian khởi động.

## Decision (superseded)

Typed provider configuration chỉ chọn Logical Model Identity; Model Artifact Manifest authoritative source, revision, remote artifact, install-relative path, declared transform và provider-facing checksum. Trước bind, Model Preparation resolve identity, reuse hoặc acquire artifact đã pin, verify, transform, verify output và atomic install dưới configured model root, rồi inject Resolved Model theo artifact role vào Provider Factory để build/warmup. Offline Model Preparation cấm network và fail trước bind khi không thể chứng minh artifact hợp lệ.

## Consequences

Provider không nhận direct filesystem path từ config, không tự download và không đoán tên upstream. Manifest path tuyệt đối, traversal hoặc escape model root bị reject.

Phase 4 giữ license model-level. `zerotts_default` dùng canonical composite declaration `MIT; bundled-codec=Apache-2.0`; deployment acknowledgement vẫn match chính xác identity, revision và license này, còn `codec_license` là required artifact. Artifact-level licensing chỉ được thêm khi deployment policy cần phân biệt component.
