---
name: wiki-synthesis-filing
description: "Workflow for saving a grounded research synthesis into the LLM Wiki and linking it into the knowledge graph. Load when a user explicitly asks to save a synthesis to the Wiki."
requires_tools: wiki_file_synthesis, wiki_patch_file
---

# Wiki Synthesis Filing (`wiki-synthesis-filing`)

Use this skill when the user explicitly asks to save a research answer, deep comparison, multi-source table, or thesis to the Wiki. `wiki-query` is read-only; this skill owns Wiki synthesis writes.

## 1. Prepare the synthesis

1. Use `wiki-query` to gather and read the evidence. Keep citations and unresolved caveats attached to their claims.
2. Choose a concise `<domain>/<topic>` destination that fits the existing Wiki taxonomy. When that choice depends on existing pages or folders, inspect them with the available Wiki read tools first.
3. Include:
   - Executive thesis and core conclusions.
   - A Markdown comparison table across sources, entities, or methods. Keep a blank line before and after the table; in aliased WikiLinks inside table cells, escape the pipe as `[[path\|Label]]`.
   - Contradictions and caveats in a `> [!WARNING] Contradiction / Caveat` callout.
   - A Mermaid diagram (`flowchart LR` or `flowchart TD`) with double-quoted labels such as `A["..."]` when it clarifies relationships.
   - Relevant `[[entities/...]]`, `[[concepts/...]]`, `[[wiki/sources/...]]`, and `[[sources/...]]` links.

## 2. File and connect it

1. Call `wiki_file_synthesis` to save the synthesis under `wiki/syntheses/<domain>/<topic>-synthesis.md`.
2. Call `wiki_patch_file` on at least one related `wiki/sources/` or `wiki/entities/` page and link to `[[wiki/syntheses/<slug>|<Title>]]`, so the synthesis is connected in Obsidian's Graph View.
3. Report the saved path and any evidence limitations to the parent.
