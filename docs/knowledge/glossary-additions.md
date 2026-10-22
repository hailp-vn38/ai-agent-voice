# Knowledge + Memory — domain vocabulary

Definitions to add under the project glossary. Keep these independent of persistence technology.

**Wiki Page**: A reusable, sourced knowledge document accepted for publication and identified independently of its title or path.
_Avoid_: Memory Fact, raw Source, Job Artifact

**Source**: Preserved input evidence with a durable version and provenance that Wiki Pages can cite.
_Avoid_: transient attachment, generated Wiki Page

**Knowledge Revision**: One accepted immutable version of a Wiki Page.
_Avoid_: Job attempt, index row

**Memory Fact**: A durable, owner-scoped assertion about the user that can be confirmed, corrected, superseded or forgotten.
_Avoid_: Wiki Page, model inference, memory index row

**Memory Summary**: A bounded, generated view of confirmed active Memory Facts used by Voice.
_Avoid_: canonical fact, user-edited MEMORY.md

**Publication Proposal**: Untrusted candidate content awaiting a Rust Core validation and expected-revision check.
_Avoid_: accepted revision, LLM filesystem instruction

**Search Projection**: A derived index of canonical Markdown that can be rebuilt without deleting Knowledge or Memory.
_Avoid_: canonical document store, Agent Job database
