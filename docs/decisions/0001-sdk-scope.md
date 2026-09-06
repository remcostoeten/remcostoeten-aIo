# ADR 0001: SDK scope

Status: accepted (Phase 0, 2026-09-06)

## Context

Three applications carry independent AI implementations (audit: `dev-docs/knowledge/architecture-research-across-my-ai-apps.md`). The same mechanisms are written repeatedly: OpenAI-compatible chat clients twice in Rust and once via the Vercel AI SDK; Gemini twice with a security-relevant difference; Ollama install/spawn three times; SSE/NDJSON decoding in every repo; three incompatible streaming event shapes; three error styles of which only Skriuw's is machine-readable. Meanwhile everything that gives these features product meaning (schema context, note extraction, financial source allowlists, prompts, consent, key storage, usage tables, UI) is different in each application and correctly so.

The question is where the line goes.

## Decision

The SDK owns **AI execution**. Applications own **product meaning**. The line is the `CompletionRequest`: everything needed to turn messages into events is SDK; everything needed to produce messages or interpret events is application.

### Core (shipped first; `crates/ai-core`, `packages/core`)

- Contracts: `ModelRef`, `ModelInfo`, `Capability`/`CapabilitySupport`, `Role`/`ContentPart`/`Message`, `CompletionRequest`/`CompletionParameters`/`ResponseFormat`, `CompletionDelta`/`CompletionEvent`/`CompletionTerminal`, `Usage`/`UsageSource`/`FinishReason`, `ProviderError`/`ErrorCategory`/`RecoveryAction`, `CredentialSource`/`Credential`/`CredentialError`, `Provider`, `Runtime`/`CompletionOutcome`, `RunRecord`/`RunRecorder`/`Pricing`
- Validation and bounds for all of the above
- The runtime: request registry, cancellation, deadline, first-terminal-wins, retry-before-first-delta, recording hook
- Structured-output strategy selection and output validation
- Deterministic fake provider with scripted outcomes
- SSE and NDJSON decoders; NDJSON encoder (TypeScript)
- Environment-variable and in-memory session credential resolvers
- Byte/token estimation and cost computation helpers
- The shared specification (`specs/`, `fixtures/`) generated from the Rust core

### Providers (`crates/ai-providers`, `packages/ai-sdk`)

- One shared skeleton per language (HTTP, stream reader, bounds, cancellation, deadline, status mapping, usage extraction)
- Descriptor-driven OpenAI-compatible adapter fed by `specs/data/providers.json`
- Custom adapters: Anthropic, Gemini, Ollama generation
- `verify` and `listModels` per adapter
- Priced model catalog data and merge logic
- In TypeScript, the Vercel AI SDK as the internal implementation of the adapter package

### Platform integrations (optional, later, each behind its own phase gate)

- `crates/ai-ollama-runtime`: local runtime lifecycle (detect, install, spawn, stop, pull, remove, progress); separate from generation by dependency set and port
- `crates/ai-tauri`, `packages/tauri`: channel sink, operation registry, blocking-run helper, renderer bridge; conditional on demonstrated duplication (Phase 7)
- `crates/ai-router`, TS `createRouter`: optional routing/fallback (Phase 9)
- `packages/react`: run hook; not scheduled

### Application concerns (never in the SDK)

| Owner | Stays |
| --- | --- |
| Dora | `SchemaContext`, dialect detection, SQL prompts and safety rules, the `{sql, explanation, warnings}` shape, insert/run flow, AES-GCM key table, `pkexec` keyring installer, usage table, all Tauri commands, studio UI, tier heuristics, recommended SQL models |
| Skriuw | editor extraction and ProseMirror/CodeMirror apply, in-place review, task/tag plans and their parsers, built-in and workspace prompts and shadowing, consent versions and disclosure copy, vault-tier UX, run-history persistence and retention, opt-in gate, all commands and renderer stores, recommended writing models |
| Betalingen | source allowlist, auth pass-through, IBAN masking, the Dutch data-assistant prompt, screen-context contract, dashboard publishing, route policy (auth, body limit, 503), UI |
| every app | credential persistence, usage persistence, settings persistence, command/route surfaces, React stores, model recommendation lists |

### Postponed features (require a consumer and a new ADR)

Routing/fallback (Phase 9 at the earliest), Ollama lifecycle (Phase 6), Tauri helpers (Phase 7), task-prompt data package, React hooks, Hono/Next.js helpers, tool calling (a `ContentPart` variant is reserved by the tagged-union design, nothing more), embeddings, transcription, browser BYOK, Specta derives (feature, Phase 7).

## Why application context construction stays application-owned

1. **It is where the domain lives.** Dora's `append_schema_block` encodes table/column/index truncation limits and dialect rules; Betalingen's `buildScreenContext` encodes an authorization pass-through and IBAN masking; Skriuw's `actionInputText` encodes ProseMirror selection semantics. None of that is AI knowledge, and an SDK that owned it would have to depend on a database schema type, a financial record type, and an editor state type.
2. **The seam is already context-free in two of three apps.** Skriuw's request carries no context object (`crates/skriuw-domain/src/ai.rs`); Betalingen renders context into the user message inside the route. Dora's `AIRequest.context: Option<SchemaContext>` is the one place the provider layer knows about a product type, and it is the source of Dora's coupling (every adapter calls `prompts::build`). The SDK must not inherit the outlier.
3. **Preview equals payload.** Skriuw deliberately sends only what the user can see so that the reviewed suggestion corresponds exactly to what the model received. Centralized context assembly would break that guarantee for the app that cares most about it.
4. **Prompts encode policy.** "NEVER emit DROP without WHERE", "Antwoord in het Nederlands", "Keep the language of the original" are product decisions with safety and localization consequences. Centralizing them would make the SDK the owner of every application's safety posture.
5. **What can be shared is small and mechanical**: a `Message` model so history is not string-packed, and byte/token estimation so applications can budget context against `contextWindowTokens`. Those are in core. A context-block helper (label + data + "data, not instructions" framing + byte budget) is a candidate for a later utility once two applications adopt the same shape; it is not in v1.

## Non-goals

The SDK is not, and must not grow into:

- an agent framework, tool-execution loop, or MCP client (no consumer sends tools; Dora's MCP spec is Dora *serving* tools)
- a workflow, chains, or pipeline engine (every existing feature is one request)
- a prompt registry, prompt marketplace, or universal prompt store (Skriuw's workspace prompts are workspace data; Dora's prompts are code)
- a task API in core (`generateSql()`, `rewriteNote()`, `askAboutScreen()` are application functions)
- an embeddings, vector, or retrieval layer (only frozen Skriuw v1 had embeddings)
- a transcription or image-generation layer
- a React UI kit, chat component set, or model picker
- a Tauri plugin with fixed commands, permissions, or generated bindings
- a Hono, Next.js, or Bun framework package
- a database abstraction, settings store, or application state library
- a shared usage database or telemetry uploader (both desktop apps forbid uploads; the SDK emits records to a port)
- a replacement for, or a public re-export of, the Vercel AI SDK
- a custom HTTP client abstraction beyond `reqwest`/`fetch` injection
- a full TypeScript reimplementation of provider wire protocols
- a browser BYOK credential store
- a source of model tier heuristics (`flagship/balanced/fast` by substring)
- a place for provider-specific option bags (`extra`, `options`, `metadata`, `Record<string, unknown>`, `serde_json::Value` on requests)
- an automatic local→remote fallback

Capabilities are added only when supported by an existing consumer or a clearly approved upcoming requirement, and each addition gets its own ADR.

## Consequences

- The initial repository has two Rust crates and two TypeScript packages of substance, plus `xtask`, `specs/`, and `fixtures/`. Empty placeholder packages are not created.
- Migrations move application prompt builders *out* of provider layers (Dora) rather than *into* the SDK.
- Any pull request adding a provider name, prompt string, storage engine, or application type to `ai-core`/`packages/core` is rejected on scope grounds.
