# Agent Job Platform v1.5.1

Canonical implementation contract: [design](../../docs/agent-job-platform-v1.5.1-integration/docs/agent-job-plugin-platform-codebase-design.md) and [phases](../../docs/agent-job-platform-v1.5.1-integration/IMPLEMENTATION-PHASES.md).

Current work is P0 only. Public test seams are Admin HTTP, JobEngine admission/read, SQLite restart and process deployment-lock acquisition. The controlled foundation harness is crate-local `cfg(test)`; production must remain disabled and return `runtime_unavailable` until P3. No executable plugin, LLM planning or Voice Job builtin is released by P0.
