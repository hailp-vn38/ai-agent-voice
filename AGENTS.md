## Agent skills

## Test Execution Policy

- Always use scripts/agent-test.sh for tests and checks.
- Run targeted tests before workspace-wide tests.
- Never execute cargo test directly.
- Never print full build or test logs.
- When tests fail, inspect the saved log selectively.
- Preserve test failures; never ignore nonzero exit codes.

## Library crates

`llm-wiki-rs`, `orchestrator-agent` and `orchestrator-flow` build from crates.io. Read their source in `~/.cargo/registry/src` when a server change needs it.

To change a library, work in its repo under `~/Desktop/project_rust/libs/` and test there:

1. To try it from the server before release, add a temporary `[patch.crates-io]` entry with `git` and `branch` on the feature branch.
2. Publish in dependency order: `llm-wiki-rs`, `orchestrator-agent`, `orchestrator-flow`.
3. Replace the patch with the published versions before merging.

### Issue tracker

Issues are tracked as local Markdown files under `.scratch/<feature-slug>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

The canonical five triage roles use the default label strings. See `docs/agents/triage-labels.md`.

### Domain docs

This repository uses a single-context domain documentation layout. See `docs/agents/domain.md`.
