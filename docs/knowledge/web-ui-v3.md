# Admin Web Knowledge and Memory V3

Keep separate top-level workspaces: **Knowledge** with Wiki Pages/Sources and **Memory** with Facts/Generated Summary. Do not merge Memory into Wiki or put both into Plugin settings. Use existing Vue admin shell and authenticated Admin HTTP. MVP: list/search/read/publish with revision conflict, source attribution, index health/rebuild; Memory requires explicit confirmation/correction/forget and indicates that MEMORY.md is generated. Reuse existing Job Detail Timeline rather than inventing an ingest Activity stream.

No new Quality/Activity/Review dashboards or vector/graph visualization in MVP. Direct file and index DB paths are never browser-controlled; return IDs and revision metadata. Admin routes do not bypass Rust Core validation. On degraded search, show the index status rather than stale search results. Future refinements may add Source preview, conflict compare and metadata graph with concrete product demand.
