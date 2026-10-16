# Knowledge + Memory V3 — data flows

Normative DB and recovery contract: [database-layout-v3.md](../knowledge/database-layout-v3.md). Domain interfaces: [implementation-v3.md](../knowledge/implementation-v3.md).

```mermaid
flowchart TD
  A[Admin Source / Job Artifact] --> B[KnowledgeCore preserve versioned Source]
  B --> C[Existing Agent Job V3 Root / Child research]
  C --> D[Untrusted Publication Proposal]
  D --> E{Rust KnowledgeCore validate provenance and revision}
  E -->|Reject| R[Typed failure]
  E -->|Accept| F[Atomic Wiki Markdown + revision]
  F --> G[Reconcile knowledge_index.db FTS5]
  G -->|Index stale| H[Keep canonical page; degrade search]
  G -->|Ready| I[Wiki search / Web read]
  C --> J[Existing Job Detail Timeline]
```

```mermaid
flowchart TD
  A[User explicit fact / authenticated Admin] --> B[Memory Proposal]
  B --> C{MemoryCore scope + confirmation + expected revision}
  C -->|Reject| X[Typed failure]
  C -->|Accept| D[Canonical Fact revision]
  D --> E[Regenerate bounded MEMORY.md]
  E --> F[Reconcile memory_index.db]
  F -->|Ready| G[Voice summary + memory.recall]
  E -->|Failed| H[Suppress stale summary; repair]
```

Each index has its own SQLx migrator and health. Full scanner failure preserves old rows but does not serve them as current. The application database remains the exclusive durable Job scheduler; no second ingest event/state store. Neither the Root nor Child receives an arbitrary filesystem-write tool.

## Implementation reality / admission policy

On this branch, authenticated Admin imports original **text Markdown** and explicitly publishes a verified Wiki revision. Background Job V3 can only search/read published Wiki pages and recall confirmed Memory after its Core group is enabled by Admin; no Agent-driven Source ingestion or Wiki publication tool is exposed. The diagrams above describe the future Job-driven proposal path, not a shipped automatic write capability.

## Corrected published Wiki edit flow

```mermaid
flowchart LR
  A[Canonical verified Wiki rev N] --> B[Admin edits staged Wiki draft]
  B --> C[Draft CAS and base rev N]
  C --> D{Verify Source and publish CAS}
  D -->|Reject| C
  D -->|Accept| E[Canonical Wiki rev N plus 1]
  E --> F[Reconcile Knowledge FTS]
  F --> G[Agent reads verified rev N plus 1]
  A --> H[Agent continues reading rev N until publish]
```

Admin and Job reuse one Rust Runtime; the Job tools cannot read staged drafts. The Knowledge index is marked stale when immutable Source evidence disappears or fails SHA256 validation. Memory index health is independent.
