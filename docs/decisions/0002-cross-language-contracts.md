# ADR 0002: cross-language contracts

Status: accepted (Phase 0, 2026-09-06)

## Context

The SDK must serve Rust (Skriuw, Dora: Tauri desktop) and TypeScript (Betalingen: Bun/Node/Vercel; the Skriuw and Dora renderers; future Next.js/serverless). The two languages must represent the same semantic states for shared contracts (`AGENTS.md`, "Cross-language contracts").

Actual interoperability requirements found in the repositories (audit §18.1):

- The only Rust↔TypeScript wire boundary is **inside the Tauri apps**: command arguments and `tauri::ipc::Channel<AiCompletionEvent>` / `Channel<AiStreamEvent>`. It is JSON. Skriuw generates JSON Schemas from Rust (`crates/xtask/src/main.rs`, `contracts/generated/ai-*.schema.json`) and hand-mirrors them in `app/src/contracts/ai.ts` with a drift check. Dora uses `tauri-specta` to generate `packages/studio/src/lib/bindings.ts`.
- No Rust code calls TypeScript AI code; no TypeScript code calls Rust AI code over HTTP. Betalingen has no Rust. Skriuw's Cloudflare Worker has no AI. The browser builds do not run providers.
- The Vercel AI SDK appears at two incompatible majors (`ai@7` in Betalingen, `ai@6` in frozen Skriuw v1).

## Options

| Option | Assessment |
| --- | --- |
| **A. Rust implementation reused everywhere** (TS calls Rust via FFI/WASM/sidecar) | Only meaningful inside Tauri, where it is already the case. Betalingen runs on Vercel's Node runtime with a single esbuild bundle; a WASM or native module for HTTP streaming there solves nothing and complicates deploys. Rejected as the general strategy; inside Tauri the Rust side *is* the implementation anyway. |
| **B. TypeScript implementation reused everywhere** (Rust embeds a JS runtime) | No consumer wants a JavaScript runtime inside a desktop process that deliberately keeps AI in Rust (Skriuw ADR-0033). Rejected. |
| **C. Independent implementations with matching concepts, no shared artifact** | The de facto state today, and the reason Dora's and Skriuw's event shapes diverged (`Token/Final/Error` vs `Delta/Done/Cancelled/Timeout/ProviderError`). Cheap, but drift is guaranteed. Rejected. |
| **D. Shared language-neutral specification, native implementations** | Skriuw already does half of this (schemas generated from Rust, TS mirror gated). Extending it to the SDK is incremental. Two native implementations, one spec, one fixture set. |
| **E. Hybrid: D plus a runtime bridge for Tauri** | The Tauri "bridge" already exists and is just JSON over `Channel`; it needs no new protocol. Anything more (a sidecar HTTP server, a shared binary protocol) solves a problem nobody has. |

## Decision

**Option D, with Rust as the schema generator.** Rust and TypeScript share semantic contracts, not implementation, and not a wire protocol beyond what the Tauri channel and NDJSON already are.

### Shared artifacts (`specs/`, `fixtures/`)

| Artifact | Form | Why shared |
| --- | --- | --- |
| `completion-request`, `completion-event`, `provider-error`, `model-ref`, `model-info`, `completion-outcome`, `run-record`, `credential-error`, `local-runtime-*` | JSON Schema, generated from Rust `schemars` derives by `xtask`, committed | The same shapes cross the Tauri channel, are accepted by a Hono route, and are persisted by applications |
| `error-categories.json` | table: `id`, `defaultRecovery`, `retryableBeforeFirstDelta`, `fallbackEligible` | UIs branch on categories in both languages; the runtime's retry rule and a future router read the flags |
| `capabilities.json` | list | catalog data and the structured-output ladder in both languages |
| `providers.json` | provider descriptors incl. OpenAI-compatible rows | drives the Rust adapter directly and configures the TS `openaiCompatible` adapter |
| `models.json` | priced catalog (Skriuw's format: integer micro-dollars, `contextWindowTokens`, `pricingAsOf`, version) | both languages display, price, and gate on it |
| `fixtures/streams/<provider>/<case>.sse` + `.events.json` | golden provider input → expected event sequence | both implementations must produce identical output |
| `fixtures/errors/<provider>/<case>.json` | status + body → category + recovery | shared mapper truth |
| `fixtures/fake-scripts/*.json` | fake-provider scripts | runtime tests behave identically in both languages |
| `VERSION` | spec semver | see versioning |

Not shared: implementation code, async model (sink-based Rust vs `AsyncIterable` TS), credential types (never serializable), the `Runtime` object, any framework glue.

### Wire conventions

- Field names camelCase; union tag field `type` with snake_case values; string enums snake_case.
- Numbers are integers: milliseconds, micro-dollars, bytes, tokens, fixed-point millis for temperature/top-p. No floats anywhere on the wire, so fixtures are byte-identical across languages and JSON number handling is a non-issue.
- Optional means absent or `null`, and both languages accept either.
- `CompletionRequest` (and its nested types) uses `deny_unknown_fields`: it crosses trust boundaries (IPC, HTTP) and Skriuw's strictness there is a security property.
- `CompletionEvent` accepts unknown *fields* on known kinds (an additive field such as `finishReason` is a minor bump and rolling consumers must tolerate it) but rejects unknown *kinds* (a new event kind is a breaking change for exhaustive consumers).

### TypeScript contracts: generated or conformance-checked

Decided: hand-written zod schemas with types inferred from them, conformance-checked against the JSON Schemas in CI. This is what Skriuw does today (`app/src/contracts/ai.ts` + drift check) and it needs no generator tooling. Generation from `specs/schema` becomes a new ADR only if conformance checks catch repeated drift. Consequently:

- zod schemas exist for every contract that crosses a trust boundary (Betalingen and Skriuw both validate at boundaries already) and are checked against the JSON Schema by validating every fixture with both.
- Every fixture is validated by the zod schema and by a JSON Schema validator in the TS test suite; every fixture round-trips through Rust serde. Divergence fails CI.

### Schema drift

`cargo run -p xtask -- check` regenerates schemas from the Rust types and fails if the committed `specs/schema` differs (Skriuw's existing mechanism). The TS suite fails if its types or zod schemas reject any committed fixture or accept a fixture the JSON Schema rejects. A change to a Rust type therefore fails CI until the schema is regenerated *and* the TS side is updated, which is the intended friction.

### Golden fixtures

Fixtures are the executable half of the spec. Seeds: Skriuw's inline test bodies in `crates/skriuw-ai-remote/src/lib.rs` (SSE framing, `stream_options` gating, Gemini and Groq streams, malformed/oversized cases), `crates/skriuw-ai-ollama/src/lib.rs` (NDJSON pull/generate streams), Betalingen's `providerResponse()` in `src/lib/ai/ai.test.ts`. A provider without fixtures fails CI (ADR 0003).

### Versioning

- `specs/VERSION` is a semver string exported as `specVersion` by both cores and embedded in every schema file's `$id`.
- **Patch**: documentation, descriptions, additional fixtures.
- **Minor**: additive optional field on a non-`deny_unknown_fields` contract; new `ErrorCategory`/`RecoveryAction`/`Capability` value (Rust enums are `#[non_exhaustive]`; TS consumers keep a documented fallback arm); new provider descriptor row; new catalog entries.
- **Major**: any field on `CompletionRequest`; any new event kind; renaming or removing anything; changing the meaning of an existing value.
- A change that compiles in both languages but alters serialized meaning is a contract change and bumps the version. Reviewers check `specs/VERSION` on any PR touching `ai-core` public types.
- Crate/package versions track the spec's major; a crate may bump minor/patch independently.

### Tauri JSON boundaries

- The Tauri channel carries `CompletionEvent` JSON exactly as specified; no Tauri-specific envelope. Skriuw's `Channel<AiCompletionEvent>` already is this.
- Dora's generated bindings require `specta::Type`; the core derives it behind a `specta` feature (Phase 7) so that Dora keeps its typed `commands` object without `specta` becoming a default dependency of `ai-core`. Dora's snake_case wire casing for AI payloads becomes camelCase on migration (Phase 8), contained to the `ai-assistant` and `ai-cmd-k` modules.
- Command names, arguments beyond the SDK types (Dora `connection_id`, Skriuw `origin` validation and consent), and IPC error envelopes remain application-owned. The SDK specifies payload types, not commands.
- Non-streaming results cross IPC as `CompletionOutcome` JSON, avoiding a channel for tiny completions.

### No runtime bridge

No Rust↔TypeScript runtime bridge, sidecar, FFI, or WASM binding is built. If a consumer ever needs one, it is a new ADR with that consumer as evidence.

## Consequences

- `crates/xtask` and `specs/` are created in Phase 1 alongside `ai-core`; `packages/core` in Phase 4 must reproduce every fixture bit-for-bit before Betalingen migrates.
- Two implementations must be maintained. The cost is bounded by the fixture set: a behavior is defined once, in a fixture, and both implementations are held to it.
- Skriuw's `contracts/generated/ai-*.schema.json` are superseded by `specs/schema` in Phase 3; Skriuw's drift check points at the SDK's schemas.
