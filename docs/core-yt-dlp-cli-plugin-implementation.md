# Core yt-dlp CLI — Implementation and Qualification

**Status:** Implemented on `feat/core-yt-dlp-cli-plugin`; qualification pending CI.
**Reference spec:** `core-yt-dlp-cli-plugin-implementation.md` (2026-10-19, approved after Ponytail Review).
**Base branch:** `feat/knowledge-memory-v3-file-first-index`.
**Architecture:** [ADR 0126](adr/0126-yt-dlp-cli-provisioned-core-runtime.md).

## Confirmed branch delta

The base branch advanced while the spec was written. The reference already includes `core.tts` and the new Job instruction/output spillover path. Before this feature there were **7** Core groups with **13** operations and migrations through `0132_knowledge_execution_version.sql`. This implementation adds the **8th** group and **17th** operation, migration `0134_core_yt_dlp_policy.sql`, without changing the pre-existing TTS or Job output semantics. Integration into `feat/knowledge-memory-v3-file-first-index` preserves its Core ASR addition: the combined catalog has **9 groups / 18 operations**, and yt-dlp uses migration **0134** after ASR migration 0133.

## Delivered interfaces

| Management group | Frozen operation | Contract summary |
| --- | --- | --- |
| `core.yt_dlp` | `yt_dlp.get_info` | Required public video URL; bounded title/description, optional raw metadata artifact |
| `core.yt_dlp` | `yt_dlp.get_transcript` | Languages, offset, limit; manual-first VTT segments plus complete JSON artifact |
| `core.yt_dlp` | `yt_dlp.download_audio` | URL, format=`mp3` or `original`; Job-owned immutable audio file |
| `core.yt_dlp` | `yt_dlp.get_playlist` | URL, offset, limit; flat entries and continuation cursor |

Frozen choice identity is always `kind=core`, `plugin_id=core`, operation `yt_dlp.*` and contract revision `1`. `core.yt_dlp` is only the management and policy ID. Historical Core revision `1`/`2` decoding is preserved.

## Actual seams changed

| Existing source | Change |
| --- | --- |
| `src/tools/core/catalog.rs`, `contracts.rs`, `mod.rs` | Static group, 4 descriptions/schemas/coverage, policy registration |
| `src/tools/core/execution.rs` | Invoke private yt-dlp implementation and validate output |
| `src/tools/core/yt_dlp/{mod,cli,provisioning}.rs` | CLI subprocess, URL allowlist, subtitle and playlist normalization, pinned downloads |
| `src/services/background/execution.rs` | Leave child-process future responsible for cancellation/reap; persist artifact IDs in yt-dlp tool result after staging |
| `src/database/jobs/submission.rs` | Exclude unavailable Core group at *new* Job admission; do not rewrite administrator policy |
| `src/services/background/runtime.rs` | Start missing-dependency provisioning lazily after runtime startup |
| `src/app/admin/plugins.rs` and `mod.rs` | Add dependency status to existing builtin catalog, asynchronous Retry/Repair endpoint |
| `apps/admin-web/src/pages/plugins/PluginStorePage.vue` | Existing Core card/modal, dependencies section and retry; existing two tabs intact |
| `apps/admin-web/src/api/*` | Optional managed dependency fields and typed Repair call |
| `migrations/0134_core_yt_dlp_policy.sql` | Forward-only seed; no old policies changed |

## Provisioning and constraints

V1 pinned and checksum-validated Linux x86_64 glibc bundle:

- Standalone yt-dlp `2026.08.19` from `yt-dlp/yt-dlp`.
- Deno `2.9.7` from `denoland/deno`.
- FFmpeg/ffprobe `n9.0` LGPL static build from `BtbN/FFmpeg-Builds`, checksum-pinned archive.

The installer downloads into private `<db-parent>/plugins/core/yt-dlp/staging`, verifies SHA-256, checks executables with `--version`, then atomically installs `versions/<bundle-id>/bin/` and updates `active.json`. It never requires root, apt, pip, or mutations to system PATH. FFmpeg extraction invokes `tar -xJf` for the preverified archive on the Linux host.

Binary paths resolve to the immutable bundle when a Step begins. Control-plane retry does not hold a SQLite transaction. Initial installation and repair status are visible in the existing Core card. Platforms outside the validated matrix report `unsupported_platform`.

## Safety and Job contracts

- Input accepts only canonical public YouTube watch, short and playlist links over HTTPS.
- Native process uses bounded stdout/stderr, CLI retries, Step cancellation token, timeout and process-group termination; if reaping fails, Job engine fails closed.
- Metadata JSON: max 8 MiB ingestion; transcript VTT max 8 MiB; inline subtitle page max 40 cues and 24 KiB; playlist page max 50 entries; audio max 512 MiB and max 4 h based on available duration metadata.
- Output files always reside in a private attempt workspace and are copied, verified and committed by the existing Job Artifact publication path. Artifact UUIDs are assigned by the server after invocation, not by yt-dlp.
- The plugin neither creates Knowledge pages nor runs ASR; a Job may call Knowledge tools separately.

## Admin and UI

`GET /api/admin/plugins/builtin` provides optional `availability`, `dependencies`, `last_error_code` on `core.yt_dlp` only.

`PATCH /api/admin/plugins/builtin/core.yt_dlp` retains the existing `If-Match` CAS and changes only policy. Enabling starts dependency provisioning asynchronously.

`POST /api/admin/plugins/builtin/core.yt_dlp/dependencies/repair` requests a non-overlapping provision attempt. No uninstall, update or separate Plugin Store installation is introduced. The Core group remains visible under **Installed** at all times.

## Verification checklist

The repository's `AGENTS.md` requires `scripts/agent-test.sh`. Qualify the branch with:

```sh
scripts/agent-test.sh targeted yt_dlp
scripts/agent-test.sh targeted core
scripts/agent-test.sh targeted background
scripts/agent-test.sh
```

Also run `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, frontend typecheck/Vitest and a real provision-and-download smoke on a supported Linux deployment. Source changes committed through GitHub may trigger CI; **do not claim a pass until its completed run has been checked**.

Key scenarios: malformed URL rejected before spawn; manually authored subtitles preferred; timestamps retain millisecond precision; playlist cursor pages do not silently drop entries; cancellation reaps process subtree; checksum mismatch cannot activate corrupted bundle; old disabled Core policies survive migration; Child receives committed artifact references; existing Plugins UI remains visually unchanged apart from the dependency subsection.

## Known qualification gates

1. FFmpeg distribution URLs are pinned by SHA-256 but the upstream `latest` download path is mutable; a future upstream replacement should fail integrity checks and requires an updated server release.
2. Initial supported deployment requires `tar` with xz support. Distroless containers need an explicit extractor or a different packaging strategy.
3. YouTube extractor traffic may follow redirects or request other hosts. Host-input validation is **not** sufficient SSRF protection; use restricted container egress for untrusted/public exposure.
4. External media metadata duration may be absent or wrong; disk and time quotas still apply. Content rights and platform rules remain the operator's responsibility.

## Post-review implementation corrections (2026-10-19)

This subsection records the resolution of review IDs F1–F6 and S1–S7 without changing the approved architecture or creating another plugin system:

- **F1**: `active.json` now contains SHA-256 digests for each installed executable. `ready()` compares actual file hashes before advertising availability. The original archives were already SHA-pinned, and health/version probes still run before activation. For the small personal server the additional synchronous hash checks are intentionally accepted as a reliability cost; measure admission/poll latency before increasing concurrency.
- **F2**: Linux provisioning takes an exclusive `.provision.lock` via `flock`, in addition to the per-process atomic guard; each publication uses an attempt-unique manifest temporary path.
- **F3**: CLI calls record a process-group marker before media extraction; startup under the existing deployment lock verifies UUID-owned workspaces and surviving process ownership before killing a group or deleting files. Unexpected ownership fails closed.
- **F4**: The pinned FFmpeg tarball is listed and **only** the two expected `bin/ffmpeg` / `bin/ffprobe` members are streamed to private files, bounded to 280 MiB per executable. No arbitrary archive directory tree is extracted.
- **F6**: Dependency availability and `last_error_code` are independent; a failed Repair remains visible while the previous immutable bundle is still usable.
- **S1**: Inline video metadata adds channel ID, upload date, language, availability, chapter count and subtitle-language catalogs, and always preserves unmodified extractor JSON in an artifact.
- **S2**: Subtitle selection supports `prefer_manual`; normalized segment pagination includes `truncated`, and overlapping automatic rolling captions are normalized conservatively.
- **S3**: Downloaded audio is checked with FFprobe for an actual audio stream, codec, container and bounded duration before artifact staging. Output includes `source_url`, `codec`, and `container`.
- **S4**: Playlists use `--flat-playlist --dump-json` (one JSON object per line), a bounded `limit+1` probe, `offset` continuation and `has_more` / `total_entries` output with an explicit `skipped_count`. Maximum 100 entries per page.
- **S5**: `PluginManagePanel` remains mounted in Settings; it accepts a Dependencies slot and optional no-value-settings mode. Functions cannot run when the managed CLI bundle is unavailable. New labels are bilingual in the existing i18n catalogs.
- **S6**: Added local fake-CLI tests for playlist boundaries, transcript full-artifact paging, audio FFprobe metadata, checksum tampering, process-workspace recovery and exclusive provisioning. Check the latest GitHub Actions run before claiming suite or smoke success.
- **S7**: This remains a **trusted-owner** feature. Canonical user-input allowlisting is enforced but extractor redirects and secondary network requests are not necessarily limited to the input host. Untrusted/public deployments must deny internal/metadata egress in host/container policy or disable this Core Plugin until a suitable network sandbox is available; do not market the URL validator as a complete SSRF defense.

Do not infer a successful real YouTube extract from the fake process tests. A deployment smoke must separately prove a fresh pinned install, subtitle extraction, MP3 and original audio, private-workspace cleanup, and egress restrictions if public access is intended.

### Orphan recovery fail-closed policy

If the startup reaper cannot prove that a leftover process group is owned by the pinned yt-dlp executable, it writes `recovery-blocked` and leaves the workspace untouched. A blocked bundle is not admitted to new Jobs and the UI reports `workspace_recovery_required`. **The main Voice Server and unrelated Core Plugins continue running.** Repair may re-attempt ownership-verified recovery under the provisioning lock; it never force-deletes an unverified directory. Clearing the marker without proving recovery is unsupported.
