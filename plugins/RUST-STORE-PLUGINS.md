# Rust Plugin Store packages (shared runtime v2)

The shared plugin runtime in [ADR 0102](../docs/adr/0102-shared-plugin-runtime.md) uses **one OCI image and two containers** (Installer and Runner), not a per-plugin Docker image. Plugin ZIPs are Admin-trusted. Their network access follows [ADR 0101](../docs/adr/0101-direct-plugin-worker-network.md): direct container networking without proxy, per-plugin network hosts or grants.

## Package contract

Use `manifest_version = 2`, `[runtime] kind = "rust"`, `entrypoint = "bin/plugin"`, `protocol = "jsonl-v1"`. The ZIP contains native Linux ELF64 executable (`bin/plugin`), agent instructions, operation schemas and `plugin.toml`. Source-only Cargo ZIPs are **not** installed in production. Compile Rust on trusted CI/build hosts with locked dependencies and the intended Linux architecture and ABI.

Python plugins use `[runtime] kind = "python"`, `python = "3.12"` and `src/main.py`; Installer prepares a private `.venv` from `pyproject.toml` and `uv.lock`.

## Build and package trusted Rust plugins

```sh
scripts/agent-test.sh run -- python3 scripts/package-rust-plugins.py
scripts/check-plugin.sh --static-only data/curated-plugins/document-analysis-1.0.0.zip
```

`package-rust-plugins.py` tests/builds Rust packages, emits deterministic ZIPs, validates their schemas and produces both versioned download and curated Store names. GitHub Actions also packages `x86_64` Linux binaries. Build separately for `aarch64`, if needed. The downloaded GitHub Actions artifact is an outer ZIP containing the uploadable plugin ZIP.

To run the production worker qualification, deploy the shared Installer/Runner first and configure `plugin_installer_socket`, `plugin_runner_socket`, `plugin_installations_dir`, `plugin_staging_dir` and `plugin_attempts_dir` in the `[background]` config. Then use the check script with `PACKAGE.zip CASES.json CONFIG.toml`. Static-only checking reports `runtime = NOT_RUN`, never an E2E PASS.

## Developer protocol

Plugin stdin: exactly one newline-terminated JSON object, including `schema_version=1`, `request_id`, `operation`, `arguments` and `artifact_dir`.

Plugin stdout: exactly one bounded newline-terminated JSON object with matching `request_id` and `ok/result/artifacts` (or a typed error). Logs go to stderr; do not include raw secrets or documents. The runner replaces `artifact_dir` with a verified attempt output directory. The server validates all artifact paths, size, hashes and output schemas before publication.

## Operational constraints

- Runtime does not receive Docker socket, server database, or provider secrets.
- Rust plugin binaries must match Linux CPU/ABI and be dynamically compatible with the Python-slim-based shared image (or be statically linked).
- Runner attempts share a container; this is not a strong per-plugin sandbox. Only install reviewed trusted packages.
- The current JSONL request limit is 65,535 bytes, output is 64 KiB, and artifacts are bounded by existing Job policy. Large binary PDF/DOCX inputs still require a separate artifact input interface.
- Installing/upgrading packages used by active Jobs remains blocked. Activation still requires an Admin revision check.
- For production cutover and rollback, see [deployment guide](../docs/shared-plugin-runtime-deployment.md).
