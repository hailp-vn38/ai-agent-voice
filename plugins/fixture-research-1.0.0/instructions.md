# Fixture Research — Test Plugin Only

This plugin contains **synthetic local posts** and never contacts Weibo, X, or other sites. Use it only to test the Agent Job pipeline, not to answer real-world research requests.

- `collect_topic`: pass `topic` (1–100 characters) and optional `limit` (1–20). The plugin searches bundled fixture records and writes `records.json` and `report.md` to the server-provided artifact directory.
- `simulate_failure`: diagnostic operation. `application_error` returns a structured business error, `crash` exits with status 23, `malformed_output` violates JSONL framing, and `sleep` tests host timeouts.
- Returned `source_id="fixture"` and `synthetic=true` must never be presented as verified live sources.
- The plugin does not use MCP, network, credentials, or background LLMs. Rust remains responsible for admission, isolation, scheduling, artifact validation, and Job status.
