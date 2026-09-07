# Phase 5: Betalingen consumes the TypeScript packages

Recorded 2026-09-07. Betalingen at `/home/remcostoeten/dev/betalingen`, on top of
commit `d13c762`. Spec version 0.2.0.

Phase 4 built `packages/core` and `packages/ai-sdk` and checked them against a
*model* of this consumer, in `packages/ai-sdk/test/consumer-shape.test.ts`. This
is the real one.

## What changed

Two files, and the manifest.

**`src/lib/ai/groq.ts`** is the whole migration. It was `createGroq` +
`streamText` from the Vercel AI SDK directly. It is now `createGroqProvider`,
`buildRequest` and `createRuntime` from the packages, and neither `ai` nor
`@ai-sdk/groq` is imported by Betalingen's own code any more.

`answerWithGroq` keeps its name and its four arguments. Its return type changed
from a `streamText` result to an explicit union:

```ts
type Answer = { ok: true; events: AsyncIterable<CompletionEvent> } | { ok: false; reason: string }
```

That is the point of the seam rather than an inconvenience: a request that was
never admitted — an unpermitted model, a missing key, a conversation ending in
an assistant turn — is now distinguishable from a run that produced no output,
and it costs the route one `if`.

**`src/routes/ai.ts`** changed inside the `try` block and nowhere else. It used
to walk `result.fullStream` and match `text-delta`, `error` and `abort`. It now
walks `answer.events` and matches the contract's `delta` and `done`, treating
every other terminal — `cancelled`, `timeout`, `provider_error` — as the failure
it already treated `error` and `abort` as. The OpenAPI declaration above the
handler is untouched.

**`package.json`** gains `@remcostoeten/ai-core` and `@remcostoeten/ai-sdk` as
`file:` dependencies into this workspace, plus an `overrides` entry pinning the
nested `@remcostoeten/ai-core` to the same path. See "What is not finished".

## What was preserved, deliberately

The three-event NDJSON wire is unchanged: `{type:'text'}`, `{type:'done'}`,
`{type:'error'}`, one JSON document per line. The roadmap allows richer typed
events behind a later explicit HTTP contract change and does not require them;
the browser island, the `ChatEventSchema` in `src/lib/ai/contracts.ts` and the
OpenAPI response description were therefore not touched, and `openapi.json`
shows no drift from this work.

Also unchanged, because they are Betalingen's and not the SDK's:

- `DATA_ASSISTANT_PROMPT` and the whole prompt-injection framing in
  `src/lib/ai/context.ts`.
- The source allowlist `isContextSource`, the IBAN masking, the 100 000
  character context ceiling, and the `/meta` prepend in `buildScreenContext`.
- Authorization: the 401 gate, the forwarded `authorization`/`x-api-key`/
  `cookie` headers, the 503 when `GROQ_API_KEY` is absent, the 48 kB body limit
  and its 413.
- The Dutch failure message, which still says nothing about the provider.
- `temperature: 0.2` (as `temperatureMillis: 200`), `maxOutputTokens: 1800`, and
  retries off — the packages hold `maxRetries` at 0 with no way to raise it.
- Where the source context goes. It is appended to the final user turn, not to
  the system prompt, so the model keeps reading it as data. The turns before it
  reach the provider as themselves.

## What the run lifecycle adds

Not a behavior change on the wire, but new on this path:

- The model authority is consulted and the credential resolves before a socket
  opens, so a wrong model id or a missing key fails without a request.
- A 60-second deadline now exists twice: the route's `AbortSignal.timeout` and
  `parameters.timeoutMs`. Whichever fires first, the wire says `error`.
- Deltas carry a sequence and a request id, the accumulated response is capped
  at 256 KiB, and exactly one terminal is emitted. None of that is visible to
  the browser, which still sees `text` and then `done`.

## Verification

Run on 2026-09-07 in the Betalingen checkout:

- `bun test`: **99 passed, 0 failed** across 14 files. The six AI tests in
  `src/lib/ai/ai.test.ts` were not modified — they assert the wire, the masking,
  the filters, the 401/503/400/413 statuses and that neither the key nor the
  provider's body escapes, and they pass against the runtime as they did against
  `streamText`.
- `bun run check` (`tsc --noEmit`): clean.
- `bun run lint` (`oxlint`, with the anti-slop plugin): clean.
- `bun run openapi`: no drift attributable to this change.
- `bun run build`: the esbuild Vercel bundle builds, 7.8 MB.
- Browser island: `src/ui/assistant.js.txt` contains no `@ai-sdk/groq`, no
  `createGroq`, no `GROQ_API_KEY`, no model id and no `DATA_ASSISTANT_PROMPT`.
  Server credentials and vendor packages stayed on the server.

The SDK workspace's own `./scripts/check.sh` is green alongside it: 148 Rust
tests, 122 TypeScript tests, clippy, fmt and the schema drift check, offline.

## What is not finished

**Betalingen cannot be deployed from this state.** The packages are consumed
through `file:` paths into a sibling checkout, so `bun install` on Vercel has
nothing to install from. ADR 0005 chose npm under `@remcostoeten` as the channel
and the import specifiers in `groq.ts` are already the final ones, so the fix is
a publish and a version range — no code change. The roadmap separates these
deliberately: "deployment is a separate authorized action, not an implicit SDK
phase requirement." Publishing has not been authorized and has not been done.

**The `overrides` entry is scaffolding.** `@remcostoeten/ai-sdk` declares
`@remcostoeten/ai-core: ^0.2.0`, which resolves from npm and 404s today. The
override pins it to the local path. Both the override and the two `file:`
specifiers are deleted when the packages are published.

**Nothing here touched a live Groq endpoint.** Every test still runs against a
fixture `fetch`, in this repository and in Betalingen. That gap is the same one
Phase 4 recorded and Phase 5 did not close.

**The AI feature is uncommitted in Betalingen.** `src/lib/ai/` and
`src/routes/ai.ts` were untracked when this work began, alongside unrelated
in-progress edits to `src/app.ts`, `openapi.json`, the dashboard and the build
gate. Nothing was committed here, so the migration sits in that same working
tree.
