# Local verification

Baseline: `0f7941ea2b9f34b8cc2a32d5f5b31dcf09d6e0ec`. Scope amendment: Settings is the only page receiving UI changes; backend P0–P3 is included.

| Check | Result |
| --- | --- |
| Existing Knowledge baseline | PASS |
| Background regression (`--lib services::background`) | PASS: 150 passed, 1 existing ignored |
| Knowledge/Graph/Settings Web regression | PASS: 4 files, 19 tests |
| Workspace Clippy all targets/all features, warnings denied | PASS |
| Rust formatter | PASS |
| SQLx placement | PASS |
| Web typecheck | PASS |
| Web build | PASS |
| Two-axis code review | PASS: 0 mandatory Standards violations, 1 nonblocking design suggestion; 0 Spec findings. See [review](review.md). |
| Full workspace tests | PASS: 61 groups, 971 passed, 0 failed, 13 existing ignored (417s) |
| Full Web suite | PASS: 71 files, 328 tests |

Existing ignored Background test: `fixture_python_runs_through_job_step_and_checked_artifact` requires a deployed shared Installer/Runner and local test volumes. These checks do not claim that operational boundary, a remote model smoke or production-process kill qualification.

Failures were retained in wrapper logs. The broad Background run initially had 11 failures because the new Job Events rebuild omitted existing `agent_decision_accepted`/`agent_decision_rejected` values; the corrected migration retains both, and a real pre-0123 database upgrade test now covers historical rows. The next Background regression passed. Strict lint also found one new needless borrow and a pre-existing nested `if` in `wiki_links.rs`; both were corrected without changing parser behavior.

Publication tests cover proposal/draft/publish-intent/Wiki/Job-ACK interruption, stale index, lost admission ACK, immutable Source quote spans, Unicode byte offsets, edited-draft revalidation, human-draft collisions and missing-vault recovery. All proposal Markdown stays in the existing vault; SQLite stores refs, offsets and checksums.

Final full-suite logs (2026-10-19):

- Rust: `agent-test.full.uybQMy` — 971 passed, 13 ignored.
- Web: `agent-test.web.pF1W3l` — 71 files, 328 tests passed.
- Logs live in `${TMPDIR}/ai-agent-voice-test-logs/`; earlier failing runs are retained.

The 13 existing ignored Rust tests require deployed Installer/Runner, live website/LLMs/device, acknowledged ASR/TTS model assets or native ONNX runtime. Local implementation and deterministic checks are complete; deployment/model/crash qualification remains pending and is not inferred from this result.
