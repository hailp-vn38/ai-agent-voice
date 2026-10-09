# voice-agent-server

A personal voice-agent server. This context coordinates a Voice Session between a Voice Protocol Client and AI providers.

## Language

## Source code structure conventions

- When a source file becomes long or takes on multiple responsibilities, split it into modules with clear responsibilities; keep public paths stable through the parent module and re-exports where needed.
- A module's tests must live in a separate test file within that module, rather than further expanding the implementation file. Integration/public-boundary tests remain in the appropriate integration test area.

**Compatibility Profile**:
The combination of voice wire protocol, audio/MCP contracts, and external reference provenance with which the server commits to being compatible.
_Avoid_: protocol version (when referring only to the wire protocol version), vendor-specific compatibility

**Voice Protocol Client**:
A software or hardware client that conforms to the voice wire protocol and Compatibility Profile. It may be firmware, a Linux application, or a Rust/Python Reference Client.
_Avoid_: vendor simulator, vendor client

**Reference Client**:
An independent Voice Protocol Client used to test protocol conformance without reference hardware.
_Avoid_: firmware simulator, fake hardware

**Reference Integration Client**:
An independent qualification client that checks public control-plane and Voice Protocol contracts through deterministic scenarios; it is not a general-purpose operational Admin CLI.
_Avoid_: operations console, production management CLI, server-side test harness

**Qualification Provider**:
A deterministic Provider Adapter compiled only into a qualification build, but passing through the same desired-configuration, startup load plan, Runtime Catalog, diagnostic, and Voice Session boundaries as a production provider.
_Avoid_: injected ProviderSet, runtime fake switch, external smoke provider

**Qualification Build**:
A build of the production `voice-agent-server` entrypoint with compile-time Qualification Providers; it is not the default/release artifact and cannot be enabled through runtime configuration.
_Avoid_: deployed release binary, test-only server binary, runtime qualification mode

**Integration Harness**:
The owner of automated Mandatory Qualification: it creates a temporary environment, spawns the production binary, coordinates controlled restarts and deterministic doubles, then runs the Reference Integration Client through the public boundary. It does not rebuild `AppState` in-process to simulate a restart.
_Avoid_: in-process restart helper, server-side test seam, operations CLI

**Process Startup Handshake**:
A machine-readable, nonce-bound artifact that the production process creates exclusively and publishes atomically after binding the listener, allowing the Integration Harness to discover the correct child/address before probing Readiness.
_Avoid_: human-log parsing, reserved-port handoff, PID-only identity, file existence as readiness

**Scenario State**:
An immutable handoff artifact between two process lifetimes of the Integration Harness, created with create-new semantics and validated before side effects. It retains only public resource identities, revisions, run/spec identities, and necessary runtime observations; it does not retain credentials, secret references, authorization headers, prompts, results, or audio.
_Avoid_: resume journal, mutable provisioning cache, secret store

**Scenario Plan**:
An immutable plan materialized and validated from ScenarioSpec before any provisioning side effect, containing the run identity and expected resource graph without yet asserting that the resources exist.
_Avoid_: Scenario State, partial provisioning record, retry journal

**Reference Client Wire Type**:
A representation of the public request/response contract owned by the Reference Integration Client independently of server implementation types, making API drift an observable failure.
_Avoid_: shared repository row, imported server handler DTO, server domain type

**Mandatory Qualification**:
A mandatory deterministic compatibility gate that runs through the public boundary using provider doubles or local fixtures, without depending on credentials, remote services, real models, or hardware.
_Avoid_: real-environment smoke test, optional runtime evidence

**Qualification Deadline**:
A hard upper bound for all of Mandatory Qualification; its remaining time limits every stage deadline, and expiry always produces a failure before teardown.
_Avoid_: sum of stage timeouts, advisory timeout, per-request timeout

**Qualification Report**:
A versioned, privacy-safe, machine-readable JSON artifact recording the mandatory result, stage outcomes, optional evidence, and cleanup without sensitive request/response content.
_Avoid_: raw log archive, transcript report, debug dump

**Optional Runtime Evidence**:
Separately reported evidence from a real provider, remote service, or hardware; its `PASS`, `FAIL`, or `NOT_RUN` status does not change Mandatory Qualification.
_Avoid_: completion gate, required CI evidence

**Firmware Baseline**:
The pinned firmware version and commit used as the compatibility source of truth.
_Avoid_: upstream main, latest firmware

**HIL Reference Profile**:
The standard physical device and network configuration used to confirm firmware compatibility.
_Avoid_: Reference Client, simulated client

**Voice Session**:
An active WebSocket connection belonging to exactly one Device ID and holding dialogue only in RAM.
_Avoid_: device session, persistent session

**SessionActor**:
The sole owner of mutable Voice Session state. The reader, provider workers, and writer communicate with it only through events or commands over bounded channels; they hold no mutable references to the session.
_Avoid_: shared session state, provider-owned session, WebSocket handler state

**WebSocket Writer**:
The only task that calls WebSocket send for a Voice Session. It owns bounded outbound lanes with priority `urgent > control > audio` and arbitrates the terminal playback outcome before SessionActor commits delivery.
_Avoid_: provider direct send, concurrent socket writer, actor-owned wire ordering

**Protocol Fault Boundary**:
The wire fault boundary distinguishes the handshake from application messages: an invalid ClientHello closes the connection before a Voice Session begins; after the handshake, malformed, unknown, or out-of-phase application messages are recorded and ignored, while a frame exceeding the transport cap always causes closure before parsing.
_Avoid_: fail-closed every application error, custom wire-error protocol, codec buffer limit

**Ready**:
The state in which a Voice Session remains connected but does not accept microphone audio.
_Avoid_: Idle, Listening

**Processing**:
The state in which a Voice Session has determined an utterance endpoint and is completing the Conversational Turn; microphone audio is dropped until the terminal outcome.
_Avoid_: Listening, queued turn

**Listening Mode**:
The capture semantics declared by a Voice Protocol Client in `listen:start`; in V1, Manual, Auto, and Realtime are distinct wire values that the server must not infer or substitute.
_Avoid_: capture option, implicit manual mode

**Conversational Turn**:
An independently cancellable unit of voice processing within a Voice Session.
_Avoid_: request, job

**Turn ID**:
A monotonically increasing identity that is never reused within a Voice Session, assigned when an utterance crosses the terminal boundary and is accepted by Active Turn admission; this identity follows the turn through ASR finalization, LLM processing, and speech delivery.
_Avoid_: Generation ID, ASR stream identity, VAD Capture Cycle ID

**Active Turn**:
A Conversational Turn that has crossed the utterance terminal boundary and holds global capacity from ASR finalization to its terminal state.
_Avoid_: listening turn, queued turn

**History-Barrier Turn**:
An Active Turn that has crossed the utterance terminal boundary and holds capacity, but whose accepted user text has not yet been committed or sent to the LLM because the preceding Conversational Turn has no terminal writer outcome yet.
_Avoid_: queued turn, capacity-free pending turn

**Active Turn Limiter**:
An application-wide capacity domain limiting concurrent Active Turns independently of provider worker capacity.
_Avoid_: ASR semaphore, provider limit

**ASR Stream Lease**:
Capacity reserved for an open recognition stream, from the start of capture until ASR finalization, cancellation, or error.
_Avoid_: Active Turn, ASR queue slot

**ASR Stream Identity**:
The identity of a recognition stream, which may exist before a Turn ID; ASR finalization after the utterance terminal boundary is associated with the Turn ID of the turn that accepted that stream.
_Avoid_: Turn ID, ASR Stream Lease

**Semantic ASR Ownership**:
The exclusive right for ASR events to produce accepted user text; this right is revoked as soon as Detect is accepted, independently of physical worker cleanup.
_Avoid_: ASR Stream Lease, cleanup acknowledgement

**ASR Cleanup Obligation**:
The cleanup-only identity of an ASR stream whose semantic ownership has been detached but whose worker has not yet acknowledged termination. Any cleanup timeout for this obligation still fails the Voice Session closed, even across a new generation.
_Avoid_: active ASR stream, stale event ignored

**Inbound Session-Scoped Command**:
Listen or Abort carrying an optional `session_id` in V1. A missing/empty field is accepted for compatibility; a non-empty string must match the Voice Session, while a present field with the wrong JSON type is Unknown.
_Avoid_: authentication credential, mandatory V1 session ID

**Inference Worker Runtime**:
An application-owned bounded group of workers, each owning a mutable provider stream/session and exchanging only commands/events carrying identities; it owns neither Voice Session state nor the WebSocket.
_Avoid_: provider pool, background task, SessionActor worker

**VAD Probability**:
A Silero inference result for exactly one continuous PCM interval of an Auto cycle, containing speech probability and the sample cursor `[start_sample, end_sample)`; it is not yet an utterance boundary.
_Avoid_: SpeechStart, SpeechEnd, speech decision

**VAD Stream Integrity Failure**:
A failure when an Auto cycle receives a VAD Probability with a gap, duplicate, incorrect order, or invalid sample range; the affected Voice Session fails closed because endpointing is no longer reliable.
_Avoid_: dropped VAD frame, recoverable VAD delay

**Worker Cleanup Acknowledgement**:
An event confirming that a worker has terminated or reset a lease's mutable runtime, the sole condition for its slot to become reusable; it is still processed when the generation logic is stale.
_Avoid_: cancel requested, Drop, logical cancellation

**Acoustic Barge-in**:
Interruption of an assistant turn in `Speaking`, triggered only by `SpeechStart` from VAD on microphone uplink declared by the client to be AEC/echo-suppressed and permitted by server trust policy. It first snapshots retained PCM, then invalidates the old turn, sends exactly one urgent `tts:stop` if playback has started, and opens ASR for the new turn.
_Avoid_: server-side AEC, any microphone packet, explicit abort

**VAD Capture Cycle**:
A VAD semantic cycle with its own identity, comprising the PCM timeline, segmenter, and retention; it differs from both the VAD worker lease lifetime and a Conversational Turn. Semantic events are valid only for the current cycle; cleanup acknowledgements are still processed when the cycle is stale.
_Avoid_: VAD worker identity, turn generation, reset requested

**Generation ID**:
An epoch used to invalidate output when a turn is cancelled or interrupted; multiple normally completed Conversational Turns may share a Generation ID.
_Avoid_: Turn ID, operation identity

**Echo-safe Client Assertion**:
`features.aec=true` in ClientHello asserts that the client uplink is echo-suppressed; it is not evidence of server-side AEC. It permits Acoustic Barge-in only when both `barge_in.enabled` and `barge_in.trust_client_aec_feature` are enabled.
_Avoid_: verified server AEC, capability unconditionally trusted

**Dialogue History**:
The in-RAM history of Exchange Atoms belonging to a Voice Session; user messages are committed after a non-empty ASR final result. Message count is an eviction target, while requests have a separate hard byte bound.
_Avoid_: persistent memory, transcript log

**Agent Persona**:
Deployment configuration shaping the assistant's name, role, and style within a Voice Session; it contains neither credentials nor provider state.
_Avoid_: OpenAI prompt, provider prompt, model personality

**Prompt Template**:
A strict, single-pass deployment template that combines Agent Persona with voice conversation rules into an immutable base System Prompt at admission; it accepts only `agent_name`, `persona`, `language`, and the optional `speakers_info` slot.
_Avoid_: provider request template, general-purpose template engine, Dialogue History

**Turn System Prompt**:
A single System message composed from the base System Prompt snapshot and the verified Speaker Context of the specific Conversational Turn, after the ASR/Speaker join and before the first LLM round. Tool continuation reuses this exact snapshot; speaker metadata enters neither Dialogue History nor the base profile.
_Avoid_: per-round prompt rebuild, mutable session prompt, persisted speaker profile

**Prompt/LLM Base Snapshot**:
The immutable message set of a Conversational Turn, comprising the Turn System Prompt, Exchange Atoms committed before the turn, and the committed current User; tool continuation only appends the completed tool prefix to this set.
_Avoid_: per-round prompt rebuild, mutable provider prompt

**Provider Asset Declaration**:
The `assets.rs` module beside a local provider declares the authoritative pinned upstream URL, revision, relative install path of each model file, voice list, and provider-specific conversion if the upstream format requires it. There is no central manifest document; the provider itself is the source.
_Avoid_: model manifest, artifact registry, database asset row

**Provider Asset Manager**:
The part of Provider Adapter Registration responsible for ensuring that the provider's model files exist. Provider Runtime Manager calls `ensure_assets()` when materializing a provider; Provider Factory then resolves paths and builds without further downloads.
_Avoid_: provider download, startup model scan, asset framework

**Provider Asset**:
A model file required by a provider and declared in its Provider Asset Declaration. It is ready when it is a regular file with a size greater than 0 — no checksum, fingerprint, or ONNX parsing. Runtime initialization confirms whether the model can be loaded.
_Avoid_: verified artifact, immutable install, content-addressed copy

**Provider Asset Download**:
An asset download writes to `<target>.part`, checks for non-zero size, then atomically renames to the final path; each asset holds a striped lock so two requests materializing the same provider do not download the same file twice. The final file never exists in a partial state, and a failed attempt leaves nothing behind, allowing a clean retry.
_Avoid_: partial final file, unlocked concurrent download

**Typed Provider Configuration**:
Configuration selecting an adapter through `[providers.<kind>].adapter` and placing concrete configuration under a table with the adapter's name; startup accepts only a table matching an adapter compiled into the binary.
_Avoid_: adapter_config table, compatibility parser, runtime provider discovery

**Worker Runtime Configuration**:
Configuration under `[workers.vad]`, `[workers.asr]`, or `[workers.tts]` controlling capacity, mailboxes, timeouts, cleanup, and quarantine of the Inference Worker Runtime; it contains no model or inference options.
_Avoid_: provider config, model option, adapter setting

**Unexpected Tool Call**:
A tool call emitted by the LLM in a round that was not supplied with tool definitions; it is a terminal generation failure, not a Device MCP request.
_Avoid_: implicit tool request, unsupported tool fallback

**Speech Segment**:
A unit of speakable text extracted by Sentence Segmenter from the Generated Assistant Response according to delivery policy and submitted atomically to SpeechOutput.
_Avoid_: token, full response, TTS chunk

**TTS Plain Text**:
The separate text representation of a Speech Segment sent to TTS: it contains only Unicode letters or numbers, periods, and commas; each run of other characters is replaced with at most one ASCII space. A segment with no remaining letters/numbers is not sent to TTS. This rule does not change the text displayed over WS.
_Avoid_: display text, formatted text, provider-normalized text

**LLM Operation**:
A streaming request scoped to a specific Voice Session and generation, holding a global LLM permit from runtime acceptance until the terminal event; it is not a persistent provider session.
_Avoid_: LLM worker session, global chat, provider connection

**Speech Output Backpressure**:
The state in which reading LLM deltas pauses when the pending Speech Segment queue reaches its hard capacity bound; the actor retains at most one partially processed delta and resumes after TTS consumes segments. An unfinished sentence buffer exceeding the emergency threshold remains a terminal failure; segments must not be dropped or overwritten.
_Avoid_: skipped sentence, best-effort speech queue

**Provider Adapter**:
A compile-time implementation of a provider trait, selected by typed provider configuration when materializing a runtime. Legacy deployment mode selects the adapter at startup; managed runtime mode may materialize the adapter at acquisition. The adapter has no knowledge of Voice Sessions, WebSockets, or worker runtimes.
_Avoid_: dynamic plugin, provider platform, service locator

**Provider Instance**:
A configuration with a stable ID for exactly one Provider Adapter; the ID is the value bound by an agent, while the adapter specifies the implementation.
_Avoid_: adapter name, active provider

**Provider Catalog**:
The read-only set of materialized Provider Instances, indexed by Provider Instance ID and independent of Voice Sessions. Legacy deployment mode builds the catalog at startup; managed runtime mode keeps the deployment catalog empty and resolves the exact Provider Version through Provider Runtime Manager.
_Avoid_: adapter registry, session provider map

**Effective Provider Bindings**:
The complete set of Provider Instance IDs after materializing provider defaults and agent overrides at the Config boundary.
_Avoid_: runtime fallback, adapter binding

**Runtime Catalog**:
The read-only set of Inference Worker Runtimes indexed by Provider Instance ID; it resolves Effective Provider Bindings into a runtime snapshot before creating a Voice Session.
_Avoid_: SessionActor provider lookup, runtime plugin registry

**Runtime Snapshot**:
The concrete runtimes resolved once for a Voice Session and kept stable throughout that connection.
_Avoid_: hot-switched segment runtime, catalog-aware SessionActor

**Provider Benchmark**:
A developer CLI that runs the same fixed, versioned TTS workload, selected Typed Provider Configuration, and run policy in two modes to measure initialization and steady-state processing on the current hardware. It separates Provider Asset Download and provider build/startup readiness (cold) from workload warmup and measured runs (steady); warmup is excluded from samples. It owns neither Voice Sessions, WebSockets, nor pacing. Mode `provider` ends at provider-facing PCM; mode `delivery` uses the same PCM stream and the same deterministic canonical downlink conversion as production, including fade, resampling, framing, Opus encoding, and tail finalization, ending at the last canonical Opus packet ready to send. By default it prints only to stdout; the JSON artifact is opt-in and contains no benchmark text, audio, filesystem paths, secrets, or deployment endpoints. It no longer distinguishes verified models from downloaded models, as these are now one concept.
_Avoid_: correctness test, end-to-end latency benchmark, playback benchmark

**Provider Factory**:
A compile-time factory that builds a Provider Adapter from typed provider configuration and, when needed, a Resolved Model; it neither acquires models itself nor knows about Voice Sessions.
_Avoid_: provider downloader, runtime plugin factory

**Provider Registry**:
The set of Provider Factories compiled into the binary, looked up at startup through typed adapter selection and changed only by a build/restart; it neither discovers nor loads code at runtime.
_Avoid_: dynamic plugin registry, service locator

**Logical Model Identity**:
The model key selected by typed provider configuration, used to select the adapter's correct Provider Asset Declaration; it is neither a filesystem path nor a directory name, and does not define file contents.
_Avoid_: model directory, latest model, adapter name

**Model License**:
A model's license, declared alongside its Provider Asset Declaration. ZeroTTS bundles an Apache-2.0 codec, so it uses the composite `MIT; bundled-codec=Apache-2.0`. The license is not configuration: a deployment neither acknowledges nor overrides it.
_Avoid_: license acknowledgement config, per-deployment license gate

**Phase Completion Gate**:
A mandatory gate for marking a phase complete. The gate must use the phase's real boundary; for Phase 3 this is real-model Voice Protocol E2E in Manual and Auto through canonical Opus to exactly one STT, while for Phase 6 it is Reference Client MCP E2E through WebSocket and SessionActor. Both are separate from the implementation gate using fake providers.
_Avoid_: ignored smoke test, compile success, hardware dependency outside the Compatibility Profile

**Canonical Audio Profile**:
The Compatibility Profile's fixed wire-audio profile: uplink Opus 16 kHz mono 60 ms and downlink Opus 24 kHz mono 60 ms.
_Avoid_: negotiated audio params, supported audio formats

**Uplink PCM Frame**:
A PCM16 mono 16 kHz frame of exactly 960 samples, decoded from exactly one Opus uplink packet before being passed to an audio capture component.
_Avoid_: PCM Frame, audio bytes, sample chunk

**Downlink PCM Frame**:
A PCM16 mono 24 kHz frame of exactly 1,440 samples, ready to be encoded into exactly one Opus downlink packet.
_Avoid_: PCM Frame, output chunk

**Uplink Audio Utterance**:
The complete canonical PCM16 mono 16 kHz audio of a capture, created only when an audio capture component completes successfully.
_Avoid_: Audio Utterance, PCM buffer, partial capture

**Manual Capture**:
An audio capture component for manual listen mode that owns a capture's PCM and capacity, then returns an Uplink Audio Utterance or an outcome without audio.
_Avoid_: actor buffer, manual listen buffer

**Uplink Audio Stream**:
The continuous stream of microphone Opus throughout a Voice Session; Conversational Turn or Manual Capture boundaries do not create a new stream.
_Avoid_: capture stream, turn stream

**Capture Outcome**:
The typed result of stopping a Manual Capture: Uplink Audio Utterance, empty, or overflowed.
_Avoid_: optional audio, capture status flag

**Trace Session ID**:
A random UUID used only to correlate telemetry for a Voice Session without recording Device ID or Client ID.
_Avoid_: device identifier, client identifier

**Discovered Tool**:
A tool advertised by a device through MCP, without implicit permission for a language model to use it.
_Avoid_: permitted tool

**LLM-visible Tool**:
A Discovered Tool that has passed server policy and is permitted in the language model's schema.
_Avoid_: discovered tool, authorized tool

**Device MCP Server**:
An MCP capability owned by a Voice Protocol Client, advertising tools through `initialize` and `tools/list`, then executing `tools/call` within the client's own state.
_Avoid_: server-side MCP plugin, ESP32-only capability, global tool registry

**Reference Client MCP Gate**:
The Phase Completion Gate for Device MCP: the Reference Client both uses the real WebSocket voice protocol and acts as a deterministic Device MCP Server, executing at least one stateful tool and proving that its tool result passes through LLM continuation to the final TTS lifecycle.
_Avoid_: mocked actor response, ESP32 HIL requirement, fixed firmware tool catalog

**Generated Assistant Response**:
Assistant content generated by the LLM for a Conversational Turn, which the user may not have heard in full.
_Avoid_: delivered response, dialogue assistant message

**Delivered Assistant Response**:
A Generated Assistant Response becomes part of the dialogue only when the WebSocket writer has successfully sent all audio for the turn and the normal `tts:stop` in the correct order; this does not imply that client playback has completed.
_Avoid_: partial response, cancelled response, client playback complete

**Persistent Transcript**:
An optional record, subject to retention, of final user text and Delivered Assistant Responses for a Voice Session; it is not the in-RAM Dialogue History and is disabled by default.
_Avoid_: dialogue history, full conversation log, audio archive

**Database-backed Device Admission**:
Mandatory policy at the WebSocket boundary: every Voice Protocol Client must resolve to a Device provisioned in SQLite. The database is an unconditional startup dependency; there is no flag to disable Device admission or WS path bypassing the database.
_Avoid_: implicit device registration, migration-based admission

**Database Desired Configuration**:
Persistent configuration requested by an admin for providers/templates, which may not yet be effective in the running process.
_Avoid_: loaded runtime, active runtime configuration

**Loaded Runtime**:
A backing resource that has completed validation, native readiness, and warmup. Provider Runtime Manager retains the resource according to generation and RAM budgets; leases keep it usable for admitted snapshots. Legacy deployment mode still uses a process-lifetime Runtime Catalog.
_Avoid_: database desired configuration, hot-reloaded provider

**Effective Session Profile**:
An immutable snapshot of Device, Agent, optional Template, Provider bindings, prompt/language, and MCP bindings resolved before a Voice Session begins.
_Avoid_: per-frame database lookup, mutable agent configuration

**Template Switch Catalog**:
The immutable set of configured Template Profiles that are enabled and valid, together with exact desired provider snapshots at admission. Only the selected profile is acquired before upgrade; switch preparation acquires a cold candidate outside the actor, then commits after the writer/history/native cleanup barrier. A legacy injected catalog may contain resolved profiles.
_Avoid_: live template query, pending restart candidate, mutable assignment list

**Session Profile Revision**:
A lifecycle counter local to a Voice Session, incremented when a successful Template Switch is applied at a normal turn boundary.
_Avoid_: template revision, provider revision, database row version

**External MCP Binding**:
An Agent's binding to an MCP Streamable HTTP server, providing a snapshot of LLM-visible tools separate from the Device MCP Server.
_Avoid_: Device MCP Server, global tool registry, verified stale tool cache

**External MCP Protocol Engine**:
The component implementing protocol lifecycle and message transport for an External MCP client, separate from Database Desired Configuration, outbound security policy, and Voice Session lifecycle.
_Avoid_: database repository, arbitrary HTTP client, SessionActor configuration source

**Tool Origin**:
The typed identity of a tool capability before routing execution, distinguishing a Device MCP original name from an External MCP server key and original name.
_Avoid_: LLM-visible name as authority, bind-order routing

**External Tool Segment**:
A server key or original MCP tool name normalized independently, with bounded and deterministic rules, to construct an LLM-visible External MCP tool name.
_Avoid_: vendor hierarchy inference, hash collision suffix, truncated name

**Expected Revision**:
The immutable revision presented by an Admin API client to conditionally mutate a Database Desired Configuration.
_Avoid_: last write wins, Session Profile Revision, database migration version

**Resource Credential**:
A secret used by a Provider Instance or External MCP Server to authenticate with an external service. Admin API accepts only a write-only value; managed snapshots are encrypted and persisted with the resource, while the deployment environment is a fallback when the resource has no managed snapshot.
_Avoid_: provider configuration, arbitrary MCP header, shared Admin token, readable Admin field

**Secret Reference**:
An opaque identifier for a credential source or encrypted credential snapshot used by a Provider or External MCP; it is not a Secret Value and cannot be read back through Admin API.
_Avoid_: API key field, resolver-specific syntax, trim/normalization, secret value

**Secret Resolver**:
An interface resolving a Secret Reference into a Secret Value at runtime; callers do not own how credentials are protected or stored.
_Avoid_: repository reads environment directly, SQLite plaintext secret storage, Admin API resolution endpoint

**Secret Value**:
A runtime wrapper that exposes credentials only for request/provider construction and redacts `Debug` output.
_Avoid_: ordinary debug string, clone/display implementation, telemetry label, persisted configuration

**Secret Rotation Snapshot**:
A credential lifecycle snapshot: the backing Provider Runtime retains the resolved secret for the resource lifetime; an admitted session holds a resource lease, and an External MCP Client retains the secret until disconnect. The resolver currently provides no credential generation, so remote/authenticated resources are not shared across desired versions; the secret of a resource in use is not silently rotated.
_Avoid_: per-request secret resolution, silent credential replacement, runtime failure refresh

**Credential-free Provider Config**:
Canonical serialization of typed adapter configuration without credentials; Provider credentials flow only through Secret Reference and Secret Resolver.
_Avoid_: arbitrary JSON bag, adapter-owned api key field, plaintext header/options escape hatch

**Provider Config Shape**:
The shared resource-abuse boundary for Provider config: raw UTF-8 bytes, JSON depth, and aggregate object-key/array-item nodes before typed validation.
_Avoid_: deep JSON allocation, separate Admin/startup validity rules, semantic adapter limits

**Forward-only Schema Migration**:
Monotonic SQLx migration history in which a binary may migrate only forward; an older binary encountering a newer schema must fail before the listener starts.
_Avoid_: automatic downgrade, unknown-schema best effort, application-owned backup restore

**SQLite Lock Contention**:
A SQLite lock persisting beyond the busy timeout, distinct from SQLx pool exhaustion and unavailable storage; it must not be retried at the application layer.
_Avoid_: pool timeout named busy, transaction retry, SessionActor database wait

**Admin JSON Transport Boundary**:
Shared pre-deserialization protection for Admin mutation bodies: content encoding/type and raw size limits, before domain validation.
_Avoid_: handler-local body checks, decompression bypass, parser error/body logging

**Single-owner SQLite Deployment**:
Exactly one Voice Agent process owns the local SQLite database path in V1.
_Avoid_: active-active writer, NFS/SMB database, implicit migration leader election

**Patch Field Intent**:
Typed update intent distinguishing an absent field, setting a value, and explicit clearing before domain validation.
_Avoid_: JSON Merge Patch, nested Option ambiguity, null clears immutable field

**Readiness**:
The process's ability to accept new connections using application-owned dependencies, separate from process liveness and without probing optional External MCP.
_Avoid_: full admission probe, per-device resolution, optional MCP availability gate

**Admission Gate**:
The single application-owned gate deciding whether new work may begin: new listeners, new DB admissions, and new Tool-round work. Shutdown closes it once before draining, and no SessionActor needs to observe shutdown before it closes.
_Avoid_: per-session shutdown flag, cancel token replacing the gate, repeated admission checks in each component

**Session Drain Registry**:
An application-owned registry of accepted, unfinished Voice Sessions, each entry holding a completion handle registered before the connection starts work, allowing shutdown to observe drain completion and issue Controlled Close to exactly those sessions still open at the deadline.
_Avoid_: task abort, uncounted broadcast, estimated session count

**Controlled Close**:
A protocol close performed by the Voice Session itself when the drain deadline arrives or the process stops, distinct from forcibly aborting a task; the client still receives a normal close code.
_Avoid_: task abort, silent socket drop, ungraceful server shutdown

**Liveness**:
The sole question of whether the process is still running, independent of the database, External MCP, or shutdown; `/health` answers only this question.
_Avoid_: readiness synonym, dependency-aware health check, restart when the database fails

**History Purge**:
An explicit destructive operation deleting Persistent Transcript within the scope of a Device, Voice Session, or the entire archive, independently of the Dialogue History of open sessions.
_Avoid_: side effect of disabling Device, implicit transcript delete, session memory reset

**Resource Key**:
The stable, lowercase ASCII, immutable public resource identity of an Agent, Template, Provider, or MCP Server; the database primary key is only an implementation detail. Agent, Template, and MCP Server keys are chosen by the client at creation; Provider Key is generated by the server at creation in the format `{provider_type}_{uuid32}` because Provider names may be duplicated and changed.
_Avoid_: mutable display name, client-supplied Provider key, identity derived from display name, case-insensitive alias, database primary key

**Protocol Device Identity**:
The opaque, immutable identity supplied by a Voice Protocol Client to provision a Device, compared without altering its bytes at the database boundary.
_Avoid_: normalized MAC address, display name, Client ID

**Device Enrollment**:
A short-lived SQLite control-plane record linking an unregistered Protocol Device Identity to an Activation Code and scrubbed metadata; it is not a credential, Voice Session, or audio state.
_Avoid_: device authentication, session record, audio-pipeline cache

**Activation Code**:
A CSPRNG-generated string of 6 ASCII digits, bound by a TTL and usable at most once for an Admin to claim a Device.
_Avoid_: device token, password, Device ID

**Enrollment Session**:
An unregistered Device's control-plane WebSocket connection that only displays/plays an Activation
Code and observes Enrollment Claim; it has no Effective Session Profile, provider,
transcript, or conversational permissions. A new Voice Session connection performs admission after the claim.
_Avoid_: anonymous Voice Session, temporary Agent, provider fallback

**Enrollment Claim**:
An Admin transaction that creates an enabled Device, consumes exactly one Device Enrollment, and records minimal audit data; it neither loads providers/runtimes nor confirms that the device is online.
_Avoid_: WebSocket admission, runtime warmup, online presence

**External MCP Network Policy**:
Policy governing External MCP HTTP/HTTPS URLs and an optional hostname allowlist; all hosts are accepted by default.
_Avoid_: mandatory LAN/CIDR configuration, redirect destination trust

**External MCP Authentication**:
Authentication using `none`, `bearer`, or a safe header, combined with Resource Credential to authenticate External MCP requests.
_Avoid_: query-string auth, template header value, Authorization header override

**Admin Audit Event**:
Bounded metadata recording an Admin API mutation or authenticated optimistic-concurrency conflict, without copying resource contents or secrets.
_Avoid_: request archive, configuration diff, authentication failure record

**Provider Load Plan**:
The partition of startup providers into required providers that must materialize before the listener and optional providers whose loading is attempted to make switch candidates available without blocking boot.
_Avoid_: all-enabled provider preload, non-default provider skip forever, duplicate load

**Runtime Status**:
The usability state of a Loaded Runtime in the process, separate from whether that runtime matches the current Database Desired Configuration revision.
_Avoid_: desired-state freshness, provider enabled flag, restart completion

**Admin Request ID**:
A server-generated UUID for an Admin API request to correlate the response, telemetry, and audit; it is not controlled by the client.
_Avoid_: client correlation identifier, database primary key, authentication credential

**External Tool Call**:
A logical LLM ToolCall routed to External MCP, with at most one outbound attempt and always ending in a normal or typed synthetic ToolResult.
_Avoid_: retried HTTP request, dangling tool call, remote error passthrough

**Tool-round Executor**:
The shared owner executing ToolCalls in the exact model order and pairing each terminal ToolResult at the same index before LLM continuation.
_Avoid_: origin-specific scheduler, parallel tool batch, reordered tool result

**Tool Execution Budget**:
A Conversational Turn's session-local time budget for Tool-round Executor, starting at the first ToolCall and limiting the start of subsequent calls.
_Avoid_: per-call timeout only, unbounded tool loop, audio pipeline budget

**Session Tool Catalog**:
An immutable snapshot of all LLM-visible tools resolved and validated at admission, retained unchanged until the Voice Session disconnects.
_Avoid_: removing capability after a runtime failure, DB availability mutation from tools/call telemetry, implicit circuit breaker

**External MCP Call Limiter**:
A process-global semaphore per MCP server identity limiting concurrent outbound External Tool Calls across all Voice Sessions.
_Avoid_: per-session-only cap, unbounded cross-session fan-out, permit held during LLM continuation

**Admin API**:
An optional administration surface for Database Desired Configuration, Resource Credential, and Persistent Transcript, mounted only when enabled and always using credentials separate from Voice/OTA.
_Avoid_: Voice API, trusted-LAN anonymous endpoint, shared OTA token

**Admin Web**:
An optional Vue application at `apps/admin-web/` that manages the server through the public Admin API. It owns presentation and the browser-side read model, but does not import internal Rust modules, read or modify `config.toml` directly, or own Voice Session state. When Admin API is disabled or the bearer token is invalid, the UI has no authority to substitute another control path.
_Avoid_: server module, Admin API handler, direct SQLite/config editor, Voice Session owner

**Exchange Atom**:
An indivisible Dialogue History unit for prompt construction or eviction: a user turn with ordered Completed Tool Rounds, each containing terminal assistant tool call/tool result pairs, and a Delivered Assistant Response if the writer closes the turn as Normal. Tool calls without terminal results are not part of the atom; a turn that fails before the first tool is a user-only atom.
_Avoid_: message, partial exchange

**ProviderVersion**:
The identity of the exact Provider source and configuration version used when granting a runtime. For a saved Provider Instance, the version corresponds to the desired revision and is not reused after deletion/recreation; for a Provider Test Draft, the identity belongs only to that test run. A Voice Session retains the version snapshotted at admission.
_Avoid_: provider key alone, latest mutable provider, Session Profile Revision

**Runtime Resource Key**:
The opaque identity of a backing runtime resource whose resource specification is equivalent under the Provider Adapter contract, including immutable artifact identity, execution settings, physical replica count, and credential scope where needed. It differs from an Admin resource's public Resource Key. It contains no voice, language, Template, or Agent, since those are logical selections.
_Avoid_: public provider key, raw JSON digest, path as model identity

**Physical Replica**:
A resident copy of a native engine owned by a Runtime Resource, declared by the Provider Adapter rather than the operator. ZeroTTS retains exactly one Physical Replica because each replica commits four ONNX sessions; generic worker concurrency does not replicate it. This count is part of the Runtime Resource Key, so topology changes still isolate resources.
_Avoid_: engine instance per voice, worker count as replica count, template runtime

**Prepared Runtime**:
The result established by preparation before `build` runs: the provider's model files already exist, together with the time spent. `build` only resolves paths and constructs the runtime, so a materialization never downloads the same model again.
_Avoid_: prepared model cache, immutable model tree

**Physical Resource Key — model identity**:
The identity portion of a physical runtime comes from the adapter, the provider's pinned MODEL_REVISION, ONNX execution identity, and thread count — not from reading model files. Bumping the revision produces a different key without hashing anything.
_Avoid_: model content hash, manifest fingerprint

**Resource Lease**:
The right to use a backing runtime resource granted by Provider Runtime Manager to a Voice Session or operation. The resource cannot be unloaded while this right or its corresponding cleanup obligation exists.
_Avoid_: inference permit, best-effort Arc count, runtime lookup without ownership

**Bounded Startup Warmup**:
A readiness pass on each retained native worker that touches each graph on the hot path exactly once, verifies finite, non-empty terminal PCM, then resets before traffic. It does not synthesize a full utterance; the deterministic full-utterance gate belongs to Optional Runtime Evidence, not the startup path.
_Avoid_: full-sentence warmup, warmup as qualification, unbounded readiness loop

**Provider Runtime Manager**:
The application owner that grants runtimes for the exact ProviderVersion and maintains shared lifecycle/backing resources within explicit budgets. It neither owns session history or stream state nor mutates Database Desired Configuration.
_Avoid_: runtime plugin registry, per-frame provider factory, automatic provider retry

## Speaker recognition

**Speaker Voiceprint**:
A versioned embedding of a Speaker in the correct embedding space, carrying enrollment provenance and validation status. It is input to Speaker Match, not a credential, authentication, or self-sufficient evidence for Required policy.
_Avoid_: Speaker profile, authentication token, authorization grant

**Quick Speaker Enrollment**:
A two-step Admin flow: captured audio is quality-checked and converted into a staged embedding with a 10-minute TTL, then an explicit commit creates a Speaker or replaces a Speaker Voiceprint. The staged waveform is not persisted; the current implementation writes the voiceprint with status `passed` after commit.
_Avoid_: persisted WAV, implicit Speaker creation, authorization grant

**Speaker Context**:
A human-facing profile containing the name and optional description of a `Verified` Speaker Match, valid only for the Turn System Prompt of the current Conversational Turn. This is untrusted, bounded data, not a credential or authorization.
_Avoid_: persistent speaker profile, speaker grant, identity inherited from a prior turn

**Speaker Match**:
The result of comparing the voice in the evaluated audio segment against Voiceprints using the corresponding calibration; it is not permission to execute a request and does not prove that the entire utterance came from the same speaker.
_Avoid_: speaker authorization, authenticated request, liveness proof

**Speaker Authorization**:
The decision permitting a Speaker to use an Agent and Template for a voice turn, based on a fresh Speaker Match together with policy, grants, and their validity. This permission is not sufficient to perform sensitive operations.
_Avoid_: speaker match, independent confirmation, blanket tool permission

**Independent Confirmation**:
Separate confirmation of a sensitive operation based on evidence independent of the Speaker Match for that request.
_Avoid_: repeated speaker match, spoken yes, Device authentication alone

**Preliminary Calibration**:
Preliminary calibration used to test audio quality, sample consistency, holdout enrollment, and Observe; it is not yet qualified for Speaker Authorization in Required.
_Avoid_: Required-qualified calibration, production authorization threshold

**Required-qualified Calibration**:
Calibration confirmed by the operator based on an independent evaluation report for the exact Agent/Template candidate set, voiceprint revisions, embedding space, preprocessing, scoring parameters, and audio/load conditions. It qualifies for Required only while the qualification remains valid; a subset of the candidate set is not automatically covered.
_Avoid_: preliminary calibration, browser holdout alone, demo threshold

**Agent Tool Allowlist**:
The set of tools reviewed and permitted by an admin for an Agent, identified by Protocol Device Identity (`device_id`) or External MCP server key together with the original tool name; tools outside the set are denied. A recreated resource does not inherit permissions from the deleted resource. This is the Agent's operational limit, independent of Speaker Match and Speaker Authorization.
_Avoid_: speaker grants, LLM tool name allowlist, read-only safety inference

**Misidentification**:
A 1:N identification result accepting an identity different from the actual speaker in a genuine trial; it counts as a genuine failure even if the system returns a match.
_Avoid_: genuine success, pure rejection, successful match

**Voice Pipeline Processing Permit**:
The exclusive right to process a Voice Session's pipeline within the pilot envelope, including armed capture and conversational work that has not reached a terminal state. It differs from the Resource Lease retaining the model and the separate permit for each inference operation.
_Avoid_: Resource Lease, Active Turn Permit, speaker inference permit

**Reviewed Tool Contract**:
An observable tool contract reviewed by an admin, including source identity, original name, input schema, description affecting usage, and relevant source configuration. The review becomes invalid when the server observes a contract change; it does not prove that the external implementation's behavior remains unchanged.
_Avoid_: tool name alone, remote implementation attestation, secret value fingerprint

**Provider Test Draft**:
Unsaved Provider configuration used for a single manual inference test. The result does not establish readiness of the Provider Instance used by an Agent.
_Avoid_: temporary Provider Instance, production readiness test

**MCP Connection Probe**:
A manual observation of the ability to handshake with an External MCP server at the time of testing; it does not assert that the server has tools or is ready for an Agent.
_Avoid_: connected flag, Agent availability

**MCP Tool Discovery**:
A complete observation of the tool catalog advertised by an External MCP server at the time of testing. This catalog is neither a Reviewed Tool Contract nor an Agent Tool Allowlist.
_Avoid_: tool approval, approved catalog
