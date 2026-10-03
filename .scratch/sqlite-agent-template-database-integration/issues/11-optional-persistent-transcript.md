# 11: Optional persistent transcript

**What to build:** Operator có thể opt in text archive with bounded retention and authenticated history operations, without making SQLite history authoritative for conversational correctness.

**Blocked by:** 02: Admin control-plane shell và Agent/Device CRUD; 05: Database-backed Device admission; 07: Provider Load Plan và Effective Session Profile; 08: Session-local Template switching.

**Status:** done

- [x] History capture default is off; when off, no transcript enqueue/write occurs, while existing archive retention and authenticated history operations remain available when database/Admin API are enabled.
- [x] HistoryWriter records only accepted final user text and assistant text after matching normal writer closure; it never stores ASR partial, LLM delta, prompt, tool arguments/results or audio. Each write takes the admission `EffectiveSessionProfile` identity snapshot and current active template ID from the session-local switch state, never queries live DB or reconstructs attribution later.
- [x] Bounded try-send writer drops individual queue-full/closed/database/shutdown records with bounded metrics and never waits, retries, fails turn, closes WS or mutates RAM Dialogue History.
- [x] Retention uses UTC created-at cutoff at startup and every 24 hours; cleanup is outside realtime path, aborts one contended run and waits for next schedule. Scoped history purge requires exactly one scope and explicit all-history confirmation.
- [x] Router/WebSocket tests prove disabled capture, normal versus aborted assistant write, partial exchange tolerance, template attribution across an applied/pending switch, retention/purge/auth behavior, exclusion of tool arguments/results, and history independence from Dialogue History/prompt/tool continuation.

## Answer

### What the archive is

`database/history.rs` owns the optional Persistent Transcript: the bounded hand-off a Voice Session
sends records to, the one task that writes them, and the retention job that keeps the archive
bounded. Read and purge live in `app/admin/history.rs`, beside the other Admin API surfaces and the
audit transaction that records a purge.

Nothing reads the archive back into a Conversational Turn. That is the whole design: Dialogue
History stays in RAM, so a dropped record costs the archive one row and nothing else. It is why the
hand-off is `try_send` and never an await, and why `SessionActor` holds a `TranscriptCapture` rather
than a pool or a repository.

`TranscriptCapture` is bound once, at the WebSocket boundary, by `AppState::transcript_capture`. It
carries the admission identity (`device_id`, `agent_id`) the Effective Session Profile already
resolved, plus the session's own monotonic `sequence`, so `UNIQUE(session_id, sequence)` holds
without the writer ever asking the database for the next number. A session admitted with
database-backed admission off has no Device or Agent row to attribute a record to — an archive row is
foreign-keyed to both — so it is never bound at all.

### Where the two records are produced

The user record comes from `commit_user_text`, the one seam where final ASR text and typed `detect`
text are both accepted for a turn. An ASR partial never reaches it, so it cannot reach the archive.

The assistant record comes from `on_writer_event` on `TurnClosed { outcome: Normal }`, immediately
before the boundary actions, which is what makes the attribution rule fall out of the code: a switch
the turn armed is still *pending* at that point, so the record keeps the Template the turn actually
ran on, and the next turn records the one it switched to.

`PendingDelivery` grew an `archives_as_assistant` flag, and that flag is the one thing this ticket
added to the actor's own state. A Device MCP tool answered with `direct_tts`, and the session-local
built-in actions, both put a tool's own result where the model's text goes. Dialogue History still
commits it exactly as before — that is RAM conversational state and not the archive's business — but
a tool result is not a Delivered Assistant Response, so it is never archived. Without the flag, a
turn could store the very text the model passed as a tool argument, which is the case the ticket
names explicitly.

### Dropping is the normal path

`HistoryDrop` is a closed set of four classes — `queue_full`, `writer_closed`, `database`,
`text_out_of_bounds` — over a fixed set of atomic counters, with no label a session id, a Device, a
Template or transcript text could reach. `HistoryWriterCounters::is_settled` is the one arithmetic
that accounts for everything: every record the hand-off accepted was stored or dropped by the
archive. A text past `MAX_TRANSCRIPT_TEXT_BYTES` is refused whole rather than truncated, because a
truncated archive entry is indistinguishable from a short answer.

The writer task does **not** stop on the shutdown token. It lives as long as its handles, so a
session still draining inside `shutdown.grace_ms` can keep archiving into the same deadline ADR 0066
allows, and the closed-writer drop is what a finished shutdown turns into once the archive is gone.

### Two policies, two existences

`RetentionCleaner` runs whenever the database does. `HistoryWriter` exists only when
`database.history.enabled = true`, and `HistoryArchive::start` is the one place that decides it, with
a single `then`. So a deployment that never opted in has no archival queue and no archival task at
all — not an idle one waiting for a record that can never come.

That split is the point, and an earlier version of this change got it wrong: it started one
`HistoryArchive` carrying both, which meant a capture-off process still held a writer and its
`mpsc` queue. Guide §21 is explicit that the writer is only started when capture is enabled, and the
privacy boundary is worth stating structurally rather than in a comment. `AppState::transcript_capture`
now asks for the writer and treats its absence as the answer, so there is no second copy of the
capture policy that could drift from the one the archive actually applied.

The cleaner is also the only part of the archive that can touch an existing record, and the only
thing it can do with one is delete it — there is no way to hand a record to it. Its `Drop` aborts the
task, so an archive that goes away leaves no task still deleting rows behind it.

### Retention, and what an abandoned run costs

`retention_cutoff(now, days)` is an absolute UTC cutoff in Unix milliseconds — the same unit the
records are stamped in — so a run that started late or was skipped still deletes exactly the same
set. The cleaner runs once at startup and then every 24 hours on its own task, outside the realtime
path. A contended run logs and is abandoned: busy timeout is the only lock wait in V1, and
maintenance that retried would hold the write lock the archival writes themselves need. The next run
is a day away either way, which is the whole cost of a skip.

What a deployment turning capture off keeps is the retention of whatever it already archived, plus
the authenticated read and the purge.

### One filter that had to fail loudly

The Admin read rejects a filter it cannot honor rather than dropping it: `device_id=0` matched by
nothing would be answered with the *whole archive*, which is the one answer an administrator must
never be handed in place of the one they asked for. The purge has the same rule in a stronger form —
`PurgeHistory::scope` returns `Err` unless exactly one of Device, Voice Session or `all` is present,
and `all` additionally requires `confirm: "PURGE_ALL_HISTORY"`. A confirmation beside a narrower
scope is rejected rather than ignored, because ignoring it would let a request look confirmed and not
be. The deletion and its `history.purge` audit row are one transaction.

`page_bounds` now takes the two bounds instead of a resource query, so the history list declares only
the filters it has; it has no `enabled`.

### Two additions to the guide's config shape

`database.history.queue_capacity` is the only new operator knob, and `MAX_TRANSCRIPT_TEXT_BYTES` the
only new hard bound. Both are deliberate. The queue bound is what makes the guide's own "queue full →
drop" policy exist at all, and a code constant would not be operator-tunable. The text ceiling is
needed because an assistant response is the one unbounded text in the voice path: `llm.prompt_budget`
bounds the request, not what comes back, so without it one turn could write an unbounded row. Both
are validated configuration, like every other bound in this repo.

### Test seams

- `tests/transcript_archive.rs` — 15 tests over a real WebSocket session with a real SQLite
  database: capture off by default *and with no writer at all*, the two archived texts and their
  numbering, a server-default session archiving with no Template, a tool argument that was delivered
  to the client and is not in the archive, an interrupted turn that keeps its user text and loses its
  answer while the same session keeps working, template attribution across a real switch, a session
  with no admission identity, the drop policy under a held exclusive write lock and under a foreign
  key the archive cannot satisfy, retention at startup and again on its schedule, a contended run
  abandoned and the next one still happening, retention still pruning with capture off, and the Admin
  read/purge with its authentication and scope rules.
- Eight unit tests in `database::history` own the drop policy, the identity rule, the monotonic
  sequence, the settle arithmetic and the cutoff — the parts a socket cannot order, including the
  closed-writer drop.
- Three config tests own the new bounds, in the same two places the existing config tests live.

Two tests block the archive on purpose. A `BEGIN EXCLUSIVE` from a second pool with a 30 s busy
timeout is the only way to make "the writer is stuck" a fact rather than a hope, so the queue-full
drop is observed rather than raced.

### One bug the tests caught

`HistoryWriter::try_send` counted an accepted enqueue as `history_written_total`, so the counter named
a stored record while counting a queued one, and the writer counted the same record a second time.
`enqueued` and `written` are now separate, and `is_settled` is what the integration test waits on.

### Review outcome

A two-axis review (standards / spec) ran over the staged diff. Everything substantive was fixed:

- **`history_written_total` was declared and documented but never emitted.** A stored record now
  raises the same `debug!` + metric name a dropped one does, so the documented pair is real.
- **`audit_scoped` restated the audit `INSERT` verbatim.** There is now one insert, and it takes an
  `AuditOutcome` instead of a free `outcome`/`error_kind` string pair — the two are never independent,
  so a mismatched pair should not be representable. The purge's audit row names the real
  `affected_rows` through the same function.
- **`FilterValue` differentiated nothing** (both arms called `push_bind` identically). The filter set
  is now one struct whose `push` adds each present clause, and it is consumed because every value it
  holds is bound into the query it builds.
- **`HistoryWriter::drop_record` was a middle man** — the metrics type logged, so the writer only
  delegated. Counters are now pure and the log lives where the drop is decided, which also let the
  one drop that still holds its record (a database failure) name the session, turn and role that
  produced it without ever naming the text.
- **A blank text was reported as `text_out_of_bounds`.** The bound now reads as what it is — between
  one byte and the ceiling of text that says something — and the doc row says "text ngoài bound".
- **A unit-test message claimed a gapless sequence the code does not guarantee.** `sequence` is a
  counter, not a position: a record the hand-off refuses has already taken a number, so the stored
  rows can have a gap. The `HistoryWrite` field and the assertion now say the half that is
  guaranteed — never a reuse.
- **No flow doc was touched**, which `docs/flows/README.md` requires. `01-websocket.md` now records
  that `session_id` is minted before the upgrade and is the archive's session key too, and
  `04-llm.md` records that the transcript write sits on the same boundary as the dialogue-history
  commit and is skipped for a delivered tool result.
- **A redundant `is_none()`-then-`expect` in the actor** became a single `if let`, and a
  fully-qualified type in `AppState` became the import it should have been.
- **The interrupted-turn test was racy**, and the suite proved it: releasing a gate right after
  sending an `abort` races the socket write, so roughly one run in twelve let the turn's tool round
  commit first. The scripted provider now stalls its first request instead, which is the ordering the
  existing `session_profile` interrupt test already relies on; ten consecutive runs are green.

Three findings were declined, with reasons:

- **`list_sessions_for_device` is not implemented, and that is settled rather than open.** Guide §22
  asks for a per-Device session listing and §25 explicitly does not require every query surface in
  this phase; the ticket's own criterion is "authenticated history operations remain available",
  which they are. It is not a follow-up for this ticket.
- **The metric vocabulary sits in `database/history.rs` rather than `telemetry.rs`.** That module
  documents itself as the one place *External MCP* numbers leave this process, so moving history
  metrics there would contradict it.
- **`queue_capacity` and the text ceiling are additions to the guide's config shape.** Both are kept
  and both are deliberate: the queue bound is what makes the guide's own "queue full → drop" policy
  exist at all, and an assistant response is the one unbounded text in the voice path — the prompt
  budget bounds the request, not what comes back. Both are validated config, like every other bound
  in this repo, rather than constants in the code.

The one remaining tool-level delta is gone: `audit` went from nine arguments to nine again by
replacing the `outcome`/`error_kind` pair, so the workspace clippy warning set is byte-identical to
the pre-change set.

### Contract deviation found after review, and fixed

`HistoryArchive` originally started the writer unconditionally, so a capture-off process still held
an archival queue and task. Guide §21 says the writer is only started when
`database.history.enabled = true`, and while the *behaviour* was already right — nothing was ever
enqueued — the structure was not: the privacy boundary was a config check rather than an absence.
`RetentionCleaner` and `HistoryWriter` are now separate, `HistoryArchive` decides between them in one
`then`, and a WebSocket test asserts the writer does not exist at all with capture off.

### Verification

`cargo fmt --check`, `cargo clippy --workspace --all-targets` at the pre-change warning set, and
`cargo test --workspace` green across 49 test binaries. The live `config.toml` and
`config.example.toml` both load and validate against the new section.
