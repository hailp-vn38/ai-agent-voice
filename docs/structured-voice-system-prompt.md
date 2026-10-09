# Structured Voice System Prompt — Rust Server

## Scope

The deployment default prompt now lives in `prompts/voice-assistant.txt` as a readable set of XML-like instruction blocks, inspired by Xiaozhi's base prompt layout. This change does **not** add a prompt scripting engine, dynamic HTTP context providers, a knowledge retrieval pipeline, or new database fields.

The server already has a deep prompt module at `crates/voice-agent-server/src/session/prompt.rs`. This change deepens that existing interface rather than introducing an independent service or provider-specific rendering.

## The two composition seams

1. **Admission / template switch**: the server resolves an immutable `ActiveTemplateProfile.system_prompt`. Deployment defaults use `render_system(&EffectiveAgentConfig)`; assigned database Agent Templates remain complete stored system prompts and are not re-rendered with deployment placeholders.
2. **Current conversational turn**: after the ASR + Speaker join succeeds or times out, `SessionActor::begin_speech_delivery` passes the base prompt and **only this turn's verified SpeakerContext** to `prompt::compose_turn_system`. The result is a *single* `ChatMessage::System` followed by history messages. Tool continuation reuses that same message snapshot, so it does not re-identify the speaker or change the prompt mid-turn.

The module's interface deliberately has only two rendering operations: `render_system(agent)` for deployment configuration and `compose_turn_system(base, speaker)` for the current LLM turn. The latter has no database, network, clock, or tool registry dependency.

## Prompt file and placeholders

The built-in file is embedded via `DEFAULT_PROMPT_TEMPLATE`. Deployment operators can select another regular text file through the existing `[agent].prompt_template` setting. A relative file path is interpreted relative to the TOML config file.

Accepted **deployment** placeholders (strict, literal, without spaces):

| Token | Timing | Source |
|---|---|---|
| `{{persona}}` | Admission | `[agent].persona` (required by deployment template grammar) |
| `{{agent_name}}` | Admission | `[agent].name` |
| `{{language}}` | Admission | `[agent].language` |
| `{{speakers_info}}` | Each LLM turn | Verified speaker matching, or an explicit unknown marker |

`{{speakers_info}}` is **optional** in custom deployment templates. Place it between `<speakers_info>` and `</speakers_info>` to keep untrusted metadata in a visibly delimited data block.

The database Agent Template's `prompt` field is **already rendered content**. It is not parsed with the deployment template grammar; therefore existing text remains unchanged, except an optional exact `{{speakers_info}}` slot at the per-turn stage. An existing Template without this slot gets a `<speakers_info>...</speakers_info>` block appended **only for a verified speaker match**. This preserves legacy Agent Templates without any DB migration.

No placeholder is left unresolved in the default LLM request. The special speaker marker is preserved *only* in the immutable admission snapshot, then replaced before the LLM request is constructed.

## Speaker rules

- Speaker is taken only from `speaker_context_for_turn`, populated after a matching `SpeakerStatus::Verified` diagnostic.
- Unknown, missing, insufficient-audio, timed-out, or unverified identity does not inherit an identity from earlier turns.
- The verified profile is **not authorization** for MCP tools, devices, template switching, or personal data.
- Names and descriptions are scalar-bounded (96 / 1,024 chars), control characters are filtered, the data is JSON-encoded, and `<` / `>` are represented by JSON `\\u003c` / `\\u003e`. This reduces accidental block-breakout; the content is still untrusted.
- Speaker data is per-turn only; neither the immutable base prompt nor the dialogue history receives these profile fields.
- Profile data over the existing **96 KiB** System Prompt limit rejects the turn before invoking the LLM. LLM request-wide **256 KiB** accounting remains in place.

## Tool behavior

`<tool_usage>` defines selection and verification rules, but **does not enumerate tools**. The authoritative tool schemas and tool availability remain `LlmRequest.tools` resolved in the actor. Template switching, tool-round limits, authorization, and MCP processing are not changed. This prevents accidental drift between the file text and runtime tool registration.

## Implementation files

- `prompts/voice-assistant.txt`: structured default voice instructions.
- `crates/voice-agent-server/src/config/mod.rs`: accepts the new optional literal speaker marker.
- `crates/voice-agent-server/src/session/prompt.rs`: preserves the marker at admission and builds per-turn speaker data with size validation.
- `crates/voice-agent-server/src/session/actor/delivery.rs`: consumes the verified SpeakerContext once and composes the actual System message before starting the LLM round.

## Validation scenarios

1. A default deployment with no speaker must emit a single System message with `No verified speaker for this turn.`, not `{{speakers_info}}`.
2. The first recognized turn must include the matched name and description. A following unknown turn must **not** include the previous speaker.
3. A stored Template without the marker must retain its original prompt text. A verified match may append a bounded block; unknown turns are unchanged.
4. A stored Template with the marker must replace it once for that turn.
5. Invalid deployment placeholders still fail configuration validation; whitespace form (`{{ speakers_info }}`) is not accepted.
6. Oversized resolved system prompts must fail before LLM execution.
7. Tool continuation must reuse the same prompt snapshot for its original turn.

The prompt module has unit tests for these seam-specific invariants; full cargo verification and WebSocket E2E qualification are still required before production deployment.

## Not included

The blocks for environment, memory, knowledge and device capabilities state *behavioral rules* only: they do not imply that live values are already injected. Future additions should expose actual context from an owned runtime seam and explicitly bound freshness, authorization, privacy and prompt size before enabling new placeholders.

Do not build a general Jinja template processor: this repository's deployment grammar intentionally remains strict and single-pass.
