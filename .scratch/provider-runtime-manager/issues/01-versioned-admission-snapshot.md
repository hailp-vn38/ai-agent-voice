# 01 — ADR/glossary và versioned DB admission snapshot

Status: resolved
Phase: D1
Dependencies: —

## Contract

Áp dụng đầy đủ mục 5.2, D1-T04/D1-T13 của [spec](../spec.md) và invariants §3.2.

## Acceptance

- [x] Implement contract.
- [x] Deterministic tests qua các seam đã chỉ định.
- [x] Required checks và review đạt; ghi evidence, không thay NOT_RUN bằng PASS.
- [x] Commit scoped changes sau gate.

## Comments

Resolved cùng implementation ba phase; bằng chứng gate và qualification được ghi tại `../spec.md` và `../evidence/`.
