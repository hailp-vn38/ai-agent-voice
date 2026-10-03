# Admin P3 — Conditional deletion và Vision Provider integration

## Decision đã chốt

- Hard-delete là explicit, CAS qua `If-Match`, và chỉ thành công khi không còn relationship
  hoặc history reference. Không cascade-unlink hay purge history ngầm.
- Vision sẽ trở thành Database Provider Instance, nhưng route Vision hiện có chưa mang quy tắc
  chọn Provider theo Device/Template. Ticket Vision phải chốt seam đó trước khi migration/code.

## Tickets

1. Conditional deletion và Agent MCP unlink — hoàn thành.
2. Vision Database Provider contract và implementation — `needs-info` cho selection seam.
