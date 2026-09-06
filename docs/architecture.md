# Architecture

Status: Phase 0 specification. This document is normative for every later phase. Where it conflicts with the audit (`dev-docs/knowledge/architecture-research-across-my-ai-apps.md`), this document wins; where it conflicts with a reference repository's implementation, the reference implementation is evidence, not authority.

Companion documents: `contracts.md` (contract shapes and invariants), `roadmap.md` (phases), `decisions/0001..0003` (scope, cross-language, providers).

---

## 1. Central flow

The audit's intended flow holds in all three reference applications, with one correction: Dora is the outlier that pushes application context (`SchemaContext`) *into* the provider layer (`apps/desktop/src-tauri/src/database/services/ai/mod.rs::AIRequest`, `prompts.rs::build` called from every adapter). Skriuw and Betalingen both render context into prompt text before the seam. The SDK adopts the Skriuw/Betalingen shape and Dora migrates to it.

```text
┌───────────────────────────────────────────────────────────────────────┐
│ APPLICATION                                                           │
│   fetch product context (schema, note text, financial sources)        │
│   build product prompts (SQL rules, writing prompts, Dutch assistant) │
│   choose ModelRef (settings / env / per-call)                         │
│   render context into Message[]                                       │
└──────────────────────────────┬────────────────────────────────────────┘
                               │ CompletionRequest (generic, serializable)
┌──────────────────────────────▼────────────────────────────────────────┐
│ RUNTIME (ai-core / packages/core)                                     │
│   validate request · reject duplicate requestId · register cancel     │
│   resolve provider by ModelRef.providerId · enforce deadline          │
│   forward deltas with sequence numbers · first terminal wins          │
│   convert Done→Cancelled if cancellation raced · record RunRecord     │
└──────────────────────────────┬────────────────────────────────────────┘
                               │ Provider.complete(request, cancel, sink)
┌──────────────────────────────▼────────────────────────────────────────┐
│ PROVIDER (ai-providers / packages/ai-sdk)                             │
│   resolve credential via CredentialSource (after validation)          │
│   shape request for the wire · stream · parse · bound · map errors    │
└──────────────────────────────┬────────────────────────────────────────┘
                               │ HTTP / local socket
                            model
                               │
                               ▼
        CompletionEvent: delta{seq}* → exactly one terminal
        done{usage?,finishReason?} | cancelled | timeout | provider_error{error}
                               │
┌──────────────────────────────▼────────────────────────────────────────┐
│ APPLICATION                                                           │
│   accumulate text · parse product result · review · apply · persist   │
└───────────────────────────────────────────────────────────────────────┘
```

Refinements over the naive diagram:

1. **Credential resolution happens inside the provider, after validation.** Skriuw's `RemoteAiProvider::complete` (`crates/skriuw-ai-remote/src/lib.rs`) resolves the credential only once the request is validated and the model is permitted, so a missing credential terminalizes before any socket opens. The SDK keeps this ordering.
2. **The runtime, not the provider, owns the terminal-ordering guarantee.** Providers return one `Terminal`; the runtime publishes it. This is `skriuw_ai::AiCompletionService` (`crates/skriuw-ai/src/lib.rs`) unchanged.
3. **Recording is post-terminal and off the delivery path.** The runtime calls `RunRecorder.record` after publishing the terminal; persistence is application-owned.
4. **Non-streaming (`complete`) and structured (`generateObject`) operations are conveniences layered on the same stream.** There is one seam.

---

## 2. Boundaries

### 2.1 Application boundary

Applications own everything with product meaning:

| Concern | Dora | Skriuw | Betalingen |
| --- | --- | --- | --- |
| Context fetch | `commands/ai.rs::build_schema_context`, `engine_for_connection` | `app/src/features/ai/editor-action-apply.ts::actionInputText` | `src/lib/ai/context.ts::buildScreenContext` |
| Prompt text | `services/ai/prompts.rs` | `crates/skriuw-domain/src/prompt.rs` built-ins, workspace prompts | `context.ts::DATA_ASSISTANT_PROMPT` |
| Result application | insert/run SQL in console | ProseMirror transactions, plan review (ADR-0036) | render Markdown |
| Credential persistence | `storage/ai_keys.rs` + `security.rs` (AES-GCM) | `app/src-tauri/src/ai_credentials.rs` (keyring / session) | server env |
| Consent / disclosure | none | `RemoteAiConsent`, `REMOTE_AI_DISCLOSURE_VERSION` | static UI text |
| Usage persistence | `storage/ai_usage.rs` | `app/src-tauri/src/ai_history.rs` + SQLite migration `0018` | none |
| IPC / HTTP surface | 34 Tauri commands | 22 Tauri commands | `POST /ai/chat` |
| Recommended models | SQL-flavored picks | writing-flavored picks with use cases | env |
| Enablement / gating | `settings.hideAi` | `settings.aiEnabled` opt-in gate | auth guard |

The application is the only layer that knows a task name. `generateSql`, `rewriteNote`, `askAboutScreen` are application functions that *produce* a `CompletionRequest`.

### 2.2 AI core boundary

The core (`crates/ai-core`, `packages/core`) owns:

- contracts and validation (`ModelRef`, `Message`, `CompletionRequest`, `CompletionEvent`, `ProviderError`, `Usage`, `ModelInfo`, `RunRecord`)
- bounds (`MAX_*` constants, as `crates/skriuw-domain/src/ai.rs` lines 10–17)
- the `Provider` contract and the `EventSink` / cancellation primitives
- the `Runtime` (request registry, cancellation, deadline, first-terminal-wins, recording)
- the deterministic fake provider
- SSE and NDJSON codecs (decoders in both languages, an NDJSON encoder in TypeScript for HTTP servers)
- ports: `CredentialSource`, `RunRecorder`, `Pricing`
- the structured-output strategy ladder and output validation
- byte/token estimation helpers

The core must not contain: a provider name, a URL, an HTTP client, prompt text, a storage engine, an async runtime as a hard dependency, Tauri, keyring, React, Hono, Node-only or Bun-only APIs, or any application type.

### 2.3 Provider boundary

Providers (`crates/ai-providers`, `packages/ai-sdk`) implement `Provider` and nothing else. See `decisions/0003-provider-boundary.md`. Summary:

- OpenAI-compatible providers are **data rows** (`ProviderDescriptor`) interpreted by one adapter. Evidence: Skriuw's `OpenAiCompatible` table (`crates/skriuw-ai-remote/src/provider.rs`, six rows) and Dora's `CompatSpec` (`services/ai/compat.rs`, seven consts) are the same idea written twice.
- Anthropic, Gemini, and Ollama generation are custom adapters on a shared skeleton (stream reader, bounds, cancellation, deadline, status mapping).
- Providers never see application types and never own credential storage.

### 2.4 Runtime boundary

The runtime is the only entry point applications use for completion. Direct provider use is allowed only for administration calls (`verify`, `listModels`) so that recording, cancellation, and future routing stay uniform.

Runtime responsibilities, all evidenced by `crates/skriuw-ai/src/lib.rs`:

| Responsibility | Skriuw evidence |
| --- | --- |
| Validate request, reject duplicate `requestId` | `AiCompletionService::start`, `AiStartError` |
| Register a cancellation handle per request | `active: Mutex<HashMap<String, AiCancellation>>` |
| Run provider off the caller | one named `std::thread` per request |
| Forward deltas with sequence numbers | `CompletionServiceSink` |
| Closed consumer cancels the provider | `a_closed_consumer_cancels_the_request` test |
| Publish exactly one terminal; `Done` becomes `Cancelled` if cancellation raced | terminal publication in `start` |
| Record after terminal | `AiRunRecorder::record` |
| Idempotent `cancel(requestId) -> bool` | `AiCompletionService::cancel` |
| `shutdown()` cancels all | `shutdown` |

New in the SDK: per-request deadline enforcement in the runtime as a backstop (providers also check it in their read loops), the structured-output ladder, and the non-streaming `complete` / `generateObject` conveniences. Retries before the first delta are a runtime concern; ADR-0033 in Skriuw specifies them but no code implements them (audit §4.5). The SDK implements the rule "retry only before the first delta, only for retryable categories, at most `retryCount` times".

### 2.5 Model representation

Identity and description are separate.

```text
ModelRef   = { providerId, modelId }          required, validated, stable, the request key
ModelInfo  = { model: ModelRef, label?, contextWindowTokens?, maxOutputTokens?,
               capabilities: Map<Capability, CapabilitySupport>, locality, pricing?, source }
```

- A dotted single string (`google.gemini-2.5-flash`, Skriuw v1) is rejected: model ids contain `.`, `/`, `:` (`openai/gpt-oss-120b`, `z-ai/glm-5.3-flash`, `llama3.2:3b`).
- Both desktop apps let users type arbitrary model ids (Dora `model-id-input.tsx`, Skriuw fetched listings), so the SDK routinely holds models it knows nothing about. `ModelInfo` is optional and carries a `source` (`catalog | listed | declared`) so consumers decide how much to trust it.
- Provider ids: one canonical id per provider across all consumers. Decided: Skriuw's ids (`moonshot`, `zai`, `dashscope`) over Dora's (`kimi`, `glm`, `qwen`) because Skriuw's are the vendor names; Dora's stored settings get a migration map in roadmap Phase 8.

### 2.6 Capability representation

`Capability` is a closed enum: `streaming`, `jsonMode`, `jsonSchema`, `tools`, `vision`, `audio`, `embeddings`, `reasoning`. Only `streaming`, `jsonMode`, `jsonSchema` have behavior in v1; the others are reserved values with no fields and no code paths (audit §8.2).

`CapabilitySupport` is tri-state: `yes | no | unknown`. Sources, in precedence order: application declaration > shipped catalog > provider listing > provider-level default (every OpenAI-compatible endpoint streams) > `unknown`.

Rules:

- Never infer a critical capability from a model-name substring. Dora's `is_openai_chat_model` and tier heuristics (`services/ai/models.rs`) stay in Dora as UI sugar.
- `no` fails fast with `unsupported_capability` before any socket opens.
- `unknown` tries and lets the provider error (typically `invalid_request`) inform the caller. A future router learns from that.

### 2.7 Credential boundary

The core defines the port; applications and optional crates own storage.

```text
CredentialSource.resolve(providerId) -> Credential | CredentialError
Credential: opaque, zeroized on drop, redacted Debug/toString, never serializable
CredentialError: missing | refused{reason} | storeUnavailable | invalid
```

Evidence: `crates/skriuw-domain/src/remote_ai.rs` (`AiCredential(Vec<u8>)` with redacted `Debug`, `AiCredentialSource`, `AiCredentialError`). Skriuw's consent variants become the generic `refused { reason }` so that consent versioning stays a Skriuw policy.

Ships with the core (no platform dependency): environment-variable resolver, in-memory session resolver. Keyring-backed resolvers with Linux vault-state detection (`app/src-tauri/src/ai_credentials.rs::detect_vault_state`) and Dora's AES-GCM SQLite store are application implementations until two applications need the same one.

Security rules carried from the audit (§11.2): credentials go in headers, never URLs (Dora `gemini.rs` line 149 is the counter-example); provider response bodies never reach the user-facing `message`; a bounded, redacted `body_excerpt` exists only in the opt-in diagnostics field; secrets never appear in any serializable configuration structure.

Multi-key rotation (Dora `key_pool.rs`) is an optional `CredentialSource` decorator, restricted to `rate_limited` and `quota_exceeded`. Rotation on `invalid_credential` is dropped: an invalid key must surface, not be skipped.

### 2.8 Structured-output boundary

`ResponseFormat` is a request field: `text | json | jsonSchema { name, schema, strict }`. The `schema` payload is the one place the contract carries dynamic JSON, because a JSON Schema is by definition dynamic; it is validated as a JSON Schema at the trust boundary.

Execution is a **strategy ladder** chosen by the runtime from model capabilities, per call:

| Strategy | Requires | Mechanism |
| --- | --- | --- |
| `native` | `jsonSchema: yes` | provider-native schema mode (OpenAI `json_schema`, Gemini `responseSchema`, Ollama `format`, Anthropic forced single tool) |
| `jsonMode` | `jsonMode: yes \| unknown` | `json_object` + schema rendered into the system prompt |
| `promptOnly` | always | schema in the prompt, no provider hint (Dora's Anthropic/Gemini/Ollama path today) |

Post-processing is uniform and lives in core: strip code fences, parse, validate against the schema, emit `structured_output_invalid { issues }` on failure. One optional repair attempt re-asks the same model with the issues appended, and only when no delta has been delivered to the consumer. Structured calls therefore default to non-streaming.

An application may force `promptOnly` and supply its own parser (Skriuw's bullet-list plans, `app/src/features/ai/action-plan.ts`, which are tuned for small local models). That choice stays in the application.

Invalid structured output is a typed failure. It never silently becomes text.

### 2.9 Streaming and cancellation model

Event contract: `delta{requestId, sequence, text}*` then exactly one terminal. Terminal kinds: `done{usage?, finishReason?}`, `cancelled`, `timeout`, `provider_error{error}`. This is Skriuw's `AiCompletionEvent` (`crates/skriuw-domain/src/ai.rs` line 227) plus an optional `finishReason` on `done`. Dora's `Final{content}` is dropped (clients accumulate); Betalingen's `done` maps to `done{usage: none}`.

Cancellation:

- **Rust**: a clonable `Cancellation` token (`Arc<AtomicBool>`, Skriuw `AiCancellation`) observed inside every read loop; a runtime registry keyed by validated `requestId`; idempotent `cancel(requestId) -> bool`.
- **TypeScript**: `AbortSignal`. The runtime holds one `AbortController` per request and composes caller signal, timeout, and `cancel()`.

Consumer close: a failed sink send cancels the provider. "Stop forwarding" is never treated as cancellation on its own; the provider is told to stop.

Deadline: `timeoutMs` per request is enforced in the read loop and as the transport timeout; the runtime enforces it as a backstop. Deadline expiry emits `timeout`, never `provider_error`.

Bounds: `maxOutputBytes` per request and a global response cap are enforced before delivery (Skriuw `rejects_stream_bytes_beyond_the_requested_output_limit`). Exceeding them is `malformed_response`.

Full invariants are listed in `contracts.md` §3.

### 2.10 Rust architecture

```text
crates/ai-core
  contracts, validation, bounds, Cancellation, EventSink, Provider trait,
  Runtime, FakeProvider, SSE/NDJSON decoders, ports (CredentialSource,
  RunRecorder, Pricing), structured-output ladder + validator, env/session
  credential resolvers, token estimate
  deps: serde, serde_json, schemars, thiserror; optional: tokio (async facade),
        specta (Dora bindings), jsonschema (structured validation)

crates/ai-providers
  features: openai-compatible, anthropic, gemini, ollama
  shared skeleton: blocking HTTP, bounded line reader, SSE/NDJSON dialects,
  status→category mapper, usage extraction, verify, list_models
  provider descriptors + priced catalog loaded from specs/data
  deps: ai-core, reqwest (blocking, rustls), serde_json

crates/ai-ollama-runtime        (Phase 6)
  LocalRuntime port + OllamaRuntime: detect/install(SHA-256)/spawn/stop/status/
  list/pull/remove/progress/shutdown
  deps: ai-core (Cancellation, progress sink), reqwest, tar, zstd, flate2, sha2, tempfile

crates/ai-tauri                 (Phase 7, conditional)
  ChannelSink, OperationRegistry, run_blocking, feature "specta"
  deps: ai-core, tauri

crates/xtask
  JSON Schema generation + drift check + fixture replay (modelled on
  /home/remcostoeten/dev/skriuw/crates/xtask/src/main.rs)
```

The canonical provider trait is **synchronous and sink-based** (Skriuw's `AiComplete`). Reasons (audit §16.2): cancellation and byte accounting are enforced inside the read loop and proven by tests; both desktop apps already run AI off the main runtime; keeping `tokio` out of core keeps the core WASM-compatible; the code that would need porting (Dora's async adapters) is mostly superseded by Skriuw's. An async facade (`Runtime::stream -> impl Stream<Item = CompletionEvent>`) is offered behind a `tokio` feature, implemented with a channel fed by the sink; adapters do not change. This is final for v1. If a Rust HTTP server ever becomes a consumer, a native async adapter family may be added behind the same event contract via a new ADR; nothing is designed for it now.

Rust type rules (from `AGENTS.md`): enums for bounded states; newtypes for validated identifiers; typed errors; `Result` for recoverable failures; no `unwrap`/`expect` in library code; `serde_json::Value` only at the JSON-Schema payload; `#[non_exhaustive]` on public enums that are intentionally extensible (`ContentPart`, `Capability`, `ErrorCategory`, `RecoveryAction`).

### 2.11 TypeScript architecture

```text
packages/core          published as @remcostoeten/ai-core   ("@ai-sdk/*" is Vercel's; not used)
  types mirroring specs/, zod schemas for trust boundaries, createCompletionConsumer
  (from skriuw app/src/features/ai/completion-consumer.ts), decodeSse, decodeNdjson,
  toNdjsonStream, Runtime (AbortController per request), createFakeProvider,
  envCredentials, structured-output ladder + zod validation
  deps: zod only. Runs in browser, Node, Bun, Workers. No node:* imports.

packages/ai-sdk        published as @remcostoeten/ai-vercel   (Vercel AI SDK adapter)
  fromLanguageModel(model, meta): Provider
  openaiCompatible(descriptor, credentials, fetch?): Provider
  maps fullStream parts and APICallError into CompletionEvent / ProviderError
  deps: core, ai, @ai-sdk/*   (regular dependencies of this package, never peers of core)

packages/react         (later, only if two React consumers share a hook)
packages/tauri         (later, with crates/ai-tauri)
```

Provider contract in TypeScript: `complete(request, signal): AsyncIterable<CompletionEvent>` that must end with exactly one terminal. Pull-based iteration matches the platform; the runtime wraps it with the same registry semantics as Rust.

TypeScript type rules (from `AGENTS.md`): `type` not `interface`; no `any`; `unknown` narrowed at the boundary; discriminated unions with exhaustive handling; no `Record<string, unknown>` bags; no `extra`/`options`/`metadata` escape hatches; no leaked React, Hono, Tauri, Node, Bun, provider-SDK, or Vercel AI SDK types.

### 2.12 Vercel AI SDK boundary

```text
application
  -> our contracts (CompletionRequest, CompletionEvent, ProviderError, ModelRef, Provider, Runtime)
  -> packages/ai-sdk adapter
  -> Vercel AI SDK (streamText / generateText / Output.object, provider packages)
  -> provider
```

The Vercel AI SDK is an **adapter implementation detail**. It is a regular dependency of `packages/ai-sdk` only. Its types (`LanguageModel`, `StreamTextResult`, `UIMessage`, `TextStreamPart`, `APICallError`) never appear in a public signature; `fromLanguageModel` accepts one as an opaque input at the adapter boundary and that is the sole contact point. Evidence for this being the correct shape: Betalingen (`src/routes/ai.ts`) and Skriuw v1 both converted `fullStream` parts into their own event contracts and never exported AI SDK objects; the two consumers already sit on incompatible majors (`ai@7` vs `ai@6`).

What the adapter reuses: provider packages with tested request shaping, SSE parsing, and error mapping; `abortSignal`; usage and `finishReason`; `Output.object` for native structured output; the `fetch` injection point that makes tests deterministic. What it does not use: the UI message stream protocol, `useChat`, the `"openai:gpt-4o"` registry syntax (collides with `ModelRef`), middleware, the gateway.

A dependency-free `fetch`-based OpenAI-compatible adapter is permitted later only if a consumer must drop the AI SDK dependency (for example a browser bundle). No consumer needs it today.

### 2.13 Shared Rust/TypeScript specification

See `decisions/0002-cross-language-contracts.md`. Summary:

```text
specs/
  schema/      JSON Schema per shared contract, generated from Rust (schemars), committed, drift-checked
  enums/       error categories (+ default recovery, retryable-before-first-delta, fallback-eligible), capabilities
  data/        providers.json (descriptors), models.json (priced catalog)
  VERSION      spec semver
fixtures/
  streams/<provider>/<case>.sse + <case>.events.json
  errors/<provider>/<case>.json
  fake-scripts/*.json
```

Rust is the schema generator (Skriuw already has `xtask` with check mode). TypeScript validates its types and zod schemas against the committed schemas and replays the same fixtures. Wire fields are camelCase; unions are tagged with `type` in snake_case; numbers are integers (millis, micro-dollars, bytes, tokens) so fixtures are byte-identical across languages.

### 2.14 Ollama generation boundary

Ollama **generation** is a provider adapter in `crates/ai-providers` (feature `ollama`) and, in TypeScript, whatever Ollama provider the Vercel AI SDK adapter wraps. It speaks `/api/chat` (message-based, matching the SDK request), `/api/tags` for listing, `/api/version` for reachability. Dependencies: the HTTP client only.

```text
messages -> Ollama HTTP API -> CompletionEvent stream
```

Error mapping specific to local runtimes: connection refused -> `local_runtime_unavailable` (recovery `startLocalRuntime`); 404 for an unpulled model -> `local_model_missing` (recovery `pullModel`).

Endpoint policy (decided): the SDK default is loopback only. A non-loopback endpoint requires an explicit `allowRemoteEndpoint` opt-in at provider construction and is then classified `locality: remote { destination }` so privacy policies treat it as remote. Skriuw never opts in, so its current refusal (`endpoint_is_loopback`) is preserved without SDK-specific code; Dora opts in to keep its editable endpoint and gains the remote classification.

### 2.15 Ollama lifecycle boundary

Ollama **lifecycle** is a separate crate (`crates/ai-ollama-runtime`, Phase 6):

```text
detect · install (GitHub release + SHA-256) · verify · spawn · stop · status
list models · pull models (progress) · remove models · shutdown
```

Why separate from generation (not merely "both use Ollama"):

1. **Different dependency sets.** Lifecycle needs `tar`, `zstd`, `flate2`, `sha2`, `tempfile`, and process spawning. A Bun server or a browser build must never compile them.
2. **Different ports.** Skriuw already models them as two traits: `AiComplete` (generation) and `LocalAiRuntime` (`crates/skriuw-domain/src/local_ai.rs` line 110). The Ollama runtime implements both today, which is an implementation coincidence, not a contract.
3. **Different error types.** `LocalAiErrorCategory` (`InvalidRequest, Unavailable, DownloadFailed, ChecksumMismatch, InstallFailed, ProcessFailed, MalformedResponse, Cancelled, Unsupported`) describes install/pull operations; `ErrorCategory` describes completions. They must not be merged.
4. **Different platform surface.** Install directories, `LD_LIBRARY_PATH`/`DYLD_LIBRARY_PATH` (Dora `ollama_installer/runtime.rs::apply_platform_env`), macOS quarantine removal, Windows policy. None of that belongs near a completion.
5. **Different consumers.** All three apps generate; only the two desktop apps manage a runtime.

The `LocalRuntime` port lives in the runtime crate, not in `ai-core`, to keep process and filesystem vocabulary out of the core. The Ollama provider may depend on the runtime crate behind a feature for "auto-start on demand"; the reverse dependency is forbidden.

### 2.16 Tauri boundary

No Tauri plugin. No fixed SDK commands. Dora and Skriuw own their command surfaces (34 and 22 commands, different naming, different serialization casing, different IPC error types; audit §19).

A future optional `crates/ai-tauri` may provide exactly three generic helpers, each duplicated in both apps today:

| Helper | Skriuw | Dora |
| --- | --- | --- |
| `ChannelSink` (implements the runtime's event channel over `tauri::ipc::Channel<CompletionEvent>`) | `app/src-tauri/src/ai.rs::TauriCompletionChannel` | forwarder task in `commands/ai.rs::ai_complete_stream` |
| `OperationRegistry` (id-keyed, duplicate rejection, idempotent cancel, `cancel_all`) | `app/src-tauri/src/ollama.rs::OperationRegistry` | three `DashMap<String, Arc<AtomicBool>>` in `lib.rs::AppState` |
| `run_blocking` | `spawn_blocking` wrapper in `commands/ai.rs` | ad hoc `tokio::spawn` |

Plus an optional `specta` feature deriving `specta::Type` on core contracts so Dora keeps its generated `commands` object. The crate depends on `ai-core` and `tauri`, never on `ai-providers`; applications assemble providers. It is created only if Phase 7 finds the duplication still present after Phases 3 and 8 begin.

### 2.17 Future router boundary

Routing is not built in Phases 0–8. Nothing routes across providers today (audit §20.1). The core must expose enough for a router to be added without breaking contracts:

- `ErrorCategory` with per-category `retryableBeforeFirstDelta` and `fallbackEligible` flags in `specs/enums/error-categories.json`
- `ModelInfo.capabilities` (tri-state) and `ModelInfo.locality`
- `Usage`, `RunRecord.durationMs`, `ProviderError.retryAfterMs`
- `CompletionOutcome.attempts` (a list with exactly one entry until a router exists; additive to extend)

Hard rules the router must obey, fixed now: never switch provider or model after the first delta has been delivered; never fall back from a local provider to a remote one unless the application explicitly permits remote fallback; never route over provider crates directly (it receives `Provider` instances and `ModelInfo`).

### 2.18 Testing and conformance strategy

Normal tests never require paid provider access. Live-provider tests are opt-in and env-gated (`AI_LIVE_TESTS=1` plus keys) and never run in CI by default.

| Suite | Content | Source pattern |
| --- | --- | --- |
| Fake provider | scripted tokens, delays, outcomes (`done`, `timeout`, `malformed`, `providerError{category}`), usage; identical script semantics in both languages | `skriuw_ai::FakeAiProvider`, `FakeCompletionScript` |
| Runtime | ordered deltas, mid-stream abort, timeout before late token, malformed output, output bound, closed consumer cancels, duplicate id, recording with reported vs estimated usage, retry-before-first-delta only | the 16 tests in `crates/skriuw-ai/src/lib.rs` |
| Provider conformance | one suite run against every adapter with a local fixture server (Rust `serve_once`) or injected `fetch` (TS): ordered deltas; exactly one terminal; cancellation before and during read; output bounds; every status code -> expected category; no socket without credential; credential never in URL; usage parsed; `[DONE]`, missing terminal, oversized event handled | `crates/skriuw-ai-remote/src/lib.rs` tests (23), Betalingen `src/lib/ai/ai.test.ts` |
| Stream fixtures | `fixtures/streams/<provider>/<case>.sse` -> `events.json`, replayed by both languages | Skriuw inline bodies, Betalingen `providerResponse()` |
| Error fixtures | status + body -> category + recovery, table-driven | `maps_provider_status_codes_onto_distinct_recoverable_states` |
| Structured output | valid, fenced, invalid, repair path, `no` short-circuit, `promptOnly` forced | new |
| Contract conformance | generated schemas equal committed `specs/`; TS types and zod validate all fixtures; golden JSON round-trips in both languages | Skriuw `xtask` check mode |
| Local runtime | extraction, checksum, endpoint policy, pull stream bounds with local servers; `#[ignore]` device tests with a real Ollama | `crates/skriuw-ai-ollama/src/lib.rs` (12 + 4) |
| Live smoke | one `maxOutputTokens: 1` call per adapter | opt-in only |

Adding a provider without at least one stream fixture and one error fixture fails CI.

---

## 3. Dependency direction

```text
specs/ + fixtures/   <== generated by crates/xtask from ai-core types; validated by packages/core tests

              ai-core (Rust)                            packages/core (TS)
             /     |      \                              /        \
   ai-providers  ai-ollama-runtime  ai-tauri     packages/ai-sdk   packages/react, packages/tauri (later)
        |              |              |                  |                 |
   Skriuw shell / Dora shell (commands, context, prompts, storage)   Betalingen / Skriuw renderer / Dora studio
```

Allowed edges point inward to the core. `ai-tauri` -> `ai-core` only. `ai-ollama-runtime` -> `ai-core` only (for `Cancellation` and a progress sink). `ai-providers::ollama` -> `ai-ollama-runtime` optionally, behind a feature, never the reverse.

### Forbidden dependency directions

| From | Must never depend on |
| --- | --- |
| `ai-core` | `reqwest` or any HTTP client, `tauri`, `keyring`, `tokio` as a default dependency, any provider crate, any provider SDK, any application crate, any prompt text |
| `packages/core` | `ai`, `@ai-sdk/*`, `react`, `@tauri-apps/api`, `hono`, `next`, `node:*`, `bun:*`, any provider SDK |
| `ai-providers`, `packages/ai-sdk` | Dora schema/SQL types, Skriuw note/editor/consent types, Betalingen financial/screen types, credential storage, process management, application prompt text |
| `ai-ollama-runtime` | `ai-providers`, completion logic |
| `ai-tauri` | `ai-providers`, any application command name or product context |
| any future router | provider crates (routes over `dyn Provider` + `ModelInfo` only) |
| any SDK unit | application prompt text, application context types, task names |
| applications | provider adapters directly for completion (must go through the runtime); direct use allowed only for `verify` and `listModels` |
| public contracts | Vercel AI SDK types, React types, Hono types, Tauri types, Node/Bun types, provider SDK types |

A change that compiles in both languages but alters the serialized meaning of a shared contract is a contract change and requires a spec version bump (ADR 0002).
