---
name: wiki-lint
description: "Health-check for the LLM Wiki: repair structure (broken links, orphans, frontmatter), merge duplicates, flag contradictions, and audit that pages still cover their sources. Load for a maintenance pass or after a batch of ingests."
---

# Wiki Lint

A lint pass leaves the vault structurally clean and reports where the wiki has lost what its sources say.

## 1. Structure (`wiki_lint`)

Call `wiki_lint` and fix each finding:

| Finding | Fix |
|---|---|
| `uningested_sources` | Report each path as `needs_ingest`; ingesting is the `wiki-ingest` workflow. |
| `broken_links` | Correct the slug with `wiki_patch_file`, or create the target with `wiki_write_file` when it is page-worthy. |
| `orphan_wiki_pages` | Link to the page from a related source summary, entity, or synthesis with `wiki_patch_file`. |
| `unlinked_mentions` | Wrap the first mention with `wiki_patch_file` (`target_text` -> `replacement_text`) as an `[[entities/<domain>/<slug>]]` or `[[concepts/<domain>/<slug>]]` link labelled with the term, leaving the surrounding prose and frontmatter as they are. |
| `missing_frontmatter_pages` | Add `title`, `category`, `summary`, `tags`. |

## 2. Duplicates and contradictions

1. In `wiki/index.md`, find entity/concept pages covering one subject under different slugs. Merge every unique claim, table, code block, and `sources` entry into the canonical page, repoint inbound links, then remove the duplicate.
2. Where two sources disagree, the entity/concept page states both views in a `> [!WARNING] Contradiction / Caveat` callout citing both `[[sources/...]]` files.

## 3. Coverage audit

Audit up to 5 source summary pages per pass: the ones ingested since the last `lint` entry in `wiki/log.md`, then the largest raw sources.

For each, read the raw source's headings and the summary's outline (`wiki_get`), and check:

- every source section has an outline entry;
- each outline entry links a home page that exists and carries that section's tables, numbers, and code, or links the raw source;
- the page's `topic/...` tags and any synthesis linking it match the source's own subject.

Report each page that fails as `needs_reingest: <source_rel_path> — <what is missing>`. A summary with no outline at all fails.

## 4. Finish

Re-run `wiki_lint` until `is_healthy: true`, then log the pass with `wiki_log(mode="append", action="lint", ...)`, listing the `needs_ingest` and `needs_reingest` paths.
