# Knowledge native Job qualification

Implementation baseline: `4317b8fe79d352cf06dd652b5b2970fc294dd366`.
The implementation follows [P0–P7](ai-ingest-fixed-job-implementation-v3.md) and [ADR 0125](../adr/0125-knowledge-job-tools-skills-and-publication.md).

## Contracts and rollout

- Existing databases and saved Jobs remain execution version 1. Migration does not enable native execution. Select version 2 explicitly in Knowledge Settings; it uses one Knowledge provider with native Tools and fixed automatic publication.
- Settings v1 retains `publication_policy` and `max_candidates_per_source`; v2 accepts `max_pages_per_source` (1–5) and rejects legacy publication fields. Disabling global Background Jobs does not disable separately authorized Knowledge Jobs.
- Instruction snapshots pin base/role prompts, permitted skill catalog bodies and output policy at admission. Bundled resources are compiled into the server. External skill directories are deployment inputs read at startup; changing them requires a new runtime and affects new Jobs only.
- New generic Jobs pin Core contract revision 2 and the shared output policy. Historical revision 1 keeps its previous limits. Update the server and shared Plugin Runtime together: the new runtime accepts a bounded per-attempt stdout limit, defaulting to the old limit for old callers. No claim is made here that a deployed Installer/Runner was updated.

`background.job_output` defaults: inline 262144 bytes, message 1048576 bytes, transcript 8388608 bytes, native request 2097152 bytes, configured input token budget 65536 and output reserve 8192. The effective request uses the smaller deployment byte bound and a conservative one-byte-per-token allowance after reserve; no model context is inferred from its name. Operators must configure the allowance within the qualified context of every selected route. A small allowance can make a valid profile or Source page unavailable; the server reports capacity instead of dropping history or evidence. Generic complete outputs above the durable Step projection bound are preserved as immutable, quota-accounted Job Artifacts, with Session-owned read continuation. Full generic output is bounded at 4 MiB. Canonical Wiki and final-report limits are unchanged.

## Evidence

All commands use `scripts/agent-test.sh`; the wrapper retains failed logs. Detailed run identities and gate outcomes are recorded in [.scratch/knowledge-fixed-job/verification.md](../../.scratch/knowledge-fixed-job/verification.md).

- Native JobModels fixtures cover full Source delivery, early finish rejection, no-change, create, replay, multi-Source edit, publication and retained partial outcomes.
- Real temporary SQLite/vault fixtures cover canonical provenance, revision CAS, private draft filtering, immutable instructions, long Unicode/CRLF cursor reads, operation intent reconciliation and terminal draft resolution. Authenticated HTTP tests exercise Admin transport; Web tests cover versioned Settings, safe Markdown, navigation and idempotent resolution requests.
- Both actual OpenAI adapters are exercised against local HTTP fixtures with Structured Outputs disabled and private Knowledge Tools enabled.
- A subprocess is killed after canonical publication and before ACK, then a fresh runtime recovers exactly one accepted revision. It runs with a temporary working directory outside the repository, exercising packaged prompts/skills. Separate in-process fault fixtures cover draft and resolution ACK loss, newer draft preservation and staged publication recovery. These are distinct evidence categories.

Workspace-wide Rust tests passed (996 passed, 14 ignored); Web passed 335 tests. Review repairs passed an additional 176 Background and 51 Knowledge tests. Independent Standards/Spec re-review found no open findings. Detailed logs and timing boundaries are recorded in the verification record. Optional remote LLM smoke, deployed Installer/Runner qualification and production deployment remain separate from local source qualification.
