# Knowledge fixed Job verification

Baseline: `4317b8fe79d352cf06dd652b5b2970fc294dd366`.
User confirmed P0–P7 and JobModels/native executor, SQLite/vault through KnowledgeCore, authenticated Admin API and Jobs/Settings Web seams. Every check uses `scripts/agent-test.sh`; failures remain in wrapper logs under `$TMPDIR/ai-agent-voice-test-logs`.

## Passing targeted gates

| Gate | Evidence |
| --- | --- |
| Historical snapshot/pins, explicit execution version, v1 regression | `agent-test.run.K4u2xA`, `agent-test.run.B29wo7` |
| Immutable catalog, independent Root/Child loads, scoped instruction inputs | `agent-test.run.zSSxeK`, covered again by Background regression |
| Canonical provenance, all Source hashes/UTF-8 spans, CRLF, legacy Memory/Admin compatibility | 50 passed: `agent-test.run.pUrYhU` |
| Background executor/Skills/native tools/recovery/regression | 173 passed, 2 ignored: `agent-test.run.QgOdzZ` |
| Latest native operations, multi-Source edit, staged recovery, authenticated resolution and actual subprocess kill | 9 passed, 1 subprocess helper ignored by default: `agent-test.run.PpeUIA` |
| Source coverage gaps/duplicate ranges/foreign hash/orphan result/EOF | `agent-test.run.1CzkdN`; native paths rerun after this tightening |
| Full generic Unicode/escaped output -> immutable artifact -> Session continuation; foreign Job denied; old contract retained | `agent-test.run.Ct30Xk`, then Background regression |
| Chat Completions and Responses local wire fixtures with Structured Outputs disabled | `agent-test.run.Gqqiek`, then Background regression |
| Settings CAS/native opt-in, Markdown safety/fallback, current draft resolution, deep-link navigation | Full Web 73 files / 335 tests: `agent-test.web.qCxTku` |
| Web typecheck and production bundle | `agent-test.web.KH3sAN` |
| Bundled skill validator | `agent-test.run.rVOQjE` |
| SQLx ownership | `agent-test.run.GtL1li` |
| Workspace Clippy, warnings denied, after final cleanup | `agent-test.run.vsEyaf` |
| Rust formatting | `agent-test.run.LMOzMk`; formatting applied after the final coverage fixture |

## Recovery evidence boundaries

`knowledge_process_kill_recovers_publication` launches the test binary in a separate process with a temporary cwd outside the repository, kills it after canonical Wiki commit and before Step ACK, then starts a fresh runtime. The accepted revision is recovered exactly once, the Job fails stop, and the terminal report retains the publication. `agent-test.run.BBJQZV` and the later native regression cover this test. The ignored `knowledge_process_kill_fixture` is a subprocess entry point executed by that parent test, not a missing acceptance case.

Separate deterministic in-process checkpoints cover draft commit, staged Wiki commit, draft resolution commit/lost ACK and newer work preservation. They are not described as process-kill evidence. The deployed Installer/Runner qualification fixture remains ignored without its external prerequisites. Optional remote LLM smoke and production deployment are NOT_RUN.

## Preserved red runs

- Contract version absent before implementation: `agent-test.targeted.5PUhfz`.
- Native Settings wire opt-in before implementation: `agent-test.run.kROhHo`; corrected: `agent-test.run.uy7F3T`.
- Native Settings Web control absent: `agent-test.run.Jvxd4u`; corrected and included in full Web.
- Generic output fixture initially used a Root where only Child plugin execution is authorized, then an out-of-range fixture Step limit: `agent-test.run.p73wOy`, `agent-test.run.XpoaLM`; fixture corrected to the public Child seam and supported budget, green in `agent-test.run.Ct30Xk`.
- Background admission fixture expected Core revision 1 for a newly admitted contract 2 Job: `agent-test.run.PzjMLi`; expected pinned revision updated, green in `agent-test.run.QgOdzZ`.
- Clippy findings fixed by shared call context, grouped plugin outcome and simpler conditions: `agent-test.run.vrXEWw`; green in `agent-test.run.vsEyaf`.

## Final gates

- Full Rust workspace: PASS, `agent-test.full.TTuGwi` (final counts below).
- Standards and Spec review: complete; fixed baseline above, staged working-tree input before commit.
- Scoped commit on the existing branch follows the final formatting check. No push or deployment requested.

## Cleanup audit

V1 execution and Review remain reachable and required by saved/queued jobs and legacy Settings. They are retained intentionally. The shared Markdown renderer has one implementation; the Knowledge path is a compatibility re-export. No new dead-code suppression is added. Native write authority remains private to the Knowledge adapter; shared Skills never grant tools. Output continuation uses the existing immutable Job Artifact store and Session authority, not a new run ledger.

## Final gate outcomes and review fixes

- Full workspace completed: **996 passed, 0 failed, 14 ignored** across 61 test/doc-test suites (`agent-test.full.TTuGwi`). This full run preceded the review repairs below.
- Review repairs were followed by affected regressions: **Background 176 passed / 2 ignored** (`agent-test.run.1JcRVK`), **Knowledge 51 passed** (`agent-test.run.pM52vr`). No unrelated suite was rerun after these focused repairs.
- Recovery regression with 256 older unknown operations and 256 older unknown resolutions: PASS (`agent-test.run.tObN9Q`). The earlier fixture (`agent-test.run.AQf7v1`) accidentally attempted a second active Job for the same Source and hit the existing admission guard; it now uses a different Source to reach the intended path-ownership seam.
- Output below 60 KiB with a smaller effective context preserves a complete immutable artifact and delivers bounded continuation: PASS (`agent-test.run.ShDcKi`), then Background regression.
- Workspace Clippy with warnings denied after repairs: PASS (`agent-test.run.jtYZVH`). SQLx placement: PASS (`agent-test.run.8XAYVe`). Final formatting check recorded below.
- Standards and Spec review were run independently against the staged changes from the fixed baseline. Initial findings and their resolution are in [review.md](review.md). Re-review: **Standards 0 hard/open findings; Spec 0 open findings**.

Repairs: unknown intents protect ID **and path**; recovery advances stable keyset cursors even when old rows remain unknown; generic execution reserves against the actual Child observation budget and creates an artifact whenever full delivery does not fit; Source/Wiki/Artifact share one byte-range union; tests live in separate files. Native v1/Review remains reachable; no dead helpers or suppression were retained from these repairs.

P0–P7 local implementation/qualification/cleanup is complete. Remote LLM smoke, deployed Installer/Runner qualification and production deployment remain NOT_RUN; migrations still default existing installations to v1. Commit follows final formatting; no push requested.

Final formatting PASS: `agent-test.run.qkOKgC`; staged whitespace check PASS: `agent-test.run.eolgkx`.

## Pull integration — 2026-10-19

Pulled upstream `acc246be` (including `508835f5`) into local `ae9d1539` on the same branch. Resolved three text conflicts by retaining both intents: TTS eligibility/provider pin plus mutable versioned catalog in Job admission; TTS exclusion plus revision-2 expectations in the admission fixture; `Invocation`/TTS dispatch plus modern output caps in Core execution. Adapted the heavy-output fixture to the new Core invocation signature. TTS retains frozen contract revision 1; text/read operations admitted with instructions retain revision 2.

Both branches had migration 0126. The published upstream `0126_core_tts_provider_pins.sql` is retained unchanged; the local Knowledge execution-version migration is now `0132_knowledge_execution_version.sql`. Its SQL is unchanged, and schema initialization/upgrade fixtures ran through the merged migration set.

Post-merge checks (all through the wrapper): Rust check PASS (`agent-test.check.cmKt5V`); Background 178 passed / 2 ignored (`agent-test.run.GfKMwV`); Core 25 passed / 1 ignored (`agent-test.run.FDm2D7`); TTS worker unit 7 passed (`agent-test.run.ZkU4ZP`); TTS worker integration 15 passed / 1 ignored (`agent-test.run.pSgszF`); selected Web 6 passed (`agent-test.run.Dvz0TB`); Web typecheck PASS (`agent-test.web.xjcIIh`); server all-target Clippy with warnings denied PASS (`agent-test.run.mpPFSU`); SQLx PASS (`agent-test.run.1tomd5`); formatting PASS (`agent-test.run.e0JeKt`). Full workspace and live native provider/deployment qualification were not rerun for this pull. No push requested.
