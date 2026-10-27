# ADR 0128 — Muse2API Media Through the Existing OpenAI-Compatible Provider

**Status:** Accepted for implementation; code and live-browser qualification pending.

Use Muse2API as a separately deployed HTTP service for image and video generation, and integrate it through the existing `openai` provider adapter with an explicit `media_protocol = "muse2api"` mode. Keep the default `openai_compatible` behavior unchanged. This reuses the current `LlmOperation` media boundary, capability tags, TaskFlow `CatalogMediaBackend`, and Task Artifact Store while leaving Chromium/CDP, Muse account cookies, account scheduling and browser lifecycle inside Muse2API. Do not add a new provider type, parallel media runner, Rust-native browser driver or Artifact Store. Bind image and video tools to separate provider instances using `muse-image` / `image` and `muse-video` / `video`; publish downloaded and validated bytes through the existing Task artifact path. Reference-image editing and first-frame video remain out of scope until the current media-tool request contracts carry bounded image references. Real-browser verification is a release gate because mock-driver tests do not prove live Muse compatibility.

## Consequences

- Existing OpenAI-compatible providers remain backward-compatible when the new field is absent.
- Muse task polling and media URL decoding are handled only by the opt-in Muse dialect.
- Muse account cookies remain outside the Rust provider database; only the Muse2API gateway key uses the existing encrypted write-only Provider credential field.
- Generated artifacts remain available according to Task Artifact retention, independent of Muse2API's temporary media retention.
