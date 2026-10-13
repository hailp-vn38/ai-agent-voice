# Rust plugins for Agent Job Plugin Store

This feature branch adds support for a **precompiled Linux Rust ELF plugin**
to the existing isolated Docker worker. It does not execute arbitrary Cargo
build scripts on the server. The Store accepts a top-level ZIP with:
`plugin.toml`, `instructions.md`, `schemas/*.json`, and `bin/plugin`.
The fixed entrypoint is `bin/plugin`. The binary must be Linux ELF64 little
endian matching the host's x86_64/aarch64 architecture.

## Packages

| Package | Operations | Permissions |
| --- | --- | --- |
| web-extractor | extract_html, fetch_url | Network hosts example.com and en.wikipedia.org; Admin approval and qualified egress required |
| document-analysis | analyze_document | Offline only |

Use GitHub Actions workflow **Rust Agent Job Plugins** on this branch to build
and test on Ubuntu 22.04 x86_64. GitHub downloads each Actions artifact as
an *outer* ZIP archive; extract it first, then upload the *inner*
`web-extractor-1.0.0.zip` or `document-analysis-1.0.0.zip` into Plugin Store.
Alternatively, on a compatible x86_64 GNU/Linux Rust host:

```sh
cargo test --manifest-path plugins/web-extractor/Cargo.toml
cargo test --manifest-path plugins/document-analysis/Cargo.toml
cargo build --release --manifest-path plugins/web-extractor/Cargo.toml
cargo build --release --manifest-path plugins/document-analysis/Cargo.toml
```

The workflow produces `web-extractor-1.0.0.zip` and
`document-analysis-1.0.0.zip` for the Store. Upload each ZIP through the
Plugin Store, review and approve declared permissions, then Test and Activate.
Uploading source directories or source-only ZIPs is not supported: `bin/plugin`
must contain the compiled ELF.

For **Web Extractor**, the listed domains are example allowlist entries.
Change `plugin.toml` before packaging to match your deployment. Admin must
approve the exact set of hosts; `plugin_egress_qualified_hosts` must include
them and the pinned egress proxy must be operational. If the proxy fails
qualification, activation fails closed. No login, JS rendering or unbounded
crawling. `extract_html` parses caller-provided HTML without fetching.

For **Document Analysis**, the current Job invocation uses a JSON envelope
limited to 65,535 bytes. Base64 PDF/DOCX input must therefore be ~45 KiB
or smaller. Arbitrary host file paths and existing artifacts are **not**
mounted into a worker. Larger upload support requires a separate vetted
artifact-input channel and must not be bypassed with filesystem mounts.
PDF analysis extracts selectable text only; scanned files are unsupported.

## Developer contract

Input: one newline-terminated JSON object
```json
{"schema_version":1,"request_id":"attempt-uuid","operation":"extract_html","arguments":{"html":"<main><h1>Demo</h1><p>Hello</p></main>"},"artifact_dir":"/work/out"}
```

Output: exactly one newline-terminated JSON object
```json
{"schema_version":1,"request_id":"attempt-uuid","ok":true,"result":{"url":null,"title":"","description":"","text":"Demo\nHello","headings":[{"level":1,"text":"Demo"}],"links":[],"word_count":2,"truncated":false},"artifacts":[{"path":"page.json","media_type":"application/json","size_bytes":100,"sha256":"..."}]}
```
(Artifact hashes and sizes shown above are illustrative.)

### Deployment requirements and caveats

- Requires the existing Linux rootless Docker, pinned Python slim base image,
  pinned uv image and seccomp preflight; Rust reuses the established container
  base and does not invoke Python. No new MCP dependency.
- Plugin ZIPs are architecture-specific. Currently CI produces **x86_64**;
  aarch64 operators must build and package binaries on a compatible host.
- Building release Rust crates downloads dependencies only on the trusted
  CI/development host, never inside Agent Job installation. Check provenance
  and dependency advisories before distributing the resulting ZIPs.
- Store's Test endpoint currently verifies sandbox/image/permissions but does
  not invoke every plugin operation. Run unit and representative integration
  tests before activating in production.
- No guarantee of arbitrary URL access; egress remains explicitly allowed by
  deployment security policy.
