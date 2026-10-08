# Speaker Recognition V1 — Web Enrollment, ESP32 Observe and Qualified Required

Status: resolved (all 19 tickets merged into integration/speaker-recognition)
Date: 2026-10-07
Baseline: main at 28caaa92becd9efc2c492eba7906b902b9220b63
Sources: implementation guide, discussion Q1–Q25, and ADR 0077–0081.

## Problem Statement

An admitted Device identifies the connection, but does not identify the person speaking. The operator cannot currently enroll a person's voice from Admin Web, compare that voice with audio received from ESP32, or restrict use of an Agent and its Templates to explicitly permitted Speakers.

Adding a voice match alone would leave important gaps: a different person could reuse an open session, short responses could inherit earlier authorization, text Detect could bypass audio verification, and unreviewed Device or External MCP tools could expose sensitive actions. Unmeasured concurrency and premature calibration could also make a seemingly protected Agent unreliable.

The operator needs a manageable homelab feature for ordinary conversation, ordinary information lookup, and low-consequence control such as lights or volume. It must fit the existing single-process architecture and make its limitations visible rather than imply protection against replay, synthetic voices, overlapping speakers, or disclosure of private context.

## Solution

Add a first-class Speaker Provider and an Admin Web enrollment wizard. The browser records bounded, correctly encoded audio; the server validates quality, produces embeddings, and publishes a finalized Voiceprint without retaining raw recordings. A Template can select an optional Speaker Provider, and an Agent controls its Speaker policy and explicit Template grants.

Deliver enrollment and ESP32 Observe together as the first milestone. Preliminary Calibration enables enrollment validation and diagnostic observation, without granting speaker-based authority. After independent evaluation establishes qualification for exact candidate sets and a declared workload, Required verifies every voice turn before accepting user text or starting downstream work.

Keep the supporting controls narrow: an Agent Tool Allowlist reviewed against observed contracts, explicit calibration reload, security invalidation, and an atomic pilot admission envelope. Reuse the existing provider manager, workers, SQLite, Admin API/Web, MCP discovery, and Reference Integration Client. Independent Confirmation for sensitive operations is not implemented in V1, so those operations remain blocked.

## User Stories

1. As an operator, I want to keep existing Device and Admin authentication, so that speaker recognition adds a control without replacing established credentials.
2. As an operator, I want to create a Speaker Provider through the existing provider interface, so that I can configure recognition without a second provider management system.
3. As an operator, I want the server to generate the provider key, so that provider identity follows existing resource conventions.
4. As an operator, I want model assets and execution settings to remain deployment-owned, so that Admin forms do not expose arbitrary model downloads or native execution controls.
5. As an operator, I want to see cold, preparing, ready, busy and failed runtime states, so that saved configuration is not mistaken for a usable model.
6. As an operator, I want exact-version acquisition through the existing Provider Runtime Manager, so that enrollment and sessions use the configuration they actually selected.
7. As an operator, I want Speaker operations to report unavailable when the runtime manager is absent, so that the server does not silently use a separate inference path.
8. As an admin, I want to create and edit a Speaker profile, so that a person's identity can exist before enrollment succeeds.
9. As an admin, I want to select an enabled Speaker Provider before enrollment, so that samples use one pinned embedding contract.
10. As an admin, I want to record three to five samples through the browser microphone, so that enrollment has multiple consistent observations of one person's voice.
11. As an admin, I want the recorder to produce genuine PCM16 mono 16 kHz WAV, so that the server receives interoperable audio.
12. As an admin, I want actionable microphone and secure-context errors, so that I can fix recording problems without a misleading success state.
13. As an admin, I want duration, quality, consistency and duplicate checks, so that invalid samples do not silently become a usable Voiceprint.
14. As an admin, I want to replace or remove a sample slot, so that I can correct a recording without rebuilding unrelated samples.
15. As an admin, I want to record an independent holdout sample, so that enrollment checks an utterance not already used to construct the Voiceprint.
16. As an admin, I want validation failure to remain distinct from HTTP request success, so that a processed request is not mistaken for a matched voice.
17. As an admin, I want finalize to publish one complete Voiceprint atomically, so that new connections cannot see a partially enrolled identity.
18. As an admin, I want to reconcile lost responses using current resource state, so that retries do not produce duplicate finalization or overwrite another tab's work.
19. As an admin, I want enrollment drafts to expire and remain bounded, so that abandoned enrollment does not retain unbounded biometric data.
20. As an admin, I want failed or cancelled re-enrollment to preserve the active Voiceprint, so that collecting a replacement does not break the existing enrollment.
21. As an admin, I want readiness shown for the selected embedding space, so that a Speaker enrolled for one model is not presented as compatible with every provider.
22. As an admin, I want compatible instances to reuse a Voiceprint while incompatible spaces remain separate, so that provider identity is not confused with embedding compatibility.
23. As an admin, I want new sessions to see finalized enrollment without restarting the server, so that enrollment becomes useful immediately when dependencies are ready.
24. As an admin, I want explicit purge and conditional hard deletion, so that removing configuration does not silently erase biometric records or unrelated history.
25. As an admin, I want an optional Speaker binding on a Template, so that existing core-provider fallback remains usable.
26. As an admin, I want explicit Speaker-to-Agent-to-Template grants, so that recognizing a person does not authorize every Template.
27. As an admin, I want Agent policies Off, Observe and Required, so that I can introduce the feature in stages.
28. As an operator, I want Preliminary Calibration to support enrollment and Observe, so that I can gather device evidence before granting authority through Required.
29. As an operator, I want enrollment and ESP32 Observe in the first milestone, so that a browser holdout is not mistaken for cross-device qualification.
30. As a speaker, I want Required to identify me on the first accepted voice turn, so that the session locks the intended identity.
31. As a speaker, I want every later Required turn to verify that locked identity again, so that another person cannot inherit my previous pass.
32. As a speaker, I want a new connection to be necessary when changing speakers, so that Dialogue History is not inherited by a different identity.
33. As a speaker, I want rejection of insufficient audio explained honestly, so that I understand why a short response may not be accepted.
34. As a Device user, I want control abort to remain available without speaker verification, so that I can stop work through the existing protocol.
35. As an operator, I want Required text Detect to be denied, so that text cannot bypass the audio gate.
36. As an operator, I want denied turns to produce no STT output, accepted history, archive, LLM, tool or TTS side effects, so that rejection happens before application work uses the utterance.
37. As an operator, I want revocation to block new dispatch and close affected sessions, so that an immutable session snapshot cannot preserve withdrawn rights.
38. As an admin, I want valid configuration changes saved even when a new candidate set lacks evidence, so that configuration can progress without automatically weakening Required.
39. As an admin, I want saved-but-unqualified responses to identify missing dependencies, so that I know why new Required admissions are unavailable.
40. As an admin, I want enabling Required to remain separately gated, so that saving configuration cannot bypass qualification.
41. As an operator, I want qualification to cover exact Agent/Template candidate sets, so that changing top-2 comparisons cannot silently inherit evidence from a different roster.
42. As an operator, I want evidence for multiple exact sets retained, so that adding a set does not revoke a still-valid old session snapshot.
43. As an admin, I want tools to default to denied until reviewed and allowed, so that an Agent's available operations have an explicit boundary.
44. As an admin, I want Device tools identified by device_id and original tool name, so that another Device with the same tool name cannot inherit permission.
45. As an admin, I want External MCP tools identified by server_key and original tool name, so that permissions cannot be confused across servers or LLM aliases.
46. As an operator, I want resource recreation to lose previous tool permissions, so that reused public identities do not reuse old approvals.
47. As an admin, I want to inspect validated contracts actually observed from admitted Device discovery, so that I review evidence rather than an invented schema.
48. As an admin, I want approval to compare Device incarnation, observed revision, fingerprint and allowlist revision, so that observation changes cannot race with review.
49. As an operator, I want observed contract changes to invalidate approvals, so that changed schemas or source configuration require a fresh review.
50. As an admin, I want conflicting observations to block the relevant tool, so that the newest connection is not automatically treated as authoritative.
51. As an admin, I want a bounded discovery recovery batch, so that consistent completed observations can produce a new reviewable contract without automatically restoring approval.
52. As an operator, I want only allowed tools advertised to the LLM and permission checked again at dispatch, so that direct tool naming cannot bypass the allowlist.
53. As an operator, I want sensitive tools blocked while Independent Confirmation is unavailable, so that a voice match alone cannot open locks or access private data.
54. As an operator, I want Observe to retain the Agent's tool limits, so that diagnostic speaker matching never expands tool authority.
55. As an operator, I want an explicit deployment pilot envelope across all Agents and modes, so that unrelated native work cannot exceed the load that was evaluated.
56. As a Device user, I want idle connections and control handling preserved during contention, so that another session holding the pipeline does not prevent basic connection control.
57. As a Device user with pipeline-status support, I want a bounded busy status and explicit retry, so that I can understand contention while keeping my connection.
58. As a legacy Device user, I want a capacity close rather than silent capture failure, so that busy work is not mistaken for accepted listening.
59. As an operator, I want one Voice Session to hold processing capacity while its capture remains armed, so that the pilot's native workload stays bounded even during silence.
60. As an operator, I want native cleanup to complete before capacity is reused, so that timeout and cancellation cannot overcommit native work.
61. As an operator, I want cold preparation to acquire shared admission atomically, so that model initialization and warmup cannot race with active capture or enrollment.
62. As an operator, I want hot acquisition of an already-ready backing runtime to remain available, so that model ownership is not confused with processing capacity.
63. As a speaker, I want a cold Template switch to fail busy while retaining the current profile, so that pilot safety does not partially change my session.
64. As an operator, I want explicit authenticated reload from a fixed calibration source, so that deployment evidence can change without accepting arbitrary files or URLs.
65. As an operator, I want failed reload to preserve the entire current catalog, so that invalid deployment changes are not partially applied.
66. As an operator, I want qualification removal to invalidate affected Required sessions, so that stale evidence cannot continue authorizing new work.
67. As an operator, I want separate exact confidence bounds for recognition and verification errors, so that pooled rates do not conceal a failing decision path.
68. As an operator, I want misidentification counted as genuine failure, so that matching the wrong identity never appears as recognition success.
69. As an operator, I want corpus, trial protocol and stopping rules versioned before evaluation, so that threshold tuning cannot reuse held-out evidence as an independent test.
70. As an operator, I want short, quality, busy, timeout, runtime, replay and overlap outcomes reported separately, so that the report accounts for every attempted trial and its limitations.
71. As an operator, I want queue time, inference time, speaker gate wait and ASR/History Barrier wait reported separately, so that latency measurements explain the workload being qualified.
72. As an operator, I want machine, model, threads, worker, queue and audio conditions pinned to evidence, so that untested load or execution changes do not inherit qualification.
73. As a participant, I want raw recordings kept out of the server database, logs and repository, so that enrolling a voice does not create a hidden recording archive.
74. As an operator, I want ordinary-context review to remain an operational condition, so that V1 does not grow a separate content-eligibility product.
75. As a speaker, I want Dialogue History isolated to my Voice Session, so that another session does not automatically receive my conversation.
76. As a maintainer, I want deterministic compatibility tests independent of real models and credentials, so that CI can reliably check the new public behavior.
77. As a maintainer, I want real ESP32 and model evidence reported separately, so that deterministic tests do not claim recognition accuracy or runtime qualification.
78. As a maintainer, I want the feature to reuse established runtime, discovery and Admin flows, so that its scope remains appropriate for the current homelab project.

## Implementation Decisions

### Scope and architectural boundaries

- V1 serves ordinary conversation, ordinary information lookup and low-consequence control. Speaker Match, Speaker Authorization and Independent Confirmation are distinct concepts. No Independent Confirmation workflow is implemented, so sensitive operations are blocked.
- Keep Device and Admin authentication. A client-provided speaker identity, session ID, transcript or earlier match is not a reusable credential.
- Reuse the existing Provider Runtime Manager, asset preparation, worker lifecycle, provider CRUD, Template relationships, Admin auth/audit, SQLite/CAS, MCP discovery, and session output machinery. Do not add a second model manager, runtime plugin registry, generic policy engine, scheduler product or content-eligibility workflow.
- The Speaker Provider produces embeddings; domain services validate, normalize, score and authorize. SessionActor owns session semantics and receives identity-bound results. Native mutable state belongs to bounded workers.
- Retain existing actor/worker ownership, Turn ID, Generation ID, cleanup acknowledgement, writer terminal outcome and History Barrier contracts. An accepted ADR describes a required contract, not proof that every part is already implemented; recheck the code and qualify missing integration before claiming completion.

### Provider and runtime integration

- Add provider type Speaker with CAM++ through the pinned sherpa-onnx Rust API. Verify the current dependency's extractor API before considering a dependency upgrade. Pin the model URL/revision, installation location and preprocessing artifacts in provider-owned asset declarations; model selection is not an Admin JSON field.
- Extend descriptor, typed config, factory/registry, logical runtime view, physical resource planning, diagnostic support and SQLite provider/Template-slot constraints. Provider keys remain server-generated; deployment identities are separate from database provider keys.
- Speaker requires managed mode. Missing Provider Runtime Manager yields unavailable; do not build an inference fallback in HTTP handlers or session actors.
- Resolve the exact DesiredProvider snapshot for enrollment, diagnostics, admission and switch preparation. Hold Resource Leases for operations/sessions and cleanup obligations; Resource Lease is not an inference permit or Voice Pipeline Processing Permit.
- Start with one physical extractor replica and bounded native execution. Compatible logical instances share backing resources under existing accounting and quotas. Additional logical instances or revisions must not multiply replicas or bypass global capacity.
- Execution settings, including threads and resource estimates, remain deployment-owned. ProviderVersion, Runtime Resource Key and embedding_space_id have different meanings. Scoring/calibration changes do not automatically change extractor compatibility; vector-affecting preprocessing must change the embedding contract.
- Create saves desired configuration without promising a loaded runtime. GET and readiness inspection do not trigger loading. Optional prepare and bounded operation acquisition use the existing manager; there is no separate activate/restart requirement.
- Template Speaker is optional and has no implicit default or fallback. Preserve absent core-slot fallback and explicit-invalid-binding failure. Correct first-assignment/default selection so an optional Speaker slot does not break core completeness checks.

### Persistent domain and mutations

- Use the existing required SQLite database and forward-only migrations. Extend existing provider and Template binding constraints instead of storing duplicate Speaker Provider configuration.
- Persist Speakers, Voiceprints per Speaker/embedding space, enrollment drafts and sample embeddings, Agent speaker policies, explicit Agent/Template grants, reviewed tool approvals, bounded Device observations and recovery batches. Use internal resource identity/FKs where incarnation matters; APIs use existing public identities.
- Preserve existing IDs, FK integrity, indexes, migration history and identity high-water marks during table rebuilds. Do not allow delete/recreate to reuse identity-dependent permissions.
- Voiceprint provenance references its enrollment provider. Provider disable does not erase a compatible Voiceprint; hard deletion is conditional on references and does not cascade biometric purge.
- Store validated, normalized embeddings with bounded dimension/length checks. Do not serialize vectors or private audio digests into API responses, debug output, logs or telemetry. Raw audio is not retained in server files or SQLite.
- Keep resource-specific revisions and conditional mutations. New speaker resources expose revision/ETag; missing or malformed If-Match uses the existing 400 convention, stale revisions use 409. Keep provider, Speaker, draft, Agent, policy and Template revisions distinct.
- Apply relevant mutations through short application-owned coordination boundaries. Inference runs outside transactions and mutation locks. Recheck revision, TTL, cancellation and configuration before commit; publish catalogs and security invalidation consistently after commit and before success.
- Admission must not combine new database state with an old published catalog. A rollback must not publish. If publication cannot be reconciled after commit, fail closed, report catalog unavailable and let the client GET to reconcile; an error response does not prove database rollback.
- Preserve guide quotas as initial bounded settings: 256 Speakers, four spaces per Speaker, 32 candidate bindings per Agent, 16 drafts globally, and one open draft per Speaker. Do not silently truncate candidates. Cleanup draft data at startup and periodically; retain terminal/tombstone and observed metadata only for bounded retention.

### Enrollment and browser recording

- Admin Web selects a Speaker and exact enabled provider, creates a draft, records/uploads three to five samples, records a fresh holdout, validates, then finalizes. Enrollment may precede Template binding and does not auto-grant or change Agent policy.
- Accept raw WAV PCM signed 16-bit little-endian, mono, 16 kHz only. Do not accept arbitrary audio URLs/paths, client embeddings, base64 JSON or renamed WebM as WAV. Validate RIFF structure, chunks, format and lengths before large allocation.
- Audio routes have a 512 KiB full-body cap, including chunked bodies, a 12-second parser cap and bounded read deadlines. Reject non-identity Content-Encoding. Preserve the existing 256 KiB JSON cap and typed-config bounds; do not select body policy by a permissive path suffix.
- Enrollment clips are five to ten seconds; holdout is two to six seconds. Select a contiguous voiced window of at most six seconds. Enrollment requires at least three seconds voiced; Required voice verification requires at least two seconds voiced. Do not concatenate disjoint clips or pad silence to create a pass.
- Run duration, energy, clipping, voiced-duration and consistency checks under the pinned preprocessing/calibration contract. VAD does not prove one speaker or detect every overlap. Template VAD remains responsible for endpointing; Manual quality analysis runs in the bounded worker after its explicit endpoint.
- Normalize each finite, dimension-correct, nonzero embedding. All sample pairs must pass consistency; do not quietly discard an inconsistent sample to reach the count. Use equal-weight normalized centroid scoring. Holdout must be new; a private exact-audio digest detects duplicate bytes, not replay resistance.
- Draft lifecycle is collecting, validated, then finalized, with cancellation/expiry terminal branches. Replacing samples invalidates earlier validation. Validation pins sample/draft, provider, space, runtime and calibration identity.
- Finalize CAS-checks the draft, Speaker and selected-space Voiceprint revisions, atomically replaces only that space, terminalizes the draft and publishes the catalog. Failed or cancelled re-enrollment preserves the previous Voiceprint. Repeated finalize must reconcile rather than create another version.
- Runtime IDs do not survive restart. A resumable unexpired draft can reacquire its exact compatible version, repin the runtime, bump revision and invalidate obsolete holdout validation while preserving compatible accepted vectors. Incompatible contracts require a new draft.
- Browser recording uses microphone access, AudioWorklet capture, downmixing and filtered resampling to the actual required format. HTTPS/localhost microphone requirements are independent of trusted-LAN CORS. Bound buffers, stop recording at ten seconds and release tracks, nodes, fetches and object URLs on cancellation/navigation.
- Do not keep browser audio in persistent browser storage or analytics. Upload sequentially using the latest draft revision. Handle permission failures, quality failures, busy, expiry, stale revisions and lost responses explicitly; do not automatically repeat finalize or overwrite another tab.
- Initial runtime preparation may be cold only under cold admission. Concurrent pilot enrollment inference requires a Ready runtime. UI reports saved/ready/preliminary/qualified conditions honestly and recommends device testing after finalize.
- Purge explicitly removes all Voiceprint spaces, samples and drafts while retaining the Speaker profile, grants and audit. Conditional hard deletion requires references to be removed. Purge does not delete transcripts or unrelated history.

### Speaker Gate, grants and session isolation

- Policy is Agent-owned, defaults to Off, and has its own revision. Speaker grants explicitly enumerate assigned enabled Templates; no wildcard or implicit permission for Templates added later.
- Recognition 1:N scores enabled Agent-bound Speakers with Voiceprints in the active Template provider's embedding space, then checks the selected Template grant. Do not pre-filter unauthorized candidates to force a match. Use calibrated threshold and top-1/top-2 margin; ambiguous, unknown and missing-candidate outcomes are distinct.
- Lock the first accepted speaker identity to the Voice Session. Every subsequent Required voice turn performs fresh 1:1 verification against that identity in the active space. Do not switch identities or inherit a previous pass for short turns; a different speaker reconnects.
- A six-second representative window only establishes a match for that window. V1 does not establish one speaker across an entire utterance, handle speaker changes mid-sentence, perform diarization or protect against overlap.
- Required needs ASR final nonempty text, a valid fresh speaker authorization and completion of the History Barrier before accepting user text. One acceptance boundary guards both ASR Final and text Detect paths; Required text Detect is audio-required denial even after a prior pass.
- ASR and speaker work may proceed in parallel after the relevant existing admission boundaries, but ASR completion alone must not commit user text. Bind every result to session, Turn ID, Generation ID, operation and exact runtime/catalog identities; stale results have cleanup-only effects.
- Denied, short, ambiguous or unavailable Required turns produce no STT output, accepted Dialogue History, persistent transcript, LLM, tools or TTS. Reuse existing ASR capture rather than delaying ASR until speaker verification; such a redesign is outside V1.
- Off does not add speaker inference or capture overhead. Observe is best-effort diagnostic matching, does not gate baseline text acceptance and never supplies speaker-specific authority; both remain subject to explicitly enabled pilot capacity and applicable Agent Tool Allowlist.
- Device control abort remains available without speaker verification. A spoken stop through ASR is an ordinary voice turn. Existing echo-safe barge-in can interrupt playback before endpoint verification; V1 does not prevent every unrecognized person from interrupting output.
- Repeated unknown/ambiguous/denied results follow the guide's three-consecutive-mismatch close policy; insufficient audio, busy, unavailable and timeout do not count as mismatches. Use policy close 1008 where applicable, not protocol close 1002. Do not invoke LLM/TTS to generate denial prompts.
- Keep history isolated between Voice Sessions and do not restore verified identity from Device ID, a previous WS or a stored voice credential. Do not automatically import private dialogue from other sources.

### Qualification and configuration changes

- Calibration is a pinned deployment profile with revision, preliminary/qualified status and report reference. Preliminary supports enrollment and Observe only. Required also needs valid evidence for the exact candidate set, scoring/preprocessing contract, Voiceprint revisions and audio/execution/load conditions.
- Candidate qualification covers actual scoring sets for each Agent/Template; even a subset is not automatically covered because its top-2/margin decisions differ. Adding/replacing Speakers, re-enrolling or changing grants that alter the scoring set needs updated evidence.
- Valid domain mutations are saved even if their resulting set lacks evidence. Keep Required mode, reject new Required admission outside evidence, and return saved-but-unavailable dependency information. Retain structural, type, FK, assignment and revision validation.
- Switching policy to Required still checks usable runtime/binding/candidate/grant and valid qualification. Saving configuration while already Required is not a policy-enable bypass.
- Old WS snapshots continue only while their exact evidence and rights remain valid. Additions do not expand snapshots. Re-enrollment, reduced rights and contract changes invalidate affected sessions.
- The calibration catalog can retain evidence for multiple exact sets. A catalog generation change does not itself revoke old evidence or alter scoring revision; explicit evidence removal/revocation and contract changes do.
- Template switch uses the admitted exact target configuration, valid locked-speaker grant and compatible target-space Voiceprint, checked before arm and apply. Prepare outside the actor and install at the normal boundary. Hot target acquisition is permitted; cold target while the session owns pipeline capacity returns busy without building, retaining the current profile.

### Agent Tool Allowlist and observed contracts

- For participating Agents, publish only reviewed permitted Device/External tools to the LLM and check permission again immediately before dispatch. Deny unreviewed tools by default and keep existing deny filters. Observe does not bypass this Agent-level operation limit.
- Device approval identity is Agent plus Protocol Device Identity device_id and original tool name; External approval identity is Agent plus server_key and original tool name. Persist internal resource incarnation FKs; no Device wildcard, display-name identity, new Device key or Client ID identity.
- Pin Reviewed Tool Contract content: resource identity, original name, input schema and usage-relevant description. External source review also covers endpoint, transport and auth scope/reference. Never include secret values in fingerprints, persistence or reports.
- When configuration mutation or discovery reveals a relevant contract change, approval becomes ineffective; block new dispatch and invalidate affected sessions. Review again using conditional mutation. Same-schema remote behavior changes and changes before rediscovery remain outside detection guarantees.
- Store bounded observed Device contract metadata only from validated completed discovery on an admitted WS. Admin-entered schema is not evidence. Inspection exposes observation time and distinguishes observed contract from current online/behavior assertions.
- Approval transaction checks exact Device incarnation, observed revision/fingerprint and allowlist revision. Observation changes between inspection and approval return conflict. Discovery never grants rights and incomplete discoveries cannot be merged into a complete contract.
- Conflicting valid observations block the relevant tool and invalidate affected sessions; do not let the latest connection win. Recovery reuses existing MCP discovery in a batch with identity, deadline and explicit state, scoped to the Device incarnation.
- Only complete observations from the current recovery batch count. Conflict, timeout or incomplete discovery stays blocked; do not discard failed members to claim consistency. Late old-batch results are stale. Old observations become superseded under bounded retention; a consistent batch becomes reviewable and still requires explicit approval.
- Grant additions apply to new WS only; reductions block new dispatch and close affected WS. No live expansion of Session Tool Catalog. Work dispatched before revocation cannot be undone.

### Pilot admission, cold preparation and wire behavior

- Enable the pilot envelope through explicit deployment configuration, never automatically through provider creation or policy mutation. It applies across the process, all Agents and all modes, including Off and text Detect when that path needs processing capacity.
- Limit the process to one Voice Session processing the voice pipeline and one concurrent native enrollment operation. Native extractor work remains bounded; this does not require two simultaneous inferences on one extractor. Other diagnostics/native workloads are restricted by the same envelope or run only when the pipeline is idle.
- Voice Pipeline Processing Permit is acquired before VAD/ASR opens and retained through Listening, Processing, Speaking and armed capture. Barge-in by the same session reuses it. Release only on Ready/teardown after writer terminal and native cleanup acknowledgement; existing provider-specific capacity remains separately enforced.
- Accept that Auto/Realtime can hold the slot for a long time; do not add time slicing. Multiple idle WS and their control paths remain connected. Capacity denial starts no worker, accepts no Required transcript and waits in no unbounded queue.
- Limit Speaker voice operation capacity to one. Permit timeout/cancel does not make running native work reusable; native cleanup acknowledgement, terminal completion or quarantine controls reuse under existing lifecycle contracts.
- Cold materialization uses the same atomic admission state: it starts only while voice pipeline and native enrollment are idle. Hold cold preparation rights through build, readiness and warmup until terminal acknowledgement, even after HTTP timeout. New capture/enrollment is busy during that ownership.
- Apply cold admission to startup and background prewarm, explicit prepare, diagnostic acquisition, admission and switch paths. Loader count alone does not serialize against voice work. Hot acquisition of a Ready backing runtime follows existing capacity. Reuse materializer/singleflight; do not implement a second loading cache.
- Add separate pipeline-status opt-in alongside optional speaker status, preserving existing mandatory hello/audio fields. Pipeline busy applies across modes, contains only bounded state/reason and no owner Device identity, and is internally bound to the correct request/lifecycle for stale-output filtering.
- Opt-in clients keep WS/control, show busy and retry work only explicitly. Legacy clients close 1013 on busy work-start requests, including capacity-dependent text Detect; do not silently return Ready. Busy is neither authentication failure nor recognition mismatch. Pilot clients should support the new capability.

### Calibration reload and security invalidation

- Provide explicit calibration reload using existing Admin Bearer authentication, reading only the configured deployment source. Accept no arbitrary path/URL or editable qualification fields. Web has no reload/edit/bypass control; Admin token holders can still call the API, which is an interface restriction rather than operator RBAC.
- Validate the entire proposed catalog before publish. Failed reload reports an error and preserves the existing catalog. Editing source files has no effect until successful reload.
- Publish catalog and affected security invalidation consistently before success. Revoking qualification or changing its contract blocks new dispatch and closes affected Required sessions without switching them to Observe.
- Reload is limited to the calibration catalog; it does not reload models, provider runtimes or all TOML configuration. Adding valid exact-set evidence preserves unaffected evidence and old qualified snapshots.
- Security epochs are separate from Effective Session Profile and profile revision. Recheck before text acceptance, every new LLM request/continuation and each tool dispatch. Invalidation covers relevant Speaker/policy/grant/Template/provider/Agent changes and does not create general profile hot reload.

### Evaluation evidence and operator conditions

- The four pilot error gates use separate one-sided 95% exact-binomial upper confidence bounds: 1:N FAR at most 1%; 1:N genuine failure at most 10%; 1:1 FAR at most 1%; 1:1 FRR at most 10%. Do not pool paths or claim simultaneous 95% coverage.
- For zero errors the upper bound is one minus 0.05 raised to one divided by the trial count. At zero false accepts, 298 independent trials give 1.00024% and 299 give 0.99691%; the latter passes only the numerical FAR gate under the independence assumption. No trials or unsupported independence remains preliminary.
- One 1:N trial is one fresh utterance evaluated against the entire actual candidate set; only the correct identity is genuine success. Wrong identity is genuine failure and separately reported misidentification; also report pure rejection. One 1:1 trial checks the locked identity; another speaker accepted is false accept and the locked speaker rejected is false reject.
- Do not multiply trials by candidate comparisons or repeat a clip to inflate evidence. Preregister quality rules, protocol, trial counts and stopping rule. Report all attempts, exclusions and operational failures rather than silently dropping busy/timeout/runtime outcomes.
- Operator-owned external corpus requires participant consent and separation of enrollment, calibration and held-out recordings by acquisition/session, not file renaming. Keep sample/person codes, capture chain, conditions, outcomes, exclusion reasons and corpus/protocol versions for reproducibility; do not commit audio or expose sensitive corpus paths/real names in reports.
- Threshold tuning after examining held-out results turns that set into development data and requires fresh held-out evidence. Report short/quality rejection and replay/overlap separately; these reports do not establish anti-spoof or overlap protection.
- Pilot measured workload is one active voice pipeline plus one enrollment, with real ASR/TTS competing. Target warm Speaker inference p95 is at most 200 ms for a four-second window; target speaker_gate_wait p95 is at most 500 ms. Report queue and inference separately.
- speaker_gate_wait starts at utterance terminal boundary and ends when the actor receives a valid decision; it includes queue, quality and inference. ASR/History Barrier wait is separate. Pin machine, model revision, threads, worker/queue configuration, workload, preprocessing, scoring and audio conditions; two/four active WS are not qualified.
- Operator reviews participating Agent/Template Persona, prompts, context sources and tool results for ordinary-information scope, and reviews changes before use. Until reviewed, disable/exclude the affected configuration. This is operational control; the server does not prove or enforce content eligibility.
- User speech can still add private information to Dialogue History. System-source review, Speaker Gate and tool limits do not guarantee conversation secrecy against replay/synthetic voices or protect private data already in context. Preserve per-session isolation and do not import private external history automatically.

### Delivery and documentation

- Milestone 1 includes provider/runtime integration, draft and Preliminary Calibration, web validation/finalize, ESP32 Observe, applicable tool review/allowlist, and explicit pilot admission/busy behavior. Do not deliver browser-only enrollment as the completed first milestone.
- Milestone 2 produces independent cross-device and real-load evidence, pinned calibration and exact-set scope. Required stays unavailable until qualification is established; deterministic test success is insufficient.
- Milestone 3 implements and qualifies Required gating, locked identity, grants, Template switch and security invalidation, then enables Required only through the guarded policy operation.
- Update the existing API documentation, Postman collection, Reference Client capability support and operational instructions. Keep resource revision variables separate, use binary WAV placeholders and do not embed recordings or real credentials.
- Detail route/DTO names and finite retention/deadline values using existing repository conventions during implementation; record their public contract and tests without adding an interview, new workflow or generalized abstraction.

## Testing Decisions

### Primary seam and prior art

- Prefer one existing high-level seam: the production process's public Admin HTTP and Voice WS boundary, driven by the Reference Integration Client/Integration Harness with independent wire types. Reuse the existing qualification build, compile-time Qualification Providers and controlled process restart conventions.
- Test observable resource state, responses, emitted frames, lack of side effects, capacity outcomes and restart behavior. Do not validate private method calls, internal object layouts or a test-only reconstruction of application state.
- Existing Admin API/diagnostic/concurrency tests, Device admission/enrollment WS tests, protocol regression gates, runtime-manager qualification and writer/worker cleanup tests provide prior art. Reuse their auth, fixtures, deadline, process lifecycle and teardown patterns.
- Extend deterministic qualification at the registered Provider Adapter/worker seam when controlled embeddings, failures, cleanup or races are necessary. Do not expose a production bypass flag or build a parallel model/session lifecycle for tests.
- A narrow follow-up check of these proposed seams was requested while synthesizing the spec. They are the preferred plan, not a claim of additional user approval; revise Testing Decisions if the user supplies a different expectation.

### Mandatory deterministic behavior

- Provider integration: Speaker create/filter/descriptor/config, exact version, cold/Ready inspection, missing manager, optional Template slot/core fallback, first assignment, and source/space compatibility.
- Enrollment/API: auth isolation, JSON versus WAV limits including chunked input, valid/malformed/truncated WAV, slot bounds, consistency, duplicate holdout, stale revisions, simultaneous tabs, cancellation, expiry, lost-response reconciliation, atomic finalize and re-enroll preservation.
- Persistence: real forward migration preserving existing data/identity/FKs, restart draft repinning/cleanup, corrupt embedding rejection, conditional deletion and explicit purge without history cascade.
- Speaker/session gate: result arrival in either ASR/speaker order, History Barrier, fresh per-turn pass, zero accepted side effects on denial, text Detect rejection, reconnect for identity change, stale operations and turn/generation separation.
- Runtime/admission: concurrent WS capture requests, multiple idle controls, retained Auto/Realtime slot, same-session barge-in, diagnostics/text capacity, bounded enrollment, hot versus cold acquisition, atomic voice/enrollment/cold races and HTTP timeout without early native reuse.
- Wire compatibility: pipeline-status opt-in busy preserving WS/control, legacy close 1013 rather than silent Ready, speaker status independently opted in, stale status filtering, policy 1008 and existing hello/audio negotiation.
- Grants/evidence: exact set and Voiceprint revision matching, unqualified subset denial, valid mutation saved without evidence, guarded Required activation, unchanged old snapshots, multiple evidence sets, explicit removal/revocation and no automatic downgrade.
- Tool contracts: only approved tools advertised, direct dispatch cannot bypass, source identity/alias collision, resource recreation, mutation/discovery drift, observation/approval CAS races, incomplete/conflicting discovery and recovery-batch deadlines/stale results, no automatic approval restoration.
- Security invalidation: revoke between match and text acceptance, between acceptance and LLM/tool dispatch, pending writer/cancellation, mailbox failure escape path, consistent commit/publication, and unchanged profile snapshot semantics.
- Calibration reload: Admin auth, fixed source only, forbidden override fields, all-or-nothing validation, old catalog retained on failure, file edit alone ineffective, consistent successful publication and affected-session closure, additive evidence preservation.
- Privacy: no raw audio/vector/digest/rejected text in logs, archive or responses; no secret values in contract fingerprints; bounded metrics without per-person identity labels; observed metadata does not assert online presence.

### Additional seams only where necessary

- Small deterministic mathematical fixtures exercise invalid vectors, normalization, centroid/cosine scoring, dimension/space mismatch and exact-binomial limits. Use independently known expected answers, including the 298/299 zero-error boundary, zero trials and all-error cases; do not duplicate implementation logic in expected values.
- Recorder browser tests validate actual WAV output/resampling behavior, finite buffers, permission/secure-context failures and microphone/fetch/object-URL cleanup. Public HTTP/WS tests cannot prove browser audio encoding or browser resource release.
- SQL migration/repository tests use real SQLite and transactions for upgrade/FK/incarnation guarantees not conveniently observable through a single API request. Keep these narrowly tied to external persistence behavior.
- Existing Rust formatting, lint, workspace tests and relevant protocol/runtime/admin regressions remain required. Use the current frontend checks for the changed Vue integration. Do not add broad test frameworks for this feature alone.

### Real-runtime qualification

- Separate Mandatory Qualification from Optional Runtime Evidence. Mandatory CI uses deterministic providers/local fixtures, requires no remote credentials or model download, and cannot establish recognition accuracy or target-machine latency.
- Real CAM++/ESP32/browser evidence must traverse production manager, workers, preprocessing and session paths. Measure the declared pilot workload with real ASR/TTS and enrollment, queue/quality/inference and end-to-end gate timing, RSS and cleanup stability.
- Real report must establish the four independent error gates and the two pilot latency targets in its declared scope. Keep exact candidate sets, independent held-out design, corpus/protocol versions, trial accounting and execution conditions reproducible.
- Report PASS/FAIL/NOT_RUN honestly. NOT_RUN or inadequate independent trials keeps calibration preliminary and Required unavailable; do not use demo thresholds or deterministic compatibility results as evidence.

## Out of Scope

- Replay/deepfake defenses, liveness proof, diarization, overlap detection, multi-window whole-utterance identity guarantees, automatic speaker change or automatic Template selection by voice.
- Voice as a replacement for Device/Admin authentication, reusable voice bearer credentials, public self-enrollment, or raw recording/vector export.
- Independent Confirmation for sensitive operations; V1 blocks those operations rather than implementing a confirmation service.
- Content classifiers, privacy guarantees for existing prompt/history, a separate eligibility-review workflow, automatic import of external private history, or a general policy engine.
- A separate operator credential/RBAC system, arbitrary calibration files/URLs through Admin API, Web qualification editing/bypass or general TOML/model hot reload.
- Time-based pipeline yielding, fairness across armed sessions, unbounded native queues, additional extractor replicas, two/four active WS qualification, or a new scheduler/runtime/cache framework.
- Arbitrary HTTP audio formats/server resampling, disguised WebM, file-upload enrollment as an additional product flow, and device microphone command/status dashboards.
- Persistent voice-derived session identity, automatic Voiceprint adaptation from WS audio, provider deletion that silently purges biometric records, or transcript deletion implied by Speaker purge.

## Further Notes

- Producing this spec changes documentation only; implementation code, tickets and deployment are separate tasks.
- This spec publishes the agreed V1 to the local issue tracker with ready-for-agent status. That status means the implementation contract is ready for agent work; it does not mean runtime calibration, model performance or Required authorization is ready.
- The source guide was untracked when the discussion began, and the newly recorded ADRs have not been committed. Preserve the user's working tree; implementation should reread its actual baseline before editing.
- Authority order: explicit Q1–Q25 decisions and the keep-V1-small constraint, corresponding accepted ADRs, then guide details. Future-looking guide examples do not expand this V1 scope.
- Corpus participants, acquisition protocol, trial counts, stopping rule, target machine and real reports are operational work to register/produce before evaluation. Their absence is not permission to invent successful evidence or enable Required; it does not prevent deterministic implementation of enrollment/Observe and the guarded Required path.
- Concrete route/DTO/bounded-retention choices may follow existing conventions without another product interview. Document them during implementation and keep them within the agreed boundaries.
- Source references: [implementation guide](../../docs/speaker-recognition-web-enrollment-implementation-guide%20%281%29.md), [domain vocabulary](../../CONTEXT.md), [ADR 0077](../../docs/adr/0077-speaker-v1-authority-and-calibration.md), [ADR 0078](../../docs/adr/0078-agent-tool-allowlist.md), [ADR 0079](../../docs/adr/0079-calibration-catalog-reload.md), [ADR 0080](../../docs/adr/0080-bounded-pilot-voice-pipeline.md), [ADR 0081](../../docs/adr/0081-exact-candidate-set-qualification.md).
