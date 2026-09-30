# ADR 0054 — Admin API dùng optimistic concurrency theo revision

## Status

Accepted

Admin API GET trả revision và mọi PATCH/PUT nhận `If-Match`, được service layer biểu diễn thành Expected Revision. Resource mutation increment revision atomically; stale revision trả `409`, missing trả `404`, không last-write-wins. Binding mutation validate và increment revision của Agent, Template hoặc Device owner trong cùng transaction; `requires_restart` không thay thế revision check.
