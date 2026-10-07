# 01: Tạo và kiểm tra Speaker Provider CAM++

**What to build:** Admin tạo Speaker Provider, thấy desired/loaded state và chạy WAV diagnostic qua runtime manager hiện có, ngay từ Provider UI.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [ ] Thêm Speaker xuyên provider descriptor, typed config, registry/factory, physical planner, DB constraints, API và Provider UI; không thêm bảng config provider riêng.
- [ ] Provider key server sinh; model/assets/revision và execution settings deployment-owned; manager absent báo unavailable, không handler inference fallback.
- [ ] Prepare/diagnostic acquire exact DesiredProvider và giữ Resource Lease; dùng một physical extractor, bounded worker/queues, singleflight và accounting hiện có.
- [ ] GET/capabilities/readiness không load model; cold/Ready/error states đúng; diagnostic raw WAV trả quality/provenance, không vector hoặc credential.
- [ ] Forward migration giữ data/IDs/FKs/high-water marks; public HTTP tests và Qualification Provider kiểm create, revision, busy, format/bounds và cleanup. Cập nhật API collection cùng slice.
