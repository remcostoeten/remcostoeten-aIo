# ADR 0004: conversation history and an output token limit

Status: accepted 2026-09-07, as the contract evolution gate Phase 4 lists as its
prerequisite. Spec version 0.1.0 → 0.2.0.

## Context

Phase 4's roadmap entry says: *"Prerequisite: a separately approved minimal
history/token-limit contract delta needed by Betalingen. Do not flatten its
conversation into one string just to avoid specifying that delta."* This is that
approval. `roadmap.md` "Contract evolution gate" requires a real consumer, exact
types and validators, positive and negative fixtures, and a migration mapping;
all four are below.

The consumer is Betalingen, at `/home/remcostoeten/dev/betalingen`, commit
`d13c762`. Two facts about its existing code force the delta:

- `src/lib/ai/contracts.ts` validates `messages: {role: 'user' | 'assistant',
  content: string}[]`, 1 to 20 entries, each 1 to 6000 characters, and refuses a
  conversation whose last entry is not from the user. `src/lib/ai/groq.ts`
  passes that array through. `AiCompletionRequest` had only `systemPrompt` and
  `userPrompt`, so the SDK could not carry it.
- `src/lib/ai/groq.ts` sets `maxOutputTokens: 1800`.
  `AiCompletionParameters.maxOutputBytes` is a local accumulation cap enforced
  on this side of the boundary and cannot express a limit asked *of* the
  provider. They are different quantities with different failure modes.

Skriuw and Dora ask for neither today.

## Decision

Add two defaulted fields. Keep `systemPrompt` and `userPrompt` exactly as they
are.

```text
Message = { role: "user" | "assistant", content: string }

CompletionRequest = {
  requestId, providerId, modelId,
  systemPrompt: string,
  userPrompt: string,
  priorMessages: Message[],       // default [], omission accepted
  parameters: CompletionParameters
}

CompletionParameters = {
  maxOutputBytes, timeoutMs, retryCount, temperatureMillis, topPMillis,
  maxOutputTokens: u32 | null     // default null, omission accepted
}
```

The conversation a provider receives is exactly: `systemPrompt`, then
`priorMessages` in order, then `userPrompt` as the final user turn.

### Why not replace the two prompts with `messages[]`

That is the tidier contract and it was rejected for this gate. It invalidates
every committed fixture, forces Skriuw's hand-written renderer types and its
Rust call sites to migrate, and turns an additive delta into a migration —
inside a phase whose purpose is a TypeScript implementation, and without the
authorization a Skriuw change requires. The tidier shape stays available later,
as its own gate with its own migration.

### One authority for the final turn

`userPrompt` is always the final user turn; `priorMessages` is strictly what
came before it. There is no second way to express the question, which is what
keeps this from becoming the dual-authority problem `contracts.md` §4.3 warns
about for schemas. A consumer holding a `messages` array sends everything but
its last entry as `priorMessages` — `buildRequest` in `packages/core` does that
split once so no call site does it by hand and drops a turn.

### `role` has no `system` value

The system instruction is `systemPrompt`. Admitting a `system` role inside the
conversation would create a second instruction channel whose precedence nobody
has specified, and providers disagree about how to merge one. A conversation
carrying `{"role": "system"}` is rejected on decode.

### No alternation rule

Consecutive turns from the same role are accepted. Whether they are meaningful
is the application's judgement and providers differ; Betalingen's "last entry
must be a user turn" rule is its own application validation, which
`contracts.md` §4.1 already said must not be promoted into a universal one. The
SDK enforces only what `userPrompt` means.

### `maxOutputTokens` is a request, not a guarantee

It is passed to the provider. A provider that ignores it has not violated the
contract, and `maxOutputBytes` still bounds accumulation locally. Making it a
guarantee would require token counting the SDK does not do.

## Bounds and validators

| Quantity | Rule |
| --- | --- |
| `priorMessages` length | 0 through 64 |
| `Message.content` | Non-empty. No separate byte cap of its own |
| Combined prompt | `systemPrompt` + every `content` + `userPrompt` ≤ 1 MiB, unchanged from 0.1.0 except that it now spans the history |
| `maxOutputTokens` | `null`, or 1 through 1,000,000 |

Two `AiValidationError` variants are added: `TooManyMessages { maximum }` and
`InvalidOutputTokenLimit { maximum }`. The TypeScript `ValidationError` union
gains `too_many_messages` and `invalid_output_token_limit`.

The combined budget deliberately spans the whole conversation rather than
bounding each turn. A per-turn cap with no aggregate would let 64 turns carry 64
MiB, which is the bound that actually matters.

## Fixtures

Positive, in `specs/fixtures/valid/`:

- `completion-request-conversation.json` — two prior turns plus a token ceiling.
- `completion-request-legacy-omitted.json` — a spec 0.1.0 producer's document,
  with both fields absent. It decodes; it does not round-trip, and the tests
  assert the defaults rather than byte equality.
- `completion-request.json` and `completion-request-empty-prompts.json` are
  updated to the canonical 0.2.0 form: `priorMessages` present, `maxOutputTokens`
  an explicit `null`, per ADR 0002's rule that canonical output spells nullable
  fields out.

Negative, in `specs/fixtures/invalid/`:
`request-message-unknown-role.json`, `request-message-unknown-field.json`,
`request-message-missing-content.json`.

A new schema, `specs/schemas/message.json`, is generated alongside the updated
request and parameter schemas.

## Migration

**Producers on 0.1.0 need no change.** Both fields are `#[serde(default)]` in
Rust and optional on decode in TypeScript, so an existing document still decodes
to the empty history and no ceiling, and behaves exactly as before.

**Rust source compatibility does break.** `AiCompletionRequest` and
`AiCompletionParameters` are plain structs, so a literal constructing one
without the new fields no longer compiles. In Skriuw that is six test helpers
and one call site on branch `ai-sdk-phase-3-extraction`; adding
`prior_messages: Vec::new()` and `max_output_tokens: None` is the whole fix.
Skriuw was **not** modified — that needs its own authorization — so its branch
will not build against this version until it is.

**Consumers gain nothing they did not ask for.** No behaviour changes for a
request that omits both fields.

Per ADR 0002's versioning rules this is a **breaking** change — fields added to
a strict request — and is recorded as such even while the SDK is prerelease.
`specs/VERSION` moves to `0.2.0` and every generated `$id` with it.

## Provider support

The TypeScript adapter transmits both: `priorMessages` becomes the message array
and `maxOutputTokens` the library's `maxOutputTokens`.

The Rust adapters in `ai-providers` do **not**. No Rust consumer asks for them,
their bodies are single-turn, and wiring seven descriptors plus the Gemini
dialect is provider work belonging to its own phase. They therefore refuse a
request carrying either field with a typed `rejected_request` terminal
(`crates/ai-providers/src/unsupported.rs`), before any socket opens.

Refusing rather than dropping is the point. A dropped history makes the model
answer without context the caller believed it had, and a dropped ceiling removes
a spend and latency control — both failures that look like a bad answer rather
than a bug.

## Consequences

Betalingen's conversation and token ceiling can cross the SDK boundary without
being flattened, which is what Phase 5 will need. The `messages`-only request
shape remains a future gate. Rust providers carrying history is a future phase,
and until then the refusal is visible rather than silent.
