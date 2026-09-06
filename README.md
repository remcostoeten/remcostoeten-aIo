# AI SDK

A small, provider-neutral AI foundation extracted from existing Rust and TypeScript applications.

The SDK owns completion execution. Applications own context, prompts, consent, persistence, UI, and result interpretation.

## Status

**Phases 0 and 1 complete. Phase 2 has not begun and is not authorized.**

`crates/ai-core` now exists: the completion contracts, ports, service, deterministic fake and recorder port, extracted from Skriuw and covered by 74 offline tests plus a schema drift check. It has no HTTP client, no credential store, no framework and no async runtime, and nothing in Skriuw, Dora or Betalingen was modified. See [the extraction inventory](docs/extraction-inventory.md) for what moved and the two approved behavior changes.

The Phase 0 review found that the original Phase 1 plan combined extraction with incompatible contract changes and overstated Skriuw's runtime guarantees. The revised plan separates the existing extraction contract from later design candidates. Both blocking decisions are now taken: Phase 1 applies narrow lifecycle hardening after source characterization, and uses a metadata-only recorder summary with the request borrowed for the callback, leaving prompt retention in the application. See [review findings](docs/architecture.md#4-phase-0-critical-review), [decisions](docs/architecture.md#5-decisions-taken), and [verdict](docs/architecture.md#6-verdict).

## Evidence

| Application | Read-only path | Relevant foundation |
| --- | --- | --- |
| Skriuw | `/home/remcostoeten/dev/skriuw` | Provider-neutral Rust contracts, completion service, deterministic fake, provider fixtures |
| Dora | `/home/remcostoeten/dev/dora` | Additional protocols, SQL consumer, multi-key behavior, Ollama lifecycle |
| Betalingen | `/home/remcostoeten/dev/betalingen` | TypeScript streaming consumer and internal Vercel AI SDK integration |

The requested `ai-sdk-architecture-research.md` does not exist in this checkout. The complete audit is [architecture research across my AI apps](dev-docs/knowledge/architecture-research-across-my-ai-apps.md), the path named by `AGENTS.md` and `CLAUDE.md`. It is historical evidence, not permission to implement its proposals. Reference implementation wins when it disagrees with documentation about existing behavior.

## The extracted core

One production crate, `crates/ai-core`: the completion contracts and their validators, cancellation, the synchronous sink-based completion trait, the service, a deterministic fake, and a metadata-only recording port. Skriuw's request/event JSON and error vocabulary are preserved; only the edge behaviors in [contracts §3.3](docs/contracts.md) changed. Schema generation is development tooling behind a feature flag, not a second production package.

```
cargo test --all-features
cargo run -p ai-core --features schema-tool --bin ai-schema -- generate --check
```

No HTTP adapters, message-history redesign, retries, structured-output engine, credential stores, codecs, async facade, or framework helpers are in it.

Later, separately approved phases may add `crates/ai-providers`, `packages/core`, and `packages/ai-sdk` (the Vercel adapter). Ollama lifecycle has a distinct platform boundary; Tauri and React helpers remain conditional. Rust and TypeScript share semantic contracts and fixtures, not a runtime bridge.

## Documents

- [Architecture, findings, decisions taken, and verdict](docs/architecture.md)
- [Phase 1 extraction inventory](docs/extraction-inventory.md)
- [Extraction contracts and deferred design constraints](docs/contracts.md)
- [Phase gates](docs/roadmap.md)
- [ADR 0001: scope](docs/decisions/0001-sdk-scope.md)
- [ADR 0002: cross-language contracts](docs/decisions/0002-cross-language-contracts.md)
- [ADR 0003: provider boundary](docs/decisions/0003-provider-boundary.md)
- [Repository instructions](AGENTS.md)
