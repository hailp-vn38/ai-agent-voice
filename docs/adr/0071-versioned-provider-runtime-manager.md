# ADR 0071 — Versioned Provider Runtime Manager

## Status

Accepted design; implementation tracked in `.scratch/provider-runtime-manager/`.

## Context

Startup-only Runtime Catalog không cấp được Provider Instance vừa tạo hay desired revision mới. Chỉ lookup theo key có thể cấp runtime cũ cho admission mới. Nạp mọi Template candidate giữ native resources không cần thiết; cấp capacity riêng theo revision làm concurrency tăng ngoài policy.

## Decision

Áp dụng [three-phase guide](../provider-runtime-manager-three-phase-guide.md). Application-owned Provider Runtime Manager cấp runtime theo ProviderVersion từ một bounded, transaction-consistent Database Desired Configuration snapshot. SessionActor không query DB, resolve secret hoặc build model. Voice Session giữ immutable Effective Session Profile và Resource Leases; mutation chỉ ảnh hưởng admission mới. Switch explicit prepare ngoài actor rồi commit tại writer/cleanup/history boundary.

ProviderVersion gồm database source namespace, Provider Instance identity và desired revision. Schema hiện tại dùng `providers.id INTEGER PRIMARY KEY AUTOINCREMENT`; Admin CRUD không nhận hay sửa id, nên delete/recreate qua supported API không tái sử dụng identity. Deployment defaults dùng source namespace khác. Restore/import hoặc explicit SQL tái sử dụng id không được thực hiện khi process owner còn chạy; nếu bổ sung supported import phải tạo instance identity mới.

D1 resources có thể gắn một-một với ProviderVersion. D2 thêm adapter-defined Runtime Resource Key, canonicalization/fingerprints/credential generation và sharing explicit. Resource Key của Admin resources trong glossary vẫn là public business key; khác với opaque Runtime Resource Key của backing resources.

Manager owns singleflight, bounded loaders/queue/waiters/metadata và estimated memory reservations. Caller timeout không release reservation hay load slot của native attempt còn chạy. Publish Ready chỉ sau readiness acknowledgement của workers thật; lỗi native init không được chuyển thành compatibility fallback. Quotas của logical provider tồn tại qua revisions, physical quotas thuộc backing resource. Unload/drain/quarantine giữ accounting tới acknowledgement; shutdown dùng application admission gate và cùng deadline.

GET/readiness không load model hoặc resolve credential. Diagnostics snapshot một desired revision, acquire exact runtime và trả safe provenance. Hot-supported provider không dùng restart làm remediation. Artifacts publish atomic theo immutable fingerprints, không overwrite file đang dùng. D3 tách operation settings khỏi engine chỉ khi adapter và audio/isolation evidence chứng minh reuse an toàn.

## Supersession

Thay startup-only/process-immutable/requires-restart và required DB-default preload policy trong ADR-0050. Giữ immutable session và explicit switch của ADR-0052, nhưng candidate catalog snapshot cấu hình thay vì pin mọi runtime. Cập nhật manager/readiness/shutdown ownership của ADR-0066; giữ single-owner SQLite, single deadline, `/health` liveness và metadata-only `/ready`. Giữ compile-time registry ADR-0043: chọn adapter đã compile khi acquire, không thêm runtime plugin. Giữ ADR-0023, ADR-0040, ADR-0042 và ADR-0062.

Các supersession trên là contract đích, không phải bằng chứng implementation đã đạt. Issue tracker và gate D1/D2/D3 ghi phạm vi thực tế.
