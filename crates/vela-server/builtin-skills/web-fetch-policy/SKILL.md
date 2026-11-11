---
name: "web-fetch-policy"
description: "Decision gate and extraction workflow for fetch_html and html_extract_* tools. Load when inspecting a user-provided http(s) URL, or when escalating to read a concrete external URL after Wiki query or search/synthesis subagents returned no sufficient answer."
requires_tools: fetch_html, html_extract_article
---

# Web HTML Fetch Gate & Extraction Skill (`web-fetch-policy`)

## 1. Gate: When to Call `fetch_html` (Preconditions)
Call `fetch_html` **ONLY** when **both** a concrete `http://` or `https://` URL is present **and** one of the two trigger branches below holds:

1. **Branch A — Explicit User URL**:
   - The delegated prompt contains a specific `http(s)://` URL provided by the user (e.g., to read, summarize, analyze, or ingest that webpage).
2. **Branch B — Post-Search / Post-Wiki Escalation**:
   - Prior search or synthesis steps (`wiki_query` / `wiki_get` in the LLM Wiki, or prior search/synthesis subagent runs such as `agent_search`) returned **no relevant answer** or **incomplete details**, **AND** surfaced a concrete `http(s)://` source URL that must be inspected in full.

### Hard Guardrails
- **Require a Verbatim URL**: Only pass an exact `http://` or `https://` URL present in the prompt, existing Wiki/source metadata, or search results. Never guess, fabricate, or construct URLs from topic keywords.
- **Local-First for Topic Questions (`wiki_researcher`)**: When asked about a general topic without an explicit user URL, run `wiki_get(file="wiki/index.md")` and `wiki_query` first. Only escalate to `fetch_html` if the Wiki lacks the answer and a concrete reference URL is available.
- **Dedicated Tool Routing**: For `youtube.com` / `youtu.be` URLs, use `yt_list_playlist` and `yt_fetch_transcript` instead of `fetch_html`. For local PDF/Office files, use `anydoc_inspect_pdf`.

## 2. Two-Step HTML Extraction Workflow
`fetch_html` caches the raw HTML in memory and returns structural counts (`article_tag_count`, `p_count`, `h1_count`) rather than the full text. Always follow `fetch_html` with the matching extractor on the **same `url`**:

1. **Step 1 — Structural Probe**:
   - Call `fetch_html(url="https://...")`.
2. **Step 2 — Content Extraction by Page Type**:
   - **Single Article Page (`article_tag_count <= 1` and `p_count >= 3`)**:
     - Call `html_extract_article(url="https://...")` and `html_extract_metadata(url="https://...")` in parallel to extract the title, sapo, headings, body paragraphs, and images.
   - **Category / Listing / Feed Page (`article_tag_count > 1` or index page)**:
     - Call `html_extract_feed(url="https://...", limit=10)` to retrieve `{title, url, snippet}` cards.
     - Select the 1–3 most relevant article URLs from the feed and call `fetch_html` + `html_extract_article` on those specific URLs.
   - **Custom DOM Structure (Fallback when `html_extract_article` is empty)**:
     - Call `html_select_elements(url="https://...", tag="main" | "article" | "div", attr_contains="content", limit=10)` or `html_extract_links(url="https://...", scope="all", limit=20)`.

## 3. Completion Criteria & Output Rules
- **Media Links**: Format every image or video URL in Markdown as `![mô tả hoặc tiêu đề](url)` (with a leading `!`), never `[link](url)`.
- **Read-Only Default**: Return the structured Vietnamese summary (**Tóm tắt cốt lõi**, **Phân tích chuyên sâu**, **Điểm nhấn trực quan**) directly to `daily_root`. Saving with `add_to_sources` is available only to agents that have that tool (currently `source_collector`); that agent calls it only when the prompt explicitly instructs saving to `sources/` or the Wiki. When available and requested, `markdown_body` is the full extracted article (title, URL, every heading, paragraph, table, and image), and the summary stays in your reply. `wiki_researcher` returns the extracted content to `daily_root`.
