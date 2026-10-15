# ADR 0116 — Versioned Plugin Value Settings and no-LLM manual execution

Status: Accepted for implementation (2026-10-15). Complements ADR 0115 Core Plugin Activation Policy and ADR 0114 OpenCode Session delegation.

Core activation and External installation readiness are live **start permissions**, not value configuration. Admin plugin settings must not change when a Job has already been accepted. Instead of adding settings to `core_plugin_settings` or `plugin_installations`, persist public values and encrypted secrets in independently versioned immutable records and pin each accepted Job to a specific revision. Existing Core's frozen `plugin_id="core"` maps to its settings owner via static operation-group metadata; no synthetic installation is created.

Manual function execution uses a distinct `plugin_operation_v3` execution mode with one durable Step and no LLM/provider pins. It reuses the same Step-start permission, cancellation, worker tombstones, artifact publication, event stream and Job deletion as Agent V3 while leaving Root/Sub-Agent Session orchestration unchanged. This avoids both arbitrary direct invocation from Admin HTTP and inventing Agent Runs for operations that do not involve an Agent.

External manifests may opt into settings through the optional `[settings]` descriptor in `manifest_version=2`; old strict `jsonl-v1` plugins continue to receive the original envelope. For declared settings, the worker gets a `settings` object resolved from the Job's pinned revision only at dispatch. Secrets are sealed with existing CredentialCipher machinery and omitted from Job documents, event payloads, public settings reads and log fields. The Admin Settings revision is never the Core activation revision or External installation revision.

The design intentionally separates **configuration authority** from **execution authority**: changing values does not revoke in-flight pinned runs, but disabling Core before a Step start prevents new starts per ADR 0115. A Core Step accepted before disable remains allowed to complete. The new manual mode is independent of Agent Background enablement and LLM availability; it retains the deployment scheduler, capacity, deadline and unreaped-worker safety checks.

Rejected alternatives: direct synchronous functions from HTTP, fake Core installations, a second plugin runner, cloning the Agent Root Session for manual jobs, storing secrets in Job arguments, and automatically applying new value settings to queued Jobs.

Credential serialization: `CredentialCipher::seal()` produces a JSON encrypted record. The value-settings repository must persist an authenticated, scope-bound `sealed:` reference (constructed via `credentials::reference()`), not the JSON record itself. The expected scope contains plugin key, settings revision, schema digest, package digest and field. Legacy encrypted JSON entries are decoded only with successful authentication to the pinned scope. Settings plaintext size is independently limited to 16 KiB after JSON serialization.

Manual Job terminal failures have a bounded retry policy. Exhausting durable checkpoint retries fail-closes the runtime so the existing restart recovery can safely mark unknown outcomes. Do not add an unbounded completion loop.

Consequences: historical Agent snapshots remain readable, new plugin-only snapshots omit LLM pins, and immutable config versions must remain while referenced by Jobs. The initial manual operation surface refuses unsupported cross-Job artifact bindings rather than bypassing Job ownership checks.
