<p align="center">
  <img src="https://raw.githubusercontent.com/remcostoeten/remcostoeten-aIo/master/assets/logo-gradient.png" width="88" alt="ai-core logo" />
</p>

<h1 align="center">@remcostoeten/ai-core</h1>

<p align="center">
  Provider-neutral AI completion contracts and run lifecycle, in TypeScript.<br />
  A validated request goes in, ordered deltas and exactly one terminal come out.<br />
  No HTTP client, no credentials, no prompts, no framework, and no dependencies.<br />
  Runs in a browser, in Node, in a Worker, in Bun.
</p>

<p align="center">
  <img src="https://shieldcn.dev/github/remcostoeten/remcostoeten-aIo/license.svg?font=jetbrains-mono" alt="license" />
  <img src="https://shieldcn.dev/badge/dependencies-zero-black.svg?font=jetbrains-mono" alt="dependencies zero" />
  <img src="https://shieldcn.dev/badge/types-strict-black.svg?font=jetbrains-mono&logo=typescript" alt="types strict" />
  <img src="https://shieldcn.dev/badge/contract-shared%20with%20Rust-black.svg?font=jetbrains-mono&logo=rust" alt="contract shared with Rust" />
  <img src="https://shieldcn.dev/badge/providers-bring%20your%20own-black.svg?font=jetbrains-mono" alt="providers bring your own" />
</p>

This package is the seam, not the integration. It defines what a completion
request is, validates one, runs it against a provider you supply, and
guarantees what comes back. Reaching a real vendor is a separate package,
[`@remcostoeten/ai-sdk`](https://github.com/remcostoeten/remcostoeten-aIo/tree/master/packages/ai-sdk),
so a browser bundle that imports this one pulls in nothing else.

- **Zero dependencies**, enforced by a test that bundles this package for the browser and fails if `ai`, `@ai-sdk/*`, `process.env` or a Node built-in appears in the output
- `buildRequest` validates against UTF-8 byte budgets and returns a rejection **as a value**, never as a thrown exception
- The runtime owns the guarantees: ids reserved before provider lookup, gapless delta sequences from zero, a 4 MiB accumulated ceiling it enforces rather than trusts, and exactly one terminal with nothing after it
- `consumeEvents` and `accumulate` turn a run into text and a typed outcome
- A deterministic fake provider driven by a script, so a consumer's tests need no network and no key
- NDJSON helpers to carry events over a wire, bounded per line
- Cancellation is an `AbortSignal`; a deadline that fires outranks it and reports `timeout`, not `cancelled`

The same seam exists in Rust as `ai-core`. Both read `specs/fixtures/` and run
the same eight fake scripts, so segmentation, terminal kind, error category and
usage are held identical across the two languages.

Reference is
[typescript-core.md](https://github.com/remcostoeten/remcostoeten-aIo/blob/master/docs/typescript-core.md),
worked usage is
[examples.md](https://github.com/remcostoeten/remcostoeten-aIo/blob/master/docs/examples.md),
and the wire contract is
[contracts.md](https://github.com/remcostoeten/remcostoeten-aIo/blob/master/docs/contracts.md).

## Install

```bash
npm install @remcostoeten/ai-core
```

## Use

```ts
import { buildRequest, consumeEvents, createFakeProvider, createRuntime, successScript } from '@remcostoeten/ai-core'

const runtime = createRuntime({ providers: [createFakeProvider(successScript(['hel', 'lo']))] })

const built = buildRequest({
  requestId: 'request-1',
  providerId: 'fake',
  modelId: 'model',
  messages: [{ role: 'user', content: 'hello' }],
})
if (!built.ok) return reject(built.error.reason)

const started = runtime.stream(built.request)
if (!started.ok) return reject(started.error.reason)

const outcome = await consumeEvents(started.events)
// outcome.status === 'done', outcome.text === 'hello'
```

## Not implemented

Structured output, retries, key rotation, provider fallback or routing, run
recording, model listing, and framework helpers for React, Hono, Next.js or
Tauri. Each is absent deliberately and recorded in the
[decision records](https://github.com/remcostoeten/remcostoeten-aIo/tree/master/docs/decisions).

<br/>

xxx,<br/>
[Remco Stoeten](https://remcostoeten.com)<br/>
MIT
