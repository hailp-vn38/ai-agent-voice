# Rust Container Plugin — implementation and qualification

Baseline: `integration/voice-reminders-direct-tts` at `4527e6b`. Implementation branch: `feature/agent-job-rust-container`.

## What changed

The existing `PluginStore` accepts `rust_container` alongside `python_container`. It builds a package through a runtime-specific offline OCI build, but dispatches **both** via the same `runner::Prepared` + `protocol::parse_response` + Artifact Store. `PluginView.runtime.language` reads from the installed manifest, so existing Plugin Store/Detail pages display **rust** without an API or layout change. No Job, Agent, WS, MCP, database migration or scheduler changes.

## Merge compatibility

The integration branch also accepts prebuilt native Linux ELF packages with `entrypoint = "bin/plugin"`. That path retains ELF architecture validation, the existing Python/uv base pins, and the `agent.plugin.runtime` image label. Cargo source packages use `entrypoint = plugin.id` and the dedicated offline Rust builder/runtime pins. Both paths share the existing runner. Python lock source restrictions apply only to Python packages.

## Required deployment prerequisites

Use dedicated rootless Linux Docker with seccomp. Configure `[background].plugin_rust_builder_image` and `plugin_rust_runtime_image` with local qualified `@sha256:` references and compatible architectures. Builder must contain `rustc`, `cargo`, and offline cached crate sources for the package; example fixture needs `serde_json` v1.0.151 and its locked transitive dependencies. Build-time `RUN --network=none` and `cargo ... --offline` prohibit dependency downloads. The runtime image must support the ABI of the produced binary (glibc vs musl); prefer a compatible glibc runtime when using the default Rust target. Do not use placeholder image references in production.

**Offline cache preparation:** provision a builder image *before* activating plugins, using a controlled deployment build pipeline that resolves the intended Cargo.lock while networking is allowed, then copy the required Cargo cache into the pinned builder image. This environment is deployment-owned, **not** produced from plugin user code, and should be audited for build-script and package-source trust. The Agent Job installer never fetches crates.

## Fixture recipe

1. `python3 scripts/package-rust-fixture.py` creates `plugins/fixture-rust-1.0.0.zip`; the source and lock are version-controlled.
2. Upload ZIP in Plugin Store, approve requested empty network permissions and Activate.
3. With Background LLM Providers configured and global Background enabled, submit a fixture Job using `required_sources=["fixture"]` and `required_capabilities=["fixture.collect"]`. The `collect` operation accepts `{"topic":"AI"}` and creates `records.json` with one synthetic record.
4. Verify `Job -> Step -> Artifact` through the existing Admin Web. Voice testing exercises the same builtin `server_create_job` used by Python.

## Required verification

| Gate | Expected evidence |
|---|---|
| Rust and Python archive validation | `scripts/agent-test.sh targeted rust_fixture_validates_with_cargo_lock_and_no_python`, existing Python ZIP test |
| Invalid runtime rejection | `scripts/agent-test.sh targeted rust_manifest_refuses_mismatched_binary_and_legacy_python_field` |
| Code standards | `scripts/agent-test.sh run -- cargo fmt --all --check`, `scripts/agent-test.sh run -- cargo clippy --all-targets --all-features -- -D warnings`, `scripts/agent-test.sh full` |
| Rust image build | Real rootless Docker build, `--locked --offline`, package/image labels and OS/arch verified |
| Execution | Real `PluginStore::invoke` Rust worker, bounded JSONL, authenticated artifact fetch |
| Regression | Python worker still runs; cancellation/reaper/Voice/Reminder and Web test/build remain intact |

Record failures as FAIL/NOT_RUN. This branch's source changes were authored through the GitHub connector because the current sandbox lacks a cloneable GitHub network connection and Cargo toolchain; **no successful local Rust compile, Docker execution or full E2E claim** is made here.
