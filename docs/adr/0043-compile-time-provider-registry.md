# ADR 0043 — Compile-time provider registry và factory boundary

## Status
Accepted

Provider selection ở startup dùng Provider Registry gồm VadFactory/AsrFactory được compile vào binary. Typed provider configuration chọn adapter, factory build adapter với runtime options và Resolved Model khi cần; thay đổi adapter cần build/restart. Điều này supersede riêng lựa chọn large `match` và không-registry của ADR-0042, nhưng giữ nguyên worker/session ownership của ADR đó. Registry không load plugin, không discovery code runtime và không cho provider tự acquire model.

## Consequences

Thêm adapter không sửa SessionActor, worker hay Model Preparation: adapter mới thêm factory compile-time, registration và manifest khi dùng model. Adapter không được chọn hoặc load lúc runtime.
