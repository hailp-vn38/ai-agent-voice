# ADR 0126 — yt-dlp CLI as a provisioned Core runtime

**Status:** Accepted for implementation, 2026-10-19
**Branch:** `feat/core-yt-dlp-cli-plugin`
**Related:** ADR 0114 Core Operations, ADR 0115 Core activation, ADR 0115 Job-owned Artifacts, Flow 11 and Flow 12.

## Context

Agent Job V3 has a compile-time Core catalog, durable policy and native Child ToolCall execution. YouTube metadata, transcripts, playlist inspection and downloadable audio require an extractor that changes independently of Rust. Running a Python worker or packaging four External Plugins would duplicate existing ownership. Core only has one source of enablement authority, while bundled tools may be unavailable on a freshly installed host.

## Decision

1. Add exactly one compiled group `core.yt_dlp` and four operations `yt_dlp.get_info`, `yt_dlp.get_transcript`, `yt_dlp.download_audio`, `yt_dlp.get_playlist`. Job frozen choices retain `kind=core, plugin_id=core, contract_revision`.
2. Invoke the standalone `yt-dlp` executable from Rust using `tokio::process::Command`; run no shell, Python worker or External Plugin Store installation. Only canonical public YouTube HTTPS video/playlist links qualify.
3. Separate **Core policy** (`enabled`, revision, SQLite CAS) from **Availability** (a verified, locally provisioned executable bundle). A new Job freezes the Core operations only if policy enables the group and the dependency bundle is available. A previously authorized Step can finish after disable. Executing a frozen operation with missing binaries fails with a known dependency error, not `unknown_outcome`.
4. Control plane provides pinned automatic binary provisioning (standalone yt-dlp + FFmpeg/ffprobe + Deno), verifying expected SHA-256 before health checking and atomic activation. Binary versions belong to local persistent server data, never Job Artifacts or system PATH. Live Steps retain immutable version-directory paths.
5. Data plane uses the existing bounded Core execution slot, deadline, cancellation token and Step/Attempt ledger. Child process descendants must exit before an attempt workspace is deleted. Retry flags are supplied to yt-dlp, not to a new Rust retry loop.
6. Core returns `InvokeResult` containing small structured JSON and optional attempt-owned files. Existing V3 publication assigns artifact UUIDs and publishes Job-owned immutable artifacts; the persisted result is enriched with references only after artifacts receive IDs.
7. Reuse `PluginStorePage.vue` and `PluginManagePanel.vue` presentation. Dependencies and Repair are additional Core metadata/actions, not a new installation or a new page. Manual Update is deferred.
8. V1 deployment matrix is pinned Linux x86_64 glibc; other platforms must explicitly report `unsupported_platform` until verified signed/pinned distribution manifests exist.

## Alternatives

- **Rust yt-dlp wrapper:** still needs a compatible external extractor and adds a crate version seam.
- **Embedded Python / External Worker:** duplicates the present Core Step execution and Store installer.
- **System-wide apt/pip on startup:** changes host state, needs elevated privileges and is nondeterministic.
- **One generic dependency manager for all plugins:** hypothetical seam without a second adopter.
- **Inline audio/base64 or unbounded LLM output:** conflicts with durable Job Artifact and context limits.

## Consequences and limitations

Binary downloads require outbound network once and a writable persistent data volume; verified cached versions work when restarted offline. The bundle's pinned version may become stale when upstream extractors change; a server release can update the pin, while a failed verification keeps the previously active version. The initial FFmpeg build is LGPL and distributed by a third party; operators must review its distribution license and trusted binary provenance. The current Linux provisioning flow invokes the host's `tar -xJf` for a hash-pinned FFmpeg archive. Hosts without that extraction utility report a provisioning error.

Input host allowlisting alone is insufficient SSRF protection if the extractor follows redirects; deployment network egress must independently block internal targets. This does not make open extractor access appropriate for untrusted/public tenants. Downloaded metadata and captions are untrusted data rather than instructions for LLM tool use.

Core enablement is never rewritten due to dependency failure. Runtime Availability is not a permissions grant. Job-native ToolCalls, V3 Sessions, Knowledge publication and voice pipeline remain unchanged.

## Qualification amendment (2026-10-19)

The review-driven implementation retains the single Core catalog and Artifact publishing seam. Binary readiness now revalidates executable file hashes from an activation manifest, and provisioning holds a Linux advisory file lock across installation. A pinned FFmpeg archive is not unpacked wholesale: only explicitly matched executables are streamed under a per-file output bound. Attempt workspaces have an owning PID marker and a startup reconciler that refuses cleanup whenever process ownership is uncertain.

Tool normalization now uses bounded JSON-line playlist output, conservative automatic-caption overlap handling, full raw metadata and normalized transcript artifacts, and FFprobe-validated audio attributes. These changes do not authorize new hosts, login cookies, DRM bypass, arbitrary scripts or installation outside private persistent data.

An extractor's own HTTPS redirects and ancillary video CDN requests cannot be fully constrained by canonicalizing the initial user URL. The security boundary for untrusted tenants therefore requires an independent deployment network policy that rejects loopback, RFC1918, link-local and cloud-instance metadata destinations after DNS resolution and redirect changes. Until that gate exists, the plugin is only qualified for the currently trusted single-owner server.
