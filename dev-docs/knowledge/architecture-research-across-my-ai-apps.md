# AI SDK architecture research: Dora, Skriuw, Betalingen

Audit date: 2026-09-06. Repositories inspected at their working-tree state on that date:

| Repository | Path | Latest commit inspected | Primary AI stack |
| --- | --- | --- | --- |
| Dora | `/home/remcostoeten/dev/dora` | `3f5fb401` (2026-09-02) | Rust (tokio + reqwest) behind Tauri commands; React 18 studio |
| Skriuw | `/home/remcostoeten/dev/skriuw` | `64827f5e` (2026-09-06) | Rust workspace crates (`skriuw-domain`, `skriuw-ai`, `skriuw-ai-ollama`, `skriuw-ai-remote`) behind Tauri commands; React 19 renderer |
| Betalingen | `/home/remcostoeten/dev/betalingen` | `d13c762` | TypeScript, Bun/Hono on Vercel Node runtime, Vercel AI SDK (`ai@7.0.93`, `@ai-sdk/groq@4.0.37`) |

All paths in this report are repository-relative unless prefixed with the repository name. Nothing was modified, installed, or committed during the audit. Where documentation and implementation disagree, the implementation is reported and the stale document is called out.

---

## 1. Executive summary

**The short answer: yes, build the SDK, but build it by extracting Skriuw's existing AI seam rather than designing a new one, and keep it small.**

Three independent AI implementations exist today. They are at very different maturity levels:

1. **Skriuw v2 already contains the core the SDK needs.** `crates/skriuw-domain/src/ai.rs` defines a provider-neutral `AiComplete` trait, a bounded `AiCompletionRequest`, an ordered streaming `AiCompletionEvent` with explicit terminal states (`done`, `cancelled`, `timeout`, `provider_error`), a typed `AiProviderError` with a stable `category` and `recovery_action`, a shared `AiCancellation` primitive, an `AiCredentialSource` port, a `LocalAiRuntime` port, a run-recording port (`AiRunRecorder`), a pricing port (`AiModelPricing`), and JSON Schemas generated from those types (`contracts/generated/ai-*.schema.json`). It has a permanent deterministic fake provider (`skriuw_ai::FakeAiProvider`), a request registry with end-to-end cancellation (`skriuw_ai::AiCompletionService`), an OpenAI-compatible adapter table covering six providers plus a Gemini adapter (`skriuw-ai-remote`), and a checksum-verified Ollama runtime manager (`skriuw-ai-ollama`). This is the best Rust foundation by a wide margin and it was designed under an accepted ADR (`docs/adr/0033-ai-provider-completion-seam.md`, 2026-08-16) with exactly the constraints the SDK needs: no Tauri, no HTTP, no credentials in the domain crate.

2. **Dora has broader provider coverage but a weaker seam.** `apps/desktop/src-tauri/src/database/services/ai/` supports eleven provider ids (Groq default, OpenAI, Anthropic, Gemini, Ollama, DeepSeek, Kimi, GLM, Qwen, OpenRouter, Mock) through an `AiClient` trait whose request type (`AIRequest`) hard-codes Dora's `SchemaContext`, whose errors are `anyhow` strings serialized as `{kind:"Internal", detail}`, whose stream has no terminal state for cancellation or timeout, and whose streaming path never captures provider usage (it estimates tokens from characters). Its valuable, extractable pieces are the OpenAI-compatible `CompatSpec` table, the Anthropic adapter, the round-robin `KeyPool`, the curated-plus-live model catalogs, the in-app Ollama installer and its AES-GCM key store. Dora has no HTTP-level AI tests.

3. **Betalingen is a single server-side Groq chat endpoint.** `src/lib/ai/groq.ts` calls `streamText` from the Vercel AI SDK, `src/routes/ai.ts` re-emits it as NDJSON `{text|done|error}` events, and `src/ui/assistant/use-data-chat.ts` consumes it in the browser with an `AbortController`. There is no BYOK, no model selection, no structured output, no usage tracking. It is the only production TypeScript AI code across the three repositories, and it demonstrates the right shape for the TypeScript side: the AI SDK is used internally and a small, app-owned stream contract is exposed.

Key architectural conclusions, each argued in the body:

- **Rust foundation:** extract `skriuw-domain`'s AI modules plus `skriuw-ai` into an `ai-core` crate essentially unchanged, then merge `skriuw-ai-remote` and Dora's `compat.rs`/`anthropic.rs` into one feature-gated `ai-providers` crate. Retire Dora's async adapters instead of porting them.
- **TypeScript foundation:** a small `@ai/core` package whose public contract mirrors the Rust event and error shapes, implemented over the Vercel AI SDK internally (option C). Do not expose AI SDK types in the public API; Betalingen and Skriuw v1 already sit on two incompatible AI SDK majors (`ai@7` and `ai@6`).
- **Cross-language sharing:** share a specification (JSON Schemas for request, event, error, model metadata; error-code and capability enums; golden stream fixtures), not an implementation and not a wire protocol beyond what Tauri channels and NDJSON already are. Skriuw's `xtask` pipeline already does this in one direction.
- **Ollama:** the HTTP generation adapter and the install/spawn/pull runtime manager are already separate traits in Skriuw (`AiComplete` vs `LocalAiRuntime`). Make them separate crates because their dependency sets differ (`tar`, `zstd`, `flate2`, `sha2`, `tempfile`, process spawning).
- **Tauri:** provide a tiny helper crate (channel sink, request registry, blocking-run helper), not a plugin. Both apps' command surfaces are app-specific and should remain so.
- **Routing/fallback:** optional layer, not core. Nothing routes across providers today; Dora rotates keys inside one provider and Skriuw v1 (frozen) had per-action model defaults. Core must expose enough error classification for a router to exist.
- **Tasks:** two layers. Core is model-oriented (`complete`, `stream`, `generate_object`). An optional task package holds Skriuw's fifteen generic writing prompts as data. Dora's SQL tasks stay in Dora.
- **Explicitly do not build:** an agent framework, tool-calling runtime, embeddings/vector layer, prompt marketplace, React UI kit, Tauri plugin, or a shared usage database.

The safest first extraction is Skriuw's domain AI modules plus `skriuw-ai` into `ai-core`, consumed by Skriuw as a path dependency with zero behavior change, followed by a provider conformance suite driven by the existing fake provider and the existing `serve_once` TCP fixtures.

---

## 2. Repository overview

### 2.1 Dora

| Aspect | Finding | Evidence |
| --- | --- | --- |
| Languages | Rust (backend), TypeScript/React 18 (studio + desktop shell), Next.js (marketing) | `apps/desktop/src-tauri/Cargo.toml`, `apps/desktop/package.json`, `apps/marketing/package.json` |
| Runtime | Tauri 2.8/2.10 desktop app; browser demo mode uses mock adapters | `apps/desktop/src-tauri/Cargo.toml`, `packages/studio/src/core/data-provider/context.tsx` (`detectTauri`, `useIsTauri`) |
| Workspace | Bun workspaces + Turbo: `apps/*`, `packages/*`; version `0.41.0` | root `package.json`, `turbo.json` |
| Frontend architecture | `@dora/studio` package holds all product UI (`packages/studio/src/features/*`); `apps/desktop` is the Vite shell. Feature modules call generated Tauri bindings directly. | `packages/studio/src/lib/bindings.ts` (tauri-specta output, `// Auto-generated`) |
| Backend architecture | One Rust crate `dora` (lib `app_lib`) with a `database` module tree: `commands/*` (Tauri commands) → `services/*` (business logic) → adapters per engine. AI lives under `database/services/ai/` and `database/commands/ai.rs` even though it is not database code. | `apps/desktop/src-tauri/src/lib.rs`, `src/database/commands/mod.rs` |
| IPC | Tauri commands with `specta`/`tauri-specta` generated TypeScript; streaming via `tauri::ipc::Channel<T>` | `src/bindings.rs` (`export_ts_bindings`), `src/database/commands/ai.rs` |
| Dependency injection | None formal. `AppState` (`lib.rs`) holds `Storage`, `DashMap`s of connections/schemas and three cancellation-flag maps; services are constructed ad hoc with `&Storage`. | `src/lib.rs` `pub struct AppState` |
| Storage/config | SQLite via `libsql-rusqlite` (`src/storage/*`), key/value `settings` table for AI config (`ai_provider`, `ai_model.<provider>`, `ollama_model`, `ollama_endpoint`); `dotenvy` loads `.env` at start. | `src/database/services/ai/mod.rs` (`resolve_model`, `ollama_endpoint`), `src/lib.rs` |
| Secrets | `security.rs` AES-256-GCM with a master key in the OS keyring (`keyring` crate) or a `0600` fallback file; `credential_storage.rs` probes and can install `gnome-keyring`/`kwallet` via `pkexec`. | `src/security.rs`, `src/credential_storage.rs` |
| Testing | Vitest at root `__tests__/` and inside `packages/studio`; Rust unit tests inline plus `apps/desktop/src-tauri/tests/*` (live DB tests gated by `DORA_LIVE_DB_TESTS`). AI: 7 Rust unit tests, 4 TS test files, no HTTP-level AI test. | `__tests__/ai-actions.test.ts`, `__tests__/model-id-input.test.ts`, `packages/studio/src/features/ai-assistant/*.test.ts` |
| Planned architecture | `docs/architecture-roadmap.md` Track 7 proposes a workspace split including a `dora-ai` crate and Track 4 proposes an `AiPort` in the studio data provider, with `features/ai-assistant` as the pilot. Neither is implemented. | `docs/architecture-roadmap.md` lines 180-252 |

### 2.2 Skriuw

| Aspect | Finding | Evidence |
| --- | --- | --- |
| Languages | Rust (edition 2024, `rust-version = 1.95`), TypeScript/React 19, ProseMirror, CodeMirror; Cloudflare Worker (TypeScript) for sync; frozen v1 tree (Next.js, Prisma, Expo, Tauri v1 desktop) | root `Cargo.toml`, `app/package.json`, `cloud/package.json`, `v1/package.json` |
| Runtime | Tauri 2.11 desktop; browser build runs the same Rust core as WebAssembly (`skriuw-sqlite-wasm`); AI is desktop-only (`requireDesktopRuntime("AI completion")`) | `app/src/bridge/runtime.ts:46`, `app/src/features/ai/completion-bridge.ts` |
| Workspace | Cargo workspace of 16 crates under `crates/`; the Tauri crate `app/src-tauri` is a separate `[workspace]` root consuming them by path; Bun scripts at root wrap `scripts/*.sh`. Default branch is `daddy`. | root `Cargo.toml`, `app/src-tauri/Cargo.toml`, `AGENTS.md` |
| Frontend architecture | Renderer store with narrow selectors; features under `app/src/features/*`; AI feature is lazy-loaded behind an opt-in gate (`settings.aiEnabled`). | `app/src/features/ai/opt-in-gate.tsx`, ADR-0020 |
| Backend architecture | Strict dependency direction: `skriuw-domain` (no DB/FS/OS deps) ← `skriuw-storage` (use-case traits) ← adapters (`skriuw-sqlite`, `skriuw-history-git`, `skriuw-ai-ollama`, `skriuw-ai-remote`) ← `skriuw-runtime` (FIFO worker) ← `app/src-tauri` shell. AI has its own three crates plus five domain modules. | `docs/ARCHITECTURE.md`, `crates/skriuw-domain/Cargo.toml` (deps: `schemars`, `serde`, `serde_json`, `sha2`, `thiserror` only) |
| IPC | Tauri commands returning domain types; streaming via `tauri::ipc::Channel<AiCompletionEvent>` and `Channel<LocalAiProgress>`; the renderer keeps hand-written mirror types in `app/src/contracts/ai.ts` checked against generated JSON Schemas. | `app/src-tauri/src/commands/ai.rs`, `contracts/README.md` |
| Dependency injection | Constructor injection with `Arc<dyn Trait>`: `AiCompletionService::new(providers)` takes `(String, Arc<dyn AiComplete>)` pairs; `RemoteAiProvider::with_model_authority(kind, Arc<dyn AiCredentialSource>, Arc<dyn RemoteAiModelAuthority>)`. The shell builds the service lazily on first request (`LazyAiCompletion`). | `crates/skriuw-ai/src/lib.rs`, `app/src-tauri/src/ai.rs` |
| Storage/config | SQLite canonical; AI model selection is a workspace setting (`settings.aiModel = {providerId, modelId}`); AI enablement is `settings.aiEnabled`; consent and fetched-model documents are native JSON files in app data (`ai-consent.json`, `ai-models.json`); AI run history in SQLite tables `ai_run_history`, `ai_history_settings` (migration `0018`). | `app/src/features/ai/model-selection.ts`, `app/src-tauri/src/ai_credentials.rs`, `crates/skriuw-sqlite/migrations/0018_ai_run_history.sql` |
| Secrets | OS keyring via `keyring` crate (service `dev.skriuw.app`, account `ai-provider:<id>`), or session-only memory; Linux vault state detected directly over `dbus-secret-service`. | `app/src-tauri/src/ai_credentials.rs` |
| Testing | Rust inline tests with local TCP fixture servers (`serve_once`) in every AI crate; `#[ignore]` device-verification tests that use a real Ollama; renderer tests under `app/__tests__/features/ai/*` (16 files); a native WebDriver e2e suite `app/e2e/run-native-ai.mjs` that pulls the `all-minilm` model. Contract drift is checked by `cargo run -p xtask -- generate` in check mode. | `crates/skriuw-ai-remote/src/lib.rs` tests, `crates/skriuw-ai-ollama/src/lib.rs` tests, `crates/xtask/src/main.rs` |
| Documentation | ADR-0033 (completion seam), ADR-0036 (in-place review), specs `docs/specs/ollama-runtime.md`, `ai-editor-actions.md`, `ai-run-history.md`; local planning in `project-management/ai/*` (gitignored per its index). | `docs/adr/0033-ai-provider-completion-seam.md` |

### 2.3 Betalingen

| Aspect | Finding | Evidence |
| --- | --- | --- |
| Languages | TypeScript only; React 19 islands bundled with esbuild into `.js.txt` files served by Hono | `package.json`, `scripts/build-gate.ts` |
| Runtime | Bun locally (`bun --hot src/index.ts`); Vercel Node runtime in production via one esbuild bundle (`dist/server.mjs`) re-exported by `api/index.ts`. Code under `src/` must stay runtime-agnostic (`node:*` only). | `CLAUDE.md` Rules, `src/vercel.ts` |
| Workspace | Single package, no monorepo | `package.json` |
| Backend architecture | `createApp(store, options)` builds an `OpenAPIHono` app; routes under `src/routes/*` declare zod schemas; `src/lib/*` holds domain logic; `src/lib/storage.ts` picks file / Vercel Blob / memory. | `src/app.ts` |
| AI location | `src/lib/ai/{groq,contracts,context}.ts`, `src/routes/ai.ts`, `src/ui/assistant/{main.tsx,use-data-chat.ts}`, test `src/lib/ai/ai.test.ts` | file listing |
| Dependency injection | Options object: `createApp(store, { ai: GroqOptions })` where `GroqOptions = { apiKey?, model?, fetch? }`; tests inject a fake `fetch`. | `src/app.ts`, `src/lib/ai/ai.test.ts` |
| Config/secrets | `GROQ_API_KEY`, `GROQ_MODEL` environment variables, server-only; no browser exposure; no BYOK. | `.env.example`, `src/routes/ai.ts:20` |
| Testing | `bun test`; the AI test injects a fake SSE `Response` through `GroqOptions.fetch` and asserts the key is not echoed and provider error bodies are not leaked. CI runs lint, tsc, tests, build, and an `openapi.json` drift check. | `src/lib/ai/ai.test.ts`, `.github/workflows/ci.yml` |

### 2.4 Cross-repository observations relevant to the SDK

- Two Tauri apps, two different Tauri IPC styles: Dora uses `tauri-specta` to generate a typed `commands` object; Skriuw hand-writes `invoke` wrappers and mirror types, with JSON Schema drift checks on the Rust side. An SDK cannot assume either.
- Two different Rust concurrency models for AI: Dora is `async` end to end (tokio + `reqwest` async + `futures_util::StreamExt`); Skriuw runs AI on dedicated `std::thread`s with `reqwest::blocking` and exposes the result through `tauri::async_runtime::spawn_blocking`. This is the single most consequential difference for the shared Rust core (see section 16).
- Two `reqwest` majors: Dora `0.12`, Skriuw `0.13.4`.
- Three Ollama implementations exist across the repositories: Dora (`ollama_installer/` + `services/ai/ollama.rs`), Skriuw v2 (`skriuw-ai-ollama`), Skriuw v1 desktop (`v1/apps/desktop/src-tauri/src/ai/{installer,ollama}.rs`). The Skriuw v1 one is frozen and is a near-copy of Dora's (same `.skriuw-managed`/`.dora-managed` marker pattern, same `find_binary_recursive`, same `platform_download` URLs).
- Vercel AI SDK appears at two incompatible majors: Betalingen `ai@7.0.93` + `@ai-sdk/groq@4`, Skriuw v1 `ai@^6` + `@ai-sdk/google@^3` + `@ai-sdk/groq@^3`. Dora's release tooling uses the raw `@google/generative-ai` SDK (`tools/scripts/generate-release.ts`), which is developer tooling, not product code.

---

## 3. Current AI architecture: Dora

### 3.1 Module map

```
apps/desktop/src-tauri/src/
  database/services/ai/
    mod.rs        AIProvider enum (11 ids), AIRequest/AIResponse, AiStreamEvent, SchemaContext,
                  AiServiceConfig, AiStatus, AIService (get/set provider+model, status, complete)
    client.rs     trait AiClient { complete, complete_stream }, build_client(), test_key(),
                  MockClient, http_client(), should_rotate(), send_with_rotation(), read_sse()
    compat.rs     CompatSpec + 7 consts (OpenAI, Groq, DeepSeek, Kimi, GLM, Qwen, OpenRouter),
                  OpenAiCompatClient (chat/completions, /models, json_object mode)
    anthropic.rs  AnthropicClient (/v1/messages, SSE content_block_delta)
    gemini.rs     GeminiClient (generateContent / streamGenerateContent?alt=sse, key in query string)
    ollama.rs     OllamaClient (/api/chat, /api/tags, /api/pull, /api/delete, /api/version),
                  OllamaStatus, OllamaCatalogEntry, OllamaPullEvent, RECOMMENDED_MODELS
    key_pool.rs   KeyPool (env {PREFIX}_API_KEY[_1..10] + stored keys, round-robin)
    models.rs     curated catalogs per provider, merge_models(), classify_* tier heuristics,
                  list_*_models() (live fetch, fallback to curated)
    prompts.rs    build(request) -> (system, user); JSON-only SQL mode vs "chat" mode;
                  append_schema_block() with truncation limits (60 tables, 40 cols, 20 indexes)
    usage.rs      AiUsageCapture, estimate_tokens_from_text (chars/4), pricing_for_model table,
                  record_usage()
    errors.rs     http_error_message() (429/401/403/404/body-mentions-model), request_error()
  database/commands/ai.rs   34 Tauri commands (ai_complete, ai_complete_stream, ai_abort_stream,
                            config/status/models, usage, ollama_*, keys_*)
  ollama_installer/          managed install: download.rs (ollama.com URLs, tar.zst/tgz/zip),
                             paths.rs (data_local_dir/dora/ollama, .dora-managed marker),
                             runtime.rs (spawn `ollama serve`, OLLAMA_HOST/OLLAMA_MODELS, kill on exit)
  storage/ai_keys.rs         ai_api_keys table (AES-GCM ciphertext), migrate_legacy_gemini_key()
  storage/ai_usage.rs        ai_usage table + totals
  security.rs                encrypt()/decrypt() AES-256-GCM, master key in keyring or file
  credential_storage.rs      keyring backend detection, install plan, pkexec install
packages/studio/src/features/ai-assistant/
  use-ai-chat.ts             Channel<AiStreamEvent>, request ids, abort, rAF batching
  build-prompt.ts            packs history as USER:/ASSISTANT: plus "Current Dora UI context"
  ai-actions.ts              askAi(), buildExplainQueryPrompt(), buildFixErrorPrompt()
  assistant-response-parser.ts  lenient JSON {sql, explanation, warnings} parse
  mock-ai.ts                 browser-demo deterministic responses, mock status/models/usage/ollama
  suggestions.ts             dynamic prompt suggestions from schema
  store.ts                   zustand persisted threads keyed by connection id
  ai-provider-section.tsx, ollama-models-section.tsx, ai-usage-section.tsx, model-id-input.tsx
packages/studio/src/features/sidebar/components/ai-keys-section.tsx   key CRUD + test
packages/studio/src/features/sql-console/components/ai-cmd-k.tsx      Cmd+K SQL generation
```

### 3.2 Request flow (streaming)

1. UI builds a single `prompt` string. The chat panel packs the whole thread and UI context into that string (`build-prompt.ts` → `buildChatPrompt`). The Cmd+K dialog sends the raw prompt with `promptMode = null`.
2. `commands.aiCompleteStream(requestId, prompt, connectionId, maxTokens, promptMode, channel)` (`bindings.ts:1196`).
3. `ai_complete_stream` (`commands/ai.rs`) builds `SchemaContext` from `AppState.schemas[connection_id]` and `engine_for_connection`, registers an `Arc<AtomicBool>` under `state.ai_cancel_flags[request_id]`, creates a tokio unbounded channel, spawns a forwarder task that copies every `AiStreamEvent` to the Tauri `Channel` and remembers the `Final` content.
4. `AIService::complete_stream` → `client::build_client(provider, storage)` → `Box<dyn AiClient>` → provider `complete_stream(request, sender, cancel)`.
5. Cloud clients call `send_with_rotation` (retries once per key on transport error or 429/401/403/5xx) then `read_sse` (line-buffered `data:` parsing, checks `cancel` between chunks). Ollama uses NDJSON on `/api/chat`.
6. Each delta is sent as `AiStreamEvent::Token { text }`; on natural end `Final { content }`; on cancellation the stream simply returns `Ok(())` with no terminal event.
7. After the provider returns, the command records usage with `AiUsageCapture::estimated_from_text` (character-based) because the streaming path never parses provider usage.
8. `ai_abort_stream(request_id)` flips the flag; the UI additionally sets a local `cancelled` bit so late events are ignored.

### 3.3 Provider selection and configuration

- Exactly one active provider (`settings.ai_provider`, default `groq`) and one model per provider (`settings.ai_model.<provider>` or `ollama_model`); resolution order is saved setting → `{PREFIX}_MODEL` env → built-in default (`resolve_model`).
- Keys: `KeyPool::from_env_and_storage` merges `{PREFIX}_API_KEY`, `{PREFIX}_API_KEY_1..10` and every active stored key; `send_with_rotation` rotates on failure. This is the only "fallback" mechanism in any repository and it is intra-provider.
- Readiness: `AIService::get_status` probes every provider (key count for cloud, `/api/tags` for Ollama) and returns `AiStatus { active_provider, active_model, ready, providers[] }`.

### 3.4 Structured output in Dora

The default (non-chat) mode asks for `{"sql","explanation","warnings"}` JSON. For OpenAI-compatible providers the request sets `response_format: {type: "json_object"}` (`compat.rs` `build_request`); Anthropic, Gemini and Ollama rely on the prompt alone. The UI parses leniently (`parseLlmJson` in `ai-cmd-k.tsx`, `parseAssistantSqlResponse` in `assistant-response-parser.ts`), stripping code fences and falling back to treating the whole text as SQL with a warning. There is no schema validation and no repair loop.

### 3.5 Errors in Dora

All AI errors are `Error::Any(anyhow!(...))` or `Error::InvalidInput`, so the wire shape is `{kind: "Internal" | "InvalidInput", detail: "<message>"}` (`error.rs` `tag()`). `errors.rs` produces user-facing copy for 429 / 401 / 403 / 404 / body-mentions-model / connection-refused-to-Ollama, but the category is lost in the string; the frontend cannot branch on it. The `AiStreamEvent::Error { message }` variant carries the same string.

### 3.6 State of documentation

- `docs/ai-providers.md` documents five providers plus Mock; the code has eleven. Stale.
- `src/database/contract.rs` (a command registry description) still describes `ai_set_provider` as "Switch between 'gemini' and 'ollama'" and `ai_complete` side effects as "Gemini or Ollama". Stale.
- `docs/product-roadmap.md` §3-5 says AI has "basic completion" without schema context; the code passes full schema, indexes and row-count estimates. Stale in the other direction.
- `docs/specs/03-mcp-server.md` plans Dora acting as an MCP *server* for coding agents. That is Dora exposing tools, not Dora consuming tool calls from a model; it does not create a tool-calling requirement for the SDK.

### 3.7 Half-implemented or notable items

| Item | Status |
| --- | --- |
| `AIResponse.suggested_queries` | Always `None` in every adapter. Dead field. |
| `MockClient` in Rust | Production code path that always errors; the real mock lives in TS (`mock-ai.ts`) for the web demo. |
| `ai_groq_status` command | Superseded by `ai_get_status`; still registered. |
| `ai_set_gemini_key` command | Legacy single-key path kept for migration; `ai_keys_add` is the general path. |
| Streaming usage capture | Estimated only (`estimated_from_text`); `stream_options.include_usage` is never requested. |
| Retry | Only key rotation; no backoff, no `Retry-After` handling. |
| Timeouts | `reqwest` client timeout 60 s (cloud), 120 s (Ollama chat), 30 min (Ollama pull). No per-request deadline. |

---

## 4. Current AI architecture: Skriuw

### 4.1 Module map (v2)

```
crates/skriuw-domain/src/
  ai.rs          AiCompletionRequest/Parameters, AiCompletionDelta, AiUsage, AiProviderError
                 (+ErrorCategory, RecoveryAction), AiCompletionEvent, AiCompletionTerminal,
                 AiCancellation, AiEventSink, trait AiComplete, bounds (MAX_AI_* consts)
  local_ai.rs    LocalAiRuntimeState/Status/Model/Operation/Progress/Error, trait LocalAiRuntime,
                 trait LocalAiProgressSink
  remote_ai.rs   AiCredential (zeroizing, redacted Debug), AiCredentialError, trait AiCredentialSource,
                 RemoteAiCatalog/Model/ModelListing/ModelDirectory, CredentialVaultState/Detection,
                 RemoteAiKeyTier, RemoteAiProviderState, RemoteAiConsent, REMOTE_AI_DISCLOSURE_VERSION
  prompt.rs      PromptInputShape, PromptParameters, BuiltInPrompt(+Library, 15 prompts), WorkspacePrompt
  ai_history.rs  AiRunRecord/State/Tokens/TokenSource, AiRunRecorder, AiModelPricing/Price,
                 AiHistorySettings/Retention, AiRunFilter, AiUsageAggregate, ai_run_cost_micros()
crates/skriuw-ai/src/lib.rs
  AiCompletionService (provider map, active-request map, one std::thread per request,
  first-terminal-wins, records runs), AiCompletionChannel trait, AiStartError,
  FakeAiProvider + FakeCompletionScript/Outcome
crates/skriuw-ai-ollama/src/lib.rs
  OllamaRuntime: impl LocalAiRuntime (status/start/stop/install/list/pull/remove/shutdown)
  + impl AiComplete (/api/generate NDJSON); GitHub releases API + SHA-256 verification;
  loopback-only endpoint; bounded readers
crates/skriuw-ai-remote/src/{lib.rs,provider.rs}, models.json
  RemoteAiProvider: impl AiComplete; RemoteProviderKind {Gemini, Groq, DeepSeek, Moonshot, Zai,
  DashScope, AimlApi}; OpenAiCompatible table; status_error(); list_models(); verify_credential();
  RemoteAiModelAuthority; embedded priced catalog (version 3, pricingAsOf 2026-08-27)
app/src-tauri/src/
  ai.rs             LazyAiCompletion (OnceLock<AiCompletionService>), TauriCompletionChannel
  ai_credentials.rs AiCredentialStore: keyring vault or session-only; consent file; Linux vault detection
  ai_models.rs      FetchedModelStore (ai-models.json), directory() merges catalog + fetched
  ai_history.rs     AiHistoryRecorder (bounded queue 64, dedicated writer thread, own SQLite conn)
  ollama.rs         OllamaManager + OperationRegistry (duplicate-id rejection, cancel_all)
  commands/ai.rs    22 commands (start/cancel completion, ollama_*, remote_ai_*, *_key, history)
app/src/contracts/ai.ts        hand-written mirror of the generated schemas
app/src/features/ai/
  completion-bridge.ts   startAiCompletion(request, origin, onEvent, signal) -> {cancel, dispose}
  completion-consumer.ts request-id + sequence gate
  use-ai-run.ts          one streaming run; rAF-batched deltas; retry with new request id
  editor-actions.ts      15 actions as data {id, promptId, scope, outcome, instruction}
  editor-action-model.ts run phases; applyRefusal()
  editor-action-apply.ts ProseMirror input extraction + accept transactions
  action-plan.ts         parse bullet lists into task/tag plans (bounded, deduped)
  inline-suggestion.tsx  ADR-0036 decoration-based review
  model-selection.ts     settings.aiModel {providerId, modelId}
  model-options.ts       provider groups from Ollama status/models + remote provider states/directory
  ollama-model-catalog.ts curated Ollama picks with useCase
  prompt-library.ts      built-in + workspace prompts, shadowing
  remote-ai-bridge.ts, ollama-bridge.ts   invoke wrappers with AbortSignal -> cancel command
  prompt-playground.tsx, playground-model.ts, usage-model.ts, history-bridge.ts, opt-in-gate.tsx
```

### 4.2 Request flow (streaming)

1. An editor action or the playground resolves `{providerId, modelId}` (`resolveAiModel(override, settings)`), a system prompt (built-in or user-customised), and builds `AiCompletionRequest` with `requestId = crypto.randomUUID()` and bounded parameters (`buildAiActionRequest`: timeout 60 s, retryCount 0, `maxOutputBytes` from the prompt).
2. `startAiCompletion(request, origin, onEvent, signal)` opens a `Channel<AiCompletionEvent>`, invokes `start_ai_completion`, and wires the `AbortSignal` to `cancel_ai_completion`.
3. `start_ai_completion` validates `origin` as an identifier and calls `LazyAiCompletion::start`, which lazily builds `AiCompletionService` with providers `fake`, `ollama`, and the seven remote kinds, plus the history recorder and catalog pricing.
4. `AiCompletionService::start` validates the request, rejects duplicate request ids, registers an `AiCancellation`, and spawns a named `std::thread` that calls `provider.complete(&request, &cancellation, &mut sink)`.
5. The sink (`CompletionServiceSink`) forwards each `AiCompletionDelta` as `AiCompletionEvent::Delta`; a failed channel send cancels the request. Adapters check cancellation and the overall deadline on every read, enforce `max_output_bytes` and the global `MAX_AI_RESPONSE_BYTES`, and validate every delta.
6. The adapter returns one `AiCompletionTerminal`; the service publishes it (converting `Done` to `Cancelled` if cancellation raced), then records an `AiRunRecord` (provider-reported usage or byte-based estimate, catalog price).
7. The renderer's `createAiCompletionConsumer` drops events for other request ids or out-of-order sequences; `useAiRun` flushes buffered deltas on animation frames and never writes to the document until accepted.

### 4.3 Remote providers, credentials and consent

- Provider construction requires an `Arc<dyn AiCredentialSource>`; the credential is resolved per request only after the request is validated and the model is permitted, so an unconsented provider "terminalizes before any socket is opened" (`RemoteAiProvider::complete`).
- `AiCredentialStore::resolve` enforces the consent version (`REMOTE_AI_DISCLOSURE_VERSION = 1`), then looks up the session-only map, then the keyring.
- Response bodies from providers are drained under a size cap and discarded; errors are mapped from HTTP status alone (`status_error`).
- Model ids are gated by a `RemoteAiModelAuthority` (shipped catalog widened by user-fetched models persisted in `ai-models.json`), so renderer text can never become a provider URL segment.

### 4.4 Structured output in Skriuw

There is none at the provider level. Task and tag extraction ask for a Markdown bullet list and parse it defensively (`action-plan.ts`: strips bullets/checkboxes, dedupes, bounds to 50 items and 500/64 bytes, rejects prose). Titles are plain text. This is deliberate: outcomes other than text are reviewed as plans before any workspace operation runs.

### 4.5 Retries

`AiCompletionParameters.retry_count` is validated (`MAX_AI_RETRIES = 2`) and ADR-0033 specifies retries "only before the first delta for a retryable failure", but no code path retries: `AiCompletionService::start` calls `provider.complete` exactly once and neither adapter loops. Editor actions send `retryCount: 0`. Classification: **documented but missing**.

### 4.6 Skriuw v1 (frozen)

`v1/apps/web/src/app/api/ai/route.ts` is a Next.js route using `generateText`/`streamText` from `ai@6` with `@ai-sdk/google` and `@ai-sdk/groq`; `v1/apps/web/src/domain/ai/constants.ts` maps thirteen actions to per-action default models (`ACTION_MODEL_DEFAULTS`, ids like `google.gemini-2.5-flash`); `prompts.json` was shared between the web route and the v1 desktop Rust backend (`v1/apps/desktop/src-tauri/src/ai/*.rs`, 2,097 lines, with Groq/Gemini SSE clients and another managed Ollama installer). v1 also had Gemini or Ollama embeddings for semantic search (`features/notes/server/semantic-embeddings.ts`). None of this is live in v2; it is evidence of what was tried and deliberately redesigned.

---

## 5. Current AI architecture: Betalingen

### 5.1 Module map

```
src/lib/ai/
  groq.ts        answerWithGroq(request, context, signal, options): streamText({model: groq(model),
                 system: DATA_ASSISTANT_PROMPT, messages, temperature 0.2, maxOutputTokens 1800,
                 maxRetries 0, abortSignal})
  contracts.ts   zod: ScreenContextSchema, ChatRequestSchema (<=20 messages, last must be user),
                 ChatEventSchema {text|done|error}
  context.ts     isContextSource() allowlist regex over read-only GET routes; buildScreenContext()
                 reloads sources with the caller's auth headers, masks IBANs, caps at 100k chars;
                 DATA_ASSISTANT_PROMPT (Dutch, grounding + injection-resistance rules)
src/routes/ai.ts POST /ai/chat: bodyLimit 48k; 503 without key; validates sources; builds context;
                 AbortSignal.any(request, controller, timeout 60s); NDJSON via hono/streaming;
                 iterates result.fullStream, forwards text-delta, throws on error/abort;
                 sanitized error event
src/ui/assistant/use-data-chat.ts   fetch + ReadableStream NDJSON parse; AbortController; retry; clear
src/ui/assistant/main.tsx           DataChat / Assistant React island; window.BetalingenAssistant.update()
src/ui/dashboard.html.txt           collects the GET paths a view loaded into `assistantSources`,
                                    lazy-loads /ai/assistant.js, publishes ScreenContext
src/lib/ai/ai.test.ts               bun tests with injected fetch
```

### 5.2 Request flow

1. The plain-JS dashboard records every non-auth GET path it fetched for the current view and publishes `{title, location, period, sources}`.
2. `useDataChat.send` posts `{context, messages}` (last 18 turns) to `/ai/chat` with `credentials: 'same-origin'`.
3. The route re-fetches each allowlisted source through `app.request(path, {headers})`, so the model only ever sees data the caller is authorized to read, then streams.
4. The client parses NDJSON lines with `ChatEventSchema.parse`, appends `text` to the assistant message, treats a missing `done` as an interrupted connection.

### 5.3 What is absent

No BYOK, no model catalog, no provider switching, no usage accounting, no structured output, no tools, no retries (`maxRetries: 0`), no local provider. The budget-plan PDF import (`src/lib/budgetplan-import.ts`) is a deterministic regex parser over `unpdf` text extraction, not an AI feature, despite the audit brief listing "AI-backed import/parser" as a candidate. The dashboard disclosure text states that sources are shared with Groq.

### 5.4 Why it matters for the SDK

Betalingen is the clearest demonstration that the TypeScript side needs very little: an abort-aware streaming call, a stable three-event NDJSON contract, an injectable transport for tests, and server-side credential resolution. It also shows the AI SDK's `streamText` API surface that a TypeScript core would wrap: `model`, `system`, `messages`, `temperature`, `maxOutputTokens`, `maxRetries`, `abortSignal`, `fullStream` with `text-delta` / `error` / `abort` parts.

---
## 6. Complete AI feature inventory

Legend for the "Class" column: **R** = reusable operation with app-supplied context, **A** = application-specific (stays in the app), **P** = platform/runtime management (candidate for an optional package).

### 6.1 Dora

| # | User-facing feature | Internal operation | Entry point (UI → IPC → Rust) | Prompt builder | Context supplied | Provider/model | Stream | Result shape | Cancel | Retry | Consumed by / mutates | Confirmation | Class |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| D1 | Cmd+K "AI SQL" natural-language → SQL | `ai_complete_stream` with `prompt_mode = None` | `ai-cmd-k.tsx` `generate` → `commands.aiCompleteStream(requestId, prompt, connId, null, null, channel)` → `commands/ai.rs::ai_complete_stream` → `AIService::complete_stream` | `prompts.rs::build_system_prompt` (JSON-only, dialect from engine, examples, destructive-statement rules) | `SchemaContext` (engine, tables, columns, PKs, FKs, indexes, row estimates) from `AppState.schemas`; no UI context | Active provider + resolved model; `json_object` response format on OpenAI-compatible | Yes (`Token`/`Final`) | Structured (JSON `{sql, explanation, warnings}`) parsed leniently in UI | `ai_abort_stream(request_id)` + local flag | Key rotation only | Inserts SQL into the console editor; optionally executes ("Insert + Run") | User clicks Insert / Insert + Run | R (structured generation) + A (schema context, SQL rules) |
| D2 | Schema-aware chat assistant panel | `ai_complete_stream` with `prompt_mode = "chat"` | `use-ai-chat.ts::send` → same command with `maxTokens 2048`, `'chat'` | `prompts.rs::build_chat_system_prompt` + UI-side `buildChatPrompt` (packs `USER:`/`ASSISTANT:` history and "Current Dora UI context") | Schema context (Rust) + active view, connection id, selected table + columns, editor draft tail (6000 chars) (TS) | Active provider; warmer temperature on Groq (0.35) | Yes | Free-text Markdown; ```sql blocks get Run/Copy/Insert actions via `sql-code-utils.ts` | Yes | Key rotation | Thread persisted in `localStorage` (`dora-ai-assistant`); Run/Insert actions mutate console | Run requires click | R (chat) + A (context) |
| D3 | "Explain query" contextual action | Prefills D2 | `ai-actions.ts::buildExplainQueryPrompt` → `askAi()` opens panel with pending prompt | Plain string template | Query text | as D2 | as D2 | Free text | as D2 | as D2 | Panel | none | A (prompt), R (chat) |
| D4 | "Fix with AI" on failed query | Prefills D2 | `ai-actions.ts::buildFixErrorPrompt` | Plain string template | Query + error | as D2 | as D2 | Free text | as D2 | as D2 | Panel | none | A |
| D5 | Dynamic suggestions / quick actions ("Seed data", "Schema design", "Debug SQL error", "Optimize query") | No AI call; local heuristics | `suggestions.ts::buildDynamicSuggestions` | n/a | Schema table names / column patterns | n/a | n/a | Strings | n/a | n/a | Panel chips | n/a | A (no AI) |
| D6 | Non-streaming completion | `ai_complete` | Command exists; no current UI caller found (`grep aiComplete(` matches only the binding) | `prompts.rs::build` | Schema | Active | No | `AIResponse { content, suggested_queries: None, tokens_used, provider }` | No | Key rotation | Records usage with provider `total_tokens` | n/a | R (dead code in UI) |
| D7 | Provider/model selection | `ai_get_config` / `ai_set_config` / `ai_set_provider` / `ai_resolve_provider_model` | `ai-provider-section.tsx`, `ai-selection-store.ts` | n/a | n/a | Persists `ai_provider`, `ai_model.<p>`, `ollama_endpoint` | n/a | `AiServiceConfig` | n/a | n/a | Settings | n/a | P/R (config) |
| D8 | Model catalog per provider | `ai_list_provider_models` | `model-id-input.tsx` filter, provider section | n/a | n/a | Curated + live `/models` merge with tier heuristics | n/a | `AiModelOption { id, label, tier }` | n/a | Fallback to curated on failure | Picker | n/a | R (catalog + discovery) |
| D9 | Provider readiness status | `ai_get_status` | `use-ai-status.ts`, badges in Cmd+K/panel | n/a | n/a | Probes key pools and Ollama `/api/tags` | n/a | `AiStatus` | n/a | n/a | Badges, gating | n/a | R |
| D10 | API key management (add, delete, toggle active, test saved/raw/configured) | `ai_keys_*` | `ai-keys-section.tsx` | Test uses fixed "Reply with the word OK only" or a user prompt | n/a | Per provider; test hits chat endpoint with `max_tokens` 4-8 | No | `AiKeyTestResult { ok, message }` | No | No | `ai_api_keys` table (AES-GCM); records `key_test` usage | n/a | P (credential store) + R (key verification) |
| D11 | Usage summary | `ai_get_usage_summary` | `ai-usage-section.tsx` | n/a | n/a | n/a | n/a | `AiUsageSummary` (totals, per provider, recent 25) | n/a | n/a | Settings | n/a | R (metadata) + A (storage) |
| D12 | Ollama manager: status, catalog, pull (progress + ETA), cancel pull, delete, list, install managed, cancel install, start | `ai_get_ollama_status`, `ai_list_ollama_catalog`, `ai_pull_ollama_model`, `ai_cancel_ollama_pull`, `ai_delete_ollama_model`, `ai_list_ollama_models`, `ai_install_ollama`, `ai_cancel_ollama_install`, `ai_start_ollama` | `ollama-models-section.tsx` with `Channel<OllamaPullEvent>` / `Channel<OllamaInstallEvent>` | n/a | n/a | Endpoint from settings | Progress channels | Typed events | Yes (request-id flag maps) | No | Filesystem + child process | Pull/install are user-initiated | P |
| D13 | Web demo mock AI | TS-only | `mock-ai.ts` (`streamMockText`, `buildMockChatResponse`, `buildMockSqlJson`, mock status/models/usage/ollama) | canned | connection id | none | Simulated | Text/JSON | Local flag | n/a | Same UI paths | n/a | A (demo) — but demonstrates the need for a fake provider |
| D14 | Hide AI setting | `settings.hideAi` | `settings-store.tsx`, `workspace-shell.tsx` | n/a | n/a | n/a | n/a | n/a | n/a | n/a | Removes all AI surfaces | n/a | A |

Not AI despite the brief's candidates: table seeding (`services/seeding.rs`, `fake` crate), schema export, query builder, Drizzle/Prisma diff. Dora does not have AI seed-data generation or AI schema design as operations; they exist only as suggested prompts to the chat (D5).

### 6.2 Skriuw v2

All completions go through one command, `start_ai_completion(request, origin, on_event)`, so the entry point column lists the renderer builder. Every text outcome is reviewed before it touches the note (ADR-0036).

| # | User-facing feature | Origin id | Scope → outcome | Prompt (built-in id) | Context supplied | Provider/model | Stream | Result | Cancel | Retry | Consumed by / mutates | Confirmation | Class |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| S1 | Rewrite | `editor:rewrite` | selection → text | `rewrite` | Selected plain text (`state.doc.textBetween`) | `settings.aiModel` or override | Yes | Free text | Yes (`AbortSignal` → `cancel_ai_completion`) | Manual retry with new request id | Inline suggestion; accept = one `replaceRange` transaction | Accept/Discard in place | R (task prompt is generic) |
| S2 | Improve writing | `editor:improve` | selection → text | `improve` | as S1 | | | | | | | | R |
| S3 | Fix spelling and grammar | `editor:fix-grammar` | selection → text | `fix-grammar` (temperature 0.1) | as S1 | | | | | | | | R |
| S4 | Make shorter | `editor:shorten` | selection → text | `shorten` | | | | | | | | | R |
| S5 | Make longer | `editor:lengthen` | selection → text | `lengthen` | | | | | | | | | R |
| S6 | Simplify | `editor:simplify` | selection → text | `simplify` | | | | | | | | | R |
| S7 | Change tone | `editor:change-tone` | selection → text | `change-tone` | + optional instruction ("Tone: …") prefixed to user prompt | | | | | | | | R |
| S8 | Translate | `editor:translate` | selection → text | `translate` | + optional target language | | | | | | | | R |
| S9 | Custom instruction | `editor:custom` | selection → text | `custom` | + required instruction | | | | | | | | R |
| S10 | Continue writing | `editor:continue` | caret → text | `continue` | Note text up to the caret | | | | | | Inserted below | | R (closest existing thing to autocomplete) |
| S11 | Summarize note | `editor:summarize` | note → text | `summarize` | Whole note as product Markdown | | | | | | Insert below / copy | | R |
| S12 | Outline note | `editor:outline` | note → text | `outline` | | | | | | | | | R |
| S13 | Suggest a title | `editor:title` | note → title | `title` | | | | Plain text | | | `renameNode` | Dialog | R |
| S14 | Extract tasks | `editor:extract-tasks` | note → tasks | `extract-tasks` | | | | Bullet list parsed by `parseTaskPlan` | | | Appends a `check_list` via one transaction | Reviewable plan with checkboxes | R (list extraction) + A (task nodes) |
| S15 | Suggest tags | `editor:suggest-tags` | note → tags | `suggest-tags` | | | | Bullet list parsed by `parseTagPlan` | | | `commitReferenceOperations` + tag_ref paragraph | Reviewable plan | R + A |
| S16 | Prompt playground | `playground` | freeform | any library prompt or ad-hoc system prompt | User-typed | Any available model incl. `fake` | Yes | Text + usage + events log | Yes | Yes | Nothing | n/a | A (dev/test surface) but exercises R |
| S17 | Prompt library (built-in, customised shadow, user prompts) | n/a | n/a | `BUILT_IN_PROMPTS` generated to `built-in-prompts.json`; `WorkspacePrompt` in SQLite | n/a | n/a | n/a | n/a | n/a | n/a | Workspace operations | n/a | A (storage) / R (prompt data) |
| S18 | Model switcher + default model | n/a | n/a | n/a | n/a | `settings.aiModel` | n/a | n/a | n/a | n/a | Settings operation | n/a | R (selection type) + A (storage) |
| S19 | Ollama runtime: status, start, stop, install (progress), list, pull (progress), delete, cancel operation | commands `ollama_runtime_status` … `cancel_ollama_operation` | n/a | n/a | n/a | Loopback endpoint (`SKRIUW_OLLAMA_ENDPOINT` override) | `Channel<LocalAiProgress>` | Typed | Yes (operation ids) | n/a | FS + process | User action | P |
| S20 | Remote BYOK: provider states, vault detection, save/remove key (vault or session tier), accept/revoke disclosure, verify key, catalog, refresh fetched models | `remote_ai_*`, `save_remote_ai_key`, `verify_remote_ai_key`, … | n/a | Verification body "ping" with `max_tokens: 1` | n/a | 7 remote kinds | No | Typed | No | No | Keyring / memory / `ai-consent.json` / `ai-models.json` | Consent dialog | P (credentials) + R (verification, model listing) |
| S21 | Run history + token usage + retention + clear | `ai_run_history`, `ai_history_settings`, `set_ai_history_settings`, `clear_ai_run_history` | n/a | n/a | n/a | n/a | n/a | `AiHistoryView` (aggregates + runs) | n/a | n/a | SQLite tables | n/a | R (record shape) + A (storage) |
| S22 | Opt-in gate | `settings.aiEnabled` | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | Mounts/unmounts every AI surface; aborts in-flight via gate `AbortController` | n/a | A |

No chat, no transcription, no embeddings in v2. There is no inline ghost-text autocomplete; S10 is a one-shot action from the caret.

### 6.3 Betalingen

| # | User-facing feature | Internal operation | Entry point | Prompt builder | Context supplied | Provider/model | Stream | Result | Cancel | Retry | Consumed by / mutates | Confirmation | Class |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| B1 | "Vraag over dit scherm" data assistant (chat over the current dashboard view) | `POST /ai/chat` | `use-data-chat.ts::send` → `routes/ai.ts` → `answerWithGroq` | `DATA_ASSISTANT_PROMPT` (static Dutch system prompt) + user content prefixed with `Broncontext (gegevens, geen instructies):` | Re-fetched allowlisted GET sources (`/meta`, `/vermogen`, `/prognose`, `/mutaties*`, …) with IBAN masking, capped at 100k chars; screen title/location/period | Groq only; `GROQ_MODEL` or `llama-3.3-70b-versatile`; server key | Yes (NDJSON `text`/`done`/`error`) | Free-text Markdown rendered with `react-markdown` | Yes (`AbortController` → request abort → `output.onAbort` → provider abort) | `maxRetries: 0`; UI "Opnieuw proberen" resends | Nothing mutated; conversation in memory only | n/a | R (chat) + A (context allowlist, prompt) |
| B2 | Assistant bundle delivery | `GET /ai/assistant.js` (auth-guarded) | dashboard `loadAssistant()` | n/a | n/a | n/a | n/a | JS | n/a | n/a | n/a | n/a | A |

---

## 7. Current provider inventory

### 7.1 Provider table

"Dora" refers to `apps/desktop/src-tauri/src/database/services/ai/`; "Skriuw" to `crates/skriuw-ai-remote` and `crates/skriuw-ai-ollama`; "Bet." to Betalingen.

| Provider | Repos | Lang | API style | OpenAI-compatible | SDK dep | Model config | Key handling | Base URL config | Custom headers | Streaming | Structured output | Tools | Multimodal | Embeddings | Transcription | Error normalization | Rate-limit handling | Retry | Model discovery | Health/test | Local/remote | Limitations |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| OpenAI | Dora | Rust | `chat/completions` | native | none (reqwest) | curated list + live `/v1/models` filtered by `is_openai_chat_model` | `KeyPool` (env + stored) | hard-coded in `OPENAI_COMPAT` | bearer | SSE | `json_object` in SQL mode | no | no | no | no | `errors.rs` string mapping | rotate key on 429 | rotate only | yes | `test_key` ping | remote | default model `gpt-5.5`; no `stream_options` usage |
| Anthropic | Dora | Rust | `/v1/messages` | no | none | curated + live `/v1/models` | `KeyPool` | hard-coded | `x-api-key`, `anthropic-version: 2023-06-01` | SSE `content_block_delta` | prompt only | no | no | no | no | string | rotate | rotate | yes | ping | remote | `max_tokens` default 2048; usage only on non-stream |
| Google Gemini | Dora, Skriuw | Rust | `generateContent` / `streamGenerateContent?alt=sse` | no | none | Dora: curated + live; Skriuw: catalog + fetched (`pageSize=1000`, filtered by `generateContent`) | Dora: key in **query string** (`?key=`); Skriuw: `x-goog-api-key` header | hard-coded | Skriuw uses `systemInstruction`; Dora concatenates system into the single user part | SSE | prompt only | no | no | no | no | Dora string / Skriuw category | Dora rotate / Skriuw `RateLimited` category | none | yes | Dora ping / Skriuw `verify_credential` (`maxOutputTokens: 1`) | remote | Dora leaks the key into URLs and therefore into any logged URL; Skriuw explicitly avoids this |
| Groq | Dora, Skriuw, Bet. | Rust, Rust, TS | OpenAI-compatible `openai/v1/chat/completions` | yes | Bet.: `@ai-sdk/groq` | Dora curated+live; Skriuw catalog (3 models) + fetched; Bet. env `GROQ_MODEL` | Dora `KeyPool`; Skriuw vault/session; Bet. server env | hard-coded | bearer | SSE (Bet. via AI SDK) | Dora `json_object` | no | no | no | no | Dora string; Skriuw category; Bet. sanitized generic message | Dora rotate; Skriuw category; Bet. none | none | Dora/Skriuw yes; Bet. no | Dora/Skriuw yes; Bet. 503 when key missing | remote | Bet. has no BYOK |
| Ollama | Dora, Skriuw | Rust | Dora `/api/chat`; Skriuw `/api/generate` | no (native API) | none | Dora setting `ollama_model`; Skriuw `settings.aiModel` | none | Dora any URL from settings; Skriuw loopback only | none | NDJSON | prompt only | no | no | no | no | Dora connection-refused special-case; Skriuw category | n/a | none | `/api/tags` both | `/api/version` both | local | Dora `num_predict` from `max_tokens`; Skriuw `temperature`/`top_p` only |
| DeepSeek | Dora, Skriuw | Rust | OpenAI-compatible | yes | none | curated / catalog | as above | `api.deepseek.com/chat/completions` | bearer | SSE (+ `stream_options.include_usage` in Skriuw) | Dora `json_object` | no | no | no | no | as above | as above | none | `/models` | yes | remote | |
| Moonshot Kimi | Dora (`Kimi`), Skriuw (`moonshot`) | Rust | OpenAI-compatible | yes | none | | | `api.moonshot.ai/v1` | bearer | SSE | | no | no | no | no | | | none | yes | yes | remote | different provider ids across repos |
| Z.ai GLM | Dora (`Glm`), Skriuw (`zai`) | Rust | OpenAI-compatible | yes | none | | | `api.z.ai/api/paas/v4` | bearer | SSE; Skriuw omits `stream_options` (unverified tolerance) | | no | no | no | no | | | none | Dora yes (`/models`); Skriuw no listing endpoint | yes | remote | ids differ |
| Alibaba Qwen / DashScope | Dora (`Qwen`), Skriuw (`dashscope`) | Rust | OpenAI-compatible `compatible-mode/v1` | yes | none | | | `dashscope-intl.aliyuncs.com` | bearer | SSE | | no | no | no | no | | | none | yes | yes | remote | ids differ |
| OpenRouter | Dora | Rust | OpenAI-compatible | yes | none | curated (`openrouter/auto`, `z-ai/glm-5.3-flash`) + live | `KeyPool` | `openrouter.ai/api/v1` | bearer | SSE | `json_object` | no | no | no | no | string | rotate | none | yes | yes | remote (aggregator) | no `HTTP-Referer`/`X-Title` headers |
| AI/ML API | Skriuw (`aimlapi`) | Rust | OpenAI-compatible | yes | none | catalog + fetched (filters `type != openai/chat-completions`) | vault/session | `api.aimlapi.com/v1` | bearer | SSE | no | no | no | no | no | category | category | none | yes | yes | remote (aggregator) | |
| Mock / Fake | Dora (`Mock`, TS `mock-ai.ts`), Skriuw (`fake`, `FakeAiProvider`) | TS / Rust | n/a | n/a | n/a | fixed | none | n/a | n/a | simulated | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | local | Dora's Rust `MockClient` only errors; the TS mock is used in the web demo. Skriuw's fake is a first-class scripted provider used in tests and the playground. |

Skriuw v1 (frozen) additionally had Groq and Gemini Rust clients (`v1/apps/desktop/src-tauri/src/ai/cloud.rs`) and Vercel AI SDK `@ai-sdk/google`/`@ai-sdk/groq` in the web route.

### 7.2 Duplication of provider code

- **OpenAI-compatible chat**: implemented twice in Rust (Dora `compat.rs`, Skriuw `provider.rs` `OpenAiCompatible`) with the same idea (a static spec row per provider) and once through the AI SDK in TypeScript. The Skriuw version carries two extra facts per row that Dora lacks: `destination` (for disclosure) and `stream_usage_option` (whether `stream_options.include_usage` is accepted). Dora's version carries `chat_temperature` and `models_url`.
- **Gemini**: implemented twice in Rust with a security-relevant difference (query-string key vs header).
- **Anthropic**: Dora only.
- **SSE parsing**: `client.rs::read_sse` (async, chunk buffer) vs `lib.rs::sse_payload` + bounded `read_line` (blocking). Equivalent semantics.
- **Ollama HTTP**: twice, on different endpoints (`/api/chat` vs `/api/generate`).
- **Ollama install/spawn**: twice (three with v1), diverging on safety: Skriuw verifies the GitHub release SHA-256 digest and refuses non-loopback endpoints; Dora downloads from `ollama.com/download/*` with no checksum and accepts any endpoint string.
- **Model catalogs**: Dora curated tiers (`flagship`/`balanced`/`fast`) vs Skriuw priced catalog with context windows. Different model id sets for the same providers (e.g. Groq `llama-3.3-70b-versatile` in Dora, `openai/gpt-oss-120b` in Skriuw).
- **Error mapping**: Dora string copy; Skriuw `(category, recovery_action)` pairs; Betalingen one generic string. Skriuw's is the only machine-readable one.

---

## 8. Model representation

### 8.1 How models are represented today

| Concern | Dora | Skriuw | Betalingen |
| --- | --- | --- | --- |
| Identity | `AIProvider` enum + free-form model `String` per provider (`ai_model.<provider>` setting) | `{providerId: String, modelId: String}` everywhere (`AiCompletionRequest`, `settings.aiModel`, `RemoteAiModel`) | model string only; provider implicit (Groq) |
| Defaults | `AIProvider::default_model()` (e.g. Groq `llama-3.3-70b-versatile`, OpenAI `gpt-5.5`, Anthropic `claude-sonnet-4-6`, Ollama `llama3.2`) | none hard-coded; user must choose (`resolveAiModel` returns `null` until set); playground falls back to `fake` | `llama-3.3-70b-versatile` |
| Catalog | curated `&[(id, label, tier)]` per provider in `models.rs`, merged with live listing; heuristics classify unknown ids into tiers | `models.json` embedded, versioned (`version: 3`, `pricingAsOf`), fields `contextWindowTokens`, `inputPriceMicrosPerMtok`, `outputPriceMicrosPerMtok`; validated by `RemoteAiCatalog::validate` (bounds, duplicates, traversal-safe ids) | none |
| Discovery | `/v1/models` (OpenAI-compatible, Anthropic), Gemini `models?key=`, Ollama `/api/tags`; failures fall back to curated | `list_models()` per kind, only on explicit "Refresh models"; results persisted (`ai-models.json`) and merged so catalog entries win (`RemoteAiModelDirectory::merge`) | none |
| Local models | `OllamaCatalogEntry { name, label, description, installed, size_bytes }` (3 recommended + installed) | `LocalAiModel { name, size_bytes, modified_at, digest, parameter_size, quantization_level }` + curated TS catalog with `useCase` (`general`/`coding`/`reasoning`/`vision`) | n/a |
| Capabilities | tier string only; `is_openai_chat_model` excludes embedding/whisper/tts/etc. by name | context window, price, `supportsModelListing` per provider; Gemini listing filtered by `supportedGenerationMethods` contains `generateContent`; aggregator listing filtered by `type == openai/chat-completions` | none |
| Context window | not modeled | `contextWindowTokens` (catalog) / `context_window` / `context_length` / `inputTokenLimit` (fetched) | none |
| Max output | `max_tokens: Option<u32>` per request (UI passes 2048 for chat, `null` for SQL) | `max_output_bytes: u32` (bytes, not tokens) per request, per prompt default | `maxOutputTokens: 1800` |
| Reasoning/thinking | none (tier heuristics mention "thinking"/"reasoner" only for labeling) | none | none |
| Multimodal | none | `useCase: "vision"` label in the Ollama TS catalog, not enforced | none |
| Pricing | `usage.rs::pricing_for_model` substring heuristics in USD floats | integer micro-dollars per million tokens in catalog; `AiModelPricing` port; fetched models are unpriced | none |

### 8.2 Should a model be `provider + modelId` or carry capabilities?

Evidence points to a layered answer:

1. **Identity must be `provider_id + model_id`.** Skriuw already uses this pair as the request key, the settings value, the catalog key, and the history key, and validates both as bounded identifiers safe for URL segments. Dora is moving that way (`model_setting_key` per provider). A single dotted string (`google.gemini-2.5-flash`, as Skriuw v1 did) is worse: it must be parsed and the separator collides with model ids that contain dots or slashes (`openai/gpt-oss-120b`, `z-ai/glm-5.3-flash`).
2. **Capabilities must be a separate, optional, tri-state description**, not part of identity. Both apps let users type arbitrary model ids (`model-id-input.tsx`; Skriuw's fetched listings), so the SDK will frequently hold a model it knows nothing about. Capabilities therefore need `true | false | unknown`, sourced from (a) a shipped catalog, (b) provider listings, (c) provider-level defaults (every OpenAI-compatible endpoint supports streaming; `json_object` support is per model), (d) an application override.
3. **Which capabilities are actually needed by existing features:** streaming (all), JSON mode (Dora SQL), context window (Skriuw catalog uses it for display; autocomplete will need it for budgeting), local/private (Skriuw's disclosure model; Dora's "no data leaves" claim for Ollama), pricing (both usage surfaces). Tools, vision, audio, embeddings, reasoning are not needed by any current feature and should be reserved enum values, not modeled fields with behavior.
4. **Trade-off.** Richer capability metadata enables routing (section 20) and honest UI ("this model cannot do JSON schema"), but every declared capability is a maintenance liability and providers change model lists monthly (Dora's curated OpenAI list already contains ids that differ from Skriuw's Gemini list). The compromise is: identity is required and stable; `ModelInfo` is an optional record with explicit `source` (`catalog | listed | declared | unknown`) so consumers can decide how much to trust it; the router treats `unknown` as "try, and learn from the error category".

Recommended core shape (Rust, sketched):

```rust
pub struct ModelRef { pub provider_id: String, pub model_id: String }   // identity, validated like Skriuw

pub enum Capability { Streaming, JsonMode, JsonSchema, Tools, Vision, Audio, Embeddings, Reasoning }

pub struct ModelInfo {
    pub model: ModelRef,
    pub label: Option<String>,
    pub context_window_tokens: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub capabilities: BTreeMap<Capability, Support>,     // Support = Yes | No | Unknown
    pub locality: Locality,                               // Local | Remote { destination: String }
    pub pricing: Option<Pricing>,                         // micro-dollars per Mtok, as Skriuw
    pub source: ModelInfoSource,                          // Catalog | Listed | Declared
}
```

---

## 9. Request/response contract comparison

### 9.1 Representative definitions

Dora (`services/ai/mod.rs`, `client.rs`):

```rust
pub struct AIRequest { pub prompt: String, pub context: Option<SchemaContext>, pub connection_id: Option<Uuid>,
                       pub max_tokens: Option<u32>, pub prompt_mode: Option<String> }
pub struct AIResponse { pub content: String, pub suggested_queries: Option<Vec<String>>, pub tokens_used: Option<u32>, pub provider: String }
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AiStreamEvent { Token { text: String }, Final { content: String }, Error { message: String } }
#[async_trait] pub trait AiClient: Send + Sync {
    async fn complete(&self, request: AIRequest) -> Result<AIResponse, Error>;
    async fn complete_stream(&self, request: AIRequest, sender: UnboundedSender<AiStreamEvent>, cancel: Arc<AtomicBool>) -> Result<(), Error>;
}
```

Skriuw (`skriuw-domain/src/ai.rs`):

```rust
pub struct AiCompletionParameters { pub max_output_bytes: u32, pub timeout_ms: u32, pub retry_count: u8,
                                    pub temperature_millis: Option<u16>, pub top_p_millis: Option<u16> }
pub struct AiCompletionRequest { pub request_id: String, pub provider_id: String, pub model_id: String,
                                 pub system_prompt: String, pub user_prompt: String, pub parameters: AiCompletionParameters }
pub struct AiCompletionDelta { pub request_id: String, pub sequence: u32, pub text: String }
pub struct AiUsage { pub input_tokens: u64, pub output_tokens: u64 }
pub enum AiProviderErrorCategory { UnavailableProvider, MissingCredential, InvalidCredential, QuotaExhausted,
                                   RateLimited, RejectedRequest, TransportFailure, MalformedResponse, InternalFailure }
pub enum AiRecoveryAction { ConfigureCredential, Retry, ChooseDifferentModel, CheckProviderStatus, ReduceRequest, ContactProvider, None }
pub struct AiProviderError { pub provider_id: String, pub category: AiProviderErrorCategory, pub message: String, pub recovery_action: AiRecoveryAction }
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum AiCompletionEvent { Delta(AiCompletionDelta), Done { request_id, usage: Option<AiUsage> },
                             Cancelled { request_id }, Timeout { request_id }, ProviderError { request_id, error: AiProviderError } }
pub enum AiCompletionTerminal { Done { usage: Option<AiUsage> }, Cancelled, Timeout, ProviderError(AiProviderError) }
pub struct AiCancellation { cancelled: Arc<AtomicBool> }
pub trait AiEventSink: Send { fn send_delta(&mut self, delta: AiCompletionDelta) -> Result<(), AiSinkError>; }
pub trait AiComplete: Send + Sync {
    fn complete(&self, request: &AiCompletionRequest, cancellation: &AiCancellation, sink: &mut dyn AiEventSink) -> AiCompletionTerminal;
}
```

Skriuw renderer mirror (`app/src/contracts/ai.ts`) is a hand-written 1:1 TypeScript copy of the above with camelCase fields; Skriuw generates JSON Schema from Rust (`contracts/generated/ai-completion-request.schema.json`, `ai-completion-event.schema.json`) and gates drift.

Betalingen (`src/lib/ai/contracts.ts`):

```ts
ChatRequestSchema = z.object({ context: ScreenContextSchema, messages: z.array(z.object({ role: z.enum(['user','assistant']), content: z.string().min(1).max(6000) })).min(1).max(20) })
ChatEventSchema = z.discriminatedUnion('type', [ {type:'text', text}, {type:'done'}, {type:'error', message} ])
```

Betalingen internally relies on AI SDK v7 types: `streamText` options and `result.fullStream` parts (`text-delta`, `error`, `abort`).

### 9.2 Concept mapping

| Concept | Rust concept | TypeScript concept | Dora | Skriuw | Betalingen | Shared already? |
| --- | --- | --- | --- | --- | --- | --- |
| Completion request | struct | object / zod schema | `AIRequest` (prompt string + app context + mode) | `AiCompletionRequest` (system + user + params) | `ChatRequest` (messages + screen context) → AI SDK `streamText` args | No. Three shapes. Skriuw's is the only provider-neutral one; Betalingen's is the only one with a messages array. |
| Message | none / strings | `{role, content}` | Packed into one string (`USER:`/`ASSISTANT:`) | `system_prompt` + `user_prompt` | `messages[]` (user/assistant) + `system` | Conceptually yes (system/user/assistant); structurally no. |
| Completion result (non-stream) | struct | object | `AIResponse` | not exposed (stream only) | not exposed | No. |
| Stream delta | enum variant | union member | `Token { text }` (no ordering) | `Delta { requestId, sequence, text }` | `{type:'text', text}` | Same idea; only Skriuw carries request id and sequence. |
| Stream terminal | enum variants | union members | `Final { content }`, `Error { message }`; cancellation has no event | `Done { usage }`, `Cancelled`, `Timeout`, `ProviderError { error }` | `done`, `error { message }`; abort has no event | Skriuw is a superset. |
| Provider | enum + trait | provider factory | `AIProvider` enum, `AiClient` trait, `CompatSpec` | `provider_id: String`, `AiComplete` trait, `RemoteProviderKind` | `createGroq()` (AI SDK provider) | Trait shape equivalent between Rust repos; TS delegates to AI SDK. |
| Model | string | string | per-provider setting | `{providerId, modelId}` | env string | See section 8. |
| Error | enum | class / string | `Error::Any(anyhow)` → `{kind, detail}` | `AiProviderError { category, recovery_action, message }`, `LocalAiError { category, message }`, `AiValidationError` | generic Dutch message; AI SDK error hidden | Only Skriuw is typed. |
| Usage | struct | object | `tokens_used: Option<u32>` (total) + `AiUsageCapture` | `AiUsage { input_tokens, output_tokens }` + `AiRunTokens { source }` | none | Skriuw's is complete; Dora's needs input/output split. |
| Abort/cancel | `Arc<AtomicBool>` keyed by request id | `Channel` + local flag / `AbortSignal` | `ai_cancel_flags: DashMap<String, Arc<AtomicBool>>` | `AiCancellation` + `AiCompletionService.active` | `AbortController` → `AbortSignal.any` → AI SDK `abortSignal` | Same primitive in Rust; TS should standardize on `AbortSignal`. |
| Structured output | none typed | zod on the *request* only | `response_format: json_object` + lenient JSON parse | bullet-list parsing | none | No. |
| Tool | none | none | none | none | none | Absent everywhere. |
| Context | app struct | app object | `SchemaContext` inside the request | none in the request (pre-rendered into prompts) | `ScreenContext` inside the request, rendered server-side | Skriuw's approach (context is rendered into the prompt by the app before the seam) is the cleanest boundary. |
| Request id | `String` | `string` | UI-generated `${Date.now().toString(36)}-${random}` | `crypto.randomUUID()`, validated as identifier | none | Should be required and validated. |
| Timeout | client-level | `AbortSignal.timeout` | 60 s reqwest timeout | `timeout_ms` per request + deadline checks | 60 s `AbortSignal.timeout` | Per-request deadline belongs in the contract. |
| Origin / source tag | string | string | `usage.source` (`chat`, `sql_gen`, `key_test`, `complete`) | `origin` (`playground`, `editor:<id>`) validated at the command | none | Same concept, both apps want it. |

### 9.3 Where the contracts already converge

- Stream shape: delta + terminal is universal; the SDK event type should be Skriuw's `AiCompletionEvent` with `requestId` and `sequence` on deltas and four terminals. Dora's `Final { content }` (full accumulated text) is redundant with client accumulation and should not be carried; Betalingen's `done` maps to `Done { usage: None }`.
- Cancellation: an `AtomicBool`-backed handle observed inside the read loop, keyed by request id in a registry. Identical in both Rust apps.
- Prompt inputs: system + user (Skriuw) is a strict subset of messages (Betalingen). The SDK request should use `messages` so multi-turn chat (Dora D2, Betalingen B1) stops being string-packed, with a convenience constructor for the system+user case.
- Parameters: `temperature`, `top_p`, `max_output_*`, `timeout`, `retry_count`. Skriuw uses integer millis to keep contracts float-free; Dora and the AI SDK use floats. The SDK should keep a float-free wire representation (or fixed-point) if it wants byte-identical cross-language fixtures.

---
## 10. Streaming and cancellation comparison

### 10.1 Traces

**Dora** (`ai_complete_stream`):

```
React (use-ai-chat.ts / ai-cmd-k.tsx)
  requestId = newId(); channel = new Channel<AiStreamEvent>(); commands.aiCompleteStream(...)
  -> Tauri command (tokio async)
       cancel = Arc<AtomicBool>; state.ai_cancel_flags.insert(request_id, cancel)
       (tx, rx) = mpsc::unbounded_channel(); tokio::spawn(forward rx -> on_event.send)
       AIService::complete_stream(request, tx, cancel)
         -> build_client() -> Box<dyn AiClient>
         -> send_with_rotation(pool, ..., Some(&cancel), build)      // checks cancel before each attempt
         -> read_sse(label, response, &cancel, on_data)               // async bytes_stream; checks cancel per chunk
              sender.send(Token{text})  per delta
         -> Final{content} only when SseOutcome::Finished
       forward task ends when tx drops; command awaits it, removes the flag, records estimated usage
  abort: commands.aiAbortStream(requestId) -> flag.store(true); UI sets abortRef.cancelled and ignores later events
```

Terminal states: `Final` (success), `Error` (adapter-emitted), or *nothing* on cancel. The command's `Result` can also be `Err` (transport/key failure) which the UI shows as `result.error.detail`. Ordering is preserved by the single mpsc channel. Dropped consumer: `on_event.send` errors are ignored (`let _ =`), so the provider stream keeps running until it ends or the flag is set. Cleanup: flag removed after completion; if the UI never calls abort and navigates away, the request runs to completion.

**Skriuw** (`start_ai_completion`):

```
React (use-ai-run.ts)
  request.requestId = crypto.randomUUID(); consumer = createAiCompletionConsumer(requestId)
  startAiCompletion(request, origin, consumer.accept, signal)
    channel = new Channel<AiCompletionEvent>(); channel.onmessage gated by `active`
    signal.abort -> invoke cancel_ai_completion(requestId)
  -> Tauri command (sync fn; returns after spawn)
       LazyAiCompletion.start -> AiCompletionService::start
         validate; duplicate-id check; active.insert(request_id, AiCancellation)
         std::thread::spawn:
           provider.complete(&request, &cancellation, &mut CompletionServiceSink)
             adapter loop: check cancellation + deadline per line; bounded reads; validate delta;
             sink.send_delta -> channel.send(Delta) ; on send error -> cancellation.cancel()
           terminal = ...; active.remove; if cancelled && Done -> Cancelled
           channel.send(terminal.into_event(request_id)); recorder.record(run)
  cancel: AiCompletionService::cancel(request_id) -> cancellation.cancel(); idempotent; returns bool
```

Terminal states: exactly one of `done | cancelled | timeout | provider_error`, always delivered (even when the provider is unknown, the service publishes `provider_error` synchronously). Ordering: `sequence` on every delta; renderer rejects out-of-order. Dropped consumer: a failed channel send cancels the provider (`inspect_err(|_| self.cancellation.cancel())`); renderer unmount disposes the consumer *then* cancels, so late events cannot hit a new surface. Deadline: `timeout_ms` observed while reading (`Instant::now() >= deadline` in `stream_completion`) and as the HTTP timeout. Cleanup: gate `AbortController` aborts on opt-out/unmount; `shutdown()` cancels all.

**Betalingen** (`POST /ai/chat`):

```
Browser (use-data-chat.ts)
  AbortController; fetch(endpoint, {signal}); ReadableStream -> TextDecoderStream -> NDJSON lines
  -> Hono route
       signal = AbortSignal.any([c.req.raw.signal, controller.signal, AbortSignal.timeout(60_000)])
       stream(c, async output => { output.onAbort(() => controller.abort());
         result = streamText({..., abortSignal: signal});
         for await part of result.fullStream: text-delta -> write {type:'text'}; error|abort -> throw
         write {type:'done'} } catch -> write {type:'error'})
  stop(): controller.abort() in browser -> request aborted -> server onAbort -> provider abort
```

Terminal states: `done` or `error`; a client abort yields neither (the client detects `AbortError`). No sequence numbers (single ordered HTTP stream). Dropped consumer: Hono's `onAbort` propagates to the AI SDK abort signal, so the provider request is cancelled.

### 10.2 Comparison

| Dimension | Dora | Skriuw | Betalingen |
| --- | --- | --- | --- |
| Transport decode | SSE (`data:` lines) for cloud, NDJSON for Ollama; async chunk buffer | SSE / NDJSON via bounded `BufRead::read_line` | AI SDK internal; re-encoded as NDJSON |
| Async runtime | tokio | none in core (std threads); Tauri `spawn_blocking` for runtime ops | Node/Bun event loop |
| IPC | `tauri::ipc::Channel<AiStreamEvent>` | `tauri::ipc::Channel<AiCompletionEvent>` | HTTP chunked NDJSON |
| Cancellation primitive | `Arc<AtomicBool>` in `DashMap<String, _>` | `AiCancellation(Arc<AtomicBool>)` in `Mutex<HashMap<String, _>>` | `AbortController` / `AbortSignal.any` |
| Request id | client-generated, not validated | client-generated UUID, validated identifier, duplicate rejected | none |
| Terminal states | success / error / (silent cancel) | done / cancelled / timeout / provider_error | done / error / (silent abort) |
| Ordering guarantee | channel order | channel order + explicit `sequence` | HTTP order |
| Partial failure | `Error{message}` mid-stream; accumulated text kept in UI | `provider_error` terminal; preview kept, never applied | `error` event; UI keeps partial text and offers retry |
| Deadline | HTTP client timeout only | per-request `timeout_ms` + HTTP timeout | 60 s `AbortSignal.timeout` |
| Consumer dropped | ignored; provider runs on | cancels provider | cancels provider |
| Output bound | none | `max_output_bytes` + global 4 MiB, enforced pre-delivery | `maxOutputTokens` |
| UI batching | `createStreamBatcher` (rAF) | `useAiRun` (rAF) | per-line `setMessages` |
| Late-event protection | `abortRef.cancelled` flag | consumer keyed by request id + sequence; `activeRequestIdRef` | `controller.current` single-flight |

### 10.3 What to standardize

1. **Event contract** = Skriuw's: `delta { requestId, sequence, text }` and exactly one terminal among `done { usage? }`, `cancelled`, `timeout`, `provider_error { error }`. Add an optional `finish_reason` to `done` (the AI SDK and OpenAI expose it; nothing today consumes it, but the router will want `length` vs `stop`).
2. **Cancellation contract** = a clonable cancellation token observed inside every read loop, plus a per-runtime registry keyed by validated request id with idempotent `cancel(request_id) -> bool`. In TypeScript the token is `AbortSignal`. The SDK must never treat "stop forwarding" as cancellation (ADR-0033 wording, proven by Skriuw's `a_closed_consumer_cancels_the_request` test).
3. **Consumer-side ordering gate** (`createAiCompletionConsumer`) is small, framework-free, and should ship in the TypeScript core; Dora's `use-ai-chat.ts` reimplements a weaker version.
4. **Frame batching** (`createStreamBatcher` / `useAiRun` flush) is a React concern and belongs in the React helper package, not the core.
5. **Transport codecs** the core should own: SSE `data:` line decoder, NDJSON line decoder, and an NDJSON *encoder* for HTTP servers (Betalingen's route body). None of these require Hono, Tauri, or React.
6. **Sync push vs async pull** is the real design fork. Skriuw's push-into-sink model is what makes its cancellation and byte-accounting airtight; Dora's async model is what a future Rust HTTP server would want. Recommendation in section 16: keep the sink-based sync adapter trait as the canonical provider contract and offer an async facade (`impl Stream<Item = AiCompletionEvent>`) built on a channel for tokio consumers.

---

## 11. Credential and secret handling

### 11.1 Inventory

| Question | Dora | Skriuw v2 | Betalingen |
| --- | --- | --- | --- |
| Where keys are entered | Settings → AI Keys (`ai-keys-section.tsx`), any number per provider with labels; env vars | Settings → AI → provider card (`remote-ai-settings-ui.tsx`), one key per provider, after accepting the disclosure | Server environment only (`.env`, Vercel project vars) |
| Where keys are stored | SQLite `ai_api_keys.ciphertext` (AES-256-GCM, hex nonce+ct); master key in OS keyring (`dora_db_client` / `dora_encryption_key`) or `~/.config/dora/encryption.key` (mode 600) | OS keyring (`dev.skriuw.app` / `ai-provider:<id>`) **or** process memory for the session (`RemoteAiKeyTier::SessionOnly`); never on disk otherwise | process env |
| OS keyring usage | master key only; per-connection DB passwords also in keyring (`credentials.rs`) | the API key itself | none |
| Encryption | AES-GCM app-level | none needed (vault) | none |
| Environment variables | `{PREFIX}_API_KEY`, `{PREFIX}_API_KEY_1..10`, `{PREFIX}_MODEL`, merged at request time (`KeyPool`) | none for AI (only `SKRIUW_OLLAMA_ENDPOINT`) | `GROQ_API_KEY`, `GROQ_MODEL` |
| Browser storage | none (web demo is mock) | none (AI is desktop-only) | none (cookie session only) |
| Multiple keys per provider | yes; round-robin rotation on 429/401/403/5xx | no | no |
| Fallback between keys | yes (`send_with_rotation`) | no | no |
| Testing keys | `ai_keys_test` (saved), `ai_keys_test_raw` (unsaved), `ai_keys_test_provider` (configured pool); optional user prompt; records `last_status` and a `key_test` usage row | `verify_remote_ai_key(provider, model, key?)`: one `max_tokens: 1` request; result is accept/reject only; runs only from the explicit action | none (503 when missing) |
| Delete/replace | `ai_keys_delete`, `ai_keys_set_active` | `remove_remote_ai_key`; `revoke_remote_ai_provider` removes key + consent and cancels active requests | redeploy |
| Secrets across IPC | plaintext key crosses IPC once on add/test-raw; never returned (`AiApiKeyRecord` has no key field) | plaintext key crosses IPC once on save/verify; `RemoteAiProviderState` exposes only `keyTier` and consent versions | key never leaves server |
| Base URL configuration | Ollama endpoint editable (any URL); cloud URLs hard-coded | Ollama loopback-only override via env; cloud hard-coded per kind | none |
| Local providers without keys | Ollama, Mock (`env_key_prefix() == None`) | Ollama, fake | n/a |
| Consent / disclosure | none (docs say Ollama keeps data local) | versioned per-provider consent (`REMOTE_AI_DISCLOSURE_VERSION`), stored in `ai-consent.json` (mode 600, atomic rename); stale consent blocks requests | static disclosure text in UI |
| Legacy migration | `migrate_legacy_gemini_key` moves plaintext `gemini_api_key` setting into the encrypted table | consent document version check; unreadable consent = re-ask | n/a |
| Linux keyring UX | detect backend once (`credential_storage.rs`), show status, offer `pkexec` install of gnome-keyring/kwallet | detect vault state on demand over D-Bus (`vault-ok`, `vault-locked`, `vault-no-collection`, `vault-absent`, `vault-blocked` incl. Snap detection); refuse saving to an unusable vault; session-only fallback | n/a |
| Redaction | `Error::Any` messages may include response bodies (`trim_body` 300 chars) | `AiCredential` has redacted `Debug`, zeroed on drop; provider bodies never surface; keyring errors reported by category only | provider error bodies hidden (`onError` logs a fixed line) |

### 11.2 Security findings worth carrying into the SDK design

1. **Gemini key in the URL** (Dora `gemini.rs`: `format!("{url}?key={key}")`). Any URL logging, proxy, or error message that includes the request URL leaks the key. Skriuw uses the `x-goog-api-key` header. The SDK's Gemini adapter must use the header.
2. **Provider error bodies**: Dora forwards up to 300 characters of the provider body to the UI; Skriuw discards bodies entirely and maps status codes; Betalingen discards. Some providers echo the request or key prefix in error bodies. The SDK should keep bodies out of the user-facing error and optionally retain a bounded, redacted excerpt in a diagnostics field that apps opt into.
3. **Session-only tier** (Skriuw) is a genuinely useful concept for the SDK's credential port: a `CredentialSource` implementation that holds bytes in memory for the process lifetime, with zeroing. It is not platform-specific and can live in core as the reference implementation.
4. **Key pools with rotation** (Dora) are an account-management feature that only matters when a user owns several keys of one provider. It is legitimate to support as an optional `CredentialSource` decorator, but rotation-on-401 is questionable: an invalid key should surface as `invalid_credential`, not be silently skipped. Rotation should be restricted to `rate_limited`/`quota_exhausted`/5xx.

### 11.3 Why persistence cannot be in the universal core

| Platform | Available store | Constraints |
| --- | --- | --- |
| Tauri desktop (Rust) | OS keyring (Keychain, Credential Manager, Secret Service), encrypted file, app SQLite | D-Bus availability on Linux varies; Snap confinement; locked collections; needs user-visible state (Skriuw's five vault states) |
| Browser (Skriuw web build, Dora demo) | `localStorage`/IndexedDB only | Not a secret store; both apps currently refuse to run AI in the browser or run only a mock. Any BYOK-in-browser design is a product decision, not an SDK default. |
| Bun/Node server (Betalingen) | environment variables, secret managers | Keys are operator-owned; per-user BYOK would need a database and encryption at rest (Skriuw v1 did this with Prisma + `encryptApiKey`) |
| Serverless (Vercel) | env vars per deployment | No process lifetime for session-only; no OS keyring |

Conclusion: the core defines a **credential resolver port** (Skriuw's `AiCredentialSource::resolve(provider_id) -> Result<AiCredential, AiCredentialError>` is already the right shape: resolve at request time, return a zeroizing value, typed failure reasons including consent) and ships two implementations that have no platform dependency: environment-variable resolver and in-memory session resolver. Keyring-backed and encrypted-SQLite-backed resolvers are separate packages/crates. Consent versioning is a Skriuw product rule; the port should allow a resolver to refuse with a `consent_required`-class reason, but the core should not own disclosure text or versions.

---

## 12. Ollama and local-model architecture

### 12.1 Side-by-side

| Concern | Dora (`ollama_installer/*`, `services/ai/ollama.rs`) | Skriuw v2 (`crates/skriuw-ai-ollama`) | Skriuw v1 (frozen, `v1/apps/desktop/src-tauri/src/ai/*`) |
| --- | --- | --- | --- |
| Detection | `probe_server(endpoint)` → `GET /api/version` (3 s timeout); `managed_install_exists()` via `.dora-managed` marker; `managed_binary_path()` searches `bin/ollama`, root, then recursive depth 4 | `server_version()` (750 ms timeout); `available_binary()` = app-owned `ollama/bin/ollama` or `find_on_path`; `reap_managed_child()` distinguishes exited child (`Failed`) | as Dora (`.skriuw-managed`) |
| States | `OllamaStatus { running, endpoint, version, installed_count, managed, install_path, binary_ready }` | `LocalAiRuntimeState { NotInstalled, InstalledStopped, Starting, Running, Failed, Unsupported }` + `LocalAiStatus { state, version, endpoint, managed, detail }` | `OllamaInstallStatus` |
| Install source | `https://ollama.com/download/ollama-{linux-amd64.tar.zst, linux-arm64.tar.zst, darwin.tgz, windows-amd64.zip, windows-arm64.zip}` | GitHub Releases API `repos/ollama/ollama/releases/latest`, asset by name (Linux amd64/arm64 `.tar.zst`, macOS `.tgz`); Windows refused (`Unsupported`, links to official installer) | as Dora |
| Verification | none | asset `digest` must be `sha256:<64 hex>`; downloaded file hashed and compared before extraction; archive must contain an executable within depth 5 | none |
| Install directory | `dirs::data_local_dir()/dora/ollama` (+ `models/`) | `<app_data_dir>/ollama` (binary published to `ollama/bin/ollama` via pending-file + atomic rename; temp dir under app data) | `<vault root>/…` |
| Process spawn | `Command::new(binary).arg("serve").env("OLLAMA_HOST","127.0.0.1:11434").env("OLLAMA_MODELS", models_dir)`; `LD_LIBRARY_PATH`/`DYLD_LIBRARY_PATH`/`PATH` adjustments; macOS `xattr -dr com.apple.quarantine` | `Command::new(binary).arg("serve").env("OLLAMA_HOST", endpoint)`; no library-path handling; `set_executable` 0o700 | as Dora |
| Ownership | `static MANAGED_CHILD: OnceLock<Mutex<Option<Child>>>`; killed on `tauri::RunEvent::Exit` | `managed_child: Mutex<Option<Child>>` per runtime; `shutdown()` kills only a child it started; externally running server is "external and unmanaged" | static |
| Startup wait | `wait_for_server` 60 s, 500 ms poll | 15 s, 100 ms poll, aborts early if child exits | 30 s |
| Health check | `/api/version` | `/api/version` | `/api/version` |
| Model listing | `/api/tags` → names + sizes; catalog merges 3 recommended | `/api/tags` → validated `LocalAiModel` (name regex, sha256 digest, bounded text); TS curated catalog with use cases | `/api/tags` |
| Pull | `POST /api/pull` `{name, stream:true}`; 30 min client timeout; progress + ETA computed; `Done` on `status == success` | `POST /api/pull` `{model, stream:true}`; per-event 64 KiB bound, stream-wide 256 MiB bound; `LocalAiProgress::{Progress, Complete, Cancelled}`; duplicate operation id rejected | similar to Dora |
| Cancellation | `ollama_cancel_flags` / `ollama_install_cancel_flags` maps; cancelled pull returns `Ok(())` silently | shared `AiCancellation`; cancelled pull returns `LocalAiError::Cancelled` after emitting `Cancelled` progress | static `AtomicBool`s |
| Delete | `DELETE /api/delete` `{name}` | `DELETE /api/delete` `{model}` + name validation | yes |
| Endpoint policy | any string from settings (`ollama_endpoint`) | loopback HTTP only; non-loopback override refused and reported in `detail` | default |
| Generation API | `/api/chat` (messages) with `num_predict` | `/api/generate` (system + prompt) with `temperature`, `top_p`; usage from `prompt_eval_count`/`eval_count` | `/api/chat` |
| Error handling | `anyhow` strings; "Ollama isn't running. Start it with `ollama serve`." special case | `LocalAiErrorCategory { InvalidRequest, Unavailable, DownloadFailed, ChecksumMismatch, InstallFailed, ProcessFailed, MalformedResponse, Cancelled, Unsupported }` | strings |
| Persistence | settings `ollama_endpoint`, `ollama_model`; marker file | none beyond the binary; endpoint from env | settings.json |
| Tests | none | 12 unit tests with local TCP servers + 4 `#[ignore]` device tests (real download ~1.4 GB, real pull of `qwen2.5:0.5b`, mid-stream cancel) + native e2e | none |

### 12.2 Boundary decision

The evidence supports the audit's principle 4 and refines it:

- **Generic AI core**: nothing Ollama-specific. The core needs the `LocalAiRuntime`-style *port* (status/start/stop/install/list/pull/remove) and its neutral types (`LocalAiStatus`, `LocalAiModel`, `LocalAiProgress`, `LocalAiError`) only if a second local runtime is plausible (llama.cpp server, LM Studio, MLX). That is plausible enough to keep the port neutral, as Skriuw did, but it should live in the local-runtime crate, not in `ai-core`, to keep the core free of process/filesystem vocabulary.
- **Rust provider crate** (`ai-providers`, feature `ollama`): the HTTP generation adapter (`/api/chat`, since the SDK request is message-based; keep `num_predict`, `temperature`, `top_p`, `stop`), `/api/tags` listing, and `/api/version` probe. Depends on reqwest only.
- **Local runtime crate** (`ai-ollama-runtime`): install (GitHub release + SHA-256, as Skriuw), spawn/stop/reap, pull/delete with progress, endpoint policy, library-path handling from Dora (Skriuw lacks `LD_LIBRARY_PATH` handling; Dora's presence of it suggests real Linux breakage was hit), macOS quarantine removal from Dora, Windows policy (Skriuw refuses; Dora downloads a zip; the SDK should follow Skriuw and link to the installer unless a signed installer flow is added). Depends on `tar`, `zstd`, `flate2`, `sha2`, `tempfile`, process APIs. This is the dependency boundary that justifies a separate crate.
- **Tauri layer**: operation registry with duplicate-id rejection (Skriuw's `OperationRegistry`), channel forwarding, `spawn_blocking`. Generic enough for `ai-tauri`.
- **Application UI**: catalogs of recommended models (Dora's three SQL-flavored picks vs Skriuw's writing-flavored picks with use cases) are product content. Install/start buttons, disclosure copy, and "installed vs available" tiers are app UI.

Local runtime management should therefore be an **optional package** that apps opt into; a Bun server or a browser app never compiles it.

---

## 13. Prompt architecture

### 13.1 Inventory

| Repo | Prompt | Location | Form | Reusable instructions | App-specific instructions | Language handling | Structured-output prompting | Safety / destructive guards |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Dora | SQL generation system prompt | `prompts.rs::build_system_prompt` | Rust `String` builder; system + user | "Respond with ONLY a JSON object…", one-statement rule, few-shot examples | dialect from engine, schema block, LIMIT 100 rule, schema-qualified names | none (English) | JSON-only, exact keys `sql`, `explanation`, `warnings`; examples | "NEVER emit DROP, TRUNCATE, DELETE without WHERE…", destructive intent → `warnings` |
| Dora | Chat system prompt | `prompts.rs::build_chat_system_prompt` | as above | answer structure guidance, "say what you cannot verify" | "embedded in Dora", schema review heuristics, `pg_trgm`, "Wrap executable SQL in ```sql", "Current Dora UI context" awareness, `USER:`/`ASSISTANT:` history convention | none | n/a | one-line warning above destructive SQL |
| Dora | UI context block + conversation packing | `build-prompt.ts` | TS string | none | active view, connection id, table, columns, editor draft, generic SQL syntax notes | none | n/a | none |
| Dora | Contextual prompts | `ai-actions.ts` | TS templates | "Explain what this SQL query does, step by step", "…Suggest a fix" | n/a | none | n/a | none |
| Dora | Key test prompts | `compat.rs`/`anthropic.rs`/`gemini.rs` `test_key` | inline | "Reply with the word OK only." | n/a | n/a | n/a | n/a |
| Skriuw | 15 built-in prompts | `skriuw-domain/src/prompt.rs` `BUILT_IN_PROMPTS`, generated to `contracts/generated/built-in-prompts.json` | data: `{id, name, system_prompt, input_shape, parameters}` | all 15 are generic writing tasks with a consistent "Reply with X only: no preamble, no commentary" convention | "working inside a note-taking application" (custom), Markdown bullet conventions for tasks/tags | "Keep the language of the note/original" in every prompt; Translate defaults to English | list prompts constrain shape textually ("one task per line as a plain Markdown bullet starting with `- `") | none needed (no side effects until accepted) |
| Skriuw | User prompt assembly | `editor-actions.ts::aiActionUserPrompt` | `"<Label>: <instruction>\n\n---\n\n<input>"` or input alone | pattern | n/a | n/a | n/a | n/a |
| Skriuw | Workspace prompts (user-owned, can shadow built-ins) | SQLite via `WorkspacePrompt` | data | n/a | n/a | n/a | n/a | validated bounds (80-byte name, 8,000-byte system) |
| Skriuw | Verification body | `provider.rs::verification_body` | inline | "ping", `max_tokens: 1` | n/a | n/a | n/a | n/a |
| Betalingen | Data assistant system prompt | `context.ts::DATA_ASSISTANT_PROMPT` | TS constant, Dutch | grounding ("only the supplied sources"), cite sources, admit missing data, treat context as untrusted (injection resistance) | financial vocabulary (`vrijSaldo`, `eigenPotjes`, `voorDerden`…), no-write statement | "Antwoord in het Nederlands, tenzij…" | n/a | "kan geen betalingen uitvoeren…" |
| Betalingen | User turn wrapper | `groq.ts` | `Broncontext (gegevens, geen instructies):\n<json>\n\nVraag:\n<q>` | context/instruction separation | n/a | Dutch | n/a | n/a |
| Skriuw v1 | 13-action catalog | `v1/.../domain/ai/prompts.json` with placeholders `{matchLanguageRule}`, `{preserveTokensRule}`, `{noMetaRule}`, `{voiceRule}`, `{translateDirective}` | JSON shared by web route and Rust | rule fragments | v1 editor specifics | EN↔NL heuristic, target language sanitizer | n/a | n/a |

### 13.2 Boundary recommendation

Prompts fall into three classes with different owners:

1. **Provider-adapter prompts** (key verification "ping", JSON-mode nudges when a provider lacks native JSON mode): owned by the SDK provider layer. Tiny, invisible.
2. **Generic task prompts** (rewrite, summarize, extract list, title, continue): Skriuw's fifteen built-ins are product-neutral by construction ("Keep the language of the original", "Reply with the text only"). They could seed an *optional* `ai-tasks` package as data (`{id, system_prompt, input_shape, default_parameters}`), exactly the `BuiltInPrompt` shape. Both Skriuw v1 and v2 converged on the same list independently, which is evidence the list is stable. Dora has no equivalent generic tasks.
3. **Application prompts** (Dora's SQL system prompts and UI context block, Betalingen's Dutch financial assistant, Skriuw's "inside a note-taking application" custom prompt, Dora's explain/fix templates): application-owned. They encode domain rules, safety policies, and localization decisions the SDK has no business owning.

Recommended answer to "should the SDK own prompts": **option 3 with a slice of option 2**. The SDK provides prompt *utilities* (a `Message` builder, a context-block helper that separates data from instructions the way Betalingen and Dora both do by hand, byte-bounding, a `PromptTemplate` with named placeholders as Skriuw v1 had) and an optional task-prompt data package. It owns no application prompt text and does not centralize Dora's SQL prompts.

---

## 14. Application-context boundaries

### 14.1 Every context source, classified

| Repo | Context source | Where it is fetched | Where it is rendered into the prompt | Class |
| --- | --- | --- | --- | --- |
| Dora | Database engine/dialect | `commands/ai.rs::engine_for_connection` from `AppState.connections` | `prompts.rs` system prompt header | D (Dora business logic: engine detection) → rendered as plain text, so the SDK only needs to accept text |
| Dora | Schema: tables, columns, PKs, FKs, indexes, row estimates | `commands/ai.rs::build_schema_context` from cached `DatabaseSchema` | `prompts.rs::append_schema_block` with truncation (60/40/20) | C (application adapter) for the rendering; D for fetching. Truncation-by-budget is a **B** reusable helper candidate. |
| Dora | Selected connection id | UI prop | echoed in UI context block | D |
| Dora | Active view, selected table/columns, editor draft tail | `ai-assistant-panel.tsx` props, `editor-context.ts` external store | `build-prompt.ts::buildContextBlock` | D |
| Dora | Conversation history | zustand store | packed as `USER:`/`ASSISTANT:` text | **A** (generic: messages belong in the request, not in a string) |
| Dora | Query + error (Fix with AI) | console state | `ai-actions.ts` template | D |
| Skriuw | Selected text / caret prefix / whole note Markdown | `editor-action-apply.ts::actionInputText` (ProseMirror state) | user prompt verbatim (+ optional instruction) | D (extraction) → text |
| Skriuw | Instruction (tone, language, custom) | UI field | `aiActionUserPrompt` | B (generic "instruction + input" assembly) |
| Skriuw | Document structure, tasks, references, workspace | **not sent** (deliberate: preview must equal payload) | n/a | n/a |
| Skriuw | Model selection | workspace settings | request identity | A |
| Betalingen | Screen title/location/period | dashboard state | JSON context | D |
| Betalingen | Allowlisted GET sources re-fetched with caller auth | `context.ts::buildScreenContext` | JSON under "Broncontext (gegevens, geen instructies)" | D (allowlist, auth pass-through, IBAN masking are business rules) → text |
| Betalingen | Import metadata (`/meta`) | always added | JSON | D |
| Betalingen | Conversation history (last 18) | React state | `messages[]` | A |

### 14.2 Validation of the principle

The principle "the SDK should know how to accept context but not how to fetch it" holds in all three codebases, and the codebases already behave that way at the seam:

- Skriuw's request carries no context object at all; the app renders context into `user_prompt` before calling `start_ai_completion`. The seam is context-agnostic today.
- Betalingen renders context into the user message inside the route; `answerWithGroq` receives a string.
- Dora is the outlier: `AIRequest.context: Option<SchemaContext>` and `connection_id` make the *provider layer* aware of database schemas (`prompts::build` is called from every adapter). This is exactly the coupling the SDK must not inherit. Dora's migration (section 38) moves `append_schema_block` and `build_system_prompt` into Dora-owned code that produces messages.

What the SDK *should* offer for context, because all three apps hand-roll it:

- A `Message`/`ContentPart` model so history is not string-packed (A).
- A context-block helper: label + data with an explicit "data, not instructions" framing and a byte budget with truncation reporting ("… N more tables truncated", Dora; 100k char cap, Betalingen) (B).
- Optional token/byte estimation (`estimate_ai_tokens` at 4 bytes/token exists in Skriuw; `estimate_tokens_from_text` at 4 chars/token in Dora) so apps can budget context against `context_window_tokens` (B).

What it must not offer: schema introspection, note extraction, HTTP source allowlists, IBAN masking, dialect detection (all D).

---
## 15. Vercel AI SDK assessment

### 15.1 Exactly how the AI SDK is used today

| Repo | Package versions | APIs used | Concepts touched |
| --- | --- | --- | --- |
| Betalingen | `ai@7.0.93`, `@ai-sdk/groq@4.0.37`, `@ai-sdk/provider@4.0.10`, `@ai-sdk/provider-utils@5.0.36` (transitive `@ai-sdk/gateway` present but unused) | `createGroq({ apiKey, fetch })`, `groq(modelId)` → `LanguageModel`, `streamText({ model, system, messages, temperature, maxOutputTokens, maxRetries: 0, abortSignal, onError })`, `result.fullStream` parts `text-delta`, `error`, `abort` | LanguageModel, streamText, abort signal, retries (disabled), fullStream protocol. Not used: `generateText`, `generateObject`/`Output`, tools, provider registry, middleware, `usage`, `finishReason`, `toUIMessageStreamResponse`. |
| Skriuw v1 (frozen) | `ai@^6`, `@ai-sdk/google@^3`, `@ai-sdk/groq@^3` | `createGoogleGenerativeAI`, `createGroq`, `generateText`, `streamText`, usage metadata (`readUsageMetadata`), provider error classification against `APICallError`-shaped objects (`statusCode`, `responseBody`, `data.error`) | generateText, streamText, usage, error objects, per-action model routing (`ACTION_MODEL_DEFAULTS`) |
| Dora | none in product code; `@google/generative-ai` in release tooling | n/a | n/a |
| Skriuw v2 | none | n/a | n/a |

Two observations follow directly from the code:

1. The AI SDK's *own* public types never leave the module that calls it. Betalingen converts `fullStream` parts into its three-event NDJSON contract (`routes/ai.ts`) and never returns AI SDK objects to callers. Skriuw v1 wrapped errors into `AiRequestError { code, status }` and `AiRateLimitError`. Nobody has ever exposed `LanguageModel` or `StreamTextResult` across a module boundary in these repositories.
2. The two consumers are on incompatible majors already (v6 vs v7), and Betalingen's `@ai-sdk/groq@4` sits on `@ai-sdk/provider@4`. The provider specification (`LanguageModelV*`) changes with each major. Any public SDK contract that embeds AI SDK types would inherit that churn.

### 15.2 What is worth reusing

Valuable, because it would be expensive to rebuild in TypeScript and is exactly what Skriuw and Dora each hand-wrote in Rust:

- Provider packages (`@ai-sdk/openai`, `@ai-sdk/anthropic`, `@ai-sdk/google`, `@ai-sdk/groq`, `@ai-sdk/openai-compatible`, community `ollama-ai-provider`) with tested SSE parsing, request shaping, and error mapping to `APICallError` with `statusCode`, `responseHeaders`, `responseBody`, `isRetryable`.
- `streamText` / `generateText` with `abortSignal`, `maxRetries`, usage and `finishReason`.
- Structured output (`generateObject` / `Output.object` with zod schemas), including provider-native JSON schema mode where available and validation errors (`NoObjectGeneratedError`).
- Tool-calling plumbing, should it ever be needed (section 23).
- The `fetch` injection point on every provider factory, which is what makes Betalingen's tests deterministic.

Less valuable for this SDK: the UI message stream protocol and `useChat` (both apps have their own stores and stream shapes), the provider registry string syntax (`"openai:gpt-4o"` collides with the `provider_id + model_id` identity decided in section 8), middleware (nothing needs it yet), and the gateway.

### 15.3 Options

| Option | Assessment |
| --- | --- |
| A. Wrap heavily | Reimplements what the SDK already exposes well, and still couples to its majors. No evidence supports this. |
| B. Expose AI SDK models directly | Lowest effort for TypeScript-only consumers, but the public contract becomes `LanguageModel` and `StreamTextResult`, which (a) cannot be mirrored in Rust, (b) forces every consumer to upgrade in lockstep with AI SDK majors, (c) contradicts what both existing TS consumers actually did. |
| **C. Use internally, expose own contracts** | Matches Betalingen and Skriuw v1 in practice. The TypeScript core owns `CompletionRequest`, `CompletionEvent`, `ProviderError`, `ModelRef` (same shapes as Rust); an `ai-sdk` adapter turns a `LanguageModel` into a `Provider` and maps `fullStream` parts and `APICallError` into the shared event and error taxonomy. AI SDK becomes a dependency of one adapter module, upgradable independently. Provider breadth comes for free. |
| D. Avoid | Would require writing SSE parsers and provider request shaping for every provider in TypeScript, which Skriuw v1 chose not to do and which no repository has any code for. Only justified if bundle size in browsers were a hard constraint; today no repository calls providers from a browser. |

**Recommendation: C.** Ship one AI SDK-backed adapter as the default TypeScript provider implementation, keep an escape hatch (`fromLanguageModel(model)`) so consumers can plug in any AI SDK provider, and keep a second, dependency-free `fetch`-based OpenAI-compatible adapter as a fallback only if a consumer ever needs to avoid the AI SDK dependency. Pin the AI SDK as a regular dependency of the adapter, not a peer dependency of the core, so the core's version is not tied to AI SDK majors. Public types must not re-export `ai` or `@ai-sdk/*` types.

---

## 16. Rust architecture assessment

### 16.1 Reusable pieces, by origin

| Piece | Source | Verdict |
| --- | --- | --- |
| `AiComplete` trait, request/event/terminal/error types, bounds, validation | Skriuw `skriuw-domain/src/ai.rs` | Adopt as `ai-core` nearly verbatim. Generalize `system_prompt + user_prompt` to `messages`, add `response_format`, `stop`, `max_output_tokens`, `finish_reason`. |
| `AiCancellation`, `AiEventSink`, `AiSinkError` | Skriuw | Adopt. |
| `AiCompletionService` (registry, threads, first-terminal-wins, recording) + `AiCompletionChannel` | Skriuw `skriuw-ai` | Adopt as the core runtime. Rename to something neutral (`CompletionRuntime`). |
| `FakeAiProvider` / `FakeCompletionScript` | Skriuw `skriuw-ai` | Adopt as first-class. Extend with scripted `RateLimited`/`Retry-After` outcomes for router tests. |
| `AiCredentialSource`, `AiCredential` (zeroizing, redacted), `AiCredentialError` | Skriuw `skriuw-domain/src/remote_ai.rs` | Adopt as the core credential port. Drop consent-specific variants into a generic `Refused { reason }` so consent stays a Skriuw policy. |
| `RemoteProviderKind` + `OpenAiCompatible` table, `status_error`, SSE line parsing, `parse_event`, `parse_model_listing`, `verify_credential`, `list_models` | Skriuw `skriuw-ai-remote` | Adopt as the base of `ai-providers`. Convert the enum into a data-driven `OpenAiCompatibleSpec` so consumers can add providers without recompiling the crate (section 36). |
| Anthropic adapter | Dora `anthropic.rs` | Port the request/response shapes (messages API, `content_block_delta`, `x-api-key`, version header) onto the Skriuw adapter skeleton. |
| OpenAI, OpenRouter rows; `chat_temperature` defaults | Dora `compat.rs` | Add as spec rows. |
| `KeyPool` round-robin | Dora `key_pool.rs` | Optional `CredentialSource` decorator with rotation restricted to rate-limit/quota/5xx. |
| Curated catalogs + live-listing merge + tier heuristics | Dora `models.rs` | Merge into the catalog module: Skriuw's priced, validated catalog format plus Dora's merge strategy. Tier heuristics are UI sugar; keep them out of core. |
| Ollama generation adapter | Both | Rewrite on `/api/chat` (message-based) in `ai-providers` feature `ollama`, reusing Skriuw's bounded reader and Dora's `num_predict` mapping. |
| Ollama runtime manager | Skriuw (`skriuw-ai-ollama`) primarily; Dora `ollama_installer` for `LD_LIBRARY_PATH`/`DYLD_LIBRARY_PATH`, quarantine removal, ETA computation | `ai-ollama-runtime` crate. |
| Run recording port, token estimation, pricing port, cost computation | Skriuw `ai_history.rs` (`AiRunRecorder`, `AiModelPricing`, `estimate_ai_tokens`, `ai_run_cost_micros`) | Adopt the ports and the record shape in core; keep SQLite persistence in apps. Dora's `usage.rs` pricing table becomes catalog data. |
| AES-GCM key store, keyring probe, `pkexec` install plan | Dora `security.rs`, `credential_storage.rs` | Application-specific. Dora may keep it; the SDK's keyring credential crate should borrow Skriuw's vault-state detection instead. |
| `SchemaContext` and prompt builders | Dora | Stay in Dora. |
| `AIProvider` enum with `env_key_prefix`, `default_model` | Dora | Replace by provider descriptors (`ProviderDescriptor { id, label, destination, env_key_prefix, default_base_url, supports_model_listing }`). |

### 16.2 HTTP stack and concurrency: the central decision

- Dora: `reqwest 0.12` async, `tokio`, `async_trait`, `futures_util::StreamExt`, one HTTP client per adapter instance (60 s timeout), a shared `http::client()` elsewhere in the app.
- Skriuw: `reqwest 0.13` `blocking`, `std::thread` per request, `tauri::async_runtime::spawn_blocking` at the command boundary. The domain crate has no async at all by design ("asynchronous runtimes, network streams, Tauri channels, keyrings, and provider response types remain in adapters and shells", ADR-0033).

Recommendation: **the canonical provider trait stays synchronous and sink-based** (Skriuw's), for four evidence-based reasons:

1. Cancellation and byte accounting are enforced inside the read loop in Skriuw and proven by tests (`a_closed_consumer_cancels_the_request`, `rejects_stream_bytes_beyond_the_requested_output_limit`, `cancels_a_real_completion_mid_stream`). Dora's async version leaks running requests when the consumer drops.
2. Both Tauri apps run AI off the main runtime anyway (Dora spawns tasks; Skriuw spawns threads). Desktop concurrency is a handful of simultaneous completions, so a thread per request is not a cost.
3. Keeping `async` out of core keeps `tokio` out of core, which keeps the core usable from Skriuw's WASM build if a browser provider ever appears (the WASM build cannot spawn threads either, but it also does not run AI today).
4. The Rust code that would need porting is Dora's ~1,700 lines of adapters, most of which are superseded by Skriuw's adapters (Groq, DeepSeek, Moonshot/Kimi, Z.ai/GLM, DashScope/Qwen, Gemini). Only Anthropic, OpenAI and OpenRouter rows are new work.

Mitigation for the future Rust-server case: `ai-core` offers an async facade behind a `tokio` feature: `fn stream(runtime, request) -> impl Stream<Item = CompletionEvent>` implemented with a `tokio::sync::mpsc` channel fed by the sink, plus `cancel_on_drop` semantics. Adapters do not change.

### 16.3 Serde and contracts

Skriuw's domain types derive `schemars::JsonSchema` and use `#[serde(rename_all = "camelCase", deny_unknown_fields)]` with `tag = "type"` unions; Dora's derive `specta::Type` with snake_case fields. The SDK should standardize on schemars-derived JSON Schema (language-neutral, already has a generator and drift check) and camelCase wire fields with snake_case Rust fields. Dora's `tauri-specta` bindings can still be generated for the *app's* commands; the SDK types would appear in those bindings via `specta::Type` derives added behind a `specta` feature so Dora does not lose its typed `commands` object.

### 16.4 Crate boundaries derived from the code

| Crate | Justification from evidence | Must not contain |
| --- | --- | --- |
| `ai-core` | Skriuw's domain modules + `skriuw-ai` already form a coherent, dependency-light unit (serde, schemars, thiserror). Includes fake provider and the completion runtime because every consumer needs a registry with cancellation and every test needs the fake. | HTTP, keyring, Tauri, tokio (except behind a feature), any provider name, any prompt text, any storage. |
| `ai-providers` (features: `openai-compatible`, `anthropic`, `gemini`, `ollama`) | One `reqwest` dependency, one SSE/NDJSON reader, one status mapper shared by all remote adapters (Skriuw's `lib.rs` is already that shape with two dialects). Separate crates per provider would duplicate the HTTP plumbing and offer no dependency boundary; Skriuw's decision to keep six providers in one crate confirms this. | Credential storage, process management, catalogs of recommended models for a product. |
| `ai-ollama-runtime` | Distinct dependency set (`tar`, `zstd`, `flate2`, `sha2`, `tempfile`, process spawning) and distinct platform surface (install directories, `LD_LIBRARY_PATH`). Skriuw's `LocalAiRuntime` trait and types move here. | Completion logic (lives in `ai-providers::ollama`). |
| `ai-credentials` (features: `keyring`, `env`, `session`) | `keyring` + `dbus-secret-service` are heavy, platform-conditional dependencies (Skriuw's `app/src-tauri/Cargo.toml` shows the per-target matrix). `env` and `session` resolvers are trivial and could live in core; putting them here keeps core free of policy. | Consent text, disclosure versions, UI copy. |
| `ai-router` (optional) | No consumer needs it on day one (section 20). Separate so `ai-core` stays small and so the router's health state has a home. | Provider adapters. |
| `ai-tauri` (optional) | Two apps share the same three helpers: channel sink, request/operation registry, blocking-run wrapper. | Tauri commands (app-specific), Specta bindings export. |

Rejected: `ai-provider-openai`, `ai-provider-anthropic`, … as separate crates (micro-crates without a dependency boundary); `ai-types` split from `ai-core` (Skriuw keeps types and validation together and it works); an `ai-catalog` crate (catalog is data plus a validator; ship it as a module in `ai-providers` with the JSON embedded, as Skriuw does).

---

## 17. TypeScript architecture assessment

### 17.1 Consumers and their needs

| Consumer | Runtime | Needs from a TS SDK | Must not be forced to take |
| --- | --- | --- | --- |
| Betalingen server (`src/`) | Bun dev, Vercel Node prod; `node:*` only | provider call with abort + timeout, stable event contract, NDJSON encoder, injectable transport, error sanitization, server-side credential resolution | React, Tauri, Hono-specific APIs in the core |
| Betalingen browser island (`src/ui/assistant`) | browser bundle via esbuild | NDJSON decoder, ordered consumer, `AbortController` lifecycle, retry helper | AI SDK (must not ship in the browser bundle), Node APIs |
| Skriuw renderer | Vite/React 19, Tauri or browser | mirror types of the Rust contract, Tauri channel bridge, ordered consumer, rAF-batched run hook | AI SDK, Hono |
| Dora studio | Vite/React 18, Tauri or mock | same as Skriuw plus mock provider for the web demo | same |
| Future Next.js / serverless | Node/Edge | same as Betalingen server | Bun APIs |

### 17.2 Package boundaries

| Package | Contents | Dependencies | Why separate |
| --- | --- | --- | --- |
| `@ai/core` | contract types (mirroring `specs/`), zod schemas for validation at trust boundaries (Betalingen and Skriuw both validate at boundaries), `CompletionEvent` ordered consumer (`createCompletionConsumer`, from Skriuw), SSE and NDJSON decoders/encoders (Web Streams), `ProviderError` taxonomy, fake provider (scripted, deterministic), `Provider` interface, `CompletionRuntime` (registry keyed by request id with `AbortController` per request), `ModelRef`/`ModelInfo` | `zod` only (both TS consumers already use zod 4) | Runs in browser, Node, Bun, Workers; no framework |
| `@ai/ai-sdk` | `fromLanguageModel(model): Provider`, `openaiCompatible(spec, credentials)`, `anthropic(...)`, `google(...)`, mapping of `fullStream` parts and `APICallError` to core events/errors | `ai`, `@ai-sdk/*` | Isolates the AI SDK major; server-only in practice |
| `@ai/react` | `useCompletionRun` (Skriuw's `useAiRun` generalized: rAF batching, retry with new id, stale-request guard), `useStreamingText` | `react`, `@ai/core` | React optional |
| `@ai/tauri` | `startCompletion(request, {invoke, Channel})` bridge (Skriuw's `completion-bridge.ts` generalized to take the command names and the `invoke` function), operation bridge with progress channel (`ollama-bridge.ts::runProgressOperation`) | `@tauri-apps/api`, `@ai/core` | Tauri optional; both desktop apps need it |
| `@ai/hono` | **Not recommended initially.** Betalingen's route is 40 lines; the reusable parts (NDJSON encoding, abort wiring, error sanitization) belong in `@ai/core` as framework-free helpers (`toNdjsonStream(events, signal)`). A Hono package earns its place only when a second Hono consumer appears. | | |

### 17.3 Mock/fake provider in TypeScript

Dora's `mock-ai.ts` (web demo) and Skriuw's Rust `FakeAiProvider` both exist because a deterministic offline provider is needed for demos, tests, and playgrounds. The TypeScript core should ship `createFakeProvider(script)` with the same script shape as Rust (`tokens[]`, `tokenDelayMs`, `outcome: done|timeout|malformed|providerError`), so golden fixtures (section 18) drive both.

---

## 18. Cross-language strategy

### 18.1 Actual interoperability requirements

- Rust ↔ TypeScript wire boundary exists **only inside the Tauri apps**, over `tauri::ipc::Channel<AiCompletionEvent>` and command arguments. It is JSON, already schema-generated in Skriuw, hand-mirrored in Dora via Specta.
- No Rust code calls TypeScript AI code and no TypeScript code calls Rust AI code over HTTP. Betalingen has no Rust. Skriuw's Cloudflare Worker has no AI.
- The browser builds (Skriuw WASM, Dora demo) do not run AI providers.

So there is no requirement for a Rust↔TS *runtime* protocol beyond the Tauri channel event shape, and no requirement for one implementation to embed the other. Options B (Rust canonical, TS calls it) and C (TS canonical, Rust embeds it) solve a problem nobody has, and option C would additionally drag a JavaScript runtime into desktop apps that deliberately keep AI in Rust.

### 18.2 Option assessment

| Option | Fit |
| --- | --- |
| A. Two independent implementations with matching concepts | Already the de facto state (Skriuw Rust vs Betalingen TS). Cheap, but the concepts drift without a shared artifact; Dora's and Skriuw's event shapes already diverged. |
| B. Rust canonical, TS communicates with it | Only meaningful inside Tauri, where it is already true. Not applicable to Bun/Vercel. |
| C. TS canonical, Rust embeds/calls it | No consumer wants a JS runtime in the Rust process. Rejected. |
| **D. Shared language-neutral specification, native implementations** | Skriuw already generates JSON Schemas from Rust and gates the TS mirror. Extending that to the SDK is incremental: `specs/` holds the schemas, enums, and fixtures; Rust generates them (schemars) and TS validates against them (zod schemas checked against the JSON Schema in CI, or generated with `json-schema-to-typescript`). |
| E. Hybrid | D plus the pragmatic fact that inside Tauri the Rust side *is* canonical for the event stream. |

**Recommendation: D, with Rust as the schema generator** (it already has the tooling: `crates/xtask/src/main.rs` `write_schema::<T>()` with check mode). Shared artifacts:

| Artifact | Share? | Reason |
| --- | --- | --- |
| Request schema (`CompletionRequest`) | Yes | Same shape must cross the Tauri channel and be accepted by a Hono route. |
| Stream event schema (`CompletionEvent`) | Yes | The single most important contract; Betalingen's NDJSON, Skriuw's channel, Dora's channel all carry it. |
| Error codes (`ProviderErrorCategory`, `RecoveryAction`) | Yes | UI branches on them in both languages. |
| Capability enum, `ModelRef`, `ModelInfo` | Yes | Catalogs are data consumed by both. |
| Provider descriptors / OpenAI-compatible spec rows | Yes (data) | The same table (`id`, `label`, `destination`, `baseUrl`, `chatPath`, `modelsPath`, `streamUsageOption`, `envKeyPrefix`) drives the Rust adapter and, where the TS side uses `@ai-sdk/openai-compatible`, its configuration. |
| Model catalog (`models.json`) | Yes (data) | Skriuw already embeds it; Dora needs it. |
| Task definitions (built-in prompts) | Optional data | Skriuw generates `built-in-prompts.json` today. |
| Wire protocol | Only the event/request JSON; no transport spec beyond "NDJSON of events" and "Tauri channel of events" | Nothing else is needed. |
| Golden fixtures | Yes | `specs/fixtures/streams/*.json`: provider SSE inputs and expected event sequences; `errors/*.json`: status + body → expected category. Both implementations replay them. |
| Generated types | Rust → JSON Schema (generated); TS types generated from JSON Schema or hand-written with a schema conformance test (Skriuw does the latter). Generating TS is preferable once the schema set is stable. | |

Versioning of the spec is covered in section 33.

---

## 19. Tauri integration strategy

### 19.1 Current command surfaces

| | Dora | Skriuw |
| --- | --- | --- |
| Command count for AI | 34 (`ai_*`) | 22 |
| Naming | `ai_<verb>_<noun>` snake_case, exposed as `commands.aiVerbNoun` via tauri-specta | `<verb>_<noun>` snake_case (`start_ai_completion`, `pull_ollama_model`), hand-wrapped `invoke` |
| Stream setup | `on_event: tauri::ipc::Channel<AiStreamEvent>` argument; command awaits completion | `on_event: Channel<AiCompletionEvent>`; command returns immediately after spawning |
| Abort | `ai_abort_stream(request_id) -> bool` | `cancel_ai_completion(request_id) -> bool`, `cancel_ollama_operation(operation_id) -> bool` |
| Credential commands | `ai_keys_list/add/delete/set_active/test/test_provider/test_raw`, `ai_set_gemini_key` | `save_remote_ai_key(provider, key, tier)`, `remove_remote_ai_key`, `verify_remote_ai_key`, `accept_remote_ai_disclosure`, `revoke_remote_ai_provider`, `remote_ai_providers`, `credential_vault_state` |
| Model management | `ai_list_provider_models`, `ai_resolve_provider_model`, `ai_set_config`/`ai_get_config` | `remote_ai_catalogue`, `remote_ai_models`, `refresh_remote_ai_models`; selection is a workspace setting operation, not an AI command |
| Local runtime | `ai_get_ollama_status`, `ai_list_ollama_catalog`, `ai_pull_ollama_model`, `ai_cancel_ollama_pull`, `ai_delete_ollama_model`, `ai_list_ollama_models`, `ai_install_ollama`, `ai_cancel_ollama_install`, `ai_start_ollama`, `ai_configure_ollama` | `ollama_runtime_status`, `start_ollama_runtime`, `stop_ollama_runtime`, `install_ollama_runtime`, `list_ollama_models`, `pull_ollama_model`, `cancel_ollama_operation`, `delete_ollama_model` |
| Serialization | `specta::Type` + `serde`, snake_case wire | `serde` camelCase wire, `schemars` schemas |
| Error type over IPC | `Error` → `{kind, detail}` | `String`, `LocalAiError`, `AiProviderError` (typed) |
| App-specific logic inside commands | schema context building, engine detection, usage recording, key-test usage rows | origin validation, lazy service init, `spawn_blocking` |

### 19.2 Plugin versus helper crate

A Tauri plugin (`tauri-plugin-ai`) would fix command names, payload shapes, permissions, and the generated TypeScript API for every app. Evidence against it today:

- The two apps disagree on naming, error types, serialization casing, and on what belongs in a command (Dora builds prompts in commands; Skriuw forbids that).
- Dora's commands carry Dora-only inputs (`connection_id`) and Skriuw's carry Skriuw-only policy (`origin`, consent).
- Dora relies on tauri-specta generation, Skriuw on schemars + hand mirrors; a plugin would have to pick one.
- Skriuw's ADR-0033 requires lazy initialization and zero startup work; a plugin's `init()` would need to honor that in a generic way.

Evidence for a small helper crate (`ai-tauri`): three identical mechanisms appear in both apps.

1. Channel sink: Skriuw `TauriCompletionChannel(Channel<AiCompletionEvent>)` implementing `AiCompletionChannel`; Dora's forwarder task doing the same by hand.
2. Request/operation registry keyed by id with duplicate rejection and idempotent cancel: Skriuw `AiCompletionService.active` + `OperationRegistry`; Dora's three `DashMap<String, Arc<AtomicBool>>`.
3. Running blocking work off the async runtime: Skriuw `run()` wrapper around `spawn_blocking`; Dora spawns tasks.

Recommendation: ship `ai-tauri` with those three helpers plus an optional `specta` feature that derives `specta::Type` for the core types, and leave every command in the application. Revisit a plugin only when a third Tauri app appears and the two existing command surfaces have converged through migration.

---

## 20. Provider/model routing and fallback

### 20.1 What exists

| Mechanism | Where | Scope |
| --- | --- | --- |
| Key rotation inside one provider on transport error, 429, 401, 403, 5xx; at most one attempt per key | Dora `client.rs::send_with_rotation` | intra-provider, no backoff, ignores `Retry-After` |
| Per-action default model | Skriuw v1 `ACTION_MODEL_DEFAULTS` (frozen) | static task → model map, no fallback |
| Live model listing with fallback to curated | Dora `models.rs::list_*_models` | catalog only |
| Refuse before network on missing credential/consent, unknown model, invalid request | Skriuw `RemoteAiProvider::complete` | pre-flight, no alternative selected |
| Fake provider fallback in the playground when inventory fails to load | Skriuw `playground-model.ts` | UI convenience |
| `maxRetries: 0`, generic error message | Betalingen | none |

Nothing selects an alternative provider or model at runtime. Every app has one active model per request, chosen by the user (Skriuw, Dora) or the operator (Betalingen).

### 20.2 Error classes and fallback semantics, validated against the code

| HTTP / condition | Skriuw category today | Dora behavior today | Recommended SDK class | Retry same route? | Fallback to another route? | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| 429 | `RateLimited` (`Retry`) | rotate key | `rate_limited { retry_after? }` | once, after `Retry-After` if present and short (< a few seconds), and only before the first delta | yes, if a route with equivalent capabilities exists | Both apps show a "wait and retry" message; the router should also record a cool-down for the (provider, key) pair |
| 408 / connect or read timeout / `timeout_ms` exceeded | `Timeout` terminal or `TransportFailure` | request error, rotate on transport error | `timeout` | no automatic retry after deltas were streamed; before first delta, one retry is acceptable | yes | Skriuw's ADR forbids retry after output has streamed; keep that rule |
| 500 / 502 / 503 / 504 | `UnavailableProvider` (`CheckProviderStatus`) | rotate key (pointless: the same provider fails) | `provider_unavailable` | one retry with backoff before first delta | yes | Dora's key rotation on 5xx is a bug class the SDK should not copy |
| 402 | `QuotaExhausted` (`ContactProvider`) | generic body message | `quota_exceeded` | no | yes, and disable the route until credentials/config change | |
| 401 / 403 | `InvalidCredential` (`ConfigureCredential`) | rotate key | `invalid_credential` | no | only to a route with a *different* credential; never blindly | Dora's rotation on 401 hides a broken key; acceptable only as an explicit multi-key policy |
| 404 model | `UnavailableProvider` (`ChooseDifferentModel`) | "Model not found" copy | `model_unavailable` | no | yes, to another model | Distinct class recommended because the recovery differs from provider outage |
| 400 / 413 / 422 | `RejectedRequest` (`ReduceRequest`) | body-mentions-model heuristic else raw | `invalid_request` | no | no, unless the rejection is a known capability gap (e.g. JSON mode unsupported) that another route satisfies | Replaying an invalid request everywhere multiplies cost; Skriuw's `ReduceRequest` action is the right default |
| connection refused (Ollama) | `UnavailableProvider` | special copy | `local_runtime_unavailable` | no | yes, if a remote route is permitted by policy (privacy flag) | The router must respect a `local_only` requirement; falling back from Ollama to a cloud provider silently would violate both apps' privacy claims |
| Ollama model not pulled | 404 → `UnavailableProvider` (`ChooseDifferentModel`) | 404 copy | `local_model_missing` | no | to another local model, or prompt to pull | |
| malformed stream / oversize | `MalformedResponse` (`Retry`) | parse errors ignored per chunk | `malformed_response` | one retry before first delta | yes | |
| cancelled | `Cancelled` | silent | `cancelled` | never | never | |
| credential missing / consent missing | `MissingCredential` | `InvalidInput` "No X API keys configured" | `missing_credential` | no | yes, to a configured route | |
| structured output failed validation | n/a | n/a (lenient parse) | `structured_output_invalid` | one repair attempt (same model, with the validation error appended) | yes, to a model with native schema support | Section 22 |

### 20.3 Router design, architecture level only

Inputs: a **task requirement** (`{ needs: [streaming?, json_schema?, tools?, vision?], locality: any|local_only, max_latency_class: interactive|batch, budget: Option<cost_class> }`), the **configured routes** (ordered `[ModelRef]` per task or a default chain), the **model info** (capabilities, tri-state), and **per-route health** (`cool_down_until`, `disabled_reason`, recent failure counts).

Algorithm sketch: filter routes by hard requirements (locality, required capabilities not known to be `No`), skip routes in cool-down or disabled, take the first; on a fallback-eligible terminal *before the first delta*, mark health and try the next; after the first delta, never switch (the consumer already displayed text). Emit `attempts: [{model, outcome}]` in the result metadata so usage recording and UI can explain what happened.

State placement:

- **Desktop, local-first (Dora, Skriuw)**: in-memory health per process is sufficient and matches how both apps hold every other runtime state (Dora `AppState`, Skriuw `AppState`). Persisting cool-downs across restarts is unnecessary; persisting *disabled by quota/credential* is a settings concern the app already owns.
- **Server (Betalingen on Vercel)**: instances are ephemeral and concurrent; in-memory health is per instance and resets on cold start. That is acceptable for a rate-limit cool-down of seconds, and the alternative (shared state in Blob/KV/Redis) is not justified by one Groq key. The router should therefore accept a `HealthStore` port with an in-memory default and no shared implementation shipped.

Circuit breakers: a simple consecutive-failure threshold with a cool-down window per (provider, credential) is enough; full half-open breakers add state for no consumer that exists.

Concurrency: the router must be safe for concurrent requests (Dora spawns per request; Betalingen handles concurrent HTTP) and must not hold a lock across network I/O, mirroring Skriuw's `AiCompletionService` locking discipline.

Recommendation: implement as the optional `ai-router` crate / `@ai/core` `createRouter()` module, keep `ai-core` unaware of routing, and require the error taxonomy above in core so the router can be written against it. Do not ship a hard-coded provider fallback list.

---
## 21. Task-oriented API analysis

### 21.1 Real tasks in the three applications

| Task (as exercised) | Dora | Skriuw | Betalingen | Generic operation underneath |
| --- | --- | --- | --- | --- |
| Chat with grounding context | D2 | (playground only) | B1 | messages + app context → streamed text |
| Generate SQL from natural language | D1 | | | structured generation (`{sql, explanation, warnings}`) with app context |
| Explain SQL | D3 (chat prefill) | | | text generation with app prompt |
| Fix SQL from error | D4 (chat prefill) | | | text generation with app prompt |
| Rewrite / improve / fix grammar / shorten / lengthen / simplify / change tone / translate / custom | | S1–S9 | | text transformation of a selection with a system prompt |
| Continue writing | | S10 | | text continuation from a prefix |
| Summarize / outline | | S11, S12 | | text over a whole document |
| Suggest title | | S13 | | short text |
| Extract tasks / suggest tags | | S14, S15 | | list extraction (currently text-parsed) |
| Key verification | D10 | S20 | | minimal completion |
| Model listing | D8 | S20 | | provider capability, not a task |

No transcription, embeddings, image, or tool tasks exist in current code (Skriuw v1 embeddings are frozen).

### 21.2 Model-oriented or task-oriented?

Evidence:

- Skriuw already separates the two layers cleanly: `AiComplete` is model-oriented (request in, events out); "tasks" are *data* (`AI_EDITOR_ACTIONS` referencing `BUILT_IN_PROMPTS`) plus per-outcome parsers (`action-plan.ts`). The seam never grew a `rewrite()` method, and ADR-0033 explicitly wants "one narrow completion seam that every AI feature calls".
- Dora's `prompt_mode` string is a task selector pushed *into* the provider layer, and it is the source of the coupling problems (every adapter calls `prompts::build`). That is the anti-pattern a task-oriented core API would institutionalize.
- Betalingen has one task and no need for a task API.

Recommendation: **both layers, with a strict dependency direction**: task helpers are built *on* the model-oriented core and live in an optional package; the core never knows task names.

### 21.3 Proposed task taxonomy and placement

| Task | Definition | Placement | Rationale |
| --- | --- | --- | --- |
| `complete` / `stream` | messages → text (events) | core | universal |
| `generate_object` | messages + schema → validated value | core | Dora needs it now; the fallback strategy must be provider-aware |
| `transform_text` (rewrite family: rewrite, improve, fix-grammar, shorten, lengthen, simplify, change-tone, translate, custom) | selection + optional instruction → text | optional tasks package as prompt data + one helper | Skriuw v1 and v2 converged on this list; prompts are product-neutral |
| `continue_text` | prefix → continuation | optional tasks package | Skriuw S10; also the shape of inline autocomplete |
| `summarize`, `outline`, `title` | document → text | optional tasks package | Skriuw |
| `extract_list` (tasks, tags, keywords) | document → `string[]` | optional tasks package, implemented on `generate_object` with a text-list fallback (Skriuw's parser) | Skriuw S14/S15; generic |
| `chat` | history + context → stream | core convenience (it is `stream` with messages) | Dora, Betalingen |
| `generate_sql`, `explain_sql`, `fix_sql` | | **application (Dora)** | schema context, dialect rules, safety rules, and result handling are Dora's domain |
| `verify_credential` | | core provider contract (`verify`) | both desktop apps need it; Betalingen returns 503 instead |
| `transcribe`, `embed` | | not built | no consumer |

The tasks package must be thin: a prompt catalog in the `BuiltInPrompt` shape, a `runTask(runtime, taskId, input, options)` helper that builds the request, and the list parser. Anything more becomes the framework the brief warns against.

---

## 22. Structured output

### 22.1 Where determinism matters today

| Repo | Need | Mechanism | Validation | Repair | Failure behavior |
| --- | --- | --- | --- | --- | --- |
| Dora D1 | `{sql, explanation, warnings}` | prompt ("Respond with ONLY a JSON object") + `response_format: json_object` on OpenAI-compatible providers only; Anthropic/Gemini/Ollama prompt-only; few-shot examples | none (`String(parsed.sql ?? '')`) | strip code fences, retry parse | falls back to treating the entire text as SQL with warning "Response was not valid JSON." |
| Dora D2 | optional JSON in chat | `parseAssistantSqlResponse` recognizes `{sql|query, explanation|reasoning, warnings, example}` if the whole message is a JSON object | none | fence stripping | renders as Markdown |
| Skriuw S13 | title | prompt constraints (≤ 8 words, no quotes) | trimmed, non-empty | none | "finished without producing any text" |
| Skriuw S14/S15 | list of tasks/tags | prompt demands `- ` bullets | `parseTaskPlan`/`parseTagPlan`: bullet/checkbox stripping, dedupe, ≤ 50 items, byte bounds, rejects prose | none | actionable message: "The model did not return a usable list of tasks. Try again, or pick a different model." |
| Betalingen | none | | | | |

No repository uses zod/serde/JSON Schema to validate model output. Skriuw's `deny_unknown_fields` serde types validate *provider* responses, not generated content.

### 22.2 Provider support observed in the code

- OpenAI-compatible: `response_format: {type: "json_object"}` used by Dora; `json_schema` type not used anywhere. Support varies per provider and model (Dora sends it to Groq, DeepSeek, Kimi, GLM, Qwen, OpenRouter uniformly; failures would surface as 400 `RejectedRequest`).
- Anthropic: no JSON mode in Dora; tool-use-based structured output not used.
- Gemini: `responseMimeType`/`responseSchema` not used.
- Ollama: `format: "json"` / schema not used.
- AI SDK (TypeScript): `generateObject`/`Output.object` exist and handle per-provider modes; unused by Betalingen.

### 22.3 Recommended abstraction

Request-level:

```rust
pub enum ResponseFormat {
    Text,
    Json,                                  // "some JSON object", no schema
    JsonSchema { name: String, schema: serde_json::Value, strict: bool },
}
```

Provider adapters declare per-model support via `Capability::JsonMode` / `Capability::JsonSchema` (tri-state). The runtime applies a **strategy ladder**, configurable per call:

1. `Native` — provider-native schema mode (OpenAI `json_schema`, Gemini `responseSchema`, Ollama `format` with schema, Anthropic via a single forced tool). Requires `JsonSchema: Yes`.
2. `JsonMode` — `json_object` plus the schema rendered into the system prompt. Requires `JsonMode: Yes` or `Unknown`.
3. `PromptOnly` — schema in the prompt, no provider hint (Dora's Anthropic/Gemini/Ollama path).

Post-processing is uniform: strip fences (Dora), parse, validate against the schema (Rust: `jsonschema` crate or typed `serde` deserialization; TS: zod), on failure emit `structured_output_invalid { issues }`. Optional `repair: { attempts: 1 }` re-asks the same model with the issues appended, only when no deltas were streamed to the consumer (structured calls should default to non-streaming for this reason; Dora streams the JSON purely as a progress indicator, which the SDK can keep as an *option* that disables repair).

Graceful degradation: a provider whose declared capability is `No` for the requested format fails fast with `unsupported_capability`, which the router (if present) treats as fallback-eligible; a provider with `Unknown` tries and learns from a 400.

Skriuw's list extraction can stay text-based (it works and its prompts are tuned for small local models where JSON mode is unreliable) or move to `generate_object` with a `string[]` schema and the existing parser as `PromptOnly` fallback. That choice stays in the tasks package, not core.

---

## 23. Tool/function calling

**None exists.** Searched terms `tool`, `function_call`, `tool_choice`, `tools:` across all three repositories' AI code: no provider adapter sends tool definitions, no request type carries them, no UI renders tool calls. Dora's `docs/specs/03-mcp-server.md` is about Dora *serving* tools to external coding agents over MCP (`rmcp`), which is unrelated to the SDK's provider contracts. Betalingen's prompt explicitly states the assistant "has no write tools".

Should the core accommodate tools now? Cheaply, yes; behaviorally, no:

- Use `messages: Vec<Message>` with `content: Vec<ContentPart>` where `ContentPart` is an enum that today has only `Text { text }`. Adding `ToolCall`/`ToolResult` variants later is non-breaking for serde with `#[serde(tag = "type")]` and `#[non_exhaustive]`, and the same is true for the TS discriminated union.
- Reserve `Capability::Tools` and a `tools: Option<Vec<ToolDefinition>>` request field behind a feature or leave it out entirely until a consumer exists. Given `deny_unknown_fields` on Skriuw's request types, adding a field later is a schema version bump either way; reserving it now costs nothing but should not imply an implementation.
- Do **not** build a tool execution loop, an agent runner, or MCP client support. No product demonstrates the need.

---

## 24. Inline autocomplete readiness

Skriuw has no inline ghost-text completion. The closest existing things are `continue` (S10: caret prefix → continuation, one-shot, reviewed in place) and the Dutch/English handling in every built-in prompt ("Keep the language of the original"). Searched `debounce|ghost|inline completion|autocomplete` in `app/src/features/ai` and `app/src/features/editor`: hits are only keywords in `editor-actions.ts` ("autocomplete" as a search keyword for `continue`).

Requirements versus the proposed architecture:

| Requirement | Supported by proposed core? | Where it belongs |
| --- | --- | --- |
| Low latency | Yes, if the request allows tiny `max_output_tokens`, `stop` sequences, and non-streaming mode. Skriuw's request lacks `stop` and uses bytes for output; add both. Route to a fast/local model via router task profile `interactive`. | core request fields; router profile |
| Debouncing | Must stay outside the core (editor plugin decides when to ask). | ProseMirror/CodeMirror integration |
| Cancellation/supersession | Yes: request ids + idempotent cancel + consumer gate already handle "new keystroke cancels old request" (Skriuw `useAiRun.fire` disposes the previous handle). A `supersede(previous_id)` convenience in the React hook is enough. | core + `@ai/react` |
| Very small outputs | `max_output_tokens: 32`, `max_output_bytes` guard. | core |
| Streaming or not | Both supported; non-streaming avoids per-token IPC for 20-token completions. | core |
| Dutch and English | Prompt-level; the `continue` prompt already says match the language. Language detection, if wanted, is app logic. | tasks package / app |
| Context window around cursor | App extracts prefix/suffix (Skriuw `actionInputText` for `caret` already extracts the prefix); a fill-in-the-middle field (`suffix`) should be an optional request extension since some providers (OpenAI completions, Ollama `suffix`) support it natively. | app extraction; optional core field |
| Provider switching | Router or explicit `ModelRef` per call. | core |
| Privacy/local option | `locality: local_only` requirement respected by router; Ollama adapter. | core + router |
| Rate of requests | A per-origin concurrency limit ("one in flight per editor") lives in the editor integration; the core's registry already rejects duplicate ids. | integration |

Blocking concerns: none, provided (1) the request type gains `stop`, `max_output_tokens`, optional `suffix`; (2) the runtime keeps per-request thread/task spawn cheap (Skriuw spawns a named thread per request, which is fine at a few requests per second); (3) the Tauri channel is not required for non-streaming calls (a plain command returning the text is cheaper for 20-token outputs).

---

## 25. Error taxonomy

### 25.1 What exists

| Category (proposed) | Skriuw today | Dora today | Betalingen today | New? |
| --- | --- | --- | --- | --- |
| `missing_credential` | `MissingCredential` | `InvalidInput("No X API keys configured…")` | 503 "nog niet ingesteld" | exists (Skriuw) |
| `invalid_credential` | `InvalidCredential` (401/403) | string "Invalid API key for X" | hidden | exists |
| `authorization` (403 distinct from 401) | folded into `InvalidCredential` | folded | hidden | new, optional; providers rarely distinguish reliably, keep folded but retain `status` in source |
| `invalid_request` | `RejectedRequest` (400/413/422/default) | raw status + body | hidden | exists |
| `model_unavailable` | folded into `UnavailableProvider` + `ChooseDifferentModel` | string "Model 'x' not found" (404 or body heuristic) | n/a | new split |
| `unsupported_capability` | n/a | n/a | n/a | new (structured output/tools) |
| `rate_limited` | `RateLimited` | string "Rate limit reached" | hidden | exists; add `retry_after` |
| `quota_exceeded` | `QuotaExhausted` (402) | raw | hidden | exists |
| `timeout` | `Timeout` terminal | reqwest timeout string | `AbortSignal.timeout` → generic | exists |
| `network` | `TransportFailure` (connect/other) | "request failed: …" | generic | exists |
| `provider_unavailable` | `UnavailableProvider` (5xx) | raw | generic | exists |
| `cancelled` | `Cancelled` terminal | none (silent) | `AbortError` client-side | exists in Skriuw only |
| `malformed_response` | `MalformedResponse` | parse errors partly swallowed | generic | exists |
| `structured_output_invalid` | n/a | UI warning string | n/a | new |
| `local_runtime_unavailable` | `LocalAiError::Unavailable` / `UnavailableProvider` "Ollama is not reachable" | "Ollama isn't running. Start it with `ollama serve`." | n/a | exists as two types in Skriuw; unify as a completion error category |
| `local_model_missing` | 404 → `UnavailableProvider` + `ChooseDifferentModel` | 404 copy | n/a | new split |
| `internal` | `InternalFailure` | `Internal` | generic | exists |
| validation of *our* input | `AiValidationError` (not a provider error) | `InvalidInput` | zod 400 | exists; keep separate from provider errors |

Skriuw's `recovery_action` enum (`ConfigureCredential`, `Retry`, `ChooseDifferentModel`, `CheckProviderStatus`, `ReduceRequest`, `ContactProvider`, `None`) is worth keeping: it is what the UI actually needs, and it decouples copy from category.

### 25.2 Retaining the source error without leaking it

```rust
pub struct ProviderError {
    pub provider_id: String,
    pub category: ErrorCategory,
    pub recovery: RecoveryAction,
    pub message: String,                       // bounded, safe, user-presentable (Skriuw's bounded_message)
    pub retry_after: Option<Duration>,         // from Retry-After / provider hints
    pub source: Option<ErrorSource>,           // diagnostics only; never serialized to UI by default
}
pub struct ErrorSource { pub http_status: Option<u16>, pub provider_code: Option<String>,
                         pub body_excerpt: Option<String> /* bounded, redacted */, pub request_id: Option<String> }
```

Rules taken from the code: `message` is bounded and control-character-stripped (Skriuw `bounded_message`); `body_excerpt` is opt-in per app (Dora wants detail in its Settings test results; Skriuw forbids it in the renderer), redacted of anything resembling a key (`sk-`, `AIza`, bearer prefixes) before storage; the wire type for IPC serializes `source` only when the app enables a `diagnostics` flag. `LocalAiError` remains a separate type for runtime-management operations (install/pull), as in Skriuw, because those are not completion errors.

---

## 26. Usage and telemetry

### 26.1 Current state

| Metric | Dora | Skriuw | Betalingen |
| --- | --- | --- | --- |
| Token usage | non-stream: provider `total_tokens` (OpenAI-compatible), `input+output` (Anthropic), `totalTokenCount` (Gemini), `eval_count` (Ollama); stream: **estimated** chars/4 with 35/65 split (`normalize_token_counts`) | provider `input_tokens`/`output_tokens` when reported (`stream_options.include_usage`, Gemini `usageMetadata`, Ollama eval counts); otherwise byte/4 estimate flagged `AiTokenSource::Estimated`; cancelled/failed runs always estimated | none |
| Request counts | `ai_usage` rows; `source` in `{complete, sql_gen, chat, key_test}` | `ai_run_history` rows with `origin` (`playground`, `editor:<id>`) | none |
| Latency | none | `duration_ms` per run | none |
| Provider/model history | per row | per row + `state` + `error_category` | none |
| Cost | `estimate_cost_usd` from a hard-coded substring pricing table (floats) | `cost_micros` from catalog `AiModelPricing` (integer micro-dollars); unpriced when model not in catalog; local = `None` | none |
| Prompt retention | none | optional (`retain_prompts`), redacted at write time | none (in-memory chat) |
| Retention policy | none | `max_runs` (500 default, ≤ 10,000), `max_age_days` (90, ≤ 3,650) | n/a |
| Aggregates | totals + per provider + recent 25 | per day × provider × model, `estimated` sticky flag | n/a |
| Diagnostics/logging | `tracing::warn!` on stream errors; `log::debug!` model list failures | `eprintln!` for provider construction failures; no AI telemetry by policy (ADR-0033) | `console.warn('Groq generation failed; response details omitted.')` |
| Uploads | none | none, by ADR | none |

### 26.2 What the SDK should emit

Both desktop apps record after terminalization, off the delivery path, into their own SQLite, with an app-chosen origin tag. The SDK should therefore:

- Return a `CompletionOutcome` with every completion (stream terminal `done` carries `usage`; the runtime attaches the rest):

```rust
pub struct CompletionOutcome {
    pub request_id: String, pub provider_id: String, pub model_id: String,
    pub usage: Option<Usage { input_tokens, output_tokens, source: Reported | Estimated }>,
    pub duration: Duration, pub finish_reason: Option<FinishReason>,
    pub attempts: Vec<Attempt { provider_id, model_id, terminal: TerminalKind, duration }>,  // > 1 only with router
    pub terminal: TerminalKind,                          // Done | Cancelled | Timeout | Error(category)
}
```

- Expose a `RunRecorder` port (Skriuw's `AiRunRecorder::record(&AiRunRecord)`) that the runtime calls once per request after publishing the terminal, and a `Pricing` port; ship a no-op recorder, an in-memory recorder for tests, and the byte-based estimator. Ship no database.
- Never log prompts, deltas, or credentials from inside the SDK; provide a `tracing` feature that emits only category-level events, matching Skriuw's policy and Dora's current `tracing` usage.

Dora's pricing table and Skriuw's catalog pricing should converge on the catalog data format (integer micro-dollars, `pricingAsOf`), with Dora's per-provider aggregate view built on the same record shape.

---

## 27. Testing strategy

### 27.1 Current AI tests

| Repo | Type | Location | Notes |
| --- | --- | --- | --- |
| Dora | Rust unit | `services/ai/mod.rs` (3: article, setting keys, provider round-trip), `key_pool.rs` (3), `prompts.rs` (1: chat prompt includes indexes) | No adapter, SSE, cancellation, or error-mapping tests. No fake HTTP server. |
| Dora | TS unit | `__tests__/ai-actions.test.ts`, `__tests__/model-id-input.test.ts`, `features/ai-assistant/assistant-response-parser.test.ts`, `sql-code-utils.test.ts` | Prompt templates, filtering, JSON parsing, SQL block handling |
| Dora | Live | `tools/scripts/verify-providers.ts` | For Turso/Neon integrations, not AI |
| Skriuw | Rust unit (domain) | `skriuw-domain/src/{ai,remote_ai,local_ai,prompt,ai_history}.rs` | Validation bounds, serialization tags, credential redaction, catalog validation, consent, pricing, redaction, retention |
| Skriuw | Rust unit (runtime) | `skriuw-ai/src/lib.rs` (16) | Ordered tokens, mid-stream abort, timeout before late token, malformed output, output bound, closed surface cancels, every built-in prompt runs on the fake, duplicate ids, recording with reported vs estimated usage |
| Skriuw | Rust adapter tests with local TCP servers | `skriuw-ai-remote/src/lib.rs` (23), `skriuw-ai-ollama/src/lib.rs` (12) | SSE framing, per-provider bodies (`stream_options` gating), Gemini and Groq streams, no socket without credential/consent, status mapping, malformed/oversized streams, closed consumer cancels, destination pinning, model listing parsing, checksum verification, archive extraction depth |
| Skriuw | Rust `#[ignore]` device tests | `skriuw-ai-ollama` (4) | real download (~1.4 GB), real start/list, real pull + complete + remove, real mid-stream cancel |
| Skriuw | Rust shell tests | `app/src-tauri/src/ai.rs` (2), `ai_credentials.rs` tests | lazy init invariants; credential gate through the seam without network |
| Skriuw | TS unit | `app/__tests__/features/ai/*` (16 files) | consumer ordering, streaming state machine via the same consumer the bridge uses, apply refusal, plans, model options/selection, prompt library, playground |
| Skriuw | Native e2e | `app/e2e/run-native-ai.mjs` | WebDriver against the debug binary; pulls `all-minilm`; results JSON |
| Betalingen | bun test | `src/lib/ai/ai.test.ts` (6) | injected fake `fetch` returning SSE; auth/503; allowlist; IBAN masking and size cap; streamed events; provider failure sanitization; system-message rejection and body limit |

### 27.2 Recommended SDK test strategy

| Suite | Content | Network/paid? |
| --- | --- | --- |
| Deterministic fake provider | Port `FakeAiProvider` to both languages with identical script semantics; every runtime, router, React, and Tauri test uses it. | no |
| Provider conformance suite | A trait-level suite run against every adapter using local fixture servers (Skriuw's `serve_once` pattern; in TS an injected `fetch`): streams ordered deltas; terminalizes exactly once; honors cancellation before and during read; enforces output bounds; maps each status code to the expected category; never opens a socket without a credential; never puts the credential in a URL; parses usage; handles `[DONE]`, missing terminal, oversized event. | no |
| Stream fixture tests | `specs/fixtures/streams/<provider>/<case>.sse` → expected `events.json`; replayed by Rust and TS. Seeds: Skriuw's inline test bodies, Betalingen's `providerResponse()`. | no |
| Cancellation tests | fake provider with delays; consumer close; abort signal; supersession. | no |
| Error mapping tests | table-driven status/body → category/recovery (`maps_provider_status_codes_onto_distinct_recoverable_states` generalized). | no |
| Fallback/router tests | scripted sequences (429 with Retry-After → next route; 401 → no fallback; post-first-delta failure → no fallback). | no |
| Structured output tests | valid, fenced, invalid JSON; repair path; capability `No` short-circuit. | no |
| Serialization contract tests | JSON Schema generated from Rust equals committed `specs/`; TS types/zod validate all fixtures; golden JSON round-trips in both languages (Skriuw's `xtask` check mode is the template). | no |
| Local runtime tests | archive extraction, checksum, endpoint policy, pull stream bounds with local servers. | no |
| Device verification | `#[ignore]` tests with a real Ollama and small model (Skriuw's). | local model download, no paid API |
| Live provider smoke | opt-in, env-gated (`AI_LIVE_TESTS=1` + provider keys), one `max_tokens: 1` call per adapter. Never in CI by default. | paid, opt-in only |

Tests that must never require paid access: everything except the live smoke suite. Dora currently has no adapter tests at all; migrating Dora onto the shared adapters immediately inherits the conformance suite.

---

## 28. Duplication matrix

| Concept | Dora | Skriuw v2 | Betalingen | Classification |
| --- | --- | --- | --- | --- |
| OpenAI-compatible chat client | `compat.rs` (7 specs) | `provider.rs` (6 rows) | via `@ai-sdk/groq` | **Duplicated, should move** (one Rust table + one TS adapter) |
| Gemini client | `gemini.rs` (query-string key) | `provider.rs` Gemini arm (header) | (v1 only) | Duplicated, should move (Skriuw's version) |
| Anthropic client | `anthropic.rs` | none | none | Single source; move to shared |
| SSE / NDJSON decoding | `client.rs::read_sse` | `lib.rs::sse_payload` + bounded `read_line`; Ollama NDJSON | AI SDK internal; route re-encodes NDJSON; browser decodes NDJSON | Duplicated, should move (Rust and TS codecs in core) |
| Provider config (URLs, headers, defaults) | `CompatSpec`, consts | `OpenAiCompatible`, `RemoteProviderKind` | env | Duplicated, should move as shared data |
| Model config / selection | per-provider setting strings | `settings.aiModel` pair | env string | Similar but storage is app-specific; the *type* (`ModelRef`) moves |
| Model catalog | curated tiers + live merge | priced catalog + fetched merge | none | Duplicated with different data; move the format and merge logic, keep app-curated picks local |
| API keys | AES-GCM in SQLite + env pool | keyring/session + consent | server env | Intentionally platform-specific; the *port* moves, storage stays |
| Key verification | `test_key` per adapter | `verify_credential` | none | Duplicated, should move (adapter contract method) |
| Error mapping | string copy | category + recovery | generic string | Duplicated; Skriuw's taxonomy moves |
| Retries | key rotation | none (documented) | `maxRetries: 0` | Unclear, needs design decision → router |
| Streaming event shape | `Token/Final/Error` | `Delta/Done/Cancelled/Timeout/ProviderError` | `text/done/error` | Duplicated, should move (Skriuw's shape) |
| Cancellation | `DashMap<String, AtomicBool>` | `AiCancellation` registry | `AbortController` | Duplicated in Rust, should move; TS standardizes on `AbortSignal` |
| Request id + ordering | client id, no sequence | UUID + sequence gate | none | Should move (Skriuw's) |
| Prompts | SQL system prompts | 15 writing prompts (data) | Dutch data prompt | Application-specific, remain; writing prompts optionally shared as data |
| Structured output | json_object + lenient parse | list parsing | none | Needs design decision → core `generate_object` |
| Ollama HTTP generation | `/api/chat` | `/api/generate` | none | Duplicated, should move |
| Ollama install/spawn/pull | `ollama_installer/*` | `skriuw-ai-ollama` | none | Duplicated (plus v1 copy), should move to `ai-ollama-runtime` (Skriuw base + Dora env handling) |
| Task definitions | none formal (`prompt_mode`) | `AI_EDITOR_ACTIONS` + `BUILT_IN_PROMPTS` | one task | Skriuw's is reusable data; Dora's SQL tasks remain |
| Usage recording | `ai_usage` table + float pricing | `ai_run_history` + micro-dollar pricing + retention | none | Similar; record shape and ports move, storage remains |
| Context construction | schema + UI context in Rust and TS | editor extraction in TS | allowlisted source fetch | Application-specific, remain |
| React integration | `use-ai-chat.ts`, `createStreamBatcher`, zustand store | `useAiRun`, consumer, gate | `useDataChat` | Similar; a generic run hook + consumer move to `@ai/react`/`@ai/core`; stores remain |
| Tauri commands | 34 | 22 | n/a | Application-specific surfaces remain; channel sink/registry helpers move |
| Web-demo mock | `mock-ai.ts` | Rust `fake` + TS playground fake group | injected `fetch` in tests | Duplicated intent; fake provider moves to core in both languages |
| Consent/disclosure | none | versioned per provider | static UI text | Application-specific (policy), remain; port must allow refusal |
| Linux keyring detection | probe + install plan | D-Bus vault states | n/a | Similar but different UX; Skriuw's detection moves to `ai-credentials`, Dora's `pkexec` installer stays in Dora |

---
## 29. What should be shared

Derived from the duplication matrix; each item has at least two consumers in the code today.

1. **Contract types**: `CompletionRequest` (messages, model ref, parameters, response format), `CompletionEvent`, `ProviderError` + categories + recovery actions, `Usage`, `ModelRef`, `ModelInfo`, `CompletionOutcome`, `RunRecord`. (Dora, Skriuw, Betalingen.)
2. **Runtime**: request registry with validated ids, duplicate rejection, cancellation, first-terminal-wins, recorder hook (Skriuw's service; Dora reimplements a weaker one).
3. **Deterministic fake provider** with scripted outcomes, in both languages.
4. **Provider adapters** (Rust): OpenAI-compatible table, Anthropic, Gemini, Ollama generation. (TS): AI SDK-backed adapter.
5. **Transport codecs**: SSE decoder, NDJSON decoder/encoder, bounded readers.
6. **Credential resolver port** with env and session implementations; keyring implementation with vault-state detection as an optional crate.
7. **Model catalog format + validation + merge** (catalog wins over listed, listed wins over unknown); provider descriptors as data.
8. **Key verification** as an adapter method (`verify(model, credential)`).
9. **Ollama runtime manager** (install with checksum, spawn/stop/reap, pull/delete with progress) as an optional crate.
10. **Tauri helpers**: channel sink, registry, blocking-run.
11. **TypeScript consumer helpers**: ordered consumer, abort-aware start function, NDJSON client decoder; React run hook.
12. **Token/byte estimation and cost computation** helpers; pricing data in the catalog.
13. **Spec + fixtures**: JSON Schemas, enums, golden streams, error tables, generated and drift-checked.

## 30. What should remain application-specific

| Item | Owner | Why |
| --- | --- | --- |
| Schema context, dialect detection, SQL prompts, SQL safety rules, JSON `{sql, explanation, warnings}` shape, insert/run actions, suggestions | Dora | Domain knowledge and product policy |
| Dora's AES-GCM key table, `pkexec` keyring installer, key labels/active flags, per-key test history | Dora | Existing users' data; product-specific UX; not needed by others |
| Dora web-demo canned responses | Dora | Demo content (can be expressed as fake-provider scripts) |
| Editor extraction (selection/caret/note), ProseMirror apply transactions, in-place review decorations, plans for tasks/tags, workspace prompt storage and shadowing, opt-in gate, consent versions and disclosure copy, vault-tier UX, run history SQLite tables and retention UI | Skriuw | Product behavior and policy under ADR-0033/0036 |
| Source allowlist, auth pass-through, IBAN masking, Dutch financial prompt, dashboard context publishing, NDJSON route wiring under Hono auth | Betalingen | Domain and security policy |
| All Tauri command definitions and names; all React stores; all settings persistence | each app | Divergent today; migration should not force convergence |
| Curated "recommended model" lists | each app | Product-flavored (SQL vs writing) |

## 31. What we should explicitly NOT build

1. **An agent framework or tool-execution loop.** No product calls tools. Reserve a content-part variant, nothing more.
2. **A workflow engine or chains.** Skriuw's editor actions are one request each; Dora's are one request each.
3. **An embeddings/vector layer.** Only frozen Skriuw v1 had embeddings.
4. **A prompt marketplace, prompt registry service, or centralized prompt store.** Skriuw's workspace prompts are workspace data; Dora's prompts are code.
5. **A React UI kit** (chat bubbles, model pickers, key forms). Each app has its own design system; only hooks are shared.
6. **A Tauri plugin with fixed commands.** Command surfaces are app-specific; a helper crate suffices.
7. **A shared usage database or telemetry uploader.** Both desktop apps forbid uploads; the SDK emits records to a port.
8. **A Hono/Next.js framework package** until a second server consumer exists; framework-free stream helpers cover Betalingen.
9. **A custom HTTP client abstraction beyond `reqwest`/`fetch` injection.** Both are already injectable.
10. **A full TypeScript reimplementation of provider protocols.** The AI SDK adapter covers it; a raw OpenAI-compatible `fetch` adapter is the only acceptable exception, and only if a consumer needs to drop the AI SDK dependency.
11. **Automatic cross-provider fallback that ignores locality.** Falling back from Ollama to a cloud provider without an explicit policy would violate both desktop apps' privacy promises.
12. **Provider-specific option bags on the request** (`extra: serde_json::Value`). ADR-0033 rejected an untyped options map; typed extensions per capability are the way to add features.
13. **Browser BYOK storage.** Neither app does it; it is a product decision with security consequences.
14. **Model tier heuristics in core** (`flagship/balanced/fast` by substring). Keep as optional UI helpers in the app or a `catalog-extras` module.

## 32. Proposed SDK monorepo

Placeholder name `ai-sdk`. Note: the npm scope `@ai-sdk/*` belongs to Vercel; the TypeScript packages need a different scope (`@ai/*` is used below as a placeholder).

```
ai-sdk/
  Cargo.toml                      # workspace: crates/*
  package.json                    # bun workspaces: packages/*
  specs/
    schema/                       # JSON Schema, generated from Rust (xtask), committed, drift-checked
      completion-request.schema.json
      completion-event.schema.json
      provider-error.schema.json
      model-info.schema.json
      run-record.schema.json
      local-runtime-*.schema.json
    enums/
      error-categories.json       # id, recovery default, fallback-eligible, retryable-before-first-delta
      capabilities.json
    data/
      providers.json              # provider descriptors + OpenAI-compatible rows (Skriuw table + Dora rows)
      models.json                 # priced catalog (Skriuw format, version N, pricingAsOf)
    VERSION                       # spec version (semver)
  fixtures/
    streams/<provider>/<case>.sse + <case>.events.json
    errors/<provider>/<case>.json
    fake-scripts/*.json           # shared FakeProvider scripts
  crates/
    ai-core/                      # contracts, validation, runtime, fake provider, codecs, ports, async facade (feature "tokio")
    ai-providers/                 # features: openai-compatible, anthropic, gemini, ollama; catalog module; verify + list
    ai-ollama-runtime/            # LocalRuntime port + OllamaRuntime (install/spawn/pull)
    ai-credentials/               # features: env, session, keyring (+ dbus vault detection on linux)
    ai-router/                    # optional routing policy + in-memory health
    ai-tauri/                     # channel sink, registry, blocking-run, feature "specta"
    xtask/                        # schema generation + drift check + fixture replay
  packages/
    core/          (@ai/core)     # types, zod schemas, consumer, codecs, runtime, fake provider, router module
    ai-sdk/        (@ai/ai-sdk)   # Vercel AI SDK adapter
    react/         (@ai/react)    # useCompletionRun, useStreamingText
    tauri/         (@ai/tauri)    # invoke/Channel bridge, progress operation bridge
    tasks/         (@ai/tasks)    # optional: built-in prompt data + runTask + list parser (mirrored by crates/ai-tasks if Rust needs it)
  examples/
    rust-cli/                     # stream from any provider with env credentials; fake by default
    bun-hono/                     # Betalingen-shaped NDJSON route
    tauri-minimal/                # commands built on ai-tauri
```

Per package/crate responsibilities:

| Unit | Responsibility | Public API (summary) | Dependencies | Must NOT contain |
| --- | --- | --- | --- | --- |
| `ai-core` | Contracts, validation, bounds, cancellation, sink, `Provider` trait, `CompletionRuntime`, `FakeProvider`, SSE/NDJSON codecs, `CredentialSource`/`RunRecorder`/`Pricing`/`ModelSource` ports, structured-output ladder, `CompletionOutcome` | see section 34 | `serde`, `serde_json`, `schemars`, `thiserror`; optional `tokio`, `specta`, `jsonschema` | `reqwest`, `keyring`, `tauri`, provider names, prompts, storage |
| `ai-providers` | Remote + Ollama generation adapters, provider descriptors, catalog loading/merging, `verify`, `list_models` | `OpenAiCompatibleProvider::new(spec, credentials)`, `AnthropicProvider`, `GeminiProvider`, `OllamaProvider`, `catalog()`, `providers()` | `ai-core`, `reqwest` (blocking, rustls), `serde_json` | credential storage, process management, UI copy |
| `ai-ollama-runtime` | `LocalRuntime` trait + `OllamaRuntime` | `status/start/stop/install/list_models/pull_model/remove_model/shutdown` with progress sink + cancellation | `ai-core`, `reqwest`, `tar`, `zstd`, `flate2`, `sha2`, `tempfile` | completion |
| `ai-credentials` | resolvers | `EnvCredentials`, `SessionCredentials`, `KeyringCredentials`, `detect_vault()`, `Rotating(vec)` decorator | `ai-core`; `keyring`, `dbus-secret-service` behind features | consent policy, encryption of app databases |
| `ai-router` | policy + health | `Router::new(routes, health).resolve(task_req)`, `RouterProvider` implementing `Provider` over the runtime | `ai-core` | adapters |
| `ai-tauri` | glue | `ChannelSink`, `RequestRegistry`, `run_blocking`, `OperationRegistry` | `ai-core`, `tauri` | commands |
| `@ai/core` | as `ai-core` in TS | `createRuntime`, `createFakeProvider`, `createCompletionConsumer`, `decodeSse`, `decodeNdjson`, `toNdjsonStream`, `errorCategory`, `createRouter` | `zod` | `ai`, `react`, `@tauri-apps/api`, `hono` |
| `@ai/ai-sdk` | AI SDK adapter | `fromLanguageModel(model, meta)`, `openaiCompatible(spec, credential)` | `ai`, `@ai-sdk/*`, `@ai/core` | UI |
| `@ai/react` | hooks | `useCompletionRun(runtime, {origin})`, `useStreamingText` | `react`, `@ai/core` | stores, components |
| `@ai/tauri` | bridge | `createTauriProvider({invoke, Channel, commands})`, `runProgressOperation` | `@tauri-apps/api`, `@ai/core` | app command names hard-coded |
| `@ai/tasks` | task data + helper | `BUILT_IN_PROMPTS`, `buildTaskRequest`, `parseList` | `@ai/core` | app-specific prompts |

## 33. Dependency graph

```
specs/ (schemas, enums, data, fixtures)  <== generated by crates/xtask from ai-core types; validated by packages/core tests

                 ai-core  (Rust)                              @ai/core  (TS)
               /    |     \       \                            /      |      \
   ai-providers  ai-credentials  ai-ollama-runtime  ai-router     @ai/ai-sdk  @ai/react  @ai/tauri  @ai/tasks
               \    |     /       /                                \      |      /
                  ai-tauri  (optional glue; depends on ai-core only, not on providers)
                       |                                                   |
      Dora app / Skriuw app (Tauri commands, context, prompts, storage)   Betalingen / Skriuw renderer / Dora studio
```

Allowed edges: everything points inward to `ai-core` / `@ai/core`; `ai-router` may depend on `ai-core` only (it receives providers, it does not import them); `ai-tauri` depends on `ai-core` (and `tauri`), never on `ai-providers` (apps assemble providers); `ai-ollama-runtime` depends on `ai-core` for cancellation and progress types but not on `ai-providers` (the Ollama *provider* may optionally depend on the runtime for "auto-start on demand", behind a feature, never the reverse).

Forbidden dependencies (refined from the brief):

- `ai-core` → `tauri`, `reqwest`, `keyring`, `tokio` (default features), any application crate.
- `@ai/core` → `react`, `ai`/`@ai-sdk/*`, `@tauri-apps/api`, `hono`, `node:*` (must run in browsers).
- `ai-providers` / `@ai/ai-sdk` → SQL schema types (Dora), note/editor types (Skriuw), financial schemas (Betalingen), consent types (Skriuw policy).
- `ai-router` → provider crates (must route over `dyn Provider` + `ModelInfo`).
- Any SDK unit → application prompt text.
- Applications → provider adapters directly for completion (they must go through the runtime, so recording, cancellation, and routing stay uniform); direct adapter use is allowed only for `verify` and `list_models` administration calls.

## 34. Proposed Rust public API (sketch)

```rust
// ai-core ---------------------------------------------------------------
pub struct ModelRef { pub provider_id: String, pub model_id: String }          // validated identifiers

pub enum Role { System, User, Assistant }
#[non_exhaustive] pub enum ContentPart { Text { text: String } }               // tool parts reserved
pub struct Message { pub role: Role, pub content: Vec<ContentPart> }

pub enum ResponseFormat { Text, Json, JsonSchema { name: String, schema: serde_json::Value, strict: bool } }

pub struct Parameters {
    pub max_output_tokens: Option<u32>, pub max_output_bytes: u32, pub timeout_ms: u32,
    pub temperature_millis: Option<u16>, pub top_p_millis: Option<u16>,
    pub stop: Vec<String>, pub response_format: ResponseFormat,
}
pub struct CompletionRequest { pub request_id: String, pub model: ModelRef, pub messages: Vec<Message>,
                               pub parameters: Parameters, pub origin: String }
impl CompletionRequest { pub fn simple(model: ModelRef, system: &str, user: &str) -> Self; pub fn validate(&self) -> Result<(), ValidationError>; }

pub struct Delta { pub request_id: String, pub sequence: u32, pub text: String }
pub struct Usage { pub input_tokens: u64, pub output_tokens: u64, pub source: UsageSource }
pub enum FinishReason { Stop, Length, ContentFilter, Other }
pub enum ErrorCategory { MissingCredential, InvalidCredential, InvalidRequest, ModelUnavailable, UnsupportedCapability,
                         RateLimited, QuotaExceeded, Timeout, Network, ProviderUnavailable, Cancelled, MalformedResponse,
                         StructuredOutputInvalid, LocalRuntimeUnavailable, LocalModelMissing, Internal }
pub enum RecoveryAction { ConfigureCredential, Retry, ChooseDifferentModel, CheckProviderStatus, ReduceRequest, ContactProvider, StartLocalRuntime, PullModel, None }
pub struct ProviderError { pub provider_id: String, pub category: ErrorCategory, pub recovery: RecoveryAction,
                           pub message: String, pub retry_after: Option<Duration>, pub source: Option<ErrorSource> }

#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum CompletionEvent { Delta(Delta), Done { request_id: String, usage: Option<Usage>, finish_reason: Option<FinishReason> },
                           Cancelled { request_id: String }, Timeout { request_id: String },
                           ProviderError { request_id: String, error: ProviderError } }
pub enum Terminal { Done { usage: Option<Usage>, finish_reason: Option<FinishReason> }, Cancelled, Timeout, Error(ProviderError) }

pub struct Cancellation(Arc<AtomicBool>);   // clone, cancel(), is_cancelled()
pub trait EventSink: Send { fn send_delta(&mut self, delta: Delta) -> Result<(), SinkClosed>; }

pub trait Provider: Send + Sync {
    fn id(&self) -> &str;
    fn complete(&self, request: &CompletionRequest, cancel: &Cancellation, sink: &mut dyn EventSink) -> Terminal;
    fn verify(&self, model: &ModelRef, credential: &Credential) -> Result<(), ProviderError> { unsupported }
    fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> { Ok(vec![]) }
    fn model_info(&self, model: &ModelRef) -> Option<ModelInfo> { None }
}

pub trait CredentialSource: Send + Sync { fn resolve(&self, provider_id: &str) -> Result<Credential, CredentialError>; }
pub trait RunRecorder: Send + Sync { fn record(&self, run: RunRecord); }
pub trait Pricing: Send + Sync { fn price(&self, model: &ModelRef) -> Option<Price>; }
pub trait EventChannel: Send + Sync + 'static { fn send(&self, event: CompletionEvent) -> Result<(), SinkClosed>; }

pub struct CompletionRuntime { /* providers, active registry, recorder, pricing, router hook */ }
impl CompletionRuntime {
    pub fn builder() -> RuntimeBuilder;                     // .provider(Arc<dyn Provider>).recorder(..).pricing(..).router(..)
    pub fn start(&self, request: CompletionRequest, channel: impl EventChannel) -> Result<(), StartError>;
    pub fn cancel(&self, request_id: &str) -> bool;
    pub fn complete_blocking(&self, request: CompletionRequest) -> CompletionOutcome<String>;      // non-stream convenience
    pub fn generate_object<T: DeserializeOwned + JsonSchema>(&self, request: CompletionRequest, policy: StructuredPolicy) -> CompletionOutcome<T>;
    #[cfg(feature = "tokio")] pub fn stream(&self, request: CompletionRequest) -> impl futures::Stream<Item = CompletionEvent>;
    pub fn shutdown(&self);
}

pub struct FakeProvider { script: FakeScript }   // FakeScript { tokens, token_delay, outcome, usage }

// ai-providers ------------------------------------------------------------
pub struct OpenAiCompatibleSpec { pub id, pub label, pub destination, pub base_url, pub chat_path, pub models_path: Option<_>,
                                  pub stream_usage_option: bool, pub json_mode: Support, pub json_schema: Support, pub env_key_prefix: Option<_> }
pub fn builtin_specs() -> &'static [OpenAiCompatibleSpec];           // openai, groq, deepseek, moonshot, zai, dashscope, aimlapi, openrouter
pub struct OpenAiCompatibleProvider; impl OpenAiCompatibleProvider { pub fn new(spec: OpenAiCompatibleSpec, creds: Arc<dyn CredentialSource>) -> Result<Self, SetupError>; pub fn with_base_url(..); }
pub struct AnthropicProvider; pub struct GeminiProvider; pub struct OllamaProvider { pub fn new(endpoint: Url) }
pub fn catalog() -> Result<Catalog, CatalogError>;                     // embedded specs/data/models.json

// ai-router ---------------------------------------------------------------
pub struct TaskRequirements { pub needs: Vec<Capability>, pub locality: Locality, pub latency: LatencyClass }
pub struct Route { pub model: ModelRef, pub credential_hint: Option<String> }
pub struct Router { pub fn new(routes: RoutePolicy, health: Arc<dyn HealthStore>) -> Self; }
impl Router { pub fn plan(&self, req: &TaskRequirements, infos: &dyn ModelSource) -> Vec<Route>; pub fn observe(&self, route: &Route, terminal: &Terminal); }

// ai-tauri ----------------------------------------------------------------
pub struct ChannelSink(pub tauri::ipc::Channel<CompletionEvent>);   // impl EventChannel
pub struct OperationRegistry;                                       // begin(id) -> Cancellation, cancel(id), cancel_all()
pub async fn run_blocking<T>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, JoinError>;
```

## 35. Proposed TypeScript public API (sketch)

```ts
// @ai/core
export type ModelRef = { providerId: string; modelId: string };
export type Message = { role: 'system' | 'user' | 'assistant'; content: Array<{ type: 'text'; text: string }> };
export type CompletionRequest = { requestId: string; model: ModelRef; messages: Message[]; parameters: Parameters; origin: string };
export type CompletionEvent =
  | { type: 'delta'; requestId: string; sequence: number; text: string }
  | { type: 'done'; requestId: string; usage?: Usage | null; finishReason?: FinishReason | null }
  | { type: 'cancelled'; requestId: string } | { type: 'timeout'; requestId: string }
  | { type: 'provider_error'; requestId: string; error: ProviderError };
export const CompletionRequestSchema: z.ZodType<CompletionRequest>;   // validate at HTTP/IPC boundaries
export const CompletionEventSchema: z.ZodType<CompletionEvent>;

export interface Provider {
  readonly id: string;
  complete(request: CompletionRequest, signal: AbortSignal): AsyncIterable<CompletionEvent>;   // must end with exactly one terminal
  verify?(model: ModelRef, credential: string): Promise<void>;
  listModels?(): Promise<ModelInfo[]>;
}
export interface CredentialSource { resolve(providerId: string): Promise<string> }    // throws CredentialError
export function envCredentials(prefixes?: Record<string, string>): CredentialSource;

export function createRuntime(options: { providers: Provider[]; recorder?: RunRecorder; pricing?: Pricing; router?: Router }): Runtime;
export interface Runtime {
  start(request: CompletionRequest, onEvent: (e: CompletionEvent) => void, signal?: AbortSignal): Promise<CompletionOutcome>;
  stream(request: CompletionRequest, signal?: AbortSignal): AsyncIterable<CompletionEvent>;
  complete(request: CompletionRequest, signal?: AbortSignal): Promise<CompletionOutcome<string>>;
  generateObject<T>(request: CompletionRequest, schema: z.ZodType<T>, policy?: StructuredPolicy, signal?: AbortSignal): Promise<CompletionOutcome<T>>;
  cancel(requestId: string): boolean;
}
export function createFakeProvider(script: FakeScript): Provider;
export function createCompletionConsumer(requestId: string, cb: { onDelta(text: string): void; onTerminal(e: TerminalEvent): void }): Consumer;
export function decodeNdjson<T>(body: ReadableStream<Uint8Array>, schema: z.ZodType<T>): AsyncIterable<T>;
export function toNdjsonStream(events: AsyncIterable<CompletionEvent>): ReadableStream<Uint8Array>;
export function decodeSse(body: ReadableStream<Uint8Array>): AsyncIterable<string>;   // data: payloads
export function createRouter(policy: RoutePolicy, health?: HealthStore): Router;

// @ai/ai-sdk
export function fromLanguageModel(model: LanguageModel /* internal type */, meta: { providerId: string; modelId: string; info?: Partial<ModelInfo> }): Provider;
export function openaiCompatible(spec: OpenAiCompatibleSpec, credentials: CredentialSource, fetch?: typeof globalThis.fetch): Provider;

// @ai/react
export function useCompletionRun(runtime: Runtime, options: { origin: string }): { run: RunState; fire(req): void; retry(): void; cancel(): void };

// @ai/tauri
export function createTauriProvider(opts: { invoke; Channel; commands: { start: string; cancel: string } }): Provider;
export function runProgressOperation<T>(opts: { invoke; Channel; command: string; args; onProgress; signal }): Promise<T>;

// @ai/tasks (optional)
export const BUILT_IN_PROMPTS: readonly BuiltInPrompt[];
export function buildTaskRequest(task: TaskId, input: { text: string; instruction?: string }, model: ModelRef, origin: string): CompletionRequest;
export function parseList(output: string, limits?: ListLimits): ListParse;
```

## 36. Provider authoring contract

Minimum code to add a provider:

- **OpenAI-compatible provider**: zero Rust/TS code. One row in `specs/data/providers.json`:

```json
{ "id": "cerebras", "label": "Cerebras", "destination": "api.cerebras.ai", "baseUrl": "https://api.cerebras.ai/",
  "chatPath": "v1/chat/completions", "modelsPath": "v1/models", "streamUsageOption": true,
  "jsonMode": "unknown", "jsonSchema": "unknown", "envKeyPrefix": "CEREBRAS", "auth": "bearer" }
```

plus optional catalog entries in `models.json` and a fixture pair under `fixtures/streams/cerebras/`. Skriuw's `OpenAiCompatible` struct and Dora's `CompatSpec` prove the shape covers Groq, DeepSeek, Moonshot/Kimi, Z.ai/GLM, DashScope/Qwen, AI/ML API, OpenAI, OpenRouter. Cerebras, SambaNova, DeepSeek, GLM, MiniMax (its OpenAI-compatible endpoint) fit the same row; providers needing a custom header (OpenRouter's optional `HTTP-Referer`/`X-Title`) get an `extraHeaders` map on the row. Known deviations become row flags, as Z.ai's `streamUsageOption: false` already is.

- **Non-compatible provider** (Anthropic-shaped, Gemini-shaped): implement the `Provider` trait/interface: build request body, authorize (header placement), parse one stream event into `{ text, usage?, finished? }`, map status codes (default mapper provided), optional `verify` body and `list_models` parser. In Rust that is the four `RemoteProviderKind` methods (`endpoint`, `authorize`, `completion_body`, `parse_event`) plus `verification_body`/`parse_model_listing`, roughly 150 lines by the Gemini arm's size. The SSE reader, bounds, cancellation, deadline, and error taxonomy come from the shared skeleton and must not be reimplemented.

- **Conformance**: every provider (row or code) must pass the shared conformance suite against a local fixture server / injected fetch, and ship at least one golden stream fixture. Adding a provider without fixtures should fail CI.

## 37. Configuration model

Layers, resolved in order (later wins), all serializable without secrets:

1. **Application defaults** (compiled in): allowed providers, default routes per task profile, default parameters, local-only default (Skriuw defaults to Ollama; Dora to Groq; Betalingen to Groq via env).
2. **Environment** (server/dev): `AI_PROVIDER`, `AI_MODEL`, `<PREFIX>_API_KEY`, `<PREFIX>_MODEL`, `AI_LOCAL_ONLY`, `OLLAMA_ENDPOINT`. Dora already has `{PREFIX}_API_KEY[_n]`/`{PREFIX}_MODEL`; Skriuw has `SKRIUW_OLLAMA_ENDPOINT`; Betalingen has `GROQ_API_KEY`/`GROQ_MODEL`.
3. **User preferences** (app-persisted): selected `ModelRef` (Skriuw `settings.aiModel`, Dora `ai_provider` + per-provider model), enabled flag, per-task overrides, allowed providers, local-only toggle.
4. **Per-call overrides**: explicit `ModelRef`, parameters, `response_format`, task requirements.

Secrets are never in this structure; they come through `CredentialSource`. Example shape:

```json
{
  "enabled": true,
  "localOnly": false,
  "allowedProviders": ["ollama", "groq", "anthropic"],
  "default": { "providerId": "ollama", "modelId": "llama3.2:3b" },
  "routes": {
    "interactive": [{ "providerId": "ollama", "modelId": "llama3.2:1b" }, { "providerId": "groq", "modelId": "llama-3.1-8b-instant" }],
    "structured": [{ "providerId": "openai", "modelId": "gpt-4.1-mini" }, { "providerId": "groq", "modelId": "llama-3.3-70b-versatile" }],
    "quality":    [{ "providerId": "anthropic", "modelId": "claude-sonnet-4-6" }]
  },
  "parameters": { "timeoutMs": 60000, "maxOutputBytes": 262144 },
  "endpoints": { "ollama": "http://127.0.0.1:11434" },
  "diagnostics": { "includeProviderBodies": false }
}
```

Validation belongs in core (bounds, identifier rules, unknown providers rejected); persistence belongs in the app (SQLite settings in both desktop apps, env/Blob in Betalingen).

---
## 38. Migration path for Dora

Incremental, behind the existing `AiClient` seam first, then command by command.

| Category | Files / symbols | Note |
| --- | --- | --- |
| **Likely removable** | `services/ai/compat.rs`, `anthropic.rs`, `gemini.rs`, `ollama.rs` (generation half), `client.rs` (`send_with_rotation`, `read_sse`, `http_client`, `should_rotate`), `errors.rs`, `key_pool.rs` (replaced by `Rotating` credential decorator), `models.rs` catalogs + classify heuristics (moved to catalog data + optional UI tiers), `ollama_installer/*` (replaced by `ai-ollama-runtime`), `AiStreamEvent`, `AIResponse.suggested_queries`, `MockClient`, `ai_groq_status`, `ai_set_gemini_key` (after migration window) | ~2,400 lines of Rust |
| **Likely reusable as-is (into the SDK)** | `CompatSpec` rows for OpenAI and OpenRouter and the `chat_temperature` idea; Anthropic request/response shapes; `KeyPool` merge of env keys `{PREFIX}_API_KEY[_1..10]`; Ollama `LD_LIBRARY_PATH`/`DYLD_LIBRARY_PATH`/quarantine handling; pull ETA computation; `merge_models` strategy; `AiApiKeyRecord` UX (labels, active flag, last test) as a reference for a multi-key resolver | |
| **Remains in Dora** | `SchemaContext` family and `build_schema_context`, `engine_for_connection`, `prompts.rs` (moved out of the provider layer into a Dora `ai_prompts` module that produces `Vec<Message>`), `prompt_mode` → replaced by two Dora task functions (`sql_generation_request`, `chat_request`), `storage/ai_keys.rs` + `security.rs` (as a `CredentialSource` implementation `SqliteAesCredentials`), `storage/ai_usage.rs` (as a `RunRecorder` implementation writing the existing table, gaining `duration_ms`, `state`, `origin`), `credential_storage.rs` keyring installer, all `commands/ai.rs` commands (thinned), all studio UI, `mock-ai.ts` (rewritten as fake-provider scripts served through the same `Provider` interface in browser mode) | |
| **Adapter boundary** | `commands/ai.rs::ai_complete_stream` becomes: build Dora messages (context + prompt) → `CompletionRequest` with `origin: "dora:chat" | "dora:sql"` → `CompletionRuntime::start(request, ChannelSink(on_event))`. `ai_abort_stream` → `runtime.cancel`. Structured SQL path uses `generate_object` with the `{sql, explanation, warnings}` schema and Dora's validation of `sql` non-empty. | |
| **Wire changes for the studio** | `AiStreamEvent` → `CompletionEvent` (`token`→`delta`, `final` dropped, terminals added). `use-ai-chat.ts`/`ai-cmd-k.tsx` switch to `@ai/core` consumer + `@ai/tauri` bridge; history stops being string-packed and becomes `messages`. `{kind, detail}` errors gain typed `ProviderError` for AI commands (`kind: "ProviderError"`, `detail` JSON) or a dedicated result type. | Breaking for the studio, contained to the `ai-assistant` and `ai-cmd-k` modules Dora's roadmap already marks as the pilot for the ports refactor |
| **Migration risk** | Medium. Behavior changes: key rotation on 401 disappears (becomes an error), streaming usage becomes provider-reported where available, cancel becomes a visible terminal, Gemini key moves to a header, Ollama endpoint restricted to loopback unless Dora keeps its own `OllamaProvider::new(any_url)` (recommended: allow non-loopback but mark as remote for policy). Existing encrypted keys keep working through the `SqliteAesCredentials` adapter. | |
| **Order** | (1) add SDK as path dependency; (2) implement `SqliteAesCredentials` + `SqliteUsageRecorder`; (3) switch `ai_complete_stream` to the runtime with Dora prompts producing messages; keep old event names via a temporary mapper for one release; (4) switch studio to the new consumer; (5) delete adapters and installer; (6) adopt `ai-ollama-runtime` behind the existing commands. | |

## 39. Migration path for Skriuw

Lowest-risk of the three because the SDK core is Skriuw's code.

| Category | Files / symbols | Note |
| --- | --- | --- |
| **Removable (moved to SDK)** | `skriuw-domain/src/ai.rs`, `local_ai.rs`, the credential/catalog/directory halves of `remote_ai.rs`, `ai_history.rs` record/port types; `skriuw-ai` entirely; `skriuw-ai-remote` entirely; `skriuw-ai-ollama` entirely; `app/src/contracts/ai.ts` (replaced by `@ai/core` types); `completion-bridge.ts`, `completion-consumer.ts`, most of `use-ai-run.ts`, `ollama-bridge.ts::runProgressOperation`, `remote-ai-bridge.ts` invoke wrappers (thin, may stay) | Re-exported from `skriuw-domain` during transition so nothing else in the workspace changes |
| **Reusable but Skriuw-owned** | `prompt.rs` built-ins (optionally donated to `@ai/tasks` as data; Skriuw keeps `WorkspacePrompt` shadowing), `editor-actions.ts` catalog, `action-plan.ts` parsers (candidate for `@ai/tasks::parseList`) | |
| **Remains in Skriuw** | consent model (`RemoteAiConsent`, `REMOTE_AI_DISCLOSURE_VERSION`, `RemoteAiProviderState`), `AiCredentialStore` (becomes a `CredentialSource` impl over `ai-credentials::KeyringCredentials` + session tier + consent check), `FetchedModelStore`, `AiHistoryRecorder` (a `RunRecorder` impl), `OllamaManager` (over `ai-tauri::OperationRegistry`), `LazyAiCompletion` (builds the runtime), all commands, opt-in gate, editor apply/review, plans, playground, settings UI | |
| **Adapter boundary** | `LazyAiCompletion::service()` becomes `CompletionRuntime::builder().provider(fake).provider(ollama).providers(remote from specs filtered by Skriuw's allowlist).recorder(history).pricing(catalog).build()`. Commands unchanged in name; payload types come from the SDK (identical JSON today). | |
| **Wire changes** | Request: `systemPrompt`/`userPrompt` → `messages` (two entries); `maxOutputBytes` stays, `maxOutputTokens` optional; `providerId`/`modelId` → `model: {providerId, modelId}` or keep flat via serde alias for one version. Events: identical except optional `finishReason`. Errors: `AiProviderErrorCategory` gains new variants (`model_unavailable`, `unsupported_capability`, `structured_output_invalid`, `local_*`); renderer copy tables need entries. | Contract drift check catches every change |
| **Migration risk** | Low. The `deny_unknown_fields` request type means the renderer must send the new shape in lockstep, which the type gate enforces. The retry semantics gap (documented, unimplemented) can be closed by the runtime with the ADR rule "before first delta only". Blocking `reqwest` stays, so no concurrency model change. | |
| **Order** | (1) extract crates to the SDK repo and point `Cargo.toml` at them (path or git), keep `skriuw-domain` re-exports; (2) run Skriuw's full AI test suite unchanged as the SDK's first conformance evidence; (3) adopt `messages` and the new error variants; (4) swap renderer bridge/consumer to `@ai/core`/`@ai/tauri`; (5) later, adopt `generate_object` for tasks/tags if desired. | |

## 40. Migration path for Betalingen

| Category | Files / symbols | Note |
| --- | --- | --- |
| **Removable** | `src/lib/ai/groq.ts` (replaced by `@ai/ai-sdk` `openaiCompatible(groqSpec, envCredentials())` or `fromLanguageModel(createGroq(...)(model))`), the manual `fullStream` loop and NDJSON writing in `routes/ai.ts` (replaced by `runtime.stream()` + `toNdjsonStream`), the NDJSON parsing in `use-data-chat.ts` (replaced by `decodeNdjson(CompletionEventSchema)` + `createCompletionConsumer`) | ~80 lines net |
| **Reusable as-is** | `ChatEventSchema` shape maps 1:1 onto `delta`/`done`/`provider_error`; the injected-`fetch` test pattern becomes the standard TS conformance harness | |
| **Remains in Betalingen** | `contracts.ts` `ScreenContextSchema`/`ChatRequestSchema` (app HTTP contract), `context.ts` (allowlist, auth pass-through, masking, prompt), the route's auth/body-limit/503 policy, the dashboard context publishing, `DataChat` UI | |
| **Adapter boundary** | Route: validate app request → `buildScreenContext` → `CompletionRequest { messages: [system, ...history, user(context+question)], model: fromEnv, origin: "betalingen:data-chat" }` → `runtime.stream(request, signal)` → `toNdjsonStream`. Client: `decodeNdjson` → consumer → state. | |
| **Wire change** | Event names: `text` → `delta` (with `requestId`, `sequence`), `error` → `provider_error` (structured), add `cancelled`/`timeout`. The client and server ship together, so no compatibility window is needed. Alternatively keep the app's three-event contract and map at the route; this is legitimate since the HTTP contract is app-owned. | |
| **Risk** | Low; the deployed artifact is one bundle. Watch bundle size: `@ai/core` must stay free of `ai` so the browser island does not grow; `@ai/ai-sdk` is server-only. Vercel Node runtime: no Bun APIs in the SDK core (already a Betalingen rule). | |
| **Gains** | typed error categories (503 vs rate-limit vs quota distinguishable), optional usage in `done`, a router if a second key/model is ever added, fake provider for tests without hand-built SSE. | |

## 41. Risks and unresolved questions

1. **Sync provider trait vs async servers.** Choosing Skriuw's blocking model is right for the two desktop apps but a future Rust HTTP service (axum) would spawn a blocking thread per stream. The `tokio` facade mitigates; a native async adapter family could be added later behind the same event contract. Decision needed before phase 2 if a Rust server is on the roadmap.
2. **`reqwest` 0.12 vs 0.13 and rustls features** across Dora and the SDK; Dora will carry two reqwest majors until it upgrades. Low risk, build-size cost.
3. **Contract churn vs `deny_unknown_fields`.** Skriuw's strictness is good for security and bad for rolling upgrades. Proposal: keep `deny_unknown_fields` on requests (trust boundary) and drop it on events (consumers should ignore unknown terminal fields such as a future `finish_reason`), with a spec version field in `done`.
4. **Identity of providers across apps.** Dora `kimi`/`glm`/`qwen` vs Skriuw `moonshot`/`zai`/`dashscope`. The shared descriptor table must pick one id per provider; Dora's stored settings need a mapping on migration.
5. **Model catalogs are already stale in both apps** (Dora's OpenAI list vs Skriuw's Gemini list; Groq ids differ entirely). A shared `models.json` needs an owner and a refresh cadence, and every consumer must tolerate ids outside it (Skriuw's "listed" tier does).
6. **Structured output on small local models** is unreliable; Skriuw's text-list parsing exists for that reason. The ladder must let apps force `PromptOnly` + custom parser.
7. **Credential rotation semantics.** Dora users may depend on multi-key rotation for Groq free tiers. Rotation on `rate_limited` should be preserved; rotation on `invalid_credential` should be dropped with a settings hint. Product decision.
8. **Ollama endpoint policy.** Dora allows remote Ollama hosts; Skriuw forbids. The SDK should allow non-loopback endpoints but classify them as `Remote { destination }` so privacy policies (Skriuw consent, router `local_only`) treat them correctly. Needs agreement.
9. **Browser builds.** If Skriuw's web build ever wants AI, credentials in the browser become a design problem the SDK deliberately does not solve. Out of scope; note it.
10. **Naming and scope collision** with Vercel's `@ai-sdk/*`. Pick an npm scope early.
11. **Specta support.** Dora's typed `commands` object depends on `specta::Type` on every type crossing IPC; the SDK must derive it behind a feature or Dora loses generated bindings for AI commands.
12. **Test cost of device verification.** Skriuw's ignored tests download ~1.4 GB; keep them ignored and documented.
13. **Ownership of prompts donated to `@ai/tasks`.** If Skriuw's built-ins move, Skriuw's `built-in-prompts.json` generation and shadowing keys (`built_in_id`) must keep stable ids.

## 42. Recommended implementation phases

Sequence is derived from dependency direction (core before adapters before glue) and migration risk (Skriuw lowest, Dora highest, Betalingen smallest).

| Phase | Goal | Packages | Reference app | Expected API | Tests required | Migration impact | Exit criteria |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | Shared spec and fixtures | `specs/`, `fixtures/`, `xtask` | Skriuw (`contracts/generated`, `xtask`) | JSON Schemas for request/event/error/model/run; enums; provider + model data; fake scripts; stream/error fixtures | schema drift check; fixture validity | none | Schemas committed; fixtures replay in a throwaway Rust test using Skriuw's types |
| 1 | Rust core extracted | `ai-core` | Skriuw (`skriuw-domain` AI modules + `skriuw-ai`) | section 34 minus providers; `messages`, `response_format`, `stop`, `finish_reason` added; async facade behind feature | all `skriuw-ai` tests ported; new tests for messages, structured ladder, first-delta retry rule | Skriuw switches to path dependency with re-exports, zero behavior change | Skriuw `./scripts/check.sh` green on the SDK core |
| 2 | Rust providers | `ai-providers` | Skriuw `skriuw-ai-remote` + Dora `anthropic.rs`/`compat.rs` rows | OpenAI-compatible spec-driven adapter, Anthropic, Gemini (header key), Ollama `/api/chat`; `verify`, `list_models`; catalog module | conformance suite against local servers for every adapter; fixture replay; no-socket-without-credential | Skriuw swaps `skriuw-ai-remote`; Dora not yet | Every provider passes conformance; Skriuw device tests still pass |
| 3 | Credentials | `ai-credentials` | Skriuw `ai_credentials.rs` (vault detection), Dora `key_pool.rs` (env merge, rotation) | `EnvCredentials`, `SessionCredentials`, `KeyringCredentials`, `detect_vault`, `Rotating` | unit tests; Linux vault-state matrix with fakes | Skriuw's store wraps keyring resolver; Dora implements `SqliteAesCredentials` locally | Both apps resolve credentials through the port |
| 4 | Ollama runtime | `ai-ollama-runtime` | Skriuw `skriuw-ai-ollama` + Dora env handling | `LocalRuntime` trait + `OllamaRuntime` | Skriuw's 12 unit + 4 ignored tests; new tests for lib-path env | Skriuw swaps crate; Dora replaces `ollama_installer` | Dora's Ollama manager UI works on the shared runtime |
| 5 | Tauri glue | `ai-tauri` | both | `ChannelSink`, `OperationRegistry`, `run_blocking`, `specta` feature | unit tests with fake channels | both thin their commands | No duplicated registry code in either app |
| 6 | TypeScript core + AI SDK adapter | `@ai/core`, `@ai/ai-sdk` | Betalingen (`routes/ai.ts`, `use-data-chat.ts`, `ai.test.ts`), Skriuw (`completion-consumer.ts`, `completion-bridge.ts`) | section 35 | fixture replay; conformance with injected fetch; consumer ordering; NDJSON round trip | none yet | Fixtures pass identically in Rust and TS |
| 7 | Migrate Skriuw fully | `@ai/tauri`, `@ai/react` | Skriuw | bridge, run hook | Skriuw renderer tests on the shared consumer/hook | renderer contract switch to `messages` | Skriuw AI e2e green |
| 8 | Migrate Betalingen | `@ai/core`, `@ai/ai-sdk` | Betalingen | route on `runtime.stream` + `toNdjsonStream`; client on `decodeNdjson` | existing 6 tests rewritten on the fake provider + fetch injection | bundle-size check for the island | CI green; deployed |
| 9 | Migrate Dora | all Rust crates, `@ai/core`, `@ai/tauri` | Dora | Dora prompts → messages; `SqliteUsageRecorder`; structured SQL via `generate_object` | Dora inherits conformance; new tests for SQL schema validation | studio module switch; key-rotation policy change; Gemini header | Dora adapters and installer deleted |
| 10 | Router (optional) | `ai-router`, `@ai/core` router | none yet (design from section 20) | `Router`, task profiles, in-memory health | scripted fallback tests | apps opt in per task | At least one app configures a two-route chain |
| 11 | Tasks package (optional) | `@ai/tasks` (+ Rust mirror if needed) | Skriuw built-ins | prompt data, `buildTaskRequest`, `parseList` | every prompt runs on the fake | Skriuw may keep its own copy | Stable ids agreed |

## 43. Final recommendation

1. **Should this SDK be built?** Yes. Three implementations exist, two of them in Rust with heavy overlap, and the third demonstrates the TypeScript need. The cost is bounded because most of the core already exists in Skriuw.
2. **Smallest useful scope?** `ai-core` + `ai-providers` (OpenAI-compatible table, Anthropic, Gemini, Ollama) + `ai-credentials` (env, session, keyring) + `@ai/core` + `@ai/ai-sdk`, with the spec/fixtures and the fake provider in both languages. Everything else is optional or later.
3. **Best Rust foundation?** Skriuw v2: `skriuw-domain` AI modules, `skriuw-ai`, `skriuw-ai-remote`, `skriuw-ai-ollama`. Dora contributes the Anthropic adapter, OpenAI/OpenRouter rows, key-pool ideas, and Ollama platform-env handling.
4. **Best TypeScript foundation?** Betalingen's `groq.ts`/`routes/ai.ts`/`use-data-chat.ts`/`ai.test.ts` for the AI SDK adapter, NDJSON stream, and test injection; Skriuw's `completion-consumer.ts`, `completion-bridge.ts`, `use-ai-run.ts` for the consumer, Tauri bridge, and React hook.
5. **Share contracts across languages?** Yes: JSON Schemas, enums, provider/model data, and golden fixtures, generated from Rust and validated in TS. No shared implementation, no new wire protocol.
6. **Use Vercel AI SDK internally in TS?** Yes, in one adapter package.
7. **Expose AI SDK types publicly?** No.
8. **Ollama runtime management separate from generation?** Yes: `ai-providers::ollama` (HTTP generation) vs `ai-ollama-runtime` (install/spawn/pull), mirroring Skriuw's two traits, split by dependency set.
9. **Tauri integration a separate layer?** Yes, a small helper crate/package; commands stay in apps; no plugin now.
10. **Routing/fallback core or optional?** Optional (`ai-router`), but the error taxonomy and `ModelInfo` capabilities it needs are core.
11. **Must remain in Dora:** schema context, dialect detection, SQL prompts and safety rules, the SQL result schema and insert/run flow, AES-GCM key table and keyring installer, usage table, all Tauri commands and studio UI, curated SQL-oriented model picks.
12. **Must remain in Skriuw:** editor extraction and apply, in-place review, plans, workspace prompts and shadowing, consent versions and disclosure, vault-tier UX, run-history storage and retention UI, opt-in gate, all commands and renderer stores.
13. **Must remain in Betalingen:** source allowlist and auth pass-through, IBAN masking, the Dutch data prompt, screen-context contract, dashboard publishing, route policy (auth, limits, 503), UI.
14. **Explicitly postponed:** tools/agents, embeddings, transcription, browser BYOK, Hono/Next.js packages, shared telemetry store, Tauri plugin, model tier heuristics in core, MCP.
15. **Initial monorepo structure:** as in section 32: `specs/`, `fixtures/`, `crates/{ai-core, ai-providers, ai-credentials, ai-ollama-runtime, ai-tauri, xtask}`, `packages/{core, ai-sdk, react, tauri}`, with `ai-router` and `tasks` added when phases 10–11 start.
16. **Safest first step:** move `skriuw-domain`'s five AI modules and `skriuw-ai` into `crates/ai-core` in the new repository, re-export them from `skriuw-domain`, point Skriuw at the new crate by path, and run Skriuw's existing AI test suite unchanged. That produces a working, tested core with zero user-visible change, and it turns Skriuw's `xtask` schema generation into phase 0 for free.

## 44. Source file index

### Dora (`/home/remcostoeten/dev/dora`)

| Path | Why it matters | Important symbols | Sections |
| --- | --- | --- | --- |
| `AGENTS.md` | repo instructions, port map | — | 2 |
| `package.json`, `turbo.json` | workspace layout, scripts (`ai:setup`) | — | 2 |
| `docs/ai-providers.md` | user docs for AI providers; stale (5 vs 11 providers) | — | 3, 7 |
| `docs/architecture-roadmap.md` | planned `dora-ai` crate, `AiPort`, ai-assistant as pilot | Track 4, Track 7 | 2, 3, 38 |
| `docs/product-roadmap.md` | stale AI description | §3-5 | 3 |
| `docs/specs/03-mcp-server.md` | Dora as MCP server (not tool calling) | — | 3, 23 |
| `apps/desktop/src-tauri/Cargo.toml` | deps: reqwest 0.12, tokio, specta, keyring, aes-gcm, zstd/tar/zip | — | 2, 16 |
| `apps/desktop/src-tauri/src/lib.rs` | `AppState` with cancel-flag maps; command registration; Ollama stop on exit | `AppState`, `run` | 3, 10, 19 |
| `apps/desktop/src-tauri/src/database/services/ai/mod.rs` | provider enum, request/response/event types, `AIService`, status, config | `AIProvider`, `AIRequest`, `AIResponse`, `AiStreamEvent`, `SchemaContext`, `AIService`, `resolve_model`, `ollama_endpoint` | 3, 6, 7, 8, 9 |
| `.../services/ai/client.rs` | trait, factory, rotation, SSE reader, mock | `AiClient`, `build_client`, `test_key`, `send_with_rotation`, `read_sse`, `should_rotate`, `MockClient` | 3, 7, 10, 16, 20 |
| `.../services/ai/compat.rs` | OpenAI-compatible specs and client | `CompatSpec`, `OPENAI_COMPAT`…`OPENROUTER_COMPAT`, `OpenAiCompatClient`, `build_request`, `fetch_model_ids` | 7, 16, 22, 36 |
| `.../services/ai/anthropic.rs` | Anthropic messages API client | `AnthropicClient`, `auth_headers` | 7, 16 |
| `.../services/ai/gemini.rs` | Gemini client (key in query) | `GeminiClient`, `generate_url`, `fetch_model_ids` | 7, 11 |
| `.../services/ai/ollama.rs` | Ollama HTTP client, status, catalog, pull | `OllamaClient`, `OllamaStatus`, `OllamaCatalogEntry`, `OllamaPullEvent`, `RECOMMENDED_MODELS` | 7, 12 |
| `.../services/ai/key_pool.rs` | env + stored key merge, round robin | `KeyPool`, `collect_env_keys` | 11, 16, 20 |
| `.../services/ai/models.rs` | curated catalogs, merge, tier heuristics, listing | `*_CURATED`, `merge_models`, `classify_*`, `list_*_models` | 8, 16 |
| `.../services/ai/prompts.rs` | SQL and chat system prompts, schema block | `build`, `build_system_prompt`, `build_chat_system_prompt`, `append_schema_block` | 3, 13, 14 |
| `.../services/ai/usage.rs` | usage capture, token estimate, pricing table | `AiUsageCapture`, `estimate_tokens_from_text`, `normalize_token_counts`, `pricing_for_model`, `record_usage` | 26 |
| `.../services/ai/errors.rs` | user-facing error copy | `http_error_message`, `request_error`, `body_mentions_model` | 3, 25 |
| `apps/desktop/src-tauri/src/database/commands/ai.rs` | 34 Tauri commands; schema context build; streaming forwarder; key tests | `ai_complete_stream`, `ai_abort_stream`, `build_schema_context`, `engine_for_connection`, `ai_keys_*`, `ai_pull_ollama_model`, `ai_install_ollama` | 3, 6, 10, 14, 19 |
| `apps/desktop/src-tauri/src/ollama_installer/{mod,download,paths,runtime}.rs` | managed install, download, paths, spawn | `install_managed`, `platform_download`, `fetch_with_progress`, `extract_archive`, `install_root`, `managed_binary_path`, `start_managed_server`, `stop_managed_server`, `apply_platform_env` | 12 |
| `apps/desktop/src-tauri/src/storage/ai_keys.rs` | encrypted key table access | `AiApiKeyRecord`, `ai_keys_*`, `migrate_legacy_gemini_key` | 11 |
| `apps/desktop/src-tauri/src/storage/ai_usage.rs` | usage table | `AiUsageInsert`, `AiUsageRow`, `ai_usage_*` | 26 |
| `apps/desktop/src-tauri/migrations/008.sql`, `009.sql` | `ai_api_keys`, `ai_usage` DDL | — | 11, 26 |
| `apps/desktop/src-tauri/src/security.rs` | AES-256-GCM, master key in keyring/file | `encrypt`, `decrypt`, `get_or_create_key` | 11 |
| `apps/desktop/src-tauri/src/credential_storage.rs` | keyring backend detection, install plan | `backend`, `status`, `install_plan`, `install` | 11 |
| `apps/desktop/src-tauri/src/error.rs` | IPC error shape | `Error`, `tag`, `BackendErrorShape` | 3, 25 |
| `apps/desktop/src-tauri/src/bindings.rs` | tauri-specta export to `packages/studio/src/lib/bindings.ts` | `generate_bindings`, `export_ts_bindings` | 2, 19 |
| `apps/desktop/src-tauri/src/database/contract.rs` | stale command registry descriptions | AI command entries | 3 |
| `packages/studio/src/lib/bindings.ts` | generated TS bindings | `commands.aiCompleteStream`, `AiStreamEvent`, `AiStatus`, … | 9, 19 |
| `packages/studio/src/features/ai-assistant/use-ai-chat.ts` | chat streaming hook, abort, batching | `useAiChat` | 6, 10 |
| `.../ai-assistant/build-prompt.ts` | history packing + UI context block | `buildChatPrompt`, `buildContextBlock` | 13, 14 |
| `.../ai-assistant/ai-actions.ts` | explain/fix prompts | `askAi`, `buildExplainQueryPrompt`, `buildFixErrorPrompt` | 6, 13 |
| `.../ai-assistant/assistant-response-parser.ts` | lenient JSON detection | `parseAssistantSqlResponse` | 22 |
| `.../ai-assistant/mock-ai.ts` | browser-demo mock provider | `streamMockText`, `buildMockChatResponse`, `buildMockSqlJson`, `buildMockAiStatus` | 6, 17, 27 |
| `.../ai-assistant/stream-batch.ts` | rAF batching | `createStreamBatcher` | 10 |
| `.../ai-assistant/store.ts` | persisted threads | `useAiAssistantStore` | 6 |
| `.../ai-assistant/types.ts`, `editor-context.ts`, `suggestions.ts`, `use-ai-status.ts`, `ai-selection-store.ts` | context types, editor draft store, suggestions, status | `AiAssistantContext`, `setAiEditorContext`, `buildDynamicSuggestions`, `useAiStatus`, `useAiSelection` | 6, 14 |
| `.../ai-assistant/ai-provider-section.tsx`, `ollama-models-section.tsx`, `ai-usage-section.tsx`, `components/model-id-input.tsx` | settings UI using commands | command calls | 6, 19 |
| `packages/studio/src/features/sidebar/components/ai-keys-section.tsx` | key CRUD/test UI | `commands.aiKeys*` | 6, 11 |
| `packages/studio/src/features/sql-console/components/ai-cmd-k.tsx` | Cmd+K SQL generation | `AiCmdK`, `parseLlmJson` | 6, 22 |
| `packages/studio/src/core/data-provider/context.tsx` | Tauri vs mock split | `useIsTauri`, `useAdapter` | 2, 17 |
| `__tests__/ai-actions.test.ts`, `__tests__/model-id-input.test.ts`, `packages/studio/src/features/ai-assistant/*.test.ts` | TS AI tests | — | 27 |
| `tools/scripts/generate-release.ts` | dev tooling using `@google/generative-ai` | — | 2 |
| `tools/scripts/setup-local-ai.ts` | dev script to install/start Ollama | — | 12 |

### Skriuw (`/home/remcostoeten/dev/skriuw`)

| Path | Why it matters | Important symbols | Sections |
| --- | --- | --- | --- |
| `AGENTS.md`, `README.md`, `docs/ARCHITECTURE.md` | architecture constraints, AI completion section | — | 2, 4 |
| `Cargo.toml`, `app/src-tauri/Cargo.toml` | workspace, reqwest 0.13 blocking, keyring per target, dbus-secret-service | — | 2, 16 |
| `docs/adr/0033-ai-provider-completion-seam.md` | accepted seam decision: trait, events, cancellation, credential tiers, consent, telemetry policy | — | 1, 4, 10, 11, 21, 26 |
| `docs/adr/0036-ai-results-are-reviewed-in-place.md` | review UX, streaming state shared in `useAiRun` | — | 4, 6 |
| `docs/specs/ollama-runtime.md`, `ai-editor-actions.md`, `ai-run-history.md` | implementation contracts | — | 12, 21, 26 |
| `project-management/ai/00-index.md`, `01-provider-seam-and-sdk.md`, `09-editor-actions.md` | planning (gitignored) for the AI platform waves | — | 4 |
| `contracts/README.md`, `contracts/generated/ai-*.schema.json`, `local-ai-*.schema.json`, `remote-ai-*.schema.json`, `built-in-prompts.json` | generated contracts and drift rule | — | 9, 18 |
| `crates/xtask/src/main.rs` | schema generation + check mode | `write_schema::<T>` | 18, 27 |
| `crates/skriuw-domain/Cargo.toml`, `src/lib.rs` | no HTTP/OS deps; AI re-exports | `pub use ai::…` | 2, 16 |
| `crates/skriuw-domain/src/ai.rs` | the seam | `AiCompletionRequest`, `AiCompletionParameters`, `AiCompletionDelta`, `AiUsage`, `AiProviderError`, `AiProviderErrorCategory`, `AiRecoveryAction`, `AiCompletionEvent`, `AiCompletionTerminal`, `AiCancellation`, `AiEventSink`, `AiComplete`, `MAX_AI_*` | 1, 4, 9, 10, 25 |
| `crates/skriuw-domain/src/local_ai.rs` | local runtime port | `LocalAiRuntime`, `LocalAiStatus`, `LocalAiModel`, `LocalAiProgress`, `LocalAiError`, `LocalAiErrorCategory` | 12 |
| `crates/skriuw-domain/src/remote_ai.rs` | credential port, catalog, vault states, consent | `AiCredential`, `AiCredentialSource`, `AiCredentialError`, `RemoteAiCatalog`, `RemoteAiModelListing`, `RemoteAiModelDirectory`, `CredentialVaultState`, `RemoteAiKeyTier`, `RemoteAiProviderState`, `RemoteAiConsent` | 8, 11 |
| `crates/skriuw-domain/src/prompt.rs` | built-in prompt library and workspace prompts | `BUILT_IN_PROMPTS`, `BuiltInPrompt`, `PromptInputShape`, `PromptParameters`, `WorkspacePrompt` | 13, 21 |
| `crates/skriuw-domain/src/ai_history.rs` | run records, token source, pricing, retention | `AiRunRecord`, `AiRunTokens`, `AiTokenSource`, `AiRunRecorder`, `AiModelPricing`, `estimate_ai_tokens`, `ai_run_cost_micros`, `AiHistorySettings` | 26 |
| `crates/skriuw-ai/src/lib.rs` | completion runtime and fake provider | `AiCompletionService`, `AiCompletionChannel`, `AiStartError`, `CompletionServiceSink`, `run_record`, `FakeAiProvider`, `FakeCompletionScript`, `FakeCompletionOutcome` | 4, 10, 16, 27 |
| `crates/skriuw-ai-ollama/src/lib.rs` | Ollama runtime + completion adapter | `OllamaRuntime`, `install_archive`, `download`, `verify_sha256`, `extract_archive`, `endpoint_is_loopback`, `read_json_capped`, tests | 12, 27 |
| `crates/skriuw-ai-remote/src/lib.rs` | remote provider adapter | `RemoteAiProvider`, `RemoteAiModelAuthority`, `remote_ai_catalog`, `stream_completion`, `status_error`, `transport_error`, `verify_credential`, `list_models`, `sse_payload`, tests | 7, 10, 20, 25, 27, 36 |
| `crates/skriuw-ai-remote/src/provider.rs` | provider kinds and dialects | `RemoteProviderKind`, `OpenAiCompatible`, `endpoint`, `authorize`, `completion_body`, `verification_body`, `parse_event`, `parse_model_listing` | 7, 36 |
| `crates/skriuw-ai-remote/models.json` | priced catalog v3 | — | 8 |
| `crates/skriuw-storage/src/lib.rs` (`AiRunHistory`), `crates/skriuw-sqlite/src/ai_history.rs`, `migrations/0018_ai_run_history.sql` | history persistence port and tables | `AiRunHistory`, `record_ai_run` | 26 |
| `app/src-tauri/src/ai.rs` | lazy runtime assembly, Tauri channel | `LazyAiCompletion`, `TauriCompletionChannel`, `UnpricedModels` | 4, 19 |
| `app/src-tauri/src/ai_credentials.rs` | keyring/session store, consent, vault detection | `AiCredentialStore`, `detect_vault_state`, `save_key`, `remove_key`, `take_key_for_verification` | 11 |
| `app/src-tauri/src/ai_models.rs` | fetched model store | `FetchedModelStore` | 8 |
| `app/src-tauri/src/ai_history.rs` | non-blocking recorder | `AiHistoryRecorder` | 26 |
| `app/src-tauri/src/ollama.rs` | operation registry | `OllamaManager`, `OperationRegistry` | 12, 19 |
| `app/src-tauri/src/commands/ai.rs` | 22 commands | `start_ai_completion`, `cancel_ai_completion`, `*_ollama_*`, `remote_ai_*`, `save_remote_ai_key`, `verify_remote_ai_key`, `ai_run_history` | 6, 19 |
| `app/src-tauri/src/lib.rs`, `state.rs` | wiring, `AppState` | — | 4 |
| `app/src/contracts/ai.ts` | renderer mirror types | all AI types | 9 |
| `app/src/bridge/runtime.ts` | desktop-only guard | `requireDesktopRuntime`, `invoke` | 2, 11 |
| `app/src/features/ai/completion-bridge.ts` | start/cancel bridge with `AbortSignal` | `startAiCompletion`, `AiCompletionHandle` | 10, 17 |
| `app/src/features/ai/completion-consumer.ts` | request id + sequence gate | `createAiCompletionConsumer` | 10, 17 |
| `app/src/features/ai/use-ai-run.ts` | run hook, rAF flush, retry | `useAiRun` | 10, 17, 24 |
| `app/src/features/ai/editor-actions.ts` | actions as data, request builder | `AI_EDITOR_ACTIONS`, `buildAiActionRequest`, `aiActionUserPrompt`, `aiActionOrigin` | 6, 13, 21 |
| `app/src/features/ai/editor-action-model.ts` | run phases, apply refusal | `AiActionRun`, `runWithDelta`, `runWithTerminal`, `applyRefusal` | 6 |
| `app/src/features/ai/editor-action-apply.ts` | input extraction, transactions | `actionInputText`, `replaceRangeTransaction`, `appendTaskPlanTransaction`, `appendTagPlanTransaction` | 14 |
| `app/src/features/ai/action-plan.ts` | list parsing | `parseTaskPlan`, `parseTagPlan` | 22 |
| `app/src/features/ai/inline-suggestion.tsx`, `editor-action-host.tsx`, `editor-action-controller.ts` | review UI, host, palette wiring | `AiInlineSuggestion`, `requestAiAction` | 6 |
| `app/src/features/ai/model-selection.ts`, `model-options.ts`, `ollama-model-catalog.ts`, `remote-ai-model.ts` | model identity, inventory groups, curated Ollama picks | `AiModelSelection`, `resolveAiModel`, `aiModelGroups`, `OLLAMA_MODEL_CATALOG` | 8 |
| `app/src/features/ai/remote-ai-bridge.ts`, `ollama-bridge.ts` | invoke wrappers, progress operations | `runProgressOperation`, `verifyRemoteAiKey` | 12, 17 |
| `app/src/features/ai/prompt-library.ts`, `built-in-prompts.ts` | prompt library with shadowing | `promptLibraryEntries`, `BUILT_IN_PROMPTS` | 13 |
| `app/src/features/ai/opt-in-gate.tsx` | gate and abort on opt-out | `AiOptInGate`, `selectAiEnabled` | 6 |
| `app/src/features/ai/playground-model.ts`, `prompt-playground.tsx`, `usage-model.ts` | playground, usage roll-ups | `playgroundModelGroups`, `usageTotals` | 6, 26 |
| `app/__tests__/features/ai/*.test.ts` (16 files), `app/e2e/run-native-ai.mjs` | renderer and native tests | — | 27 |
| `v1/apps/web/src/app/api/ai/route.ts`, `v1/apps/web/src/domain/ai/{constants,prompts,provider-errors,provider-keys,usage}.ts`, `v1/apps/web/src/features/ai/service.ts`, `v1/apps/desktop/src-tauri/src/ai/{mod,cloud,installer,ollama}.rs` | frozen v1: Vercel AI SDK v6 usage, per-action model defaults, shared prompts.json, third Ollama installer | `ACTION_MODEL_DEFAULTS`, `buildAiPrompt`, `classifyAiProviderError`, `ai_complete_stream` | 4, 7, 15, 20 |

### Betalingen (`/home/remcostoeten/dev/betalingen`)

| Path | Why it matters | Important symbols | Sections |
| --- | --- | --- | --- |
| `CLAUDE.md`, `README.md` | rules (runtime-agnostic `src/`), Groq assistant docs | — | 2, 5 |
| `package.json`, `node_modules/ai/package.json`, `node_modules/@ai-sdk/groq/package.json` | `ai@7.0.93`, `@ai-sdk/groq@4.0.37` | — | 15 |
| `.env.example` | `GROQ_API_KEY`, `GROQ_MODEL` | — | 11 |
| `src/lib/ai/groq.ts` | AI SDK usage | `answerWithGroq`, `GroqOptions` | 5, 15 |
| `src/lib/ai/contracts.ts` | zod request/event contracts | `ScreenContextSchema`, `ChatRequestSchema`, `ChatEventSchema` | 9 |
| `src/lib/ai/context.ts` | allowlist, context build, masking, prompt | `isContextSource`, `buildScreenContext`, `DATA_ASSISTANT_PROMPT` | 13, 14 |
| `src/routes/ai.ts` | `POST /ai/chat` NDJSON streaming, abort, timeout | `aiRoutes` | 5, 10 |
| `src/app.ts` | app assembly, `options.ai`, `/ai/assistant.js` | `createApp` | 2, 5 |
| `src/ui/assistant/use-data-chat.ts` | client streaming, abort, retry | `useDataChat` | 5, 10 |
| `src/ui/assistant/main.tsx` | React island, disclosure text | `DataChat`, `Assistant`, `update` | 5 |
| `src/ui/dashboard.html.txt` | source collection and lazy load | `assistantSources`, `publishAssistant` | 14 |
| `src/lib/ai/ai.test.ts` | injected-fetch tests | — | 27 |
| `src/lib/budgetplan-import.ts` | deterministic PDF parser (not AI) | `extractText` via `unpdf` | 5 |
| `.github/workflows/ci.yml`, `vercel.json` | CI and deploy gates | — | 2 |
