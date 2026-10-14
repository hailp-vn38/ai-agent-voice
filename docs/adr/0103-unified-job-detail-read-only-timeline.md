# Unified Job Detail Timeline Is a Read-Only Projection

Status: accepted (Web implementation, 2026-10-14).

The Job Detail page uses the approved **B · Timeline focus** layout and projects one chronological view from the existing Job read view and cursor-paginated metadata-only Job Events. It groups LLM lifecycle stages into one activity, correlates plugin Steps and fork/join events by durable IDs, and keeps the Inspector read-only. We chose this over a second server-side activity log, a new event-streaming transport, or separate legacy/V2 timelines because the database already owns the authoritative Job/Step states and ordered event cursor. SSE remains an invalidation hint, not a source of activity truth. Agent Run IDs are distinct from LLM invocation IDs; legacy Jobs never receive fabricated child runs or coverage. The approved visual composition (four KPIs, left Unified Timeline, right Active Agents/Inspector/Source Coverage) is unchanged.

Consequences: client-side grouping must be deterministic, stable under partial cursor pages and reconnects, and compatible with Jobs lacking V2 fields. Server support for actual hierarchical runs is a separate deliverable; Web must display unavailable values rather than invent them.
