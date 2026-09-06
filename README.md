# AI SDK

A small, provider-agnostic AI foundation for Rust and TypeScript applications.

The SDK owns **AI execution**: completion requests, streaming events, cancellation, timeouts, provider adapters, typed provider errors, model identity and capabilities, credential resolution ports, structured output, usage metadata, and a deterministic fake provider. Applications own **product meaning**: context, prompts, consent, persistence, UI, and what to do with a result.

## Status

**Phase 0: architecture and contracts.** No production code exists yet. This repository currently contains only documentation. See [`docs/roadmap.md`](docs/roadmap.md) for the phase gates.

## Why it exists

Three applications by the same author each grew their own AI layer:

| Application | Path | AI stack today |
| --- | --- | --- |
| Skriuw | `/home/remcostoeten/dev/skriuw` | Rust crates (`skriuw-domain`, `skriuw-ai`, `skriuw-ai-remote`, `skriuw-ai-ollama`) behind Tauri commands |
| Dora | `/home/remcostoeten/dev/dora` | Rust module tree under `apps/desktop/src-tauri/src/database/services/ai/` behind Tauri commands |
| Betalingen | `/home/remcostoeten/dev/betalingen` | TypeScript, Bun/Hono, Vercel AI SDK (`src/lib/ai/`, `src/routes/ai.ts`) |

The audit in [`dev-docs/knowledge/architecture-research-across-my-ai-apps.md`](dev-docs/knowledge/architecture-research-across-my-ai-apps.md) found the OpenAI-compatible client written twice in Rust, Gemini written twice (once with the key in the URL), Ollama install/spawn written three times, three incompatible streaming event shapes, and only one machine-readable error taxonomy. Skriuw's seam (`crates/skriuw-domain/src/ai.rs`, `crates/skriuw-ai`) is already the shape the SDK needs. The SDK is an **extraction** of that seam, generalized for the other two consumers, not a new framework.

These three repositories are read-only reference implementations and the first consumers.

## Languages

- **Rust**: the canonical core. Extracted from Skriuw first. Synchronous, sink-based provider trait; no async runtime, HTTP client, Tauri, or keyring dependency in the core crate.
- **TypeScript**: a native core with the same semantic contracts, implemented over the Vercel AI SDK behind an adapter. Runs in Bun, Node, browsers, and serverless runtimes. The core has no React, Hono, Tauri, Node-only, or Vercel AI SDK dependency.

Rust and TypeScript share a **specification** (JSON Schema, enums, provider and model data, golden fixtures), not an implementation and not a runtime bridge.

## High-level architecture

```text
application
  -> builds product-specific context and prompts (app-owned)
  -> CompletionRequest { requestId, model: ModelRef, messages, parameters }
  -> Runtime (registry, cancellation, deadline, first-terminal-wins, recording)
  -> Provider (descriptor-driven OpenAI-compatible, or custom adapter)
  -> model
  -> CompletionEvent stream: delta* then exactly one terminal
       (done | cancelled | timeout | provider_error)
  -> application interprets the result
```

Dependency direction is strictly inward: applications -> integrations -> providers -> core. The core knows no provider names, no application types, no prompt text, no storage.

Planned units (created only when their phase begins):

```text
crates/ai-core             contracts, validation, runtime, fake provider, codecs, ports
crates/ai-providers        OpenAI-compatible (data-driven), Anthropic, Gemini, Ollama generation
crates/ai-ollama-runtime   install / spawn / pull / remove (separate from generation)
crates/ai-tauri            channel sink, operation registry, blocking-run helper (conditional)
packages/core              @remcostoeten/ai-core: TypeScript contracts, consumer, codecs, runtime, fake provider
packages/ai-sdk            @remcostoeten/ai-vercel: Vercel AI SDK adapter (the only unit that imports `ai`)
specs/                     JSON Schema, enums, provider/model data, fixtures
```

## Non-goals

Not an agent framework, workflow engine, tool-execution loop, prompt registry, embeddings/vector layer, React UI kit, Tauri plugin with fixed commands, Hono or Next.js framework, shared telemetry database, or replacement for the Vercel AI SDK. No routing/fallback until the basic runtime has real consumers. No task APIs such as `generateSql()` in core. No Ollama process management in the core or provider crates.

## Where decisions live

- [`docs/architecture.md`](docs/architecture.md): boundaries, dependency rules, runtime and streaming model, testing strategy.
- [`docs/contracts.md`](docs/contracts.md): conceptual v1 contracts in pseudocode, stream invariants, error semantics.
- [`docs/roadmap.md`](docs/roadmap.md): gated implementation phases.
- [`docs/decisions/`](docs/decisions/): architecture decision records.
  - `0001-sdk-scope.md`
  - `0002-cross-language-contracts.md`
  - `0003-provider-boundary.md`
- [`AGENTS.md`](AGENTS.md): canonical repository instructions for contributors and agents.
