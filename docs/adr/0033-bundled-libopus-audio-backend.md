# ADR 0033 — Bundle libopus sau wrapper opus2 cho V1

V1 dùng `opus2` với backend `libopus_sys` bundled/static thay vì phụ thuộc libopus hệ thống hoặc codec pure Rust. Đây là lựa chọn có lock-in build native nhưng giữ CI và deployment tái lập được, đồng thời ưu tiên interoperability với Voice Protocol Client và fixture Opus thật đã pin. Module audio che toàn bộ type `opus2` sau concrete `UplinkOpusDecoder` và `DownlinkOpusEncoder`; chưa tạo codec trait vì Phase 2 chỉ có một backend.
