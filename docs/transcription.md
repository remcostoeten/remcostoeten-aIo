# Transcription inventory

What moved into `crates/ai-providers` for ADR 0006, what deliberately did not,
and where each behavior is proven.

Source: `skriuw-ai-remote/src/transcribe.rs` and `skriuw-domain/src/transcribe.rs`
at Skriuw `9c474ed9`, read-only and unmodified while this was written.
Destination: `crates/ai-providers`. This is not an extraction in the Phase 2
sense — the source repository keeps its half rather than losing it — so it is
recorded here rather than appended to `extraction-inventory-providers.md`.

## Symbols

### From `skriuw-ai-remote/src/transcribe.rs`

| Source symbol | Destination | Change |
| --- | --- | --- |
| `transcription_endpoint` | `remote::descriptor::RemoteProviderKind::transcription_endpoint` | unchanged joins; now filtered through `stays_on_destination` like every other endpoint (T1) |
| `gemini_transcription_body` | `remote::descriptor` (private) | body and instruction text byte-identical |
| `groq_transcription_form` | `remote::descriptor` (private) | field names, `response_format`, `temperature` and the optional `language` unchanged |
| `recording_filename` | `remote::descriptor` (private) | mapping unchanged |
| `parse_transcript` | `remote::descriptor::RemoteProviderKind::parse_transcript` | unchanged, including the top-level `error` key check |
| `ai_transcription_models` | `remote::transcription_models` | same four entries; returns the SDK's `AiTranscriptionModel` |
| `supports_transcription_model` | `remote::descriptor::RemoteProviderKind::transcribes` | now public: a consumer can ask before building a request |
| `RemoteAiProvider::transcribe` (the `AiTranscribe` impl) | `remote::RemoteAiProvider::transcribe` | inherent, not a trait method (T2); takes `&AiCancellation` and returns the SDK terminal |
| `transcription_error` | — | folded into the crate's existing `error`/`status_error`/`transport_error` |
| `TRANSCRIPTION_TIMEOUT`, `MAX_TRANSCRIPTION_RESPONSE_BYTES` | `remote` (private) | values unchanged |

### From `skriuw-domain/src/transcribe.rs`

| Source symbol | Destination | Change |
| --- | --- | --- |
| `AiTranscriptionRequest` + `validate` | `transcription::AiTranscriptionRequest` | same fields; identifiers now use the path-safe grammar (T3); `Debug` redacts the recording (T4) |
| `AiTranscriptionTerminal` | `transcription::AiTranscriptionTerminal` | same four states, now `#[non_exhaustive]` |
| `AiTranscriptionModel` + `validate` | `transcription::AiTranscriptionModel` | same fields, no serde derives (T5) |
| `MAX_AI_AUDIO_BYTES`, `MAX_AI_TRANSCRIPT_BYTES`, `MAX_AI_LANGUAGE_BYTES`, `AI_AUDIO_MIME_TYPES` | `transcription` | values unchanged; Skriuw keeps its own copies for its renderer contract |
| `AiValidationError` | `transcription::AiTranscriptionError` | a closed static field name, matching `AiModelListingError` rather than importing the core's request vocabulary |
| `AiTranscribe` | — | stays Skriuw's: it is the seam Skriuw's shell depends on, and the SDK is one implementation of it |
| `AiTranscriptionResult` | — | stays Skriuw's: it is an IPC contract with a generated schema, and the SDK returns a transcript rather than a DTO |

## Behavior changes

Five, each deliberate and each with a test.

**T1. The transcription endpoint is destination-checked.** Skriuw built the URL
and sent it. Here it goes through the same `stays_on_destination` filter as the
completion and listing endpoints, so a model id that escaped its path segment
could not reach another host. Proven by
`every_endpoint_stays_on_the_disclosed_destination`, which already covered the
other endpoints and now has one more to walk.

**T2. `transcribe` is inherent rather than a trait method.** Skriuw implemented
its domain `AiTranscribe` trait on the provider. Most descriptors do not
transcribe at all, and the crate already refuses to make listing a trait method
for that reason. The projection onto `AiTranscribe` moves to Skriuw, where the
trait lives. Proven by `refuses_transcription_on_a_descriptor_that_has_no_adapter`.

**T3. Identifiers use the path-safe grammar.** Skriuw's `validate_identifier`
allowed `..` and empty path segments; `valid_model_identifier` does not, and a
Gemini transcription model id is joined into the request URL. This is the same
rule `AiModelListing` already uses. Proven by
`rejects_identifiers_that_could_traverse_an_endpoint_path`, which the shared
validator already covered.

**T4. `Debug` no longer prints the recording.** Skriuw derived `Debug` on a
struct holding `Vec<u8>` of audio, so any debug formatting of a request emitted
the recording as a byte list. The impl here prints `audio: <N bytes>`. Proven by
`never_debug_prints_the_recording`.

**T5. Nothing here is serialized.** Skriuw's `AiTranscriptionModel` derives
`Serialize`/`JsonSchema` because it crosses an IPC boundary. The SDK's does not,
and `AiTranscriptionRequest` deliberately has no serde derives at all. No entry
was added to `specs/`, and `generate_all()` is unchanged. Enforced by the schema
drift check, which stays clean because nothing was added to it.

## Preserved defects and known holes

Characterized, not fixed. Correcting any of these is a separate approved change
with its own fixtures, per ADR 0003 §6.

**H1. Cancellation cannot interrupt an upload.** A transcription is one request
and one response, so cancellation is checked before the socket opens and again
before the transcript is parsed, and nowhere in between. A cancelled request that
has already sent a 20 MiB recording still uploads it. Same hole the completion
path has for a blocked read.

**H2. The timeout is one fixed 120 seconds.** It covers upload plus inference
together, and is not derived from the recording's size, so a large upload on a
slow link consumes the same budget a small one gets. Skriuw's value, kept.

**H3. `MAX_AI_AUDIO_BYTES` is one provider's cap applied to all of them.** 25 MiB
is Groq's published limit. Gemini's differs. A recording between the two is
refused for both, which is the deliberate trade — predictable refusal over a
per-provider surprise — but it is a policy choice, not a fact about Gemini.

**H4. A Gemini transcription is a generation call, so it can hallucinate.** The
instruction asks for a verbatim transcript, and nothing verifies that what comes
back is one. A model that answers the audio instead of transcribing it produces a
`Done` terminal. Groq's Whisper endpoint has no such failure mode. This asymmetry
is inherited, not introduced.

**H5. The transcript size check counts bytes after the provider sent them.** A
response under `MAX_TRANSCRIPTION_RESPONSE_BYTES` but over
`MAX_AI_TRANSCRIPT_BYTES` is fully received before it is refused.

## Tests

### Ported from Skriuw

| Source test | Here |
| --- | --- |
| `uploads_groq_recordings_as_multipart_and_parses_the_transcript` | same name |
| `sends_gemini_recordings_inline_and_parses_the_transcript` | same name |
| `never_opens_a_socket_without_a_credential` | `never_opens_a_socket_to_transcribe_without_a_credential` |
| `refuses_a_model_the_provider_does_not_transcribe_with` | same name |
| `maps_rejected_keys_without_echoing_the_body` | `maps_a_rejected_transcription_key_without_echoing_the_body` |
| `fails_visibly_on_malformed_transcription_data` | same name |
| `stops_before_sending_when_the_request_is_already_cancelled` | `stops_before_sending_when_the_transcription_is_already_cancelled` |
| `ships_a_valid_transcription_catalogue_for_shipped_adapters` | same name |
| `validates_bounded_transcription_requests` | `accepts_a_bounded_recording_with_or_without_a_hint` + `rejects_unbounded_or_empty_audio` |
| `refuses_unsupported_containers_and_malformed_hints` | same name; also asserts every accepted container |
| `validates_catalogue_entries` | same name |

### Deliberately not ported

| Source test | Why |
| --- | --- |
| `routes_transcription_through_the_credential_gate` (Skriuw `app/src-tauri`) | asserts Skriuw's lazy startup and consent gate, which the SDK does not own |
| the staged-audio queue tests | Skriuw's queue, not a provider concern |
| `voice-dictation.test.ts` | renderer behavior |

### New here

| Test | What it proves |
| --- | --- |
| `refuses_transcription_on_a_descriptor_that_has_no_adapter` | a DeepSeek transcription is refused before the network, not attempted (T2) |
| `never_debug_prints_the_recording` | T4 |

No live transcription test exists, not even an `#[ignore]`d one: a live check
would upload a real recording to a paid endpoint on every opt-in run, and there
is no fixture-free way to make that cheap. The completion path's live check
stays the only one.

## Dependencies

`base64` enters the workspace, and `reqwest` gains its `multipart` feature. Both
are transcription request syntax. Neither adds a transport: the client, the
`redirect::Policy::none()` and the destination check are the ones already there.
The transitive additions `Cargo.lock` records are `mime`, `mime_guess` and
`unicase`, all pulled by `reqwest`'s multipart support.

Everything lives behind the existing `remote` feature. `--no-default-features`
compiles the crate without any of it.
