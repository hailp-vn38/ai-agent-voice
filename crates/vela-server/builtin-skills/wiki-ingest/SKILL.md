---
name: wiki-ingest
description: "Ingest workflow for the LLM Wiki: read a raw source in full, write its source summary page, integrate every detail into entity/concept pages, cross-link. Load when ingesting any file under sources/."
requires_tools: wiki_ingest_source, wiki_write_file, wiki_patch_file
---

# Wiki Ingest

Ingest integrates one raw source (`sources/<source_rel_path>`) into `wiki/`. The raw source stays the immutable source of truth; the wiki holds what was learned from it.

Each layer has one job:

- **Source summary page** (`wiki/sources/…`) is a **map** of the source: thesis plus an outline of every section, linking to where the detail lives.
- **Entity and concept pages** hold the **detail**: every rule, number, table, parameter, and code block from the source, merged with what other sources said.

Page paths, frontmatter, tags, links, templates, and Mermaid rules live in the `wiki-page-format` skill; page-worthiness and folder routing live in `wiki-taxonomy`. Follow them for every page written here.

Write prose in the user's requested language (default: the source's language). Keep names, identifiers, code, file paths, and URLs in their original form.

## 1. Read the whole source and build an inventory

1. Read `wiki/index.md` and `wiki_tree(scope="wiki")` to learn existing pages, slugs, aliases, and topic folders.
2. Read the source with `wiki_get(file="sources/<source_rel_path>")`. While the output ends with a continuation footer, call again with the `from_line` it names until the last line is read.
3. Build the **inventory** as you read: every H2/H3 section in order (for a source without headings, every distinct topic), with the tables, code blocks, number sets, and rule lists each one contains.
4. Assign each inventory item a **home**: an existing or new page-worthy entity/concept page. Reuse existing slugs. An item with no page-worthy home is marked `raw-only`.

Done when the last line of the source is read and every inventory item has a home or is marked `raw-only`.

## 2. Write the source summary page

Call `wiki_ingest_source` with `source_rel_path`, `title`, `one_line_summary`, `tags`, `related_entities`, `related_concepts`, and a `wiki_markdown_body` containing:

1. **Thesis**: what the source is, its core claim, and how it relates to existing `[[wiki/sources/…]]` pages.
2. **Outline**: one entry per inventory item, in source order. Each entry says in one to three sentences what the section establishes, names what it contains ("14-row failure table", "sample manifest JSON"), and links its home page, or `[[sources/<source_rel_path>]]` when `raw-only`.
3. **Caveats**: limits and contradictions with other wiki pages, in a `> [!WARNING] Contradiction / Caveat` callout.

Keep this page a map: it points to detail, and the detail itself goes to the home pages in step 3.

Done when the outline has one entry per inventory item.

## 3. Integrate the detail into entity and concept pages

For each home page, write the full detail of the inventory items assigned to it:

- Carry every claim, rule, step, condition, number, date, and link. Keep tables as tables with every row; copy code, JSON, config, formulas, and diagrams verbatim.
- Keep each claim attached to the exact subject it describes in the source.
- Cite `[[wiki/sources/<source_rel_path>|<Title>]]` beside what this source contributed.
- **New page**: `wiki_write_file`. Open with one or two self-contained sentences saying what it is and where it applies; include `aliases`.
- **Existing page**: `wiki_patch_file` with `append_text` for an addition; for a full merge, `wiki_get` then `wiki_write_file`, keeping every prior claim and unioning `sources`, `tags`, `aliases`. When sources disagree, keep both claims in a `> [!WARNING] Contradiction / Caveat` callout citing both.
- Write a long page in parts: `wiki_write_file` for the first sections, then `wiki_patch_file` `append_text` for the rest, re-reading the source window when its detail is no longer in context.

Done when every inventory item that has a home appears in full on that page.

## 4. Cross-link and report

1. For each existing `wiki/sources/` page on the same subject, `wiki_patch_file` `append_text` a link to the new source page.
2. Walk the inventory once more against the written pages and patch any gap.
3. End the reply with `Coverage: <integrated>/<total> items integrated, <n> raw-only`, listing the `raw-only` items.
