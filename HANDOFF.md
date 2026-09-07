# Session handoff: Phase 4 is closed

Updated: 2026-09-07.
Workspace: `/home/remcostoeten/dev/remcostoeten-aIo`.

## State

**Phases 0 through 4 are COMPLETE. Phase 5 has not begun and is not
authorized.** Spec version is 0.2.0.

Four units:

- `crates/ai-core` — the Rust completion seam.
- `crates/ai-providers` — seven remote descriptors, the Gemini dialect, Ollama
  generation, credential and model-authority ports.
- `packages/core` — the same contracts, run lifecycle, event consumer, NDJSON
  helpers and deterministic fake in TypeScript. Zero dependencies.
- `packages/ai-sdk` — the Vercel AI SDK adapter and typed provider factories.

Read before touching anything:

- `docs/typescript-core.md` — what Phase 4 built, the symbol map, the six shape
  differences between the languages, the two vendor quirks, and what is not
  implemented.
- `docs/decisions/0004-history-and-token-limit.md` — the contract gate that ran
  first, and the delta it approved.
- The earlier inventories: `docs/extraction-inventory.md`,
  `docs/extraction-inventory-providers.md`, `docs/integration-skriuw.md`.

## Verification at closure

`./scripts/check.sh` is green end to end:

- `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features
  -- -D warnings` clean, and clean again with `--no-default-features`.
- `cargo test --workspace --all-features`: 148 passed, 0 failed, 2 ignored
  (live).
- Both schema tools pass `--check`: 7 core schemas, 1 provider schema.
- `tsc --build --force` clean under strict TypeScript with
  `noUncheckedIndexedAccess` and `exactOptionalPropertyTypes`.
- `bun test packages`: 122 passed, 0 failed, across 8 files.

No step needs a network, a provider key, or a sibling checkout.

`/home/remcostoeten/dev/{skriuw,dora,betalingen}` are untouched.

## What Phase 4 proved

The two languages implement one seam, and the claim is checked rather than
asserted. Both read `specs/fixtures/` for wire conformance. Both run the eight
shared fake scripts in `specs/fixtures/fake/` and must agree on segmentation,
identity, sequence, terminal kind, error category and usage — the list ADR 0002
defines. The TypeScript suite also asserts the committed schemas match what its
decoder accepts, so a Rust-only contract change fails on the TypeScript side.

The boundary holds. `core` bundles for the browser with no `ai`, no vendor
package, no `process.env` and no Node built-in, and its emitted JavaScript is
executed under real Node in the suite. `createAdapter` and `ModelFactory` are
unexported, so no third-party type is reachable from either entry point and
there is no `fromLanguageModel`. `maxRetries` is 0 and a 429 is attempted
exactly once. No provider response body reaches a caller.

The consumer shape works. `packages/ai-sdk/test/consumer-shape.test.ts` runs
Betalingen's actual shape — a multi-turn Dutch conversation, a system prompt, a
1800-token ceiling, NDJSON out, key on the server — end to end against a fixture
`fetch`, and asserts the conversation reaches the provider as four turns rather
than one flattened prompt.

## What it did not prove, and what it broke

**Distribution is still unresolved, now in two languages.** The crates are path
dependencies into a sibling checkout; the packages are workspace-only under a
placeholder `@ai-sdk-local/*` scope, unpublished. This was the open question at
the end of Phase 3 and Phase 4 did not answer it.

**Skriuw no longer builds against this version.** ADR 0004 adds two fields to
two public Rust structs, which breaks struct-literal construction. In Skriuw
that is six test helpers and one call site on branch
`ai-sdk-phase-3-extraction`; adding `prior_messages: Vec::new()` and
`max_output_tokens: None` is the entire fix. Skriuw was deliberately not
modified — that needs its own authorization — so the Phase 3 integration proof
is pinned to 0.1.0 until someone does it. Nothing in this repository depends on
it.

**Nothing has touched a live provider.** Every adapter test runs against a
fixture `fetch`. That keeps the suite offline, and it also means no one has
confirmed that Groq, Gemini or Anthropic actually behave as the fixtures claim.
Two Rust tests are `#[ignore]`d as live checks; there is no TypeScript
equivalent.

Also still unresolved from earlier phases: no retries or key rotation, no
structured output, no provider fallback or routing, no run recording in
TypeScript, `sse_payload` is a line helper rather than an SSE decoder, and H3 (a
Rust stream reaching the 4 MiB cap is indistinguishable from one that finished)
still has no fixture.

The Rust adapters do not carry `priorMessages` or `maxOutputTokens`. They refuse
a request containing either, with a typed `rejected_request` before any socket
opens, rather than dropping it silently. Wiring them is provider work for its
own phase.

## The next authorized boundary

**Phase 5 requires an explicit instruction to begin.** Its scope is
`docs/roadmap.md` "Phase 5": adapt Betalingen's route to the runtime while
preserving its financial context, authorization, masking, prompts, and its
existing three-event `text`/`done`/`error` NDJSON wire by default. Its typed SDK
events are available but not required on an unchanged wire.

Two things deserve a decision before it rather than a default: how these
packages are distributed to a consumer that is not in this workspace, and
whether Skriuw's seven-line 0.2.0 fix should be authorized so both consumers sit
on one version.
