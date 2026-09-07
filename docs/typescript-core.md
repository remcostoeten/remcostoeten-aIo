# TypeScript core and Vercel adapter

What Phase 4 built, what it deliberately did not, and where the two languages
are actually held to the same behaviour. Companion to
`docs/extraction-inventory.md` and `docs/extraction-inventory-providers.md`.

Closed 2026-09-07 at spec version 0.2.0.

Worked examples of the API described here are in `docs/examples.md`.

## Packages

```text
packages/core     no dependencies at all
packages/ai-sdk   -> packages/core, ai@7; vendor packages are optional peers
```

`core` is what a browser can import: contracts, decoders, validators, the run
lifecycle, the event consumer, NDJSON helpers, and the deterministic fake.
`ai-sdk` is server code: it reaches real providers.

The boundary is enforced, not asserted.
`packages/ai-sdk/test/portability.test.ts` bundles `core` for the browser and
fails if `@ai-sdk/`, `ai`, `process.env`, or a Node built-in appears in the
output; it also fails if anything reachable from either entry point names a type
from the underlying library.

## Symbol map

Rust and TypeScript are the same seam, not a translation. What corresponds to
what:

| `crates/ai-core` | `packages/core` |
| --- | --- |
| `AiCompletionRequest`, `AiCompletionParameters`, `AiMessage` | `CompletionRequest`, `CompletionParameters`, `Message` |
| `AiCompletionEvent`, `AiCompletionTerminal` | `CompletionEvent`, `CompletionTerminal` |
| `AiProviderError`, `AiProviderErrorCategory`, `AiRecoveryAction` | `ProviderError`, `ProviderErrorCategory`, `RecoveryAction` |
| `AiValidationError` | `ValidationError` |
| `validate()` methods | `validateRequest`, `validateParameters`, `validateDelta`, `validateUsage`, `validateProviderError` |
| serde `deny_unknown_fields` | `decodeRequestShape`, `decodeEvent`, and the rest of `decode.ts` |
| `AiComplete` | `CompletionProvider` |
| `AiCompletionService` | `createRuntime` |
| `AiStartError` | `StartError`, returned rather than thrown |
| `AiCancellation` | `AbortSignal` and `AbortController` |
| `AiEventSink` / `AiCompletionChannel` | the returned `AsyncIterable<CompletionEvent>` |
| `FakeAiProvider`, `FakeCompletionScript` | `createFakeProvider`, `FakeScript` |
| `AiRunRecorder` and the run summary | **absent**, see below |

| `crates/ai-providers` | `packages/ai-sdk` |
| --- | --- |
| `AiCredentialSource` | `CredentialSource` |
| `AiCredentialRefusal` | `CredentialRefusal` |
| `AiModelAuthority` | `ModelAuthority` |
| `RemoteAiProvider` + descriptors | `createGroqProvider` and the other named factories |
| descriptor error mapping | `toProviderError` |

## Shape differences, and why each one

These are places the TypeScript is deliberately not a transliteration. Each is a
language difference, not a contract difference.

**S1 — start rejection is a value, not an exception.** Rust returns
`Result<(), AiStartError>`. `runtime.stream()` returns
`{ok: true, events} | {ok: false, error}`. An iterator can only signal failure
by throwing, and a caller who forgot a `try` would read a rejected request as a
run that produced no events. Same two outcomes, same reasons, no exception.

**S2 — cancellation is an `AbortSignal`, not a flag object.** It is the platform
primitive every consumer already has, it composes with `AbortSignal.any` and a
caller's own controller, and it is what the underlying library accepts. The
semantics are unchanged: cooperative, and unable to interrupt a provider that
blocks.

**S3 — the provider port is an async generator.** `AsyncGenerator<string,
CompletionTerminal>`: `yield` text, `return` a terminal. It is the closest thing
to Rust's "write to a sink, return a terminal" that reads naturally in this
language, and it gives the runtime a `return()` hook so a provider's `finally`
runs when a consumer stops reading early — which `runtime.test.ts` checks.

**S4 — no recorder port.** D2's `AiRunSummary` is an in-process Rust port with a
borrowed request, and ADR 0002 already excluded it from the shared wire
contract. No TypeScript consumer asks for run history. It is not implemented
rather than invented.

**S5 — the deadline outranks cancellation.** Rust's fake polls a deadline itself
and returns `Timeout`. The TypeScript runtime enforces the deadline by aborting
the run's signal, so a cooperative provider reports back `cancelled` for what
was in fact a timeout — only the runtime knows which it was, and it rewrites
accordingly. A provider error is never rewritten, in either language.

**S6 — decoders are hand-written, not Zod.** Every bound here is a UTF-8 byte
budget, and `String.length` counts UTF-16 code units; a schema library measuring
that would accept requests Rust rejects. Hand-written decoding also keeps `core`
at zero dependencies, which is what makes the browser bundle assertion possible.
`utf8ByteLength` is checked against `TextEncoder` in `validate.test.ts`, and
unpaired surrogates — which have no UTF-8 encoding and which a Rust `String`
cannot hold — are rejected at the boundary rather than replaced with U+FFFD.

## What the runtime guarantees

Identical to `docs/contracts.md` §3.3, and tested per item in
`packages/core/test/runtime.test.ts`:

- ids are reserved before provider lookup, so an unknown provider and a
  registered one refuse a duplicate identically;
- deltas carry the run's own id, with gapless sequences from zero;
- the accumulated byte budget is enforced by the runtime, not trusted to the
  provider, and capped at the same 4 MiB ceiling;
- out-of-range provider usage and non-text chunks commit `malformed_response`;
- a provider that throws becomes `internal_failure` carrying only its message;
- exactly one terminal, and nothing after it;
- `shutdown()` cancels active runs and leaves admission open, as in Rust.

And what it does not, stated rather than papered over: no delivery guarantee to
a consumer that stopped reading, no interruption of a provider wedged in an
un-abortable await, no retry, no fallback, no provider switch.

## Cross-language conformance

Three layers, all offline.

**Shared wire fixtures.** `packages/core/test/conformance.test.ts` and
`crates/ai-core/tests/contracts.rs` read the same `specs/fixtures/` directory.
Every valid request and event fixture must decode, validate and round-trip in
both; every invalid one must be refused by both.

**Shared fake scripts.** `specs/fixtures/fake/` holds eight scripts with their
agreed output. `packages/core/test/fake-parity.test.ts` and
`crates/ai-core/tests/fake_parity.rs` run them through their own runtime and
compare segmentation, identity, sequence, terminal kind, error category and
usage — the list ADR 0002 defines. Wall-clock timing is not compared, and a
fake's own diagnostic copy is display text: a scripted error's message and
recovery action are compared because the script fixed them, a malformed-output
diagnostic is not, and the fixture says so.

**Generated schemas.** The TypeScript suite asserts that every committed schema
pins the spec version and that the request schema declares exactly the fields
the decoder accepts, so a Rust-side contract change that never reached
TypeScript fails there.

## The adapter

`createAdapter` and `ModelFactory` are **not exported**. `ModelFactory` returns
a third-party `LanguageModel`, so exporting it would be the public
`fromLanguageModel(unknown)` escape hatch ADR 0001 rules out — a consumer could
hand in any model instance and would be coupled to that library's major versions
through our own types. Reaching a new vendor means adding a named factory in
`providers.ts`, with its destination stated.

Ordering is a guarantee: the model authority is consulted, then the credential
resolves, then a socket opens. An unauthorized model or an unconfigured provider
terminalizes with nothing sent, which `adapter.test.ts` checks by asserting the
fixture `fetch` was never called.

`maxRetries: 0`, always. Every retry-policy question — deadline accounting,
partial output, `Retry-After` bounds, key identity — is unanswered, and a hidden
retry would spend budget and time the caller allotted to one attempt. A 429 is
attempted exactly once, and the test counts the calls.

No provider response body crosses the boundary. Bodies carry echoed prompts and,
from some providers, a key prefix; only a status-derived sentence, bounded and
whitespace-normalized by the same rule Rust uses, reaches the caller. A test
feeds back a body containing both a prompt and a fake key and asserts neither
appears in the error.

Environment lookup is the application's. Nothing in `ai-sdk` reads `process.env`
or `Deno.env`, and a test greps the sources to keep it that way, so the package
makes no assumption about which runtime it is in.

## Known vendor quirks

**V1 — `ai@7` refuses a system message inside `messages`.** It takes the system
instruction as `instructions`. The adapter passes it there, and omits it
entirely when the system prompt is empty rather than sending an empty
instruction some providers reject. The wire body still carries the system turn
first; `adapter.test.ts` asserts the full four-message order the provider sees.

**V2 — Groq reports streaming usage under `x_groq.usage`,** not the
OpenAI-compatible top-level `usage`, and its own parser reads only that. A
fixture using the generic shape reports no usage and the assertion passes
vacuously. The fixture builder says so where it is written.

## Not implemented

Structured output. Retries and key rotation. Provider fallback or routing. Run
recording. Model listing or capabilities. A React, Hono, Next.js or Tauri
integration. Rust adapters carrying `priorMessages` or `maxOutputTokens` — they
refuse both, see ADR 0004.

## Unresolved

**Distribution is decided and both publishing acts are done.** ADR 0005 settles
the channels after Phase 4 closed: Rust ships as a tagged git dependency,
TypeScript as `@remcostoeten/ai-core` and `@remcostoeten/ai-sdk` on npm. Both
packages are published at 0.2.0 and `ai-v0.2.0` is pushed, so a consumer outside
this workspace can now install either unit. See the addendum in ADR 0005.

**Skriuw builds against 0.2.0 again.** ADR 0004's Rust source break was fixed in
Skriuw on 2026-09-07 under its own authorization: `prior_messages: Vec::new()`
at six request literals plus the regenerated
`contracts/generated/ai-completion-request.schema.json`. Skriuw still consumes
the crates by path, not by the tag ADR 0005 chose.

**No live-provider check exists.** Every test here runs against a fixture
`fetch`. That is deliberate and it is also a gap: nothing has confirmed that the
real Groq, Gemini or Anthropic endpoints behave as the fixtures claim.
