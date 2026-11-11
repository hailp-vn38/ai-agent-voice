---
name: wiki-taxonomy
description: "Decides which subjects from a source get their own wiki page, whether a page is an entity or a concept, and which topic folder it belongs to. Load before creating or routing any wiki/ page."
requires_tools: wiki_tree
---

# Wiki Taxonomy

This skill decides **whether** a subject gets a page and **where** it belongs. How the page looks (paths, frontmatter, tags, links, templates) is the `wiki-page-format` skill's job.

## 1. Layers

- `sources/` holds raw sources: immutable, the source of truth.
- `wiki/sources/` holds one source summary page per raw source, at the same relative path.
- `wiki/entities/` and `wiki/concepts/` hold the knowledge, merged across sources.
- `wiki/syntheses/` holds cross-source analyses and filed query answers.
- `wiki/index.md` and `wiki/log.md` are maintained by the tools.
- If your assigned tools include `wiki_schema`, read `SCHEMA.md`; its vault-specific routing rules win over this skill, including custom folders such as `wiki/decisions/`. Agents without `wiki_schema` use `wiki/index.md` and `wiki_tree` to follow the visible vault structure.

## 2. Reuse before creating

Match each subject against the slugs, titles, and `aliases` in `wiki/index.md`. When a page for the subject exists, that page is its home: reuse its exact slug and add any new spelling, abbreviation, or translation to `aliases`.

## 3. Page-worthy threshold

A subject is `Page-worthy: yes` when at least one holds:

- it is central to the source;
- it recurs across sources;
- it carries substantive technical or empirical claims;
- it is a reusable building block for other pages.

Everything else (passing mentions, generic roles, chapter labels) stays as prose inside the page of the subject it belongs to.

Group symmetric pairs and sub-components inside their parent page: `ITH`/`ITL` live in `swing-points`, sub-steps of a model live in the model's page. A sub-component earns its own page only when it meets the threshold by itself.

## 4. Entity or concept

A subject lives in exactly one of the two:

- **Entity** (`wiki/entities/`): a concrete named thing — person, organization, product, platform, language, library, model, dataset, specific tool or indicator (`openai`, `rust`, `metatrader-5`).
- **Concept** (`wiki/concepts/`): an idea, method, algorithm, strategy, pattern, protocol, or phenomenon (`reciprocal-rank-fusion`, `dca-strategy`).

## 5. Topic folders

- Reuse an existing `<domain>` and `<topic>` folder from `wiki_tree` before proposing a new one. New names are `kebab-case`, at most two levels (`<domain>/<topic>`).
- Raw sources and source summaries sit two levels deep (`<domain>/<topic>/<filename>.md`); entities, concepts, and syntheses sit one level deep (`<domain>/<slug>.md`) so topics in one domain share pages. Tools and languages used across domains go in `entities/shared/`.
- Tags follow the same routing, in three facets: the category tag (`wiki/source` | `wiki/entity` | `wiki/concept` | `wiki/synthesis`), 1–3 `topic/<domain>/<topic>` tags reused from the vault, then 2–5 `kebab-case` subject keywords.
- When working as `wiki_ingestor`, for a raw source stored flat at `sources/<filename>.md`, pass `target_wiki_path="sources/<domain>/<topic>/<filename>.md"` to `wiki_ingest_source`. Other agents should return the route recommendation to their parent.
