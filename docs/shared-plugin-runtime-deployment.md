# Shared Plugin Runtime deployment

Baseline: `refactor/plugins-direct-network-no-proxy`; implementation: `feature/shared-plugin-runtime`. See ADR 0102 for the architecture choice and ADR 0101 for direct outbound networking.

## Layout and configuration

One `voice-plugin-runtime` OCI image is used by two long-lived containers: `plugin-installer` and `plugin-runner`. Only the Server owns SQLite, Job admission, Plugin Store publication and Artifact Store. Server talks over two Unix-domain sockets; neither process needs Docker socket access or the network-host proxy.

Set these paths in the server's real `[background]` section (not just `config.example.toml`):

```toml
plugin_installer_socket = "/run/plugin-installer/installer.sock"
plugin_runner_socket = "/run/plugin-runner/runner.sock"
plugin_installations_dir = "/app/data/plugins/installations"
plugin_staging_dir = "/app/data/plugins/staging"
plugin_attempts_dir = "/app/data/plugins/workspaces"
```

Run Compose with the existing `compose.yaml` and deployment override (`compose.prod.yaml` or `compose.test.yaml`). Both plugin containers must use the **same immutable image digest** in production; `PLUGIN_RUNTIME_TAG=local` is intended for development, not a release pin. Installer mounts staging read-only and installation storage read/write. Runner mounts installation storage read-only and attempt output read/write. Server mounts private control socket volumes and can read installations and attempts. Review Linux UID/GID ownership and read-only mounts on the target.

## Cutover

1. Disable new Background Job submissions, wait for active Jobs to finish or terminalize them using existing cancellation semantics. Take a SQLite snapshot.
2. Build and deploy the shared image and two containers. Confirm both Unix sockets are present and have expected UID/GID/mode, without mounting secrets or Docker socket.
3. Apply migration `0110_shared_plugin_installations.sql` (old per-image installations are set disabled); server must not execute old queued `image_digest` choices. Reinstall and explicitly activate trusted plugin ZIPs using manifest v2.
4. Run the Rust and Python Plugin Store operations and the full Voice ToolCall -> Job -> artifact -> Admin Web tracer.
5. Remove unused per-plugin Docker image settings from production deployment. Keep External MCP policy unchanged.

Rollback requires restoring a compatible database snapshot and the old runtime together. Do not relabel an OCI image digest as an installation digest.

## Packaging and verification

- Python: `manifest_version = 2`, `runtime.kind = "python"`, `python = "3.12"`, pinned `pyproject.toml` and `uv.lock`; Installer creates a private `.venv`.
- Rust: `manifest_version = 2`, `runtime.kind = "rust"`, `entrypoint = "bin/plugin"` (native Linux ELF). Cargo source builds happen in trusted CI, not production.
- The `jsonl-v1` stdin/stdout contract and JSON-schema/artifact bounds remain unchanged. Runner replaces the plugin-provided artifact directory with an attempt-owned directory.
- Operator qualification needs the three shared paths above. Static package checks alone must continue to report runtime `NOT_RUN`.
- Confirm cases for correct/failing output, timeout/cancellation, malformed JSONL, restart ambiguity, parallel Python/Rust operations and direct network behavior on the actual Linux deployment. Report PASS/FAIL/NOT_RUN truthfully.

## Security note

A long-lived Runner container is not a per-plugin sandbox. Package install and Python dependency preparation can execute code. Only Admin-trusted code should be installed or activated. A direct-network plugin may reach internal services reachable from its container.
