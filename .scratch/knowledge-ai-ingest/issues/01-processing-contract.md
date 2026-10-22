# P0 — Settings, qualified provider and single-provider Job admission
Status: resolved

Persist disabled/review_all/manual defaults in app SQLite with CAS. Add explicit typed knowledge_ingest_v3 snapshots; share scheduler/admission/limits/tracing, preserve old modes. Parse bounded structured candidates; one repair maximum. Fail closed on provider revocation or oversized immutable Source.

## Answer

Implemented stored defaults/CAS, qualified single-provider admission, immutable snapshots, bounded structured execution and one repair. Background regression and full workspace tests pass. See [verification](../verification.md) and [review](../review.md). Resolution covers local implementation; remote model/deployment/production-process crash acceptance remains pending.
