---
name: daily-synthesis-staging
description: "Workflow for synthesizing a user chat/discussion into a structured Markdown document with provenance, conflict, and open-question guardrails via `add_to_sources` or exporting a standalone report."
---

# Daily Discussion Synthesis & Wiki Staging Skill (`daily-synthesis-staging`)

Use this skill when the user asks to summarize/synthesize a conversation, research thread, or daily report, especially when they ask to **"add/save it to the Wiki"**.

## 1. Distinguish Target Destination
1. **Case A — User wants to add/update knowledge in the Wiki**:
   - Inspect existing `<domain>/<topic>` folders via `wiki_tree(scope="sources")` and `wiki_get(file="wiki/index.md")` when needed.
   - Reuse an existing `<domain>/<topic>` folder whenever the discussion fits an established theme; otherwise define a clean `kebab-case` `<domain>/<topic>` path (max 2 levels).
   - Call `add_to_sources` with a comprehensive, self-contained Markdown synthesis.
2. **Case B — User wants a standalone Daily/Weekly Report file**:
   - Call `markdown_create` to write the report file, then call `markdown_read` to verify.

## 2. Provenance, Conflict & Anti-Hallucination Rules (Javis OS Discipline)
1. **Provenance Gate (`provenance`)**:
   - Use `"user"` for explicit user decisions/preferences, `"source"` for external document/web findings, or `"verified_synthesis"` for confirmed technical conclusions.
   - **NEVER** promote unverified assistant-only speculation as established Wiki truth (`provenance="assistant"` is hard-blocked by `add_to_sources`). Put unverified hypotheses into `open_questions: [...]`.
2. **Non-Destructive Conflict Handling (`conflict_with`)**:
   - If the discussion contradicts an existing Wiki page, pass its path/slug in `conflict_with` so Flow 1 (`wiki_ingestor`) preserves both perspectives under a `> [!WARNING] Contradiction / Caveat` callout rather than overwriting prior knowledge.
3. **Numeric & Version Identity Preservation**:
   - Keep version numbers, model generations, and numeric identifiers exact (e.g., never conflate `GPT-4` with `GPT-5` or `v1` with `v2`).
