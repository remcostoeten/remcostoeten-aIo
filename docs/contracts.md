# Contracts (conceptual v1)

Status: Phase 0 design. Pseudocode only. Nothing here compiles; everything here is normative for the shapes Phase 1 extracts and Phase 4 mirrors.

Notation: `type Name = { field: Type }` for records, `A | B` for discriminated unions (tag field `type` on the wire), `Name?` for optional, `Id<Rule>` for a validated identifier. Wire field names are camelCase; union tags are snake_case. Rust field names are snake_case with `#[serde(rename_all = "camelCase")]`.

For every contract the table records: **Purpose**, **Owner** (which unit defines it), **Serialization** (wire requirements), **Shared** (must Rust and TS represent the same semantics), **Schema** (belongs in `specs/schema`).

Primary source for shapes: `/home/remcostoeten/dev/skriuw/crates/skriuw-domain/src/ai.rs`, `remote_ai.rs`, `ai_history.rs`, `local_ai.rs`, and `crates/skriuw-ai/src/lib.rs`.

---

## 1. Bounds and identifiers

Carried from `skriuw-domain/src/ai.rs` lines 10–17, renamed:

```text
MAX_IDENTIFIER_BYTES     = 128
MAX_PROMPT_BYTES         = 1 MiB          (sum of all message text in a request)
MAX_RESPONSE_BYTES       = 4 MiB          (global cap, independent of the request's maxOutputBytes)
MAX_DELTA_BYTES          = 64 KiB
MAX_DURATION_MS          = 300_000
MAX_RETRIES              = 2
MAX_ERROR_MESSAGE_BYTES  = 1_024
MAX_TOKEN_COUNT          = 1_000_000_000
MAX_MESSAGES             = 256            (new; Betalingen caps at 20, chat needs more headroom)
MAX_STOP_SEQUENCES       = 4              (new; provider minimum common denominator)
```

Identifier rules (validated at every trust boundary, newtype in Rust, branded type behind a parse function in TS):

```text
Id<Provider>   non-empty, ≤ 128 bytes, matches [a-z0-9][a-z0-9-]*                 e.g. "groq", "openai", "ollama", "fake"
Id<Model>      non-empty, ≤ 128 bytes, printable ASCII, no whitespace, no "?#/.." traversal, safe as a URL path segment after encoding
               (must admit ".", "/", ":", "-", "_": "openai/gpt-oss-120b", "llama3.2:3b", "z-ai/glm-5.3-flash")
Id<Request>    non-empty, ≤ 128 bytes, [A-Za-z0-9._:-]+  ; UUID v4 recommended; unique per runtime while active
Id<Origin>     same rule as Id<Request>; application-chosen tag ("editor:rewrite", "dora:sql", "betalingen:data-chat")
Id<SchemaName> same rule as Id<Request>; the name given to a json_schema response format
```

Invalid identifiers are `ValidationError`, which is not a `ProviderError` (validation of *our* input is a separate failure family, as Skriuw's `AiValidationError`).

---

## 2. Contracts

### 2.1 ModelRef

```text
type ModelRef = {
  providerId: Id<Provider>
  modelId:    Id<Model>
}
```

| | |
| --- | --- |
| Purpose | Identity of a model. The request key, the settings value, the catalog key, the history key. |
| Owner | core |
| Serialization | plain object; both fields required; no defaults |
| Shared | yes |
| Schema | yes (`model-ref.schema.json`, referenced by others) |

Not a single dotted string (see `architecture.md` §2.5).

### 2.2 Capability and CapabilitySupport

```text
enum Capability      = streaming | json_mode | json_schema | tools | vision | audio | embeddings | reasoning
enum CapabilitySupport = yes | no | unknown
```

| | |
| --- | --- |
| Purpose | Closed vocabulary of things a model may do, and tri-state knowledge about each. Only `streaming`, `json_mode`, `json_schema` have behavior in v1; the rest are reserved names. |
| Owner | core; the enum list is also published as `specs/enums/capabilities.json` |
| Serialization | snake_case string enums |
| Shared | yes |
| Schema | yes |

Tri-state is justified: both desktop apps accept user-typed model ids, so `unknown` is the common case. `no` short-circuits with `unsupported_capability`; `unknown` attempts.

### 2.3 ModelInfo

```text
enum ModelInfoSource = catalog | listed | declared

type Locality =
  | { type: "local" }
  | { type: "remote", destination: string }        // host shown in disclosures, e.g. "api.groq.com"

type Pricing = {
  inputMicrosPerMtok:  u64
  outputMicrosPerMtok: u64
  pricingAsOf:         Date (ISO-8601 date)
}

type ModelInfo = {
  model:               ModelRef
  label?:              string
  contextWindowTokens?: u32
  maxOutputTokens?:    u32
  capabilities:        Map<Capability, CapabilitySupport>   // absent key == unknown
  locality:            Locality
  pricing?:            Pricing
  source:              ModelInfoSource
}
```

| | |
| --- | --- |
| Purpose | Optional description of a model; what a UI shows, what the structured-output ladder and a future router read. |
| Owner | core (type); `ai-providers` (catalog data + merge: catalog beats listed beats unknown, from Skriuw `RemoteAiModelDirectory::merge` and Dora `merge_models`) |
| Serialization | integers only (micro-dollars per million tokens, as `crates/skriuw-ai-remote/models.json`); no floats |
| Shared | yes (catalog data is consumed by both languages) |
| Schema | yes |

### 2.4 Role, ContentPart, Message

```text
enum Role = system | user | assistant

ContentPart =                                   // #[non_exhaustive] in Rust; TS union may grow
  | { type: "text", text: string }              // v1: the only variant

type Message = {
  role:    Role
  content: ContentPart[]                        // non-empty
}
```

| | |
| --- | --- |
| Purpose | Prompt input. Replaces Skriuw's `systemPrompt + userPrompt` pair and Dora's `USER:/ASSISTANT:` string packing with a proper history (Betalingen already has `messages[]`). |
| Owner | core |
| Serialization | tagged `content` parts so tool/vision parts can be added later without breaking; `role` is a string enum |
| Shared | yes |
| Schema | yes |

Constraints: at most one `system` message and it must be first (providers differ in how they carry it; the adapter maps it); last message must be `user` for completion; total text ≤ `MAX_PROMPT_BYTES`. A convenience constructor `simple(system, user)` covers Skriuw's case. No `tool` role, no `ToolCall`/`ToolResult` parts, no tool definitions in v1 (audit §23: no consumer uses tools).

### 2.5 CompletionParameters

```text
type CompletionParameters = {
  maxOutputBytes:     u32            // 1 ..= MAX_RESPONSE_BYTES; enforced pre-delivery (Skriuw)
  maxOutputTokens?:   u32            // forwarded to the provider when supported (Dora, Betalingen, autocomplete)
  timeoutMs:          u32            // 1 ..= MAX_DURATION_MS; per-request deadline
  retryCount:         u8             // 0 ..= MAX_RETRIES; retries happen only before the first delta
  temperatureMillis?: u16            // 0 ..= 2000
  topPMillis?:        u16            // 0 ..= 1000
  stop:               string[]       // 0 ..= MAX_STOP_SEQUENCES, each non-empty ≤ 64 bytes
  responseFormat:     ResponseFormat
}
```

| | |
| --- | --- |
| Purpose | Bounded generation controls. Everything a provider may need that is not the model or the messages. |
| Owner | core |
| Serialization | integers only (fixed-point millis keep fixtures byte-identical across languages, as Skriuw); all fields present except optionals; `deny_unknown_fields` |
| Shared | yes |
| Schema | yes |

There is deliberately **no** `extra`, `options`, `metadata`, or provider-specific bag. Provider-specific behavior is expressed through typed capabilities and typed fields added by spec version, never by an untyped map (AGENTS.md; Skriuw ADR-0033 rejected an options map).

### 2.6 ResponseFormat

```text
ResponseFormat =
  | { type: "text" }
  | { type: "json" }                                              // "a JSON object", no schema
  | { type: "json_schema", name: Id<SchemaName>, schema: JsonSchemaDocument, strict: bool }
```

| | |
| --- | --- |
| Purpose | Provider-neutral structured-output request. Execution strategy is chosen by the runtime from capabilities (`architecture.md` §2.8). |
| Owner | core |
| Serialization | tagged union; `schema` is the single dynamic-JSON payload in the request, validated as a JSON Schema document (bounded size) at the boundary |
| Shared | yes |
| Schema | yes |

### 2.7 CompletionRequest

```text
type CompletionRequest = {
  requestId:  Id<Request>
  model:      ModelRef
  messages:   Message[]
  parameters: CompletionParameters
  origin:     Id<Origin>
}

validate(request) -> Ok | ValidationError
```

| | |
| --- | --- |
| Purpose | The single seam every AI feature calls. |
| Owner | core |
| Serialization | `deny_unknown_fields` (it crosses a trust boundary: Tauri IPC, HTTP); every field required |
| Shared | yes |
| Schema | yes (`completion-request.schema.json`) |

`origin` is an application tag (Skriuw `origin`, Dora `usage.source`), validated but uninterpreted by the SDK; it lands on the `RunRecord`.

### 2.8 CompletionDelta

```text
type CompletionDelta = {
  requestId: Id<Request>
  sequence:  u32          // 0-based, strictly increasing by 1 within a request
  text:      string       // non-empty, ≤ MAX_DELTA_BYTES, no invalid UTF-8
}
```

| | |
| --- | --- |
| Purpose | One streamed text fragment. |
| Owner | core |
| Serialization | as above; validated on emit by the runtime |
| Shared | yes |
| Schema | yes (inside the event schema) |

### 2.9 Usage and UsageSource

```text
enum UsageSource = reported | estimated

type Usage = {
  inputTokens:  u64      // ≤ MAX_TOKEN_COUNT
  outputTokens: u64
  source:       UsageSource
}
```

| | |
| --- | --- |
| Purpose | Token accounting. `reported` comes from the provider (`stream_options.include_usage`, Gemini `usageMetadata`, Ollama eval counts); `estimated` is byte/4 (Skriuw `estimate_ai_tokens`). |
| Owner | core |
| Serialization | integers |
| Shared | yes |
| Schema | yes |

In the `done` event `usage` is present only when reported. The runtime fills an estimate on the `RunRecord` when absent, and always estimates for cancelled/failed runs.

### 2.10 FinishReason

```text
enum FinishReason = stop | length | content_filter | other
```

| | |
| --- | --- |
| Purpose | Why the provider stopped. New relative to Skriuw; nothing consumes it today; a future router needs `length` vs `stop`, and UIs can warn on truncation. |
| Owner | core |
| Serialization | string enum; optional on `done` |
| Shared | yes |
| Schema | yes |

### 2.11 ErrorCategory

```text
enum ErrorCategory =
  | missing_credential          // no credential resolvable for the provider (incl. refused by policy)
  | invalid_credential          // 401 / 403
  | invalid_request             // 400 / 413 / 422; our request was rejected
  | model_unavailable           // 404 model; model retired or not permitted on this account
  | unsupported_capability      // pre-flight: capability known "no" (e.g. json_schema)
  | rate_limited                // 429
  | quota_exceeded              // 402 / billing
  | timeout                     // deadline reached at transport level before terminalization  (see note)
  | network                     // DNS / connect / reset / TLS
  | provider_unavailable        // 5xx / maintenance
  | cancelled                   // see note
  | malformed_response          // unparseable or oversized stream / body
  | structured_output_invalid   // parsed JSON failed schema validation after any repair attempt
  | local_runtime_unavailable   // local server not reachable (Ollama connection refused)
  | local_model_missing         // local runtime reachable, model not pulled
  | internal                    // SDK bug / unexpected state; never a provider condition
```

Note on `timeout` and `cancelled`: as **stream terminals** these are their own event kinds (`timeout`, `cancelled`), never `provider_error`. The categories exist so that non-stream results (`CompletionOutcome`, `RunRecord.errorCategory`, `verify`, `listModels`) and the shared error-category table are total. A `provider_error` event must not carry `cancelled` or `timeout`.

| | |
| --- | --- |
| Purpose | The bounded semantic state applications branch on. Provider HTTP status codes are never the primary contract. |
| Owner | core; also published as `specs/enums/error-categories.json` with columns `defaultRecovery`, `retryableBeforeFirstDelta`, `fallbackEligible` |
| Serialization | snake_case string enum; `#[non_exhaustive]` in Rust; TS consumers must handle exhaustively with a documented `internal` fallback arm for forward compatibility |
| Shared | yes |
| Schema | yes |

Mapping to Skriuw's current `AiProviderErrorCategory`: `UnavailableProvider` splits into `provider_unavailable`, `model_unavailable`, `local_runtime_unavailable`, `local_model_missing`; `RejectedRequest` becomes `invalid_request`; `TransportFailure` becomes `network`; `QuotaExhausted` becomes `quota_exceeded`; `InternalFailure` becomes `internal`. New: `unsupported_capability`, `structured_output_invalid`.

### 2.12 RecoveryAction

```text
enum RecoveryAction =
  | configure_credential | retry | choose_different_model | check_provider_status
  | reduce_request | contact_provider | start_local_runtime | pull_model | none
```

| | |
| --- | --- |
| Purpose | What a UI should offer. Decouples copy from category (Skriuw `AiRecoveryAction`). |
| Owner | core |
| Serialization | string enum; `#[non_exhaustive]` |
| Shared | yes |
| Schema | yes |

Default mapping (overridable per error by the adapter):

```text
missing_credential → configure_credential      invalid_credential → configure_credential
invalid_request → reduce_request               model_unavailable → choose_different_model
unsupported_capability → choose_different_model rate_limited → retry
quota_exceeded → contact_provider              timeout → retry
network → retry                                provider_unavailable → check_provider_status
cancelled → none                               malformed_response → retry
structured_output_invalid → retry              local_runtime_unavailable → start_local_runtime
local_model_missing → pull_model               internal → none
```

### 2.13 ProviderError

```text
type ErrorSource = {                      // diagnostics only
  httpStatus?:   u16
  providerCode?: string                   // bounded
  bodyExcerpt?:  string                   // bounded, control chars stripped, secrets redacted (sk-, AIza, Bearer …)
  providerRequestId?: string
}

type ValidationIssue = { path: string, message: string }   // both bounded; path is a JSON pointer

type ProviderError = {
  providerId:    Id<Provider>
  category:      ErrorCategory
  recovery:      RecoveryAction
  message:       string                   // ≤ MAX_ERROR_MESSAGE_BYTES, user-presentable, never a raw body
  retryAfterMs?: u32                      // from Retry-After or provider hint
  issues?:       ValidationIssue[]        // present only for structured_output_invalid; ≤ 32 entries
  source?:       ErrorSource              // serialized only when the runtime's diagnostics flag is on
}
```

| | |
| --- | --- |
| Purpose | Typed, provider-independent failure. |
| Owner | core |
| Serialization | `source` omitted from the wire by default (Skriuw forbids bodies in the renderer; Dora wants them in its settings key-test result, so it is opt-in per runtime) |
| Shared | yes |
| Schema | yes (`provider-error.schema.json`); `source` marked optional |

### 2.14 CompletionEvent and CompletionTerminal

```text
CompletionEvent =
  | { type: "delta",          ...CompletionDelta }
  | { type: "done",           requestId, usage?: Usage, finishReason?: FinishReason }
  | { type: "cancelled",      requestId }
  | { type: "timeout",        requestId }
  | { type: "provider_error", requestId, error: ProviderError }

CompletionTerminal =                               // what a Provider returns; the runtime attaches requestId
  | { type: "done",           usage?, finishReason? }
  | { type: "cancelled" }
  | { type: "timeout" }
  | { type: "provider_error", error: ProviderError }
```

| | |
| --- | --- |
| Purpose | The stream contract. Skriuw's `AiCompletionEvent` (`ai.rs` line 227) plus optional `finishReason`. |
| Owner | core |
| Serialization | tag `type`; every event carries `requestId`; events do **not** use `deny_unknown_fields` (consumers ignore unknown *fields* on known event kinds so an additive field is a minor bump); unknown event *kinds* are rejected |
| Shared | yes; the single most important shared contract |
| Schema | yes (`completion-event.schema.json`) |

No `final { content }` event: consumers accumulate deltas. No provider or model field on events: which model ran is on `CompletionOutcome` and `RunRecord`.

### 2.15 CredentialSource and Credential

```text
type Credential            // opaque; bytes zeroized on drop; Debug/toString print "<redacted>"; never Serialize

CredentialError =
  | { type: "missing" }
  | { type: "refused",          reason: string }     // application policy (consent, allowlist); bounded
  | { type: "store_unavailable", reason: string }    // keyring locked/absent, D-Bus failure
  | { type: "invalid" }                              // present but unusable (empty, wrong shape)

port CredentialSource {
  resolve(providerId: Id<Provider>) -> Credential | CredentialError
}
```

| | |
| --- | --- |
| Purpose | Resolve a secret at request time without the core owning storage (Skriuw `AiCredentialSource`, `remote_ai.rs` line 103). |
| Owner | core (port + env and session implementations); applications and optional crates (keyring, encrypted DB, rotation decorator) |
| Serialization | **none**. `Credential` is not serializable. `CredentialError` may be serialized for UI status but never contains the secret. |
| Shared | semantics yes (a TS `CredentialSource` returns a `Promise<Credential>` and throws/returns the same error kinds); the type itself is not on the wire |
| Schema | `CredentialError` only |

`missing` and `refused` both map to `ErrorCategory.missing_credential` on the completion path; the distinction is available to the application through `verify` and provider-state UIs.

### 2.16 Provider

```text
port Provider {
  id() -> Id<Provider>

  // Rust: synchronous, sink-based; runs on the runtime's thread; must observe `cancel` and the deadline in its read loop
  complete(request: &CompletionRequest, cancel: &Cancellation, sink: &mut EventSink) -> CompletionTerminal

  // TypeScript: pull-based; must yield exactly one terminal as the last item; must abort on `signal`
  complete(request: CompletionRequest, signal: AbortSignal) -> AsyncIterable<CompletionEvent>

  verify(model: ModelRef, credential: Credential) -> Ok | ProviderError      // default: unsupported_capability
  listModels() -> ModelInfo[] | ProviderError                                // default: empty
  modelInfo(model: ModelRef) -> ModelInfo?                                   // default: none
}

port EventSink   { sendDelta(delta: CompletionDelta) -> Ok | SinkClosed }    // Rust only
type Cancellation { cancel(); isCancelled() -> bool }                        // Rust; clonable; TS uses AbortSignal
```

| | |
| --- | --- |
| Purpose | The adapter contract. Skriuw `AiComplete` (`ai.rs` line 318) with `verify`/`listModels`/`modelInfo` added because both desktop apps need them (Dora `test_key`, Skriuw `verify_credential`, both `list_models`). |
| Owner | core (trait/type); adapters implement |
| Serialization | not serializable |
| Shared | semantics yes; shapes differ by language idiom (sink vs async iterable) |
| Schema | no |

Provider obligations, enforced by the conformance suite: validate nothing the runtime already validated, but never trust `ModelRef.modelId` as a URL segment without encoding; resolve credentials after validation; put credentials in headers only; check `cancel`/`signal` and the deadline on every read; enforce `maxOutputBytes` and `MAX_RESPONSE_BYTES` before emitting; return one terminal; never emit after returning.

### 2.17 Runtime

```text
type CompletionOutcome<T = string> = {
  requestId:    Id<Request>
  model:        ModelRef                  // the model that actually ran
  terminal:     CompletionTerminal
  value?:       T                         // present only when terminal is done (text, or the validated object)
  usage?:       Usage                     // reported or estimated
  durationMs:   u32
  attempts:     Attempt[]                 // exactly one entry until a router exists
}
type Attempt = { model: ModelRef, terminal: TerminalKind, durationMs: u32 }
TerminalKind =
  | { type: "done" } | { type: "cancelled" } | { type: "timeout" }
  | { type: "error", category: ErrorCategory }

enum StructuredStrategy = auto | native | json_mode | prompt_only
type StructuredPolicy = { strategy: StructuredStrategy, repairAttempts: 0 | 1 }

port Runtime {
  build(providers: Provider[], recorder?: RunRecorder, pricing?: Pricing, diagnostics: bool) -> Runtime

  start(request, channel: EventChannel) -> Ok | StartError        // streaming; returns after spawn (Rust) / after registration (TS)
  stream(request, signal?) -> AsyncIterable<CompletionEvent>       // TS native; Rust behind the tokio feature
  complete(request, signal?) -> CompletionOutcome<string>          // non-streaming convenience
  generateObject<T>(request, schema, policy?, signal?) -> CompletionOutcome<T>
  cancel(requestId) -> bool                                        // idempotent; true if it was active
  shutdown()                                                       // cancel all, stop accepting

  StartError = validation(ValidationError) | duplicate_request_id | unknown_provider | shutting_down
}
port EventChannel { send(event: CompletionEvent) -> Ok | SinkClosed }   // Rust: what Tauri/HTTP shells implement
```

| | |
| --- | --- |
| Purpose | The application-facing entry point. `skriuw_ai::AiCompletionService` generalized (`crates/skriuw-ai/src/lib.rs` line 44) plus conveniences. |
| Owner | core |
| Serialization | `CompletionOutcome` is serializable (for Tauri commands returning a non-stream result); `Runtime` is not |
| Shared | semantics yes |
| Schema | `CompletionOutcome` yes; `StartError` yes |

`generateObject` runs the ladder in `architecture.md` §2.8, validates the output, and returns a typed value; on failure the terminal is `provider_error` with category `structured_output_invalid` and `ProviderError.issues` populated. Structured requests default to non-streaming so repair is possible.

### 2.18 RunRecord and RunRecorder

Justified by the audit: both desktop apps persist a per-run record after terminalization (Dora `ai_usage`, Skriuw `ai_run_history`) with the same fields, and both want an origin tag. The SDK owns the record shape and the port; storage stays in the application.

```text
enum RunState = done | cancelled | timeout | error

type RunRecord = {
  requestId:      Id<Request>
  origin:         Id<Origin>
  model:          ModelRef
  state:          RunState
  errorCategory?: ErrorCategory           // when state == error
  usage:          Usage                   // always present; estimated when not reported
  costMicros?:    u64                     // from Pricing; none for local or unpriced models
  durationMs:     u32
  startedAt:      Timestamp (ms since epoch, u64)
  outputBytes:    u32
  // no prompt text, no deltas, no credentials, ever (Skriuw ADR-0033 telemetry policy)
}

port RunRecorder { record(run: RunRecord) }          // must not block delivery; called once after the terminal
port Pricing     { price(model: ModelRef) -> Pricing? }
```

| | |
| --- | --- |
| Purpose | Usage/history metadata emitted to a port. Skriuw `AiRunRecord`/`AiRunRecorder`/`AiModelPricing` (`ai_history.rs` lines 136, 335, 305) minus Skriuw's retention settings and prompt retention, which are Skriuw policy. |
| Owner | core (shape + ports); applications (persistence: Skriuw SQLite `0018`, Dora `ai_usage`) |
| Serialization | integers, no floats; camelCase |
| Shared | yes |
| Schema | yes (`run-record.schema.json`) |

---

## 3. Stream invariants

Enforced by the runtime, tested by shared fixtures, and required of every provider by the conformance suite.

1. **One request id.** Every event carries the `requestId` of the request that produced it. A consumer may reject any event with a foreign id; the reference consumer (`packages/core` `createCompletionConsumer`, from Skriuw `completion-consumer.ts`) drops them.
2. **Delta sequence is monotonic and gapless.** `sequence` starts at 0 and increases by exactly 1. Consumers may reject out-of-order or repeated sequences; the reference consumer drops them and does not resynchronize.
3. **Exactly one terminal.** Every started request emits exactly one terminal event, including when the provider is unknown, the request is rejected after registration, or the provider panics/throws (the runtime converts that to `provider_error { internal }`).
4. **Success is explicit.** Natural completion emits `done`. `done` may carry `usage` and `finishReason`.
5. **Cancellation is explicit.** A cancelled request emits `cancelled`, never silence (Dora's and Betalingen's silent abort are not carried over). If a provider returns `done` after cancellation was requested, the runtime publishes `cancelled`.
6. **Timeout is explicit.** Deadline expiry emits `timeout`, never `provider_error { timeout }`.
7. **Provider failure is typed.** Every failure is `provider_error` with a `ProviderError` whose `category` is never `cancelled` or `timeout`.
8. **No events after the terminal.** Any delta or second terminal produced by a provider after its terminal is dropped by the runtime and is a conformance failure for the provider.
9. **Closed consumer cancels.** When the channel/sink reports closed, the runtime cancels the request; the provider must stop reading promptly. "Stop forwarding" never substitutes for cancellation.
10. **Foreign and out-of-order events are the consumer's to reject, not to repair.** The runtime never reorders or re-numbers.
11. **No provider or model switch after the first delta.** Retries (`retryCount`) and any future router act only before the first delta has been delivered to the consumer. Once a delta is out, the only remaining outcomes are `done`, `cancelled`, `timeout`, or `provider_error` from the same provider and model.
12. **Retries are bounded and category-gated.** At most `retryCount` retries, only for categories flagged `retryableBeforeFirstDelta` in `specs/enums/error-categories.json` (`rate_limited` honoring `retryAfterMs` when short, `network`, `provider_unavailable`, `malformed_response`), never for `invalid_credential`, `invalid_request`, `quota_exceeded`, `cancelled`.
13. **Bounds precede delivery.** A delta exceeding `MAX_DELTA_BYTES`, or cumulative output exceeding `maxOutputBytes` or `MAX_RESPONSE_BYTES`, is never delivered; the request terminalizes with `provider_error { malformed_response }`.
14. **Duplicate request ids are refused at start**, not silently replaced (`StartError.duplicate_request_id`).
15. **Recording happens once, after the terminal**, with the terminal's state; recorder failures never affect delivery.

---

## 4. Error semantics

### 4.1 Distinguishable conditions

Each condition below is one `ErrorCategory`; applications branch on it, never on strings or status codes.

| Condition | Category | Typical source | Retry before first delta | Router fallback-eligible |
| --- | --- | --- | --- | --- |
| Missing credential (incl. refused by policy) | `missing_credential` | `CredentialSource` | no | yes, to a configured route |
| Invalid credential | `invalid_credential` | 401/403 | no | only to a route with a different credential, never blindly |
| Invalid request | `invalid_request` | 400/413/422 | no | no (except a known capability gap) |
| Unavailable model | `model_unavailable` | 404 model | no | yes, another model |
| Unsupported capability | `unsupported_capability` | pre-flight | no | yes |
| Rate limited | `rate_limited` | 429 | once, after `retryAfterMs` if short | yes |
| Quota exceeded | `quota_exceeded` | 402 | no | yes; disable route until config changes |
| Timeout | `timeout` (terminal) | deadline | once (before first delta) | yes |
| Network failure | `network` | connect/reset/TLS | once | yes |
| Provider unavailable | `provider_unavailable` | 5xx | once with backoff | yes |
| Cancellation | `cancelled` (terminal) | consumer | never | never |
| Malformed response | `malformed_response` | parse/bounds | once | yes |
| Invalid structured output | `structured_output_invalid` | validator | one repair | yes, to a model with native schema |
| Local runtime unavailable | `local_runtime_unavailable` | connection refused to local endpoint | no | only if the application permits remote fallback |
| Local model missing | `local_model_missing` | local 404 | no | to another local model |
| Internal failure | `internal` | SDK bug | no | no |

### 4.2 Rules

- Status code -> category mapping is table-driven and shared (`fixtures/errors/`), with the default mapper in the provider skeleton; adapters override only for documented provider quirks (a comment is required there, per `AGENTS.md`).
- Dora's key rotation on 401/403/5xx is not carried over. Rotation is a `CredentialSource` decorator for `rate_limited` and `quota_exceeded` only.
- `message` is bounded, control-character-stripped, and never contains a provider body. `ErrorSource.bodyExcerpt` is bounded, redacted, and only serialized under the runtime's diagnostics flag.
- `LocalRuntimeError` (install/pull/spawn failures in `ai-ollama-runtime`) is a separate type with its own category enum (Skriuw `LocalAiErrorCategory`). It is never a `ProviderError`.
- `ValidationError` (our own input is invalid) is a separate type surfaced as `StartError.validation`, never as `provider_error`.

---

## 5. Serialization summary

| Contract | Wire | `deny_unknown_fields` | Schema file |
| --- | --- | --- | --- |
| `ModelRef` | object | yes | `model-ref` |
| `Capability`, `CapabilitySupport` | string enum | n/a | `enums/capabilities.json` |
| `ModelInfo` | object, integers | no (catalog data may gain fields) | `model-info` |
| `Message`, `ContentPart`, `Role` | tagged parts | request-level yes | inside `completion-request` |
| `CompletionParameters`, `ResponseFormat` | integers, tagged | yes | inside `completion-request` |
| `CompletionRequest` | object | **yes** (trust boundary) | `completion-request` |
| `CompletionEvent` | tagged union | **no** on fields, unknown kinds rejected | `completion-event` |
| `ProviderError`, `ErrorCategory`, `RecoveryAction` | object + string enums | no | `provider-error`, `enums/error-categories.json` |
| `Usage`, `FinishReason` | integers, string enum | n/a | inside `completion-event` |
| `CompletionOutcome` | object | no | `completion-outcome` |
| `RunRecord` | object, integers | no | `run-record` |
| `CredentialError` | tagged union | n/a | `credential-error` |
| `Credential`, `Provider`, `Runtime`, `Cancellation`, `EventSink` | **not serializable** | — | — |

A `specVersion` (semver string) is embedded in every schema file and exported by both cores. Changing the meaning of any wire value bumps it (ADR 0002).
