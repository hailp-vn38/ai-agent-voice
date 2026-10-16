# ADR 0119 — Memory and Knowledge have separate ownership

Status: Accepted.

Knowledge Core owns reusable evidence-backed Wiki/Sources, while Memory Core owns confirmed, owner-scoped personal facts. Inference, technical Wiki text or a completed Job must not silently become personal Memory. Both modules reuse internal Markdown/indexing implementation but expose distinct validation and policy interfaces. `MEMORY.md` is regenerated from confirmed active Facts and is not a second source of truth; forgetting must suppress the fact from both live recall and summary before success. Each domain has a separate, independently rebuildable index and its own Admin workspace.
