# ADR 0066 — Single-owner DB và Bounded Runtime Lifecycle

## Status

Accepted; affected runtime clauses superseded by [ADR-0071](0071-versioned-provider-runtime-manager.md). Implementation/gates are tracked separately in `.scratch/provider-runtime-manager/`.

V1 chỉ hỗ trợ một Voice Agent process owner trên mỗi local SQLite path; không NFS/SMB, active-active hoặc multi-process writer. Migration chỉ tạo schema/index, không implicit seed; provision là explicit. Shutdown dừng listener/admission/tool call mới, drain session tối đa `shutdown.grace_ms` (default 15.000, range 1.000..=60.000), rồi controlled-close; HistoryWriter chỉ flush best-effort trong cùng deadline.

`/health` là liveness. `/ready` chỉ chứng minh dependencies application-owned đủ nhận connection mới: startup/schema complete, required RuntimeCatalog entries, admission resolver và DB nếu feature active yêu cầu. Nó không full admission, resolve Device, probe optional External MCP hay reload model/secret. Optional MCP failure không non-ready; DB failure làm non-ready khi admission DB required nhưng không đổi session đã admit.
