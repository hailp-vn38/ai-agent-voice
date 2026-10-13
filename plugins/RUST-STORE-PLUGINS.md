# Rust plugins for Agent Job Plugin Store

Rust plugins run in the existing isolated rootless Docker worker through
`jsonl-v1`. Store accepts two ZIP layouts:

- **Native binary:** `plugin.toml`, instructions, operation schemas and
  `bin/plugin` (Linux ELF64 little endian, matching the host architecture).
  This is the distribution format produced by the script below; Cargo and
  dependency downloads run only on the trusted development/CI host.
- **Cargo source:** a locked non-workspace binary project whose entrypoint is
  the plugin ID. Store builds it offline using deployment-pinned Rust images.
  The builder must already contain all locked crate dependencies. The stock
  Rust image in local config has no such cache; use the native ZIP for the fixture.

## Build and check

Requires Rust/Cargo, Python 3.11+ and the server's native build prerequisites.
Runtime checks additionally require the rootless Docker socket and the pinned
Python/uv images configured under `[background]` in `config.toml`.
No extra Python packages are needed.

```sh
# Tests, release builds, reproducible ZIPs and the actual server ZIP validator.
# Defaults to all three plugins and data/curated-plugins (the local config path).
scripts/agent-test.sh run -- python3 scripts/package-rust-plugins.py

# Select one plugin or another output directory.
scripts/agent-test.sh run -- python3 scripts/package-rust-plugins.py web-extractor --output /tmp/rust-plugins

# Static check only: package/manifest/native architecture and valid operation schemas.
scripts/check-rust-plugin.sh --static-only data/curated-plugins/document-analysis-1.0.0.zip

# Real offline operations in the production container sandbox, with JSONL,
# result schema, expected values, artifact count/size/hash/path and cleanup checks.
scripts/check-rust-plugin.sh data/curated-plugins/document-analysis-1.0.0.zip plugins/document-analysis/check-cases.json
scripts/check-rust-plugin.sh data/curated-plugins/web-extractor-1.0.0.zip plugins/web-extractor/check-cases.json
scripts/check-rust-plugin.sh data/curated-plugins/fixture-rust-1.0.0.zip plugins/fixture-rust-1.0.0/check-cases.json
```

Checks use `scripts/agent-test.sh`: output is bounded, the full log contains the
JSON report and the command exits nonzero on failure. An optional third argument
selects another deployment config. Checks use temporary workspaces, containers
and an image tag, and never change the Store database.
Do not run qualification alongside startup reaping on the same daemon.
The script launches containers with the same owner labels as Job workers.

A `package: PASS` static report keeps `runtime: NOT_RUN`. Runtime checks always
use `--network=none` to exercise deterministic/offline cases. This is not proof of
successful URL fetches on the regular Job worker network, activation or a full LLM Job.

Each plugin provides `check-cases.json`. For a new plugin, supply a JSON array
with at least one case for every declared operation:

```json
[
  {
    "name": "sample",
    "operation": "collect",
    "arguments": {"topic": "AI"},
    "expected": {"/topic": "AI", "/count": 1},
    "expected_artifacts": 1
  }
]
```

`expected` keys are JSON pointers within `result`. Use `expected_error` with
the application error code for a negative case; omit successful result assertions
in that case. Inputs must satisfy the operation's input schema. Files returned
as artifacts are read only after the worker is removed and checked by the same
code as Job dispatch.

## Packages

| Package | Operations | Network behavior |
| --- | --- | --- |
| web-extractor | extract_html, fetch_url | `fetch_url` uses the worker's Docker network directly |
| document-analysis | analyze_document | Requires no network for document parsing |
| fixture-rust | collect | Deterministic offline fixture |

GitHub Actions workflow **Rust Agent Job Plugins** uses the same packaging
script and produces native ZIPs for x86_64 Linux. GitHub wraps each downloadable
artifact in an outer ZIP: extract that wrapper and upload the inner plugin ZIP.
Other architectures must build on a compatible host and pass the check script.
The packaging script discovers Cargo's actual target directory, honors
`CARGO_TARGET_DIR`, and uses committed lockfiles with `--locked`.

Upload the ZIP through Plugin Store, then Test and Activate. The curated Store path may also read the generated ZIPs
from `data/curated-plugins`. A bare source directory is not a Store package.
The script publishes both `<id>-<version>.zip` for downloads and `<id>.zip`
for the existing curated install endpoint; both contain the same validated bytes.

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
- Building native release ZIPs downloads dependencies only on the trusted
  CI/development host; source installation uses an already populated offline cache. Check provenance
  and dependency advisories before distributing the resulting ZIPs.
- Store's Test endpoint currently verifies sandbox/image but does
  not invoke every plugin operation. Run unit and representative integration
  tests before activating in production.
- Real Job workers use Docker's default network without a server-controlled egress proxy or host allowlist. Web Extractor still validates input URLs locally and does not follow redirects.
- Network access depends on the Docker/host environment; plugins can attempt connections to reachable internal services. Install only trusted packages.
