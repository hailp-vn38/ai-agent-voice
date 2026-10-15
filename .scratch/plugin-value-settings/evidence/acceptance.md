# Plugin value settings / manual Jobs acceptance

Baseline: `8173476c`. All checks ran through `scripts/agent-test.sh`; full logs remain under `/tmp/ai-agent-voice-test-logs/`.

## Checks

| Gate | Result | Log basename |
| --- | --- | --- |
| `cargo fmt --all`, then `cargo fmt --all -- --check` | PASS | `agent-test.run.BSaBAA`, `agent-test.run.v0iTr5` |
| `cargo check --workspace` | PASS | `agent-test.run.jfG1cm` |
| `cargo test --workspace` | PASS: 900 passed, 0 failed, 13 ignored across 61 suites | `agent-test.full.IW1BeH` |
| Web typecheck | PASS | `agent-test.web.CMeUuC` |
| JobUnifiedTimeline | PASS: 9 tests | `agent-test.run.Se0xap` |
| Legacy DAG/V2 populated database migration | PASS: 1 test | `agent-test.run.im6Xvj` |
| Manual Job tests | PASS: 6 tests | `agent-test.run.kgdV5e` |
| Plugin qualification tests | PASS: 5 tests | `agent-test.run.2bAnqD` |
| External Plugin through deployed Installer/Runner | PASS: 1 explicitly enabled live test | `agent-test.run.GdcW5S` |
| Populated deployment database backup migration | PASS | `agent-test.run.ctOIBs`, `agent-test.run.YhVMVn` |
| `git diff --check` | PASS | `agent-test.run.zDeKip` |

## Populated database migration

Used Python's SQLite backup API with a read-only connection to `data/voice-agent.db`, then upgraded the private copy through the production `Database::connect` startup migrator. The source database was not upgraded.

Schema advanced from **119 to 121**. Row counts and SHA-256 fingerprints of every pre-existing table's complete row contents match before/after: **44 tables preserved**. This includes 6 succeeded V3 Jobs, 6 Steps, 12 Agent Sessions, 12 Agent Runs, 74 messages and 188 events. This deployment copy had no artifact ledger rows; the separate DAG/V2 migration test exercises populated artifact rows, retry references, indexes and immutability constraints.

`PRAGMA foreign_key_check` returned no violations; `PRAGMA integrity_check` returned `ok`.

See [database-migration-report.json](database-migration-report.json) for counts and fingerprints. No database rows, credentials or plaintext settings are included in the report.

The private database backup and transient state file were removed after successful verification. Only the report and reproduction sources remain in this evidence directory; the temporary Cargo example was also removed.

Reproduction helpers: [database-copy.py](database-copy.py) creates/verifies a consistent backup; [migrate-copy.rs](migrate-copy.rs) runs the real startup migrator. To rerun, temporarily copy the Rust helper into `crates/voice-agent-server/examples/acceptance-migrate-copy.rs`, run the Python `backup` action through the test wrapper, set `ACCEPTANCE_DB_COPY` to the generated state's `path`, run the example through the wrapper, and run the Python `verify` action through the wrapper. Remove the temporary example afterward.

## Live External Plugin gate

Explicitly ran `services::background::tests::execution::fixture_python_runs_through_job_step_and_checked_artifact` with `--ignored --exact`, using the existing local Installer/Runner sockets and their mounted staging/installations/workspace directories.

The gate installs and activates the Python fixture, completes a manual `plugin_operation_v3` Job while Background is disabled, verifies no Agent Runs and downloads both nonempty artifacts. It also exercises the existing planned Step path, rejects package replacement while pinned, and checks timeout termination records `unknown_outcome` / `worker_timeout` without marking the installation unhealthy.

## Corrections required to reach the gates

- Applied workspace Rust formatting.
- Imported the existing `engine::now` helper and removed an unused import.
- Updated the legacy Agent snapshot fixture with `mode: None`.
- Included the manifest's new `schemas/settings.json` in the qualification fixture package.
- Made Plugin installation and execution honor configured `plugin_staging_dir` and `plugin_attempts_dir`, retaining database-local defaults when unset. The live gate failed before this change and passed afterward.
- Extended that live gate with the manual no-LLM Job and artifact assertions.

Initial failures were preserved, including format (`agent-test.run.pFSPYn`), missing import (`agent-test.run.fysiO8`), missing snapshot field (`agent-test.targeted.STdSN2`), missing fixture schema (`agent-test.full.U3OtVd`, `agent-test.run.W5FReH`) and the unmounted staging path (`agent-test.run.slFlmz`).

## Scope limits

The live gate uses the deterministic Python fixture and real Installer/Runner services. It does not qualify every External package, real LLMs, remote services or hardware. No production database migration, deployment or commit was performed.

The workspace's 13 ignored tests remain opt-in. The External Plugin gate was separately enabled and passed; its ignored status in the broad run does not substitute for that explicit runtime evidence.
