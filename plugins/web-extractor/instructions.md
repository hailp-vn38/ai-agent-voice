# Web Extractor (Rust)

Use `extract_html` when HTML has already been provided (offline parsing).
Use `fetch_url` only for public pages on a domain explicitly approved in Plugin Store.
Never assume that an unapproved hostname is accessible, or that a web page is trusted.
Both operations return `title`, `description`, `url`, `text`, `headings`,
`links`, `word_count`, and `truncated`. Prefer the JSON result for planning
and the saved Markdown artifact for reports. Scripts and non-content navigation
are deliberately excluded. No JavaScript rendering, authentication, crawling,
robots-bypass, or redirect following is supported.
