# Review from 4317b8fe

Two independent reviewers used the complete staged diff against `4317b8fe79d352cf06dd652b5b2970fc294dd366`. HEAD still equalled the baseline, so this was a working-tree review before the scoped commit. Review did not itself execute tests; wrapper evidence is in [verification.md](verification.md).

## Standards

Initial report: one documented violation and two judgement calls.

1. `CONTEXT.md:28` requires tests in separate files; the two new inline modules moved to `job_reads_tests.rs` and `observations_tests.rs`.
2. Possible Duplicated Code: Source/Wiki and output artifacts separately merged byte ranges. Both now use `services/range_coverage.rs`; callers retain ownership/hash/revision checks.
3. Possible Primitive Obsession / Repeated Switches: durable command/phase strings alongside `JobChange`. Retained after review: SQL CHECK constrains the closed value sets, native inputs use the typed command, and recovery verifies operation marker, revision and hashes. No current behavior requires a second enum/persistence framework.

Re-review: **0 hard findings, 0 judgement calls requiring work before commit**. SQLx remains owned by `database/`; canonical writes stay in KnowledgeCore; shared Skills do not grant tools.

## Spec

Initial report identified three correctness gaps:

1. Unknown create intent protected only Wiki ID, allowing another ID to select the same path. Native reservation and resolution guards now check both ID and physical relative path. Regression uses another Job/key and another assigned Source to reach the path guard.
2. One `LIMIT 256` batch could terminalize before recovering a later accepted publication. Global recovery now traverses stable keyset pages for operations and resolutions, advancing past rows that remain unknown. Regression seeds 256 older failed Jobs/intents before a lost-ACK publication, and 256 older unknown resolutions before a lost-ACK adopt; the actual outcomes settle exactly once.
3. Generic output below the durable Step threshold could exceed remaining provider context without receiving artifact fallback. The Child now supplies its exact observation budget to execution; complete output becomes an immutable artifact when full projection does not fit. The bounded fallback may omit preview, retaining artifact identity and authorized continuation. Regression covers output below 60 KiB with a smaller configured context.

Re-review: **0 open findings**; regression checks required before commit have passed. No material scope creep identified.

Summary: Standards **0 open**, Spec **0 open**. Local gates pass; external/deployment qualification remains separately NOT_RUN.
