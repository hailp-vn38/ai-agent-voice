# ADR 0050 — Database Desired Configuration tách khỏi Loaded Runtime

## Status

Accepted; affected runtime clauses superseded by [ADR-0071](0071-versioned-provider-runtime-manager.md). Implementation/gates are tracked separately in `.scratch/provider-runtime-manager/`.

Provider CRUD persist Database Desired Configuration nhưng Runtime Catalog giữ immutable trong lifetime process; mọi thay đổi runtime-affecting phải trả `requires_restart=true` và chỉ effective sau restart. Runtime Status báo process usability, còn runtime_matches_desired so loaded revision với desired revision; loaded không đồng nghĩa newest. Provider Load Plan load Server Provider Defaults và enabled Agent default Template như required, còn provider chỉ của non-default Template như optional; optional load failure exclude candidate thay vì block boot, provider unbound skip và có runtime status coarse. Template chỉ activate/switch khi toàn bộ binding đã validate và có Loaded Runtime; không hot-load model hay instantiate provider từ DB trong Voice Session.
