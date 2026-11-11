---
name: wiki-page-format
description: "Page templates for this project's LLM Wiki: exact path, frontmatter, tags, body layout and [[WikiLink]] form for source summaries, entities, concepts and syntheses. Load before writing or patching any wiki/ page."
---

# Wiki Page Format

Every page you write must look like the pages already in the vault. This skill is the **house style**: the templates below are copied from live pages. Deciding *whether* a subject deserves a page is `wiki-taxonomy`'s job; this skill decides *how the page looks*.

## 1. Before writing: read the house style

1. `wiki_schema(mode="read")` — `SCHEMA.md` wins if it disagrees with this skill.
2. `wiki_get(file="wiki/index.md")` — existing slugs, titles and domains.
3. `wiki_tree(scope="wiki", include_metadata=true)` — existing `<domain>` folders and the `topic/...` tags already in use.

Done when you know, for the page you are about to write: its `<domain>`, its `<topic>`, whether its slug already exists anywhere under `entities/` or `concepts/`, and which existing `topic/<domain>/<topic>` tag you will reuse.

## 2. Paths

| Category | Path | Written by |
|---|---|---|
| `source_summary` | `wiki/sources/<domain>/<topic>/<filename>.md` — mirrors `sources/<domain>/<topic>/<filename>.md` exactly | `wiki_ingest_source` |
| `entity` | `wiki/entities/<domain>/<slug>.md` (cross-domain tools/languages: `wiki/entities/shared/<slug>.md`) | `wiki_write_file` |
| `concept` | `wiki/concepts/<domain>/<slug>.md` | `wiki_write_file` |
| `synthesis` | `wiki/syntheses/<domain>/<topic>-synthesis.md` | `wiki_file_synthesis` |
| `system` | `wiki/index.md`, `wiki/log.md` | tools only — never hand-edit |

- `<domain>`, `<topic>`, `<slug>`: `kebab-case` ASCII. Vietnamese names are de-accented (`khong-khi-lanh`, `trung-tam-du-bao-khi-tuong-thuy-van-quoc-gia`).
- One `<domain>` per page, reused from the existing folder list.
- A slug is globally unique across `entities/` and `concepts/`. If it exists, patch that page.
- Pass the full path to `wiki_write_file`: `rel_path="concepts/trading/fair-value-gap.md"`.

## 3. Tags (3 facets, always in this order)

```yaml
tags:
  - "wiki/concept"                 # facet 1: category tag
  - "topic/trading/fair-value-gap" # facet 2: topic/<domain>/<topic>, 1–3 entries
  - "fvg"                          # facet 3: 2–5 subject keywords
  - "pd-array"
```

- Facet 1: `wiki/source` | `wiki/entity` | `wiki/concept` | `wiki/synthesis`. The tools add it for source summaries and syntheses; you write it for entities and concepts.
- Facet 2: first segment is the page's own `<domain>` folder. Reuse a `topic/...` tag already in the vault before inventing one; a source summary uses its source folder (`sources/trading/fvg/...` → `topic/trading/fvg`).
- Facet 3: lowercase `kebab-case`, no `#`, no spaces, no Vietnamese diacritics.
- `qmd.metadata.tags` repeats the top-level `tags` list exactly.

## 4. Links

Body links are raw Obsidian `[[path|Label]]`:

| Target | Form |
|---|---|
| entity | `[[entities/<domain>/<slug>\|Label]]` |
| concept | `[[concepts/<domain>/<slug>\|Label]]` |
| source summary | `[[wiki/sources/<domain>/<topic>/<filename>\|Title]]` (no `.md`) |
| synthesis | `[[wiki/syntheses/<domain>/<file>\|Title]]` (no `.md`) |
| raw source | `[[sources/<domain>/<topic>/<filename>.md\|<filename>.md]]` (keeps `.md`) |

(The `\|` above is only for this table; in prose write a plain `|`.)

- Internal links are always `[[...]]`, written bare — outside backticks, never `[text](file.md)`.
- Inside a Markdown table cell, escape the pipe: `[[wiki/sources/trading/fvg/the-secret-fvg-breakaway-gap\|The Secret FVG]]`.
- Frontmatter `sources:` entries are quoted raw-source links: `"[[sources/trading/fvg/x.md|x.md]]"`.
- Images and videos: `![mô tả](url)`.

## 5. Templates

Prose and headings are in Vietnamese (the vault language). Proper nouns, tickers, code, formulas and file paths stay in their original form.

### Entity / Concept — `wiki_write_file`

```markdown
---
title: "Fair Value Gap"
category: "concept"
domain: "trading"
topic: "fair-value-gap"
summary: "Một câu tự đứng được, dùng làm dòng mô tả trong wiki/index.md."
aliases:
  - "FVG"
  - "khoảng giá trị hợp lý"
sources:
  - "[[sources/trading/sharp-turn-entry/the-sharp-turn-entry-model.md|the-sharp-turn-entry-model.md]]"
tags:
  - "wiki/concept"
  - "topic/trading/fair-value-gap"
  - "fvg"
  - "price-action"
updated_at: "2026-11-03T10:00:00Z"
qmd:
  metadata:
    category: "concept"
    domain: "trading"
    topic: "fair-value-gap"
    tags:
      - "wiki/concept"
      - "topic/trading/fair-value-gap"
      - "fvg"
      - "price-action"
---

# Fair Value Gap

[[concepts/trading/fair-value-gap|Fair Value Gap (FVG)]] là ... (1–2 câu mở đầu tự đứng được: nó là gì, thuộc domain nào, dùng khi nào).

## <Các mục nội dung theo nguồn>

...

> [!WARNING] Contradiction / Caveat
> Chỉ thêm khi có mâu thuẫn hoặc giới hạn bằng chứng; trích cả hai nguồn bằng [[sources/...]].

## Liên kết

- **Source:** [[wiki/sources/trading/sharp-turn-entry/the-sharp-turn-entry-model|The Sharp Turn Entry Model]]
- **Raw source:** [[sources/trading/sharp-turn-entry/the-sharp-turn-entry-model.md|the-sharp-turn-entry-model.md]]
- **Concept liên quan:** [[concepts/trading/sharp-turn-entry-model|Sharp Turn Entry Model]]
```

- `category: "entity"` + `wiki/entity` for named things; everything else is identical.
- `domain` and `topic` appear both top-level and in `qmd.metadata`; `topic` is the page's own topic, usually its slug.
- `summary` has no line breaks and no unescaped `"`.
- `## Liên kết` lists every source summary and raw source the page draws on; one `Source` + `Raw source` pair per source.

### Source summary — `wiki_ingest_source`

The tool writes the frontmatter (`category`, `source_file`, `source_count`, `sources`, `updated_at`, `qmd`) and the `> **Raw Sources:** … > **Summary:** …` header. You supply arguments only:

```text
source_rel_path:    "trading/fvg/the-secret-fvg-breakaway-gap.md"
title:              "The Secret FVG that we don't trade back into (Breakaway Gap)"
one_line_summary:   "<một câu cho index>"
tags:               ["topic/trading/fvg", "breakaway-gap", "market-structure"]
related_entities:   []                               # "<domain>/<slug>", page-worthy only
related_concepts:   ["trading/fair-value-gap", "trading/breakaway-gap"]
wiki_markdown_body: <body below, no frontmatter>
```

```markdown
# <Title>

## Định vị và luận điểm cốt lõi
## Dàn ý nguồn
### 1. <Tên mục trong nguồn>
<1–3 câu: mục này xác lập điều gì, chứa gì (bảng, code, số liệu)>. Chi tiết: [[concepts/<domain>/<slug>|Label]]
### 2. ...
## Giới hạn và mâu thuẫn
> [!WARNING] Contradiction / Caveat
## Nguồn và liên kết
- Nguồn gốc: [[sources/<domain>/<topic>/<filename>.md|<filename>.md]]
- Khái niệm liên quan: [[concepts/<domain>/<slug>|Label]]
```

### Synthesis — `wiki_file_synthesis`

The tool writes the frontmatter (`category: "synthesis"`, `query`, `updated_at`, `qmd`) and the `> **Filed Query:** … > **Summary:** …` header. Arguments: `rel_path="<domain>/<topic>-synthesis.md"`, `query`, `title` (`"Tổng hợp <domain>/<topic>: <chủ đề>"`), `one_line_summary`, `tags` (`topic/<domain>/<topic>` + keywords), and `markdown_body`:

```markdown
# Tổng hợp <domain>/<topic>: <chủ đề>

## Tóm tắt điều hành
## Tiến triển bằng chứng theo nguồn   (bảng: Nguồn | Luận điểm chính | Cách liên hệ | Giới hạn)
## So sánh khái niệm                  (khi có ≥2 concept/entity)
## <Quy trình / phân tích>            (Mermaid tùy chọn)
## Caveat về bằng chứng
> [!WARNING] Contradiction / Caveat
## Kết luận
```

Every source summary, entity and concept the synthesis draws on is cited with a `[[...]]` link.

### Mermaid

Use a multi-line `flowchart TD` or `flowchart LR`, with every node label double-quoted: `A["Vùng cân bằng"] --> B["Cú phá vỡ"]`.

## 6. Updating an existing page

- When the page is a home for a new source's detail, append it with `wiki_patch_file(append_text=...)` as a full section `## <Chủ đề> (theo [[wiki/sources/...|Title]])` carrying the tables, numbers and code, plus a `Source` + `Raw source` pair for `## Liên kết`.
- When the page is only related to the new source, append one bullet: `- [[wiki/sources/...|Title]] (Nguồn: [[sources/....md|file.md]]) — <liên hệ>`.
- On a full rewrite, keep `title`, `category`, `domain` and the path; union `sources`, `aliases` and `tags` (top-level and `qmd.metadata` together); set `updated_at` to now; keep every prior claim and write one unified article.
- When a new source links to a related existing source summary, patch that summary to link back.

## 7. Finish

Run `wiki_lint` after your writes. Done when every page you touched:

- starts with `---` on line 1 and has `title`, `category`, `summary`, `tags` (entities/concepts also `domain`, `topic`, `aliases`, `sources`, `qmd`);
- sits at the path from section 2, with `tags` in 3-facet order and `qmd.metadata.tags` identical;
- links only with `[[...]]` in the forms from section 4;
- appears in `wiki_lint` with zero broken links and is not listed as orphan or missing frontmatter.
