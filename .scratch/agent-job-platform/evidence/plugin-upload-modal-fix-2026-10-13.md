# Plugin upload and modal reset — 2026-10-13

Result: PASS for the fixed HTTP size boundary and modal reopen behavior.

The shared Admin transport applied its 256 KiB JSON cap to multipart plugin installation despite the route having a 21 MiB extractor limit. The transport and extractor now share the upload limit; ZIP payload validation remains at 20 MiB. Other Admin JSON requests retain 256 KiB. Opening the install modal clears its selected file, native file input, drag state and old error; a failed upload can still retain its selection while the modal remains open for retry.

## Regression evidence

All commands ran through `scripts/agent-test.sh`. Logs are preserved under `/tmp/ai-agent-voice-test-logs/`.

| Check | Result | Log |
| --- | --- | --- |
| Original UI reopen after close/failed upload | RED: previous.zip retained | agent-test.run.Nt7IPV |
| Original HTTP boundary with 1 MiB multipart | RED: 413 instead of package validation | agent-test.run.PbUpUY |
| Original live Vite upload boundary | RED: request_too_large for 1 MiB | agent-test.run.f4rE8m |
| Targeted Plugin Store/API client | PASS: 16 tests, including successful/failed install and close/reopen | agent-test.run.9zfD0o |
| HTTP upload regression | PASS: ZIP sizes 32 bytes, 1 MiB and 20 MiB reach validation; upload over 21 MiB and JSON over 256 KiB retain 413/request IDs | agent-test.run.pfJxAY |
| Background HTTP tests | PASS: 4 | agent-test.run.9SzA7W |
| All Admin Web tests | PASS: 240 / 57 files | agent-test.web.V2aUsn |
| Web typecheck | PASS | agent-test.web.Odas5B |
| Backend binary build | PASS | agent-test.run.igjFjM |
| Live Vite and direct backend repro after process reload | PASS: small and 1 MiB multipart reach authentication (401 without credentials) | agent-test.run.3qyArT |

The running backend was reloaded during this work; no existing server process was stopped by this agent. The live probe uses no credentials and cannot install a package. No real installation, permission approval or artifact/database mutation was performed by it. Full workspace and a real browser install are outside this regression run.

Reproduction script: `plugin-upload-size-repro-2026-10-13.py`, intentionally local evidence with the reported host URL. Run via `scripts/agent-test.sh run -- python3 .scratch/agent-job-platform/evidence/plugin-upload-size-repro-2026-10-13.py`.
