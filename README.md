# AI SDK

A small, provider-neutral AI foundation extracted from existing Rust and TypeScript applications.

The SDK owns completion execution. Applications own context, prompts, consent, persistence, UI, and result interpretation.

## Status

**Phases 0 through 4 complete. Phase 5 has not begun and is not authorized.**

Spec version 0.2.0. Four units exist:

| Unit | What it is |
| --- | --- |
| `crates/ai-core` | Rust completion contracts, ports, service, deterministic fake, recorder port |
| `crates/ai-providers` | Seven remote descriptors, the Gemini dialect, Ollama generation, credential and model-authority ports |
| `packages/core` | The same contracts, run lifecycle, event consumer, NDJSON helpers and fake in TypeScript — zero dependencies |
| `packages/ai-sdk` | The Vercel AI SDK adapter and typed provider factories |

`./scripts/check.sh` runs everything: 148 Rust tests, 122 TypeScript tests, clippy, fmt and the schema drift check. No step needs a network, a provider key, or a sibling checkout.

Skriuw runs on the Rust crates ([integration notes](docs/integration-skriuw.md)); nothing in Dora or Betalingen has been modified. Distribution is unresolved for both languages — the crates are path dependencies and the packages are workspace-only.

## Evidence

| Application | Read-only path | Relevant foundation |
| --- | --- | --- |
| Skriuw | `/home/remcostoeten/dev/skriuw` | Provider-neutral Rust contracts, completion service, deterministic fake, provider fixtures |
| Dora | `/home/remcostoeten/dev/dora` | Additional protocols, SQL consumer, multi-key behavior, Ollama lifecycle |
| Betalingen | `/home/remcostoeten/dev/betalingen` | TypeScript streaming consumer and internal Vercel AI SDK integration |

The requested `ai-sdk-architecture-research.md` does not exist in this checkout. The complete audit is [architecture research across my AI apps](dev-docs/knowledge/architecture-research-across-my-ai-apps.md), the path named by `AGENTS.md` and `CLAUDE.md`. It is historical evidence, not permission to implement its proposals. Reference implementation wins when it disagrees with documentation about existing behavior.

## The seam

A validated request goes in; ordered deltas and exactly one terminal come out. The SDK knows nothing about products, prompts, persistence or UI.

```ts
import { buildRequest, consumeEvents, createRuntime } from '@ai-sdk-local/core'
import { createGroqProvider, staticCredential } from '@ai-sdk-local/ai-sdk'

const provider = await createGroqProvider({
  credentials: staticCredential(serverSideKey),
  models: { permits: (_, modelId) => modelId === 'llama-3.3-70b-versatile' },
})

const built = buildRequest({
  requestId, providerId: 'groq', modelId: 'llama-3.3-70b-versatile',
  systemPrompt: 'Be brief.',
  messages: conversation,
  parameters: { maxOutputTokens: 1800 },
})
if (!built.ok) return reject(built.error.reason)

const started = createRuntime({ providers: [provider] }).stream(built.request)
if (!started.ok) return reject(started.error.reason)

const outcome = await consumeEvents(started.events)
```

More of both languages — streaming over NDJSON, cancellation, untrusted requests, run accounting, model listing — is in [example usage](docs/examples.md).

Rust and TypeScript share semantic contracts and fixtures, not a runtime bridge. Both read `specs/fixtures/`; both run the same eight fake scripts and must produce the same segmentation, terminal, error category and usage.

No retries, structured output, routing, provider fallback, agents, or framework helpers exist in any of it. Ollama lifecycle has a distinct platform boundary; Tauri and React helpers remain conditional.

```
./scripts/check.sh
```

## Documents

- [Example usage, in both languages](docs/examples.md)
- [Architecture, findings, decisions taken, and verdict](docs/architecture.md)
- [Contracts and deferred design constraints](docs/contracts.md)
- [Phase gates](docs/roadmap.md)
- [Phase 1 extraction inventory: `ai-core`](docs/extraction-inventory.md)
- [Phase 2 extraction inventory: `ai-providers`](docs/extraction-inventory-providers.md)
- [Phase 3: what consuming the SDK cost Skriuw](docs/integration-skriuw.md)
- [Phase 4: TypeScript core and Vercel adapter](docs/typescript-core.md)
- [ADR 0001: scope](docs/decisions/0001-sdk-scope.md)
- [ADR 0002: cross-language contracts](docs/decisions/0002-cross-language-contracts.md)
- [ADR 0003: provider boundary](docs/decisions/0003-provider-boundary.md)
- [ADR 0004: conversation history and an output token limit](docs/decisions/0004-history-and-token-limit.md)
- [Repository instructions](AGENTS.md)
