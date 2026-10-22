# P1/P2 — Proposals, gate and publication recovery
Status: resolved

Persist metadata only; stage exclusively through KnowledgeCore. Verify quote spans, source checksum, target CAS, duplicate/index/graph integrity. Default Review; auto only direct-excerpt creates. Persist publish intent before canonical commit; monotonic recovery and idempotent Admin decisions. Never overwrite human drafts.

## Answer

Implemented metadata-only proposals, deterministic evidence/graph gate, staging, CAS review, publication intent and restart reconciliation. Local fault/restart and full workspace tests pass. See [verification](../verification.md) and [review](../review.md). Resolution covers local implementation; remote model/deployment/production-process crash acceptance remains pending.
