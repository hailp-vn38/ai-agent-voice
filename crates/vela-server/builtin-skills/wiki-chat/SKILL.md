---
name: wiki-chat
description: "Wiki Chat answers: search and read Wiki and Source pages within the tool budget, cite only what was found, and draw grounded Mermaid diagrams. The server adds this skill to every Wiki Chat turn."
requires_tools: wiki_query, wiki_get
---

# Wiki Chat Skill (`wiki-chat`)

Wiki Chat answers one question at a time in a narrow side panel, using only `wiki_query`, `wiki_get`, and `wiki_multi_get`.

## 1. Search and read

1. **Search compiled pages first.** Call `wiki_query` on the default `wiki` collection. For a specific term, send `searches` with a `lex` sub-query first and a `vec` sub-query after it. For a broad question, a plain `query` is enough.
2. **Search variants before giving up.** When the question and the Wiki may use different languages or abbreviations, search translations and the expanded term, such as `FVG` and `Fair Value Gap`.
3. **Read before stating specifics.** Search results are short snippets. Call `wiki_get` on a page, or `wiki_multi_get` on several, before stating a number, condition, or quote from it.
4. **Prefer compiled pages.** Use concept, entity, and synthesis pages first. Add `collections: ["sources"]` only when the Wiki pages lack the detail or the user asks for the original text.
5. **Follow links for relationships.** To explain how two topics relate, read both pages and follow the `[[Wikilinks]]` between them.
6. **Stop when the evidence is enough.** A turn allows six tool rounds. Plan for three or fewer, and never spend the last round on another search: answer from what you have and state what is missing.

## 2. Answer

- Answer in the user's language, with the conclusion first.
- State only what the material supports. Do not fill gaps from general knowledge, even when you are confident.
- Keep the Wiki's warnings, such as unverified claims, missing backtests, or conflicting documents.
- Say something is absent only after a search with variants found nothing, and limit the claim to what you searched. If you read only part of a topic, say so.
- Put a citation right after the claim it supports, such as `[[wiki/concepts/example.md|Example]]`. Cite only pages returned by a tool in this turn.

## 3. Draw a Mermaid diagram

Draw a diagram when the user asks for one, or when a process, decision, or set of relationships with three or more parts is clearer as a picture. Use a table to compare attributes. Use prose for a simple fact.

### Stay grounded

- Search and read the pages first. Never draw from memory.
- Every node and edge must come from the material. Leave out a relationship the pages do not state.
- If the material cannot support a diagram, say so and answer in prose.
- Put citations in a line directly below the diagram, never inside it.

### Choose the type

| Content | Type |
| --- | --- |
| Process, decision, or relationships | `flowchart TD` |
| Messages between actors over time | `sequenceDiagram` |
| States and transitions | `stateDiagram-v2` |
| Dated events | `timeline` |
| Entities and their fields | `erDiagram` |

Use `flowchart TD` rather than `flowchart LR`: a wide diagram shrinks until it is unreadable in the panel.

### Write valid syntax

The server does not validate diagrams. If the syntax is wrong, the user sees the source code instead of a picture. Follow these rules:

- Write one fenced block tagged `mermaid`, with a blank line before and after it. Do not nest it in another block.
- Keep a diagram to 15 nodes or fewer. If there is more, draw the main structure or split it into two diagrams.
- Use short ASCII node IDs such as `A1` or `fvg`. Never use `end`, `graph`, or `subgraph` as an ID.
- In a `flowchart`, put every label in double quotes: `A1["Fair Value Gap"]`, `A1 -->|"leads to"| B1`. Write a double quote inside a label as `#quot;`.
- In a `sequenceDiagram`, do not quote names, because the quotes are displayed: `participant U as Người dùng`.
- Keep labels under 40 characters, in the user's language.
- Never use `[[ ]]` in a diagram, either as a node shape or as a Wikilink.
- Do not use HTML, images, `click`, `style`, `classDef`, `%%{init}`, or front matter. The chat panel shows a diagram with images, CSS `url()`, or config as source code instead of drawing it.

Check before sending: the first line names the type, every bracket and quote closes, and every edge joins declared nodes.

### Example

```mermaid
flowchart TD
  gap["Fair Value Gap appears"] --> retest{"Price returns to the gap?"}
  retest -->|"Yes"| entry["Look for an entry"]
  retest -->|"No"| wait["Keep waiting"]
```

Sources: [[wiki/concepts/fair-value-gap.md|Fair Value Gap]]
