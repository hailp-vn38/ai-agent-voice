# 10 — TTL/LRU/hotset/pressure budgets và bounded aliases

Status: resolved
Phase: D2
Dependencies: 08–09

## Contract

Áp dụng đầy đủ mục 6.4, D2-T08…D2-T11/D2-T13 của [spec](../spec.md) và invariants §3.2.

## Acceptance

- [x] Implement contract.
- [x] Deterministic tests qua các seam đã chỉ định.
- [x] Required checks và review đạt; ghi evidence, không thay NOT_RUN bằng PASS.
- [x] Commit scoped changes sau gate.

## Comments

Resolved cùng implementation ba phase; bằng chứng gate và qualification được ghi tại `../spec.md` và `../evidence/`.
