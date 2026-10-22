# ADR 0125 — Knowledge Job uses native tools, shared Job Skills and automatic publication

**Status:** Accepted and locally implemented. Native v2 remains an explicit opt-in; production deployment is separate. See [qualification](../knowledge/ai-ingest-fixed-job-qualification-v3.md).

**Date:** 2026-10-19.

**Requirements:** [Knowledge fixed Job](../knowledge/ai-ingest-fixed-job-requirements-v3.md).

**Implementation:** [Implementation plan](../knowledge/ai-ingest-fixed-job-implementation-v3.md).

The user requires a Knowledge Job that uses existing Job V3 native tool communication, chooses additional Job Skills, and publishes Wiki changes automatically. ADR 0123's one-turn Structured Output and review-first updates prevent that workflow. Add an explicit version 2 Knowledge execution contract on the existing `knowledge_ingest_v3` mode; retain version 1 for historical Jobs and Review decisions. Do not reinterpret queued version 1 work or require Structured Output for version 2.

Agent Job V3 keeps execution, budgets, Sessions and operation evidence. A shared Job Skills module owns catalog discovery, immutable instruction snapshots and per-Session loading; skills guide work without changing tool grants. A Knowledge-specific tool adapter delegates source reads, draft ownership, provenance, publication and recovery to KnowledgeCore. KnowledgeCore remains the canonical writer; private native write tools are not additions to the globally read-only `core.knowledge` adapter from ADR 0117.

Canonical Markdown records multi-Source Wiki Provenance and immutable revision history. Every write has durable intent tied to the existing Job Step ledger; matching filesystem evidence settles an interrupted commit, while uncertainty stops further writes. The trade-off is that automatic publication can preserve a mistaken paraphrase and partial publication cannot be rolled back as one transaction. Source grounding, CAS and history constrain and expose these risks; they do not prove semantic truth.

Rejected alternatives: a second Knowledge scheduler duplicates Job ownership; giving generic Jobs write tools when they load a skill conflates instructions with permission; replacing all Source references on update loses provenance; copying all skill bodies into every prompt wastes context; requiring Structured Output adds a provider restriction absent from the existing Job tool interface.

This decision replaces ADR 0123's execution/publication rules **for version 2 only**, and extends ADR 0117 with a separately authorized private write adapter. ADRs 0118 and 0124 retain canonical Markdown/revision and Wiki Link ownership. Version 1 remains governed by its saved contract. Adoption requires the migration and acceptance gates in the implementation plan, not merely this document's presence.
