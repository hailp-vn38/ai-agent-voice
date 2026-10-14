## Agent skills

## Test Execution Policy

- Always use scripts/agent-test.sh for tests and checks.
- Run targeted tests before workspace-wide tests.
- Never execute cargo test directly.
- Never print full build or test logs.
- When tests fail, inspect the saved log selectively.
- Preserve test failures; never ignore nonzero exit codes.

### Issue tracker

Issues are tracked as local Markdown files under `.scratch/<feature-slug>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

The canonical five triage roles use the default label strings. See `docs/agents/triage-labels.md`.

### Domain docs

This repository uses a single-context domain documentation layout. See `docs/agents/domain.md`.
