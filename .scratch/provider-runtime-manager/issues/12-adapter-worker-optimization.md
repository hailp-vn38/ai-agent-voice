# 12 — Adapter field separation, worker reuse/readiness optimization

Status: resolved
Phase: D3
Dependencies: 11

## Contract

Áp dụng đầy đủ mục 7.2–7.3 của [spec](../spec.md) và invariants §3.2.

## Acceptance

- [x] Implement contract.
- [x] Deterministic tests qua các seam đã chỉ định.
- [x] Required checks và review đạt; ghi evidence, không thay NOT_RUN bằng PASS.
- [x] Commit scoped changes sau gate.

## Comments

Resolved cùng implementation ba phase; bằng chứng gate và qualification được ghi tại `../spec.md` và `../evidence/`.
