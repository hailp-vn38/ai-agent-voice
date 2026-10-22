# Deferred Agent Job V3 background subagent roadmap

Status: deliberately out of scope for native foreground release. Follow [V3 flow](flows/10-agent-job-opencode-v3.md).

`subagent(background=true)` must currently fail with `background_subagent_not_enabled`; do not silently run foreground.

When foreground is accepted: introduce a durable parent inbox with notifications deduplicated by (child_run_id, generation). Child terminal outcome is committed separately from promotion into Root history. Promote a synthetic notification only at a Root turn boundary; never inject mid-provider turn. `finish_job` must gate on unresolved required Child work. Keep fixed ProviderPins, authority, cancellation, deadline, unknown-outcome no-replay and metadata-only SSE.

Deferred tests: B01 late finish; B02 duplicate callback; B03 cancellation/revocation; B04 early Root finish denied; B05 restart between terminalization and inbox delivery. Do not infer exactly-once plugin execution from notification dedupe.
