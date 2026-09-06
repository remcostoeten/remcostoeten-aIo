# ADR 0001: SDK scope

Status: accepted at Phase 0 closure, 2026-09-06. The blocking decisions formerly cited here are settled in `architecture.md` §5; Phase 1 now waits only on an explicit instruction to begin. This revision supersedes the original broader "shipped first" list.

## Context

The audit documents duplicated completion mechanics in Skriuw, Dora, and Betalingen. Its recommendations also include capabilities, wrappers, prompts, and routing that current execution does not require. Extraction and redesign must be separate deliverables.

## Decision

The SDK owns AI execution. Applications own product meaning.

### Phase 1 core

One production crate, `crates/ai-core`, contains the provider-neutral portion of Skriuw's `ai.rs` and `skriuw-ai`: request/event/error contracts, existing validation and bounds, cancellation, sink and channel ports, the synchronous completion trait, completion service, and deterministic fake provider. Recording enters core only as the metadata-only summary port selected in `architecture.md` §5 and specified in `contracts.md` §2.8, plus the token estimation and cost arithmetic it needs. The legacy prompt-bearing history record does not.

Preserve existing serialized names, accepted identifier grammar, empty-prompt behavior, sampling bounds, error categories, explicit terminals, and inactive `retryCount`. Keep `origin` as a separate service argument. Do not require `id`, `verify`, `listModels`, or `modelInfo` on the completion trait.

Use modules within this crate. Schema generation/checking may be a development-only target of the same crate; create a separate `xtask` crate only if a concrete build/dependency need emerges. It is not an SDK dependency.

### Later provider boundary

`crates/ai-providers` earns its boundary through HTTP dependencies and provider protocols. Provider SSE/NDJSON parsing, endpoint construction, authorization headers, model discovery, catalog data, and provider response decoding live there. Core does not need an SSE parser merely because the parser is framework-independent.

`packages/core` and `packages/ai-sdk` earn separate boundaries because renderers need contracts and event consumption without the Vercel AI SDK. The adapter exposes our typed factories; third-party model instances are internal.

Credential resolution belongs at the provider boundary; persistence and consent remain application-owned. Do not extract Skriuw's consent/vault enums into core, and do not add unused credential resolvers in Phase 1.

### Optional platform boundaries

- Ollama generation and lifecycle remain separate. Lifecycle needs process/filesystem/archive dependencies. The application composes startup and generation; the provider crate does not gain a lifecycle dependency, even behind a feature.
- Tauri helpers require evidence that common mechanisms remain after migration. Command names and generated binding ownership remain in applications. Specta support is a later explicit compatibility decision.
- React hooks require demonstrated shared behavior. No UI package is scheduled.
- Routing has no initial package, port, attempt list, fallback flags, or health store in core.

### Application concerns

| Owner | Remains application-owned |
| --- | --- |
| Dora | Schema context, dialect and SQL prompts, SQL output schema and safety, key pool policy and storage, usage persistence, recommended models, commands, UI |
| Skriuw | Editor extraction/apply/review, task/tag parsing, built-in and workspace prompts, consent/disclosure, vault/session storage, prompt retention and history persistence, lazy startup and opt-in gate |
| Betalingen | Source allowlist, authorization pass-through, masking, financial prompt and screen context, route limits/auth/503 policy, browser UI |
| All applications | Settings, credential persistence, product defaults, result interpretation, command/HTTP compatibility |

No application prompt catalog or context builder is extracted. A provider's minimal verification prompt may live in its adapter. A future generic structured-output instruction requires explicit opt-in and belongs to provider execution, not product context assembly.

## Non-goals

No agents, workflows, chains, tool execution, embeddings, transcription, prompt marketplace, task framework, React UI kit, fixed-command Tauri plugin, Hono/Next.js package, shared telemetry database, browser secret store, or replacement for Vercel AI SDK.

No public `any`, broad `unknown`, provider metadata escape hatch, or arbitrary JSON option bag. Dynamic provider bodies are decoded inside adapters. Future schema payloads need a bounded schema contract; they do not justify weakening all request types.

## Consequences

Phase 1 is an extraction with a reviewable compatibility inventory plus the two approved edge-behavior changes (D1, D2), not implementation of every audit sketch. New request shapes and behavior require a later approved contract gate. Fewer packages are created, and no application must migrate its renderer simply to consume the extracted Rust core.
