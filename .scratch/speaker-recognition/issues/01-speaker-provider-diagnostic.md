# 01: Tạo và kiểm tra Speaker Provider CAM++

**What to build:** Admin tạo Speaker Provider, thấy desired/loaded state và chạy WAV diagnostic qua runtime manager hiện có, ngay từ Provider UI.

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] Thêm Speaker xuyên provider descriptor, typed config, registry/factory, physical planner, DB constraints, API và Provider UI; không thêm bảng config provider riêng.
- [x] Provider key server sinh; model/assets/revision và execution settings deployment-owned; manager absent báo unavailable, không handler inference fallback.
- [x] Prepare/diagnostic acquire exact DesiredProvider và giữ Resource Lease; dùng một physical extractor, bounded worker/queues, singleflight và accounting hiện có.
- [x] GET/capabilities/readiness không load model; cold/Ready/error states đúng; diagnostic raw WAV trả quality/provenance, không vector hoặc credential.
- [x] Forward migration giữ data/IDs/FKs/high-water marks; public HTTP tests và Qualification Provider kiểm create, revision, busy, format/bounds và cleanup. Cập nhật API collection cùng slice.

## Answer

Implemented Speaker/CAM++ managed provider vertical slice: typed deployment/logical config, descriptor/factory/planner, one bounded native worker, exact leased acquisition, forward migration, conditional WAV diagnostic, UI prepare/status/upload, and Postman collection. See `docs/speaker-provider.md` for deployment and transport contracts.

Validation: Admin API23 + database/bootstrap9 + provider registry7 passed (one existing installed-model test ignored); frontend typecheck and11 focused checks passed. Qualification providers prove deterministic transport/revision/native ownership, not real CAM++ or ESP32 accuracy. Diagnostic duration/energy/clipping is not calibrated voiced acceptance; shared enrollment/Observe quality/window integration follows in06.
