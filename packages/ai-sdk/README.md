<p align="center">
  <img src="https://raw.githubusercontent.com/remcostoeten/remcostoeten-aIo/master/assets/logo-gradient.png" width="88" alt="ai-sdk logo" />
</p>

<h1 align="center">@remcostoeten/ai-sdk</h1>

<p align="center">
  Provider execution for <code>@remcostoeten/ai-core</code>, over the Vercel AI SDK.<br />
  Typed factories for Groq, Gemini, Anthropic and any OpenAI-compatible endpoint.<br />
  The library underneath stays underneath: no vendor type is exported here.<br />
  Server-side only, and it never reads your environment for you.
</p>

<p align="center">
  <img src="https://shieldcn.dev/github/remcostoeten/remcostoeten-aIo/license.svg?font=jetbrains-mono" alt="license" />
  <img src="https://shieldcn.dev/badge/vendors-optional%20peers-black.svg?font=jetbrains-mono" alt="vendors optional peers" />
  <img src="https://shieldcn.dev/badge/retries-off-black.svg?font=jetbrains-mono" alt="retries off" />
  <img src="https://shieldcn.dev/badge/bodies-never%20forwarded-black.svg?font=jetbrains-mono" alt="bodies never forwarded" />
  <img src="https://shieldcn.dev/badge/types-strict-black.svg?font=jetbrains-mono&logo=typescript" alt="types strict" />
</p>

[`@remcostoeten/ai-core`](https://github.com/remcostoeten/remcostoeten-aIo/tree/master/packages/core)
owns the contracts and the run lifecycle. This package owns what it takes to
reach a real provider, and its whole job is that the vendor library stays an
implementation detail: no `LanguageModel`, `UIMessage`, `TextStreamPart` or
`APICallError` appears in anything it exports. Vendor packages are optional
peer dependencies, so you install only the one you use.

- Named factories only: `createGroqProvider`, `createGeminiProvider`, `createAnthropicProvider`, `createOpenAiCompatibleProvider`. There is no `fromLanguageModel(unknown)` escape hatch
- Ordering is a guarantee. The model authority is consulted, then the credential resolves, then a socket opens, so an unauthorized model or an unconfigured provider terminalizes with **nothing sent**
- **`maxRetries: 0`, always.** A 429 is attempted exactly once, because a hidden retry spends budget and time the caller allotted to one attempt
- **No provider response body crosses the boundary.** Bodies echo prompts and, from some vendors, a key prefix; only a bounded, status-derived sentence reaches the caller
- Credentials arrive through a `CredentialSource` port and model permission through a `ModelAuthority` port, both yours to implement
- Nothing here reads `process.env` or `Deno.env`, so the package makes no assumption about which runtime it is in

Each of those is asserted by a test, including the one that greps the sources
for environment access and the one that asserts the fixture `fetch` was never
called on a refusal.

The adapter is documented in
[typescript-core.md](https://github.com/remcostoeten/remcostoeten-aIo/blob/master/docs/typescript-core.md#the-adapter),
worked usage is in
[examples.md](https://github.com/remcostoeten/remcostoeten-aIo/blob/master/docs/examples.md),
and the reasoning behind the boundary is
[ADR 0003](https://github.com/remcostoeten/remcostoeten-aIo/blob/master/docs/decisions/0003-provider-boundary.md).

## Install

Install this package, `ai-core`, and the vendor package for the provider you
reach.

```bash
npm install @remcostoeten/ai-sdk @remcostoeten/ai-core @ai-sdk/groq
```

The optional peers are `@ai-sdk/groq`, `@ai-sdk/google`, `@ai-sdk/anthropic`
and `@ai-sdk/openai-compatible`.

## Use

```ts
import { buildRequest, consumeEvents, createRuntime } from '@remcostoeten/ai-core'
import { createGroqProvider, staticCredential } from '@remcostoeten/ai-sdk'

const provider = await createGroqProvider({
  credentials: staticCredential(process.env.GROQ_API_KEY ?? ''),
  models: { permits: (_, modelId) => modelId === 'llama-3.3-70b-versatile' },
})

const runtime = createRuntime({ providers: [provider] })
```

From there the run is `@remcostoeten/ai-core`: build a request, stream it, and
consume the events.

## Not implemented

No retries and no key rotation. No fallback or routing between providers. No
structured output. No model listing or capability discovery. Every adapter test
runs against a fixture `fetch`, so nothing here has been confirmed against a
live endpoint.

<br/>

xxx,<br/>
[Remco Stoeten](https://remcostoeten.com)<br/>
MIT
