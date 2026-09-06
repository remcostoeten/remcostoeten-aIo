# Contracts: extraction baseline and deferred designs

Status: Phase 0 closed 2026-09-06. No production code is authorized until Phase 1 is explicitly approved. Sections 1–3 describe the Phase 1 extraction baseline, its known gaps, and the two selected lifecycle/recording decisions. Section 4 constrains later design; it is not a Phase 1 implementation list. The selection rationale is in `architecture.md` §5.

Pseudocode uses records and tagged unions. Wire fields are camelCase; tags and bounded string enums are snake_case. Keep existing Rust public names initially to minimize re-export changes. Names shortened below are conceptual, not a requirement to rename every source type.

## 1. Bounds and identifiers

Source: Skriuw `crates/skriuw-domain/src/ai.rs`, constants and validation functions.

| Quantity | Existing rule to preserve |
| --- | --- |
| Identifier | Non-empty, at most 128 UTF-8 bytes, ASCII letters/digits or `- _ . : /` |
| Prompt | Combined system/user text at most 1 MiB; either or both may be empty |
| Output | Requested limit 1 through 4 MiB |
| Delta | At most 64 KiB; empty deltas are currently accepted |
| Timeout | 1 through 300,000 milliseconds |
| Retry count | 0 through 2; accepted but does not cause retries |
| Temperature/top-p | Nullable integers, 0 through 1,000 |
| Error message | Non-empty, at most 1,024 UTF-8 bytes; constructor normalizes whitespace/control characters |
| Input/output usage | Each 0 through 1,000,000,000 tokens |

Do not replace the identifier grammar with vendor-only lowercase ids in Phase 1. Skriuw's own test uses `provider.local/v1`. The core grammar allows `..`; provider model authority and URL construction are separate boundaries. Do not describe a validated core identifier as a safe filesystem path or unencoded URL segment.

These DTOs have explicit validation methods; Rust deserialization alone does not enforce every bound. Preserve the methods and their invocation. A later validated newtype layer must retain the accepted set and serialization, and be justified against the impact on Rust struct construction. Never silently narrow accepted inputs while calling the change a rename.

## 2. Phase 1 contract inventory

### 2.1 CompletionParameters

```text
CompletionParameters = {
  maxOutputBytes: u32,
  timeoutMs: u32,
  retryCount: u8,
  temperatureMillis: u16 | null,
  topPMillis: u16 | null
}
```

Default values remain 262144, 60000, 0, null, null. Source serde also accepts missing nullable sampling fields. No stop sequences, response format, token limit, provider options, or retry implementation are added.

### 2.2 CompletionRequest

```text
CompletionRequest = {
  requestId: Identifier,
  providerId: Identifier,
  modelId: Identifier,
  systemPrompt: string,
  userPrompt: string,
  parameters: CompletionParameters
}
```

Every non-nullable field is required. Unknown fields are rejected, including nested parameters. `origin` remains a separate argument to the service and is never sent to a provider in the request. Skriuw validates origin at its command boundary; extraction must not silently move or change that policy.

This is already provider-neutral. A nested `ModelRef` and `messages` are useful later for multi-turn consumers, but are breaking wire changes rather than prerequisites for extraction.

### 2.3 CompletionDelta and Usage

```text
CompletionDelta = { requestId: Identifier, sequence: u32, text: string }
Usage = { inputTokens: u64, outputTokens: u64 }
```

Both have strict unknown-field rejection and explicit validators. Usage on `done` is provider-reported. Estimation source belongs to run accounting, not a new required field on the existing usage object.

### 2.4 ProviderError and recovery

```text
ErrorCategory =
  unavailable_provider | missing_credential | invalid_credential |
  quota_exhausted | rate_limited | rejected_request | transport_failure |
  malformed_response | internal_failure

RecoveryAction =
  configure_credential | retry | choose_different_model |
  check_provider_status | reduce_request | contact_provider | none

ProviderError = {
  providerId: Identifier,
  category: ErrorCategory,
  message: BoundedMessage,
  recoveryAction: RecoveryAction
}
```

Preserve these closed enums and wire field names. Neither `timeout` nor `cancelled` is in this error enum: they are terminal variants. Recovery is a UI hint, not permission for automatic retry or fallback. No HTTP status, response body, generic diagnostics bag, or provider SDK error is added.

`ValidationError` remains separate from provider failures. The existing Rust type uses bounded variants and static field labels; those labels must not be turned into arbitrary caller-controlled states. A future shared validation DTO should use a closed field enum or validated JSON pointer plus a closed issue code. It is not necessary to serialize the existing Rust validation error in Phase 1.

### 2.5 CompletionEvent and CompletionTerminal

```text
CompletionEvent =
  { type: "delta", requestId, sequence, text } |
  { type: "done", requestId, usage: Usage | null } |
  { type: "cancelled", requestId } |
  { type: "timeout", requestId } |
  { type: "provider_error", requestId, error: ProviderError }

CompletionTerminal =
  Done { usage: Optional<Usage> } |
  Cancelled |
  Timeout |
  ProviderError(ProviderError)
```

Events use strict unknown-field rejection in the source; keep it. Unknown tags and enum values are rejected. Rust emits `usage: null`; serde accepts omitted usage. CompletionTerminal is an in-process enum, not a separately serializable wire contract today. Its conversion to an event attaches the request id.

No `finishReason`, duplicate final text, duplicated usage source, or router attempt list is added.

### 2.6 Provider, sink, channel, cancellation

```text
AiComplete.complete(&request, &cancellation, &mut deltaSink) -> CompletionTerminal
AiEventSink.send_delta(delta) -> Result<(), Closed>
AiCompletionChannel.send(event) -> Result<(), Closed>
AiCancellation.cancel()
AiCancellation.is_cancelled() -> bool
```

The completion trait is synchronous, sink-based, Send + Sync. Cancellation is a clonable atomic flag. Sinks receive deltas only; the service publishes the terminal. The channel is the application's delivery port, not a Tauri type.

The existing trait has no `id()`, registry factory, admin methods, or credential dependency. Applications register `(providerId, provider)` pairs. A future provider-administration API can be a separate typed capability; fake and local providers should not need dummy credentials or pretend that unsupported model listing is a successful empty list.

These ports are not serializable. A later TS implementation can use AsyncIterable and AbortSignal without trying to serialize callbacks, signals, futures, or trait objects.

### 2.7 CompletionService and StartError

```text
new(providerRegistrations) -> service
recording(recorder, pricing) -> service
start(origin, request, channel) -> Result<(), StartError>
cancel(requestId) -> bool
shutdown()

StartError = DuplicateRequest(requestId) | InvalidRequest | WorkerUnavailable
```

No new `unknown_provider` or `shutting_down` start error is added. The D1 hardening adds no variant to this enum; it only makes the existing `DuplicateRequest` reachable on the unknown-provider path, where the source currently returns success with a terminal instead. Unknown provider otherwise keeps producing a synchronous error terminal and returning success. Invalid requests and spawn failures return start errors and do not produce a terminal. `shutdown()` currently requests cancellation of active runs; it neither joins workers nor permanently closes admission.

Duplicate provider registrations currently overwrite via collection into a BTreeMap; rejecting them is a sensible later hardening change, not existing behavior. Duplicate request checks occur only on the registered-provider path today. §3.3 records the selected Phase 1 correction to that path; duplicate *provider registration* rejection is not part of it.

### 2.8 Recording and pricing: selected seam (D2)

Skriuw's service depends on `AiRunRecord`, `AiRunTokens`, `AiRunState`, `AiRunPrompts`, `AiRunRecorder`, `AiModelPrice`, `AiModelPricing`, token estimation, and cost arithmetic. It does not need history filters, retention settings, aggregates, SQLite, consent, or the remote catalog's implementation of the pricing port.

Existing record vocabulary must be accounted for explicitly:

```text
AiRunRecord = {
  runId, startedAtMs: i64, origin, providerId, modelId,
  prompts: Optional<{systemPrompt, userPrompt}>,
  state: done | cancelled | timed_out | failed,
  errorCategory: Optional<ErrorCategory>,
  durationMs: u32,
  tokens: { inputTokens, outputTokens, source: provider | estimated },
  costMicros: Optional<u64>
}
```

The service currently supplies prompts; the application's storage layer applies retention/redaction. Removing prompts without an application adapter breaks retention. This record also permits contradictory state/error combinations structurally; `run_record` constructs consistent combinations, but its validator does not enforce the relationship.

**Selected (D2): metadata-only SDK summary plus a borrowed request, with an application-owned adapter.** The alternative — temporarily preserving this optional-field record and its prompt handoff inside the SDK — is rejected as the Phase 1 seam because it would publish a prompt-bearing, structurally contradictory record as the extracted contract. The legacy record shape stays in Skriuw, where retention already lives, and must never become the universal cross-language contract.

The selected recorder port is the following (pseudocode; the Phase 1 requirement is this shape and these rules, not these exact identifiers):

```text
RunStatus =
  { type: "done" } | { type: "cancelled" } | { type: "timeout" } |
  { type: "provider_error", category: ErrorCategory }

RunSummary = {
  runId: Identifier, startedAtMs: i64, origin: Identifier,
  providerId: Identifier, modelId: Identifier,
  status: RunStatus, durationMs: u32,
  tokens: AiRunTokens, costMicros: Optional<u64>
}

RunRecorder.record(&self, summary: RunSummary, request: &CompletionRequest)
```

RunRecorder is Send + Sync. The request borrow is valid only during this synchronous callback; an application queuing history must copy the prompt fields it needs into its own record. The SDK summary has no prompt field or retention settings and need not be serializable in Phase 1. The application maps status to its existing state/errorCategory pair and retains its current storage-time retention decision. Invalid/failed starts do not invoke the recorder; unknown-provider completion invokes it synchronously after the terminal attempt. This avoids a prompt cache and preserves correlation even if request ids are later reused.

Keep one source of truth for status and accounting; do not add optional `errorCategory` beside a status enum. Preserve byte estimation as ceil(bytes / 4), capped at the token bound, and cost as round-half-up of the combined input/output cost, using exact intermediate arithmetic. Source cost/timestamp limits must not be advertised as JS-safe without an explicit encoding decision.

#### Required adapter mapping

A Phase 1 compatibility harness must demonstrate this mapping without editing Skriuw:

| SDK `RunSummary.status` | Application `AiRunRecord.state` | `errorCategory` |
| --- | --- | --- |
| `done` | `done` | absent |
| `cancelled` | `cancelled` | absent |
| `timeout` | `timed_out` | absent |
| `provider_error { category }` | `failed` | that category, always present |

No other combination is constructible from the summary, which is the point of the discriminated status. `runId`, `origin`, `providerId`, `modelId`, `startedAtMs`, `durationMs`, the `provider | estimated` token-source vocabulary, and `costMicros` pass through unchanged, preserving the exact source arithmetic. The adapter supplies `prompts` by copying from the borrowed request when its retention settings call for it; the copy happens inside the callback, before any queueing.

#### Invocation rules

Recorder invocation occurs after the terminal send attempt, on the same thread, and only where the source invokes it today:

- An invalid request or a spawn failure returns a `StartError` and records nothing.
- An unknown provider records synchronously on the caller's thread, after its terminal send attempt.
- A registered-provider run records on its worker thread. The source returns early without sending a terminal *or* recording when the registry entry was already removed; §3.3's committed-terminal rule replaces that early return, so a committed run records exactly once.

Storage durability is best-effort: Skriuw's application recorder uses a bounded queue and can drop records. No promise of durable exactly-once storage follows from one callback invocation, and a recorder failure after terminal commitment must not produce another terminal.

### 2.9 Fake provider

Preserve `FakeAiProvider`, `FakeCompletionScript`, and outcomes: Done with optional usage, Timeout, MalformedOutput, ProviderError. Keep token order, delay/cancellation polling, output bounds, usage/error validation, and explicit terminal behavior.

Fake is an intentional test/playground provider inside core; "no provider names in core" excludes real vendor dispatch and URLs, not the existing `fake` registration.

Tests invoking Skriuw's built-in prompts must remain application tests. Replace their test inputs with neutral strings for SDK contract coverage; do not copy the prompt library just to port every test verbatim.

## 3. Streaming, cancellation, and timeout semantics

### 3.1 Existing guarantees and limitations

For a valid request to a cooperative registered provider that returns normally, the service forwards deltas and attempts one terminal send. A live, non-failing channel can receive that terminal. A closed channel cannot be guaranteed to receive it.

Providers own increasing sequence numbers starting at zero, request identity, delta validation, and output bounds today. The service sink only counts bytes and forwards. Skriuw's consumer rejects foreign ids and unexpected sequence numbers; it does not repair gaps. Consumer filtering does not prove that the producer enforced the contract.

A failed delta send sets cancellation. A returning Done becomes Cancelled if the cancellation flag is set at the service's final check. Provider errors and Timeout are preserved even if that same flag was set internally. Blindly converting every flagged result to Cancelled would erase real timeout/malformed outcomes.

The registry removes a request before terminal delivery. Consequently `cancel()` can return false before terminal delivery, and the id can be reused while an old terminal is still being sent. Unknown-provider lookup precedes duplicate detection, so the same id can receive an unknown-provider terminal while a registered-provider run is active. A worker that finds its registry entry already gone returns without sending a terminal or recording at all. These are gaps in the source, not approved SDK invariants; §3.3 selects the corrections.

No service catch-unwind exists. A provider panic can leave a registry entry and omit terminal/recording. Process abort, worker non-return, blocking sink/recorder, and process termination are outside any exactly-once delivery promise, before or after the §3.3 hardening.

### 3.2 Deadline baseline

No core watchdog exists. Fake measures a deadline at provider entry. The remote adapter begins its deadline after credential resolution, just before the HTTP call; its read errors can become TransportFailure. Ollama generation uses request timeout but has no equivalent explicit deadline check in its read loop. Neither cancellation flag interrupts a blocked synchronous read immediately.

Thus `timeoutMs` is not currently a service-wide acceptance-to-terminal SLA. Do not claim that credential resolution, worker scheduling, channel backpressure, or recording are covered. Preserve these facts in characterization fixtures; HTTP fixes belong to Phase 2.

### 3.3 Selected narrow hardening (D1)

**Selected (D1): narrow lifecycle hardening, in Phase 1, as a step separate from and after source characterization.** Exact extraction with documented defects is rejected: the defects below are reachable from ordinary concurrent use, and an SDK that advertises explicit terminals should not ship a path where a request receives two terminals or none. Normal completion behavior and the request/event JSON in §2 do not change; only the edge behaviors listed here do, and each needs its own characterization test before and its own expectation test after.

The Phase 1 requirements are:

1. Reserve ids consistently before provider lookup and reject duplicates on every path.
2. Serialize the cancellation decision and terminal commitment per run; reserve the id through the terminal send attempt. Once committed, later cancellation returns false and cannot rewrite the terminal, even while the id remains reserved for delivery.
3. Preserve the source precedence: a cancellation observed before commitment changes Done only; provider error and Timeout remain typed outcomes.
4. Validate identity, gapless sequence, delta bytes, cumulative bytes, error and usage before forwarding/terminalizing. Invalid provider output commits MalformedResponse and requests cancellation without relabeling it Cancelled.
5. Clean up on all ordinary failures and unwind panics; convert an unwind panic before terminal commitment into InternalFailure. No promise applies to aborting panics or an uncooperative provider that never returns.
6. Keep source shutdown semantics (cancel-current-runs only), with tests and clear naming/documentation. A permanent shutdown/join lifecycle is a separate change.

Use an internal run identity to prevent stale cleanup removing a replacement. Never hold the registry lock while calling application sinks, channels, or recorders; a sink or recorder that re-enters the service must not deadlock, and a recorder failure after commitment cannot create another terminal. Test cancellation/completion and id reuse with synchronization barriers, not wall-clock guesses.

Explicit limitations that this hardening does **not** remove, and that Phase 1 documentation must state rather than paper over:

- No guaranteed delivery to a closed or failing channel. Commitment guarantees one terminal is produced and one send is attempted, not that a consumer receives it.
- No interruption of an uncooperative provider that blocks or never returns. Cancellation stays a cooperative flag.
- No recovery from process abort, an aborting panic, or termination between commitment and delivery.
- No whole-request deadline watchdog; `timeoutMs` remains provider-observed, per §3.2.
- A failed start remains a `StartError`. It never becomes a second terminal path, and it never records.

A future hard deadline requires a monotonic acceptance timestamp, an execution context carrying the remaining budget, interruption/worker cleanup rules, and a bounded delivery strategy. No such watchdog, async executor, or attempt machinery is implicitly included in the narrow hardening above.

## 4. Constraints on later contract changes

These are design corrections, not approved production signatures.

### 4.1 Model identity, messages, and capabilities

A future `ModelRef = {providerId, modelId}` is justified by shared identity. Provider id identifies a registered provider instance; it must not be forced to mean one vendor globally when multiple endpoints/accounts exist. Built-in aliases are provider catalog data and application migration mappings.

Prefer a text-only `Message = {role: system | user | assistant, content: string}` until another content type has a consumer. Do not add ContentPart nesting, tools/vision/audio/embeddings/reasoning enum values, suffix, or stop sequences merely for future autocomplete. Betalingen's last-user constraint is application validation, not automatically a universal rule. Explicitly check source prompt/order semantics before adopting any shared history constraint.

Capabilities introduced with a consumer use a closed vocabulary and yes/no/unknown support. Unknown is absence of knowledge, not authorization to degrade. No provider-name or model-name substring check decides critical support. Catalog/listing/application information merges per field with documented provenance; unknown must not erase known support. Declared endpoint locality is construction information, not model catalog authority or proof of privacy. Loopback is only endpoint locality and may front a remote service.

### 4.2 Credentials and provider errors

CredentialSource resolves a runtime secret at the provider boundary. It stores nothing. Application consent/vault errors map to a small typed refusal/unavailability outcome; arbitrary reason strings must not become machine-readable policy states. Credential failures may carry bounded safe display copy, with typed mapping. Do not add serialized secrets, provider-body excerpts, or catch-all metadata.

A Rust credential can be non-Clone and non-Serialize with redacted Debug; source `fill(0)` is an overwrite attempt, not proof of compiler-resistant zeroization. A later hardening may use an appropriate zeroization primitive. JavaScript GC and string copies cannot offer Rust-style zeroization-on-drop. State those lifecycle differences explicitly.

Expanded error taxonomies need separate CompletionFailureCategory (excluding timeout/cancelled) and administration outcomes. If structured validation adds issues, use a category-discriminated variant requiring non-empty bounded issues only on structured_output_invalid. Never leave `issues?` available on every error or classify by arbitrary strings.

### 4.3 Structured output

No current consumer has a validated JSON Schema execution/repair loop. Dora uses json_object or prompt-only output and lenient application parsing; Skriuw uses text lists; Betalingen uses text. Native schema support must be verified with provider fixtures when implemented, not inferred from protocol names.

Do not add an automatic strategy ladder. A future operation selects one execution strategy from explicit allowed strategies before network access. No silent provider/model switch, downgrade, or repair. A provider known to lack native schema can use a weaker strategy only when the caller explicitly permits it; unknown support never silently grants permission.

Use one schema authority, not both request.responseFormat.schema and a separate generateObject schema. Resolve the schema dialect/subset, root JSON object versus arbitrary JSON value, strictness meaning, finite-number handling, maximum document size/depth/validation work, and reference policy before approval. External references must not cause network/filesystem access. Cross-language validators must agree on the supported subset; plain Zod is not an arbitrary JSON Schema interpreter.

A typed result needs a decoder proving T matches the schema (Rust typed deserialization with a matching schema; TS schema/decoder inference). `generateObject<T>(untypedSchema)` does not prove T. Runtime-only decoders and transformations are not serialized. Structured results may contain fractional numbers; the envelope's integer conventions do not ban them.

Buffer unvalidated output within explicit bounds and expose a value only after validation. If a future API exposes preview deltas, they are provisional and cannot be treated as a validated object; repair is disabled after visible output. Fence stripping and lenient parsing require explicit policy. Preserve Dora's existing parsing until Dora approves a change.

### 4.4 Outcomes, retries, and validation

A future non-stream outcome is a discriminated union: success requires a value and has one usage field; cancelled/timeout/error prohibit a success value. Start rejection is a separate result before an accepted run. Avoid `terminal + value? + usage?` bags, arbitrary generic serialized T, and duplicate terminal-kind enums.

Remove attempt arrays and fallback eligibility tables until routing has a consumer. Retry eligibility is contextual, not a category-only promise. Request deadlines, Retry-After bounds, total byte/token/cost accounting, key identity, backoff, and partial output must be specified before enabling retries. A deadline already exhausted cannot be reset by retry.

Every later shared contract needs an ownership boundary, exact null/bounds rules, positive/negative fixtures, and a compatibility migration. See ADR 0002. Strict enum growth is a wire change even when Rust source uses non_exhaustive.
