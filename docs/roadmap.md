# Roadmap

Gated phases. A phase begins only after explicit approval; a phase ends only when its exit criteria are met; nothing continues automatically. Every phase makes the smallest change that satisfies its objective and preserves the dependency direction in `architecture.md` §3.

The sequence is the one proposed in `AGENTS.md`, checked against the audit (`dev-docs/knowledge/architecture-research-across-my-ai-apps.md` §42). Two adjustments relative to the audit's eleven-phase table, both to keep the count of empty packages low:

- The audit's separate "credentials" phase is folded away: the port plus the env and session resolvers are trivial and ship inside the core (Phase 1); keyring/vault detection stays application-owned until Phase 8 shows Dora and Skriuw need the same crate.
- The audit's "spec and fixtures" phase is the first deliverable of Phase 1 rather than a phase of its own, because the schemas are generated from the extracted types and cannot precede them meaningfully.

Package creation rule: a crate or package is created in the phase that first needs it, never earlier.

---

## Phase 0: architecture and contracts

| | |
| --- | --- |
| Objective | Produce the authoritative architecture, contract design, and phase plan. |
| Reference | The audit; Skriuw `crates/skriuw-domain/src/{ai,remote_ai,local_ai,ai_history}.rs`, `crates/skriuw-ai`; Dora `services/ai/`; Betalingen `src/lib/ai/`. |
| Allowed | Documentation, ADRs, pseudocode, naming, package boundary and dependency design, schema and fixture planning, testing strategy. |
| Forbidden | Any production Rust or TypeScript; `Cargo.toml`, `package.json`, crates, packages; installing dependencies; modifying reference repositories. |
| Deliverables | `README.md`, `docs/architecture.md`, `docs/contracts.md`, `docs/roadmap.md`, `docs/decisions/0001..0003`. |
| Tests | None (no code). Review: no contradictions across the seven documents; no application types in core; TS core independent of Vercel AI SDK; Rust core independent of Tauri and HTTP clients; routing and Ollama lifecycle postponed. |
| Exit | All seven documents exist and pass the review checklist; unresolved questions listed. |

Status: **current phase**.

---

## Phase 1: extract the Rust core from Skriuw

| | |
| --- | --- |
| Objective | Create `crates/ai-core` from Skriuw's proven seam with zero behavior change for Skriuw, then apply the contract generalizations from `contracts.md`. |
| Reference | Skriuw: `crates/skriuw-domain/src/ai.rs` (request, delta, usage, error, event, terminal, cancellation, sink, `AiComplete`, bounds), `remote_ai.rs` (`AiCredential`, `AiCredentialSource`, `AiCredentialError` only), `ai_history.rs` (`AiRunRecord`, `AiTokenSource`, `AiRunRecorder`, `AiModelPricing`, `estimate_ai_tokens`, `ai_run_cost_micros`), `crates/skriuw-ai/src/lib.rs` (`AiCompletionService`, `AiCompletionChannel`, `AiStartError`, `FakeAiProvider`, `FakeCompletionScript`), `crates/xtask/src/main.rs` (schema generation, check mode). |
| Allowed | Create `Cargo.toml` workspace and `crates/ai-core`, `crates/xtask`. Copy and rename the listed Skriuw modules. Step A: verbatim extraction under new names (`Provider`, `Runtime`, `CompletionRequest`, …) with Skriuw's tests ported. Step B: contract generalization: `messages` replacing `systemPrompt`/`userPrompt` with a `simple()` constructor; `ModelRef`; `responseFormat`, `stop`, `maxOutputTokens`, `finishReason`; the split `ErrorCategory` and extended `RecoveryAction`; `Capability`/`CapabilitySupport`/`ModelInfo` types; `CredentialError.refused`; `CompletionOutcome`; env and session `CredentialSource` implementations; retry-before-first-delta in the runtime; SSE and NDJSON line decoders (moved from `skriuw-ai-remote`'s `sse_payload` and bounded reader); structured-output validator and ladder selection (no provider execution yet). Generate `specs/schema/*.json`, `specs/enums/*.json`, `fixtures/fake-scripts/*.json`. Optional `tokio` feature with the async facade. |
| Forbidden | Any HTTP client, provider name, URL, or adapter; `tauri`; `keyring`; prompt text; `LocalRuntime` types (they go to Phase 6); modifying Skriuw (Skriuw's switch to the crate is Phase 3); creating `ai-providers` or any other crate; routing; task helpers. |
| Deliverables | `crates/ai-core` with public API per `contracts.md`; `crates/xtask` generating and drift-checking `specs/`; `specs/` and `fixtures/fake-scripts/` committed; `specs/VERSION`. |
| Tests | All 16 `skriuw-ai` tests ported and green on the fake provider; domain serialization and validation tests ported; new tests: messages validation (system-first, user-last, byte budget), `ModelRef` identifier rules, every `ErrorCategory` round-trips, event fixtures round-trip, retry only before first delta, duplicate id rejected, `Done`→`Cancelled` race, closed channel cancels, output bounds, structured validation (valid / fenced / invalid), `specVersion` present; schema drift check passes in CI; no `unwrap`/`expect` outside tests (clippy lint). |
| Exit | `cargo test -p ai-core` green; `cargo run -p xtask -- check` green; the crate's dependency list is exactly `serde`, `serde_json`, `schemars`, `thiserror` plus optional features; a reviewer can diff Step A against Skriuw and see only renames. |

Skriuw is the first extraction source. Dora contributes nothing to this phase.

---

## Phase 2: extract and generalize Rust providers

| | |
| --- | --- |
| Objective | Create `crates/ai-providers` with a descriptor-driven OpenAI-compatible adapter plus Anthropic, Gemini, and Ollama generation adapters on one shared skeleton, each passing a conformance suite against local fixture servers. |
| Reference | Skriuw: `crates/skriuw-ai-remote/src/lib.rs` (`RemoteAiProvider`, `stream_completion`, `status_error`, `transport_error`, `verify_credential`, `list_models`, the 23 `serve_once` tests), `src/provider.rs` (`RemoteProviderKind`, `OpenAiCompatible` rows incl. `stream_usage_option`, Gemini arm with `x-goog-api-key`), `models.json`; `crates/skriuw-ai-ollama/src/lib.rs` (the `AiComplete` half only: bounded NDJSON reader, `read_json_capped`, usage from eval counts). Dora: `services/ai/compat.rs` (OpenAI and OpenRouter rows, `chat_temperature`), `anthropic.rs` (messages API, `content_block_delta`, `x-api-key`, version header), `ollama.rs` (`/api/chat`, `num_predict` mapping). |
| Allowed | Create `crates/ai-providers` with features `openai-compatible`, `anthropic`, `gemini`, `ollama`. `ProviderDescriptor` type and `specs/data/providers.json` (Skriuw's six rows plus OpenAI and OpenRouter; canonical ids decided here). `specs/data/models.json` in Skriuw's priced format, validated. Catalog merge (catalog > listed). `verify` and `list_models` per adapter. Native structured-output execution for adapters that support it; `json_mode` and `prompt_only` strategies in the shared skeleton. Stream and error fixtures under `fixtures/`. Gemini uses the header, never the query string. |
| Forbidden | Credential storage; process management or any install/spawn/pull code; consent; UI copy; recommended-model lists; application types; touching `ai-core`'s public contracts except to fix a defect found by conformance (spec bump); routing; modifying Skriuw or Dora. |
| Deliverables | `crates/ai-providers`; `specs/data/providers.json`, `models.json`; `fixtures/streams/<provider>/`, `fixtures/errors/<provider>/` for every adapter and every descriptor row; conformance suite as a reusable test module. |
| Tests | Conformance per adapter and per descriptor row: ordered deltas; one terminal; cancel before and during read; deadline honored mid-stream; `maxOutputBytes` and global cap; every status code in the error table; no socket without credential; credential absent from URL and logs; usage parsed (`stream_options.include_usage` where the row allows it); `[DONE]`, missing terminal, oversized event, malformed JSON; `verify` sends `max_tokens: 1`-class body; `list_models` parses each provider's listing shape; catalog validation (bounds, duplicates, traversal-safe ids). CI rule: a descriptor row or adapter without fixtures fails. |
| Exit | Every adapter and row passes conformance; Skriuw's remote and Ollama-completion tests are reproduced on the shared crate; `ai-providers` depends on `ai-core` and `reqwest` (blocking, rustls) only. |

---

## Phase 3: prove Skriuw works against the extracted SDK

| | |
| --- | --- |
| Objective | Skriuw consumes `ai-core` and `ai-providers` by path dependency with no user-visible change; Skriuw's full AI test suite and native e2e stay green. |
| Reference | Skriuw: `crates/skriuw-domain/src/lib.rs` (re-exports), `app/src-tauri/src/ai.rs` (`LazyAiCompletion`), `ai_credentials.rs`, `ai_history.rs`, `commands/ai.rs`, `app/src/contracts/ai.ts`, `app/src/features/ai/{completion-bridge,completion-consumer,use-ai-run}.ts`, `contracts/README.md`, `scripts/check.sh`. |
| Allowed | This is the first phase that modifies a reference repository, and only Skriuw, and only with explicit instruction. Step 1: point Skriuw's `Cargo.toml` at the SDK crates; re-export SDK types from `skriuw-domain` under the old names; run Skriuw's checks unchanged. Step 2: switch Skriuw to `messages` and `ModelRef` (renderer mirror `app/src/contracts/ai.ts` updated in lockstep; `deny_unknown_fields` makes this a hard gate), adopt new `ErrorCategory` variants in renderer copy tables, replace `skriuw-ai-remote`/`skriuw-ai` usage with `ai-providers`/`ai-core`. `AiCredentialStore` becomes a `CredentialSource` implementation (consent check stays in Skriuw). `AiHistoryRecorder` becomes a `RunRecorder` implementation. Skriuw's `skriuw-ai-ollama` keeps its `LocalAiRuntime` half; its completion half is replaced by `ai-providers::ollama`. |
| Forbidden | Moving consent, disclosure, run-history storage, retention, opt-in gate, editor apply/review, plans, prompts, workspace prompt storage, or any command into the SDK; changing Skriuw behavior beyond the wire renames listed; creating `ai-tauri` (Skriuw keeps `TauriCompletionChannel` and `OperationRegistry` locally for now); touching Dora or Betalingen. |
| Deliverables | Skriuw on the SDK crates; deleted `crates/skriuw-ai`, `crates/skriuw-ai-remote`, and the completion half of `crates/skriuw-ai-ollama`; Skriuw's generated JSON Schemas for AI now sourced from `specs/`. |
| Tests | Skriuw `./scripts/check.sh` green; Skriuw's 16 renderer AI tests green on the new contract; `app/e2e/run-native-ai.mjs` green with the `fake` provider and with the pulled small model; `#[ignore]` device tests still pass when run; Skriuw contract drift check green against `specs/`. |
| Exit | Skriuw builds and passes all AI tests with zero SDK-specific patches inside the SDK; any defect found is fixed in the SDK with a spec bump, not with a Skriuw-only workaround. |

---

## Phase 4: TypeScript core and Vercel AI SDK adapter

| | |
| --- | --- |
| Objective | Create `packages/core` mirroring the shared contracts natively in TypeScript, and `packages/ai-sdk` implementing `Provider` over the Vercel AI SDK. Fixtures pass identically in both languages. |
| Reference | Betalingen: `src/lib/ai/groq.ts` (`streamText` usage), `src/routes/ai.ts` (NDJSON re-emission, `AbortSignal.any`), `src/ui/assistant/use-data-chat.ts` (NDJSON client decode), `src/lib/ai/ai.test.ts` (injected `fetch`). Skriuw: `app/src/features/ai/completion-consumer.ts` (ordered consumer), `completion-bridge.ts` (abort wiring). `specs/` and `fixtures/` from Phases 1–2. |
| Allowed | Create root `package.json` workspace, `packages/core`, `packages/ai-sdk`. Core: hand-written zod schemas with inferred types, conformance-checked against `specs/schema` (ADR 0002), `createCompletionConsumer`, `decodeSse`, `decodeNdjson`, `toNdjsonStream` (Web Streams), `Runtime` with `AbortController` per request, `createFakeProvider` reading `fixtures/fake-scripts`, `envCredentials`, structured-output ladder with zod validation. Adapter: `fromLanguageModel`, `openaiCompatible(descriptor, credentials, fetch?)` reading `specs/data/providers.json`, mapping of `fullStream` parts and `APICallError` to `CompletionEvent`/`ProviderError`. |
| Forbidden | Any `ai`/`@ai-sdk/*` import in `packages/core`; React, Hono, Tauri, `node:*`, `bun:*` in core; exporting any AI SDK type; a Hono package; a React package; browser BYOK storage; routing; modifying Betalingen (Phase 5). |
| Deliverables | `packages/core`, `packages/ai-sdk`; TS conformance harness (injected `fetch`); fixture replay in TS. |
| Tests | Every stream and error fixture produces the identical `events.json` in TS as in Rust; fake-provider scripts behave identically; consumer ordering (foreign id, out-of-order, post-terminal drop); NDJSON round-trip (encode in core, decode in core); abort before and during stream yields `cancelled`; `AbortSignal.timeout` yields `timeout`; adapter maps each `APICallError` status to the shared category; `generateObject` valid/fenced/invalid/repair; a bundle check proving `packages/core` has no `ai` in its dependency graph; `tsc --strict`; lint rules `func-style: declaration`, `prefer-arrow-callback`, no `interface`, no `any`. |
| Exit | Cross-language fixture parity is green in CI; `packages/core` runs in Bun, Node, and a browser test environment; `packages/ai-sdk` pins `ai` and `@ai-sdk/*` as regular dependencies. Package names: `@remcostoeten/ai-core`, `@remcostoeten/ai-vercel`. |

---

## Phase 5: migrate Betalingen as the first TypeScript consumer

| | |
| --- | --- |
| Objective | Betalingen's route and client run on `packages/core` and `packages/ai-sdk` with its application HTTP contract, context building, allowlist, masking, and prompt untouched. |
| Reference | Betalingen: `src/lib/ai/{groq,contracts,context}.ts`, `src/routes/ai.ts`, `src/ui/assistant/use-data-chat.ts`, `src/lib/ai/ai.test.ts`, `CLAUDE.md` runtime rules (`node:*` only, Vercel Node runtime), `.github/workflows/ci.yml`. |
| Allowed | Modify Betalingen only, with explicit instruction. Replace `groq.ts` with `openaiCompatible(groqDescriptor, envCredentials())` or `fromLanguageModel(createGroq(...)(model))`; route: `runtime.stream(request, signal)` + `toNdjsonStream`; client: `decodeNdjson(CompletionEventSchema)` + `createCompletionConsumer`. Either adopt `CompletionEvent` on the wire or keep the app's three-event contract and map at the route (both legitimate; the HTTP contract is app-owned). Rewrite the six tests on the fake provider and injected `fetch`. |
| Forbidden | Moving `ScreenContextSchema`, `ChatRequestSchema`, `buildScreenContext`, allowlist, auth pass-through, IBAN masking, `DATA_ASSISTANT_PROMPT`, route auth/body-limit/503 policy, or UI into the SDK; adding BYOK, model selection, or usage persistence to Betalingen; a Hono package; shipping `packages/ai-sdk` in the browser island. |
| Deliverables | Betalingen on the SDK; ~80 lines removed; CI green; deployed. |
| Tests | Betalingen's existing six behaviors preserved (auth/503, allowlist, masking and cap, streamed events, provider failure sanitized, system-message rejection and body limit) plus typed error categories on the wire; bundle-size check on `src/ui/assistant` proving `ai` is absent; `openapi.json` drift check green. |
| Exit | Betalingen CI green and deployed on the SDK; no Betalingen-specific code in the SDK. |

---

## Phase 6: extract Ollama lifecycle management

| | |
| --- | --- |
| Objective | Create `crates/ai-ollama-runtime` with the `LocalRuntime` port and `OllamaRuntime` (detect, install with SHA-256, verify, spawn, stop, status, list, pull, remove, progress, shutdown), separate from generation. |
| Reference | Skriuw: `crates/skriuw-domain/src/local_ai.rs` (`LocalAiRuntime`, `LocalAiStatus`, `LocalAiModel`, `LocalAiProgress`, `LocalAiError`, `LocalAiErrorCategory`, `LocalAiRuntimeState`), `crates/skriuw-ai-ollama/src/lib.rs` (install, `verify_sha256`, `extract_archive`, `endpoint_is_loopback`, managed child, 12 unit + 4 ignored tests), `docs/specs/ollama-runtime.md`. Dora: `ollama_installer/runtime.rs::apply_platform_env` (`LD_LIBRARY_PATH`/`DYLD_LIBRARY_PATH`/`PATH`, macOS quarantine), `services/ai/ollama.rs` pull ETA computation. |
| Allowed | Create the crate. Port Skriuw's runtime as the base; add Dora's platform env handling and ETA; endpoint policy (loopback default, explicit remote opt-in classified as remote locality); Windows: refuse install and link to the official installer (Skriuw's policy). Optional feature on `ai-providers::ollama` for auto-start via this crate. Skriuw switches its remaining `skriuw-ai-ollama` half to this crate (with instruction). |
| Forbidden | Any completion logic; putting `LocalRuntime` types into `ai-core`; recommended-model catalogs (product content); install/start UI copy; touching Dora (Phase 8). |
| Deliverables | `crates/ai-ollama-runtime`; `specs/schema/local-runtime-*.schema.json`; Skriuw on the shared runtime. |
| Tests | Skriuw's 12 unit tests ported (archive extraction depth, checksum mismatch, endpoint policy, pull stream bounds, duplicate operation id, cancelled pull emits `Cancelled`); new tests for platform env application; the 4 `#[ignore]` device tests kept and documented (≈1.4 GB download). |
| Exit | Skriuw's Ollama UI works unchanged on the shared crate; the crate depends on `ai-core` (cancellation, progress sink) and never on `ai-providers`. |

---

## Phase 7: extract Tauri helpers only if duplication still justifies the package

| | |
| --- | --- |
| Objective | Decide, with evidence from Phases 3 and 6 and the Phase 8 plan, whether `ChannelSink`, `OperationRegistry`, and `run_blocking` are still duplicated between Skriuw and Dora; if yes, create `crates/ai-tauri` (and `packages/tauri` if the renderer bridge is likewise duplicated). |
| Reference | Skriuw: `app/src-tauri/src/ai.rs::TauriCompletionChannel`, `app/src-tauri/src/ollama.rs::OperationRegistry`, `commands/ai.rs` `spawn_blocking` wrapper, `app/src/features/ai/{completion-bridge,ollama-bridge}.ts`. Dora: `commands/ai.rs::ai_complete_stream` forwarder, `lib.rs::AppState` cancel-flag maps, `bindings.rs` (tauri-specta). |
| Allowed | Written go/no-go decision recorded in a new ADR (`docs/decisions/0004-tauri-helpers.md`). If go: the three helpers, a `specta` feature deriving `specta::Type` for core contracts, a renderer bridge taking `invoke`, `Channel`, and command names as parameters. |
| Forbidden | A Tauri plugin; fixed command names; any application command, product context, consent, or prompt; depending on `ai-providers`; Specta as a default dependency of `ai-core`. |
| Deliverables | ADR 0004; optionally `crates/ai-tauri` and `packages/tauri`. |
| Tests | Unit tests with fake channels: duplicate id rejected, cancel idempotent, `cancel_all`, closed channel cancels the request, blocking work runs off the async runtime. |
| Exit | ADR 0004 accepted; if go, Skriuw's local copies of the three helpers are deleted. |

---

## Phase 8: migrate Dora

| | |
| --- | --- |
| Objective | Dora runs on the shared crates and `packages/core`; Dora's adapters and installer are deleted; Dora's schema context, SQL prompts, SQL safety, key store, usage table, commands, and studio UI remain Dora's. |
| Reference | Dora: `services/ai/mod.rs` (`AIRequest`, `AiStreamEvent`, `AIService`), `client.rs`, `compat.rs`, `anthropic.rs`, `gemini.rs`, `ollama.rs`, `key_pool.rs`, `models.rs`, `prompts.rs`, `usage.rs`, `errors.rs`, `commands/ai.rs`, `ollama_installer/*`, `storage/ai_keys.rs`, `storage/ai_usage.rs`, `security.rs`, `packages/studio/src/features/ai-assistant/*`, `packages/studio/src/features/sql-console/components/ai-cmd-k.tsx`, `packages/studio/src/lib/bindings.ts`; `docs/architecture-roadmap.md` Track 4/7. Audit §38 for the ordered plan. |
| Allowed | Modify Dora only, with explicit instruction. Order: (1) SDK as path dependency; (2) `SqliteAesCredentials` (over `storage/ai_keys.rs` + `security.rs`) and `SqliteUsageRecorder` (over `storage/ai_usage.rs`, gaining `durationMs`, `state`, `origin`) as Dora-owned implementations; (3) Dora `ai_prompts` module producing `Vec<Message>` from `SchemaContext`; `ai_complete_stream` → `Runtime::start` with `origin: "dora:chat" | "dora:sql"`; SQL path on `generateObject` with Dora's `{sql, explanation, warnings}` schema; temporary event-name mapper for one release; (4) studio on `packages/core` consumer; history as `messages`; (5) delete adapters and installer; (6) Ollama manager on `ai-ollama-runtime`. Provider-id migration map (`kimi`→`moonshot`, `glm`→`zai`, `qwen`→`dashscope`) for stored settings. Multi-key rotation via the `CredentialSource` decorator, restricted to rate-limit/quota. |
| Forbidden | Moving `SchemaContext`, dialect detection, SQL prompts, SQL safety rules, the SQL result schema, insert/run flow, AES-GCM key table, `pkexec` keyring installer, usage table, commands, studio UI, tier heuristics, or recommended SQL models into the SDK; keeping the Gemini query-string key; keeping rotation on 401. |
| Deliverables | Dora on the SDK; ~2,400 lines of Rust removed; `MockClient`, `ai_groq_status`, `ai_set_gemini_key`, `AIResponse.suggested_queries` retired (after the migration window). |
| Tests | Dora inherits the provider conformance suite; new Dora tests for SQL schema validation via `generateObject`; studio tests on the shared consumer; behavior changes verified explicitly: cancel is a visible terminal, streaming usage is provider-reported where available, 401 surfaces as `invalid_credential`. |
| Exit | Dora builds, its AI features work on the SDK, adapters and installer are deleted, and no Dora type exists in any SDK unit. |

---

## Phase 9: optional routing and fallback

| | |
| --- | --- |
| Objective | Add an optional router (Rust `crates/ai-router`, TS `createRouter` module) once at least one consumer has a concrete two-route need. Core contracts are not changed. |
| Reference | Audit §20 (design), Dora `key_pool.rs` (the only intra-provider fallback that exists), Skriuw v1 `ACTION_MODEL_DEFAULTS` (frozen per-action defaults). |
| Allowed | `TaskRequirements { needs: Capability[], locality: any \| local_only, latency: interactive \| batch }`, ordered routes, in-memory `HealthStore` port with cool-downs, `Router.plan` and `Router.observe`, a `RouterProvider` wrapping the runtime; `CompletionOutcome.attempts` grows beyond one entry. Scripted fallback tests on the fake provider (429 with `retryAfterMs` → next route; 401 → no fallback; post-first-delta failure → no fallback; `local_only` never reaches a remote route). |
| Forbidden | Routing inside `ai-core`; depending on provider crates; a hard-coded fallback list; silent local→remote fallback; switching after the first delta; shared/persistent health stores; starting before a consumer asks for it. |
| Deliverables | `crates/ai-router`, `createRouter`; ADR for routing policy. |
| Tests | Scripted fallback matrix; concurrency safety (no lock across I/O); at least one application configures a two-route chain in its own tests. |
| Exit | One real consumer uses a two-route chain; `ai-core` unchanged. |

---

## Explicitly not scheduled

No phase exists for tool calling, agents, embeddings, transcription, workflows, a prompt registry, a task-helper package, a React hook package, a Hono or Next.js package, browser BYOK, a Tauri plugin, or a shared telemetry store. Each of those requires a real consumer and a new ADR before it can be scheduled. (A thin `tasks` data package holding Skriuw's fifteen generic writing prompts was judged reusable by the audit; it is not scheduled until Skriuw asks to donate them.)
