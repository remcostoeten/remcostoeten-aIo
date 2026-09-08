# ADR 0006: speech-to-text in the provider adapters

Status: accepted 2026-09-09.

## Context

Skriuw shipped voice dictation in `9c474ed9`, on the same bring-your-own-key
adapters it already used for completions. The provider half of that work lives
in `skriuw-ai-remote/src/transcribe.rs`: a Groq `multipart/form-data` upload to
a Whisper endpoint, a Gemini base64 `inlineData` part on an ordinary
`generateContent` call, an endpoint join per provider, a filename mapping, a
transcript parser, and the mapping from HTTP status onto the typed error
vocabulary.

That is provider request syntax, which is exactly the category `ai-providers`
was extracted to own. Leaving it in Skriuw is the anomaly, and it is a blocking
one: Skriuw's `ai-sdk-phase-3-extraction` branch cannot rebase onto `daddy`
without conflict, because a thin `skriuw-ai-remote` and a 513-line transcription
adapter cannot both be true of the same file.

Three ways out were considered.

1. Move the adapter into `ai-providers`. Chosen.
2. Keep the adapter in Skriuw and add `base_url()`, `client()` and
   `credentials()` accessors to `RemoteAiProvider`. Rejected: those are exactly
   what ADR 0003 §4 hid, and re-exposing them would let request input redirect a
   saved vendor key to an arbitrary host. Unblocking a rebase is not a reason to
   undo a security decision.
3. Keep the adapter in Skriuw with its own HTTP client and credential path.
   Rejected: a second credential-authorization path is a second thing to audit,
   and it would drift from the one the completion path uses.

ADR 0001 lists transcription under Non-goals, and `docs/roadmap.md` lists it as
not scheduled "until a concrete approved requirement exists". A shipped feature
in a real consumer, blocked on this exact boundary, is that requirement. It was
approved explicitly on 2026-09-09.

## Decision

### 1. The adapter moves; the policy stays

`ai-providers` takes the request syntax and nothing else: endpoint construction,
the two request bodies, the filename mapping, transcript parsing, the status and
transport error mapping, and the catalogue of models an adapter mapping exists
for.

Skriuw keeps what is its own: the `AiTranscribe` domain seam, its serialized
`AiTranscriptionResult` and `AiTranscriptionModel` IPC contracts, the renderer's
`MediaRecorder` container agreement, the consent and credential gate, the staged
audio queue, and prompt retention. `ai-providers` gains no dependency on
`skriuw-domain`; Skriuw projects the SDK's types onto its own, exactly as it
already projects `AiModelListing` onto `RemoteAiModelListing`.

### 2. Exact types

Rust-side, all in `ai_providers`, all behind the existing `remote` feature:

```rust
pub struct AiTranscriptionRequest {
    pub request_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub mime_type: String,
    pub language: Option<String>,
    pub audio: Vec<u8>,
}

pub enum AiTranscriptionTerminal {   // #[non_exhaustive]
    Done { transcript: String },
    Cancelled,
    Timeout,
    ProviderError(AiProviderError),
}

pub struct AiTranscriptionModel { pub provider_id: String, pub model_id: String, pub label: String }
pub struct AiTranscriptionError { pub field: &'static str }

impl RemoteAiProvider {
    pub fn transcribe(&self, request: &AiTranscriptionRequest, cancellation: &AiCancellation)
        -> AiTranscriptionTerminal;
}
impl RemoteProviderKind { pub fn transcribes(self, model_id: &str) -> bool; }
pub fn transcription_models() -> Vec<AiTranscriptionModel>;
```

plus `MAX_AI_AUDIO_BYTES` (25 MiB), `MAX_AI_TRANSCRIPT_BYTES` (512 KiB),
`MAX_AI_LANGUAGE_BYTES` (16) and `AI_AUDIO_MIME_TYPES`.

`transcribe` is inherent rather than a trait method, for the reason listing is:
most providers do not transcribe, and a fake or local adapter should not have to
pretend otherwise.

### 3. Validators

`AiTranscriptionRequest::validate` checks the three identifiers against
`valid_model_identifier` — the path-safe grammar, because a Gemini model id is
joined into the request URL — requires `mime_type` to be one of
`AI_AUDIO_MIME_TYPES`, requires any language hint to be non-empty, at most 16
bytes, and ASCII alphanumeric or `-`, and requires the recording to be non-empty
and at most `MAX_AI_AUDIO_BYTES`. `AiTranscriptionModel::validate` checks the two
identifiers and the label. Both return the first violated rule as a closed
static field name, never provider or application text.

### 4. No shared contract, and therefore no TypeScript counterpart

Nothing added here is serialized. `AiTranscriptionRequest` is deliberately not
`Serialize`: audio is the most sensitive payload the SDK handles, and one derive
is all it takes for a recording to reach a log, a history record or a crash
report. Its `Debug` impl prints `audio: <N bytes>` for the same reason. The
catalogue is descriptor data, in the sense `crates/ai-providers/src/schema.rs`
already uses — "descriptors are code rather than data a consumer exchanges" — so
it gets no schema either.

No entry is added to `specs/schemas/`, `specs/fixtures/` or `generate_all()`, and
`packages/ai-sdk` gains no transcription surface. This is consistent with
`list_models`, whose `AiModelListing` also has no TypeScript counterpart. The
cross-language parity rule in AGENTS.md governs *shared contracts*; a capability
that serializes nothing is not one. Should a TypeScript consumer ever need
transcription, that is a new contract gate, not an extension of this one.

### 5. Fixtures

Positive: a Groq 200 whose captured request must contain
`POST /openai/v1/audio/transcriptions`, `authorization: Bearer`,
`multipart/form-data`, the model id and `filename="recording.webm"`; a Gemini 200
whose captured request must contain
`POST /v1beta/models/gemini-2.5-flash:generateContent`, `x-goog-api-key`,
`inlineData` and `audio/webm`, and whose two `parts` concatenate into one
transcript.

Negative: a refused credential that opens no socket; a completion model asked to
transcribe, refused before the network; a descriptor with no transcription
adapter, likewise refused; a 401 whose body embeds the key, asserted absent from
the error message; a 200 carrying `not json`, mapped to `MalformedResponse`; a
pre-cancelled request that never connects. Plus five unit cases over the
validators and one asserting `Debug` never prints the recording.

All against local fixture servers. No live transcription test exists, not even an
`#[ignore]`d one: a live check would upload a real recording to a paid endpoint,
and there is no fixture-free way to make that cheap.

### 6. Endpoint and credential binding are unchanged

The new endpoints are joined onto the descriptor's own base URL, filtered
through `stays_on_destination`, and refused if they leave the disclosed host —
the same construction `list_models` and `complete` use. `with_base_url` stays
crate-private. Credentials attach through the existing `authorize`, so a Gemini
transcription sends `x-goog-api-key` and a Groq one sends
`Authorization: Bearer`, and no key enters a URL. Credential resolution follows
request validation and catalogue admission, and cancellation is rechecked after
the resolver returns, per ADR 0003 §4.

### 7. Migration mapping

| Skriuw, before | After |
| --- | --- |
| `skriuw_ai_remote::transcribe` request/response syntax | `ai_providers::RemoteAiProvider::transcribe` |
| `skriuw_ai_remote::ai_transcription_models` | `ai_providers::transcription_models`, projected onto `skriuw_domain::AiTranscriptionModel` |
| `skriuw_domain::AiTranscriptionRequest` | built from the domain type into `ai_providers::AiTranscriptionRequest` at the adapter |
| `ai_providers::AiTranscriptionTerminal` | projected onto `skriuw_domain::AiTranscriptionTerminal` |
| `skriuw_domain::{AiTranscribe, AiTranscriptionResult, MAX_AI_*}` | unchanged, still Skriuw's |

Skriuw's `AiTranscribe` impl becomes a projection: build, call, map. Its own
transcription tests stay where they are; the ones that assert provider wire
syntax are the ones this ADR moves.

## What this ADR does and does not authorize

Authorized: the code above, a `workspace.package.version` move to 0.3.0, and the
annotated git tag `ai-v0.3.0`.

**Not** authorized, and each still its own instruction: publishing anything to
npm, adding a transcription surface to `packages/ai-sdk`, adding a serialized
transcription contract, extending transcription to a provider not listed here,
and any live transcription test.

## Consequences

`specs/VERSION` stays 0.2.0 and both npm packages stay 0.2.0, because no shared
contract moved. This amends ADR 0005's versioning rule: the single number binds
`specs/VERSION` to the *published* artifacts, and a Rust-only change that adds no
shared contract may take a git tag without an npm release. The invariant that
matters — that a consumer cannot pair a 0.2.0 package with a crate speaking a
different contract — is preserved, because the contract did not move. The cost is
that `workspace.package.version` and `specs/VERSION` are no longer the same
string, and a reader must now know which of the two answers a given question.

ADR 0001's Non-goals no longer lists transcription. Everything else in that list
stands, and one approved capability is not a precedent for the next: agents,
tools, embeddings and the rest still require their own requirement and their own
gate.

`reqwest` gains its `multipart` feature and the workspace gains `base64`. Both
are transcription request syntax rather than a new transport; the HTTP client,
the redirect policy and the destination check are the ones already there.
