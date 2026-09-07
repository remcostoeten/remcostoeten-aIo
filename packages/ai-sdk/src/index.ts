/**
 * Provider execution for the TypeScript core, over the Vercel AI SDK.
 *
 * `@remcostoeten/ai-core` owns the contracts and the run lifecycle; this package
 * owns what it takes to reach a real provider. Its whole job is that the
 * library underneath stays underneath — no `LanguageModel`, `UIMessage`,
 * `TextStreamPart` or `APICallError` appears in anything exported here.
 *
 * ```ts
 * import { createRuntime, buildRequest, consumeEvents } from '@remcostoeten/ai-core'
 * import { createGroqProvider, staticCredential } from '@remcostoeten/ai-sdk'
 *
 * const provider = await createGroqProvider({
 *   credentials: staticCredential(process.env.GROQ_API_KEY ?? ''),
 *   models: { permits: (_, modelId) => modelId === 'llama-3.3-70b-versatile' },
 * })
 * const runtime = createRuntime({ providers: [provider] })
 * ```
 *
 * # What it guarantees
 *
 * The model authority is consulted and the credential resolves before any
 * socket is opened; the conversation reaches the provider in contract order;
 * the underlying library's retries are off; and provider failures arrive as
 * typed categories rather than status codes.
 *
 * # What it does not
 *
 * No retries and no key rotation. No fallback between providers. No structured
 * output. Environment lookup is the application's: nothing here reads
 * `process.env`, so this package makes no assumption about which runtime it is
 * in.
 */

// `createAdapter` and `ModelFactory` are deliberately not exported. Their
// `createModel` argument returns a third-party `LanguageModel`, so exporting
// them would be the public `fromLanguageModel(unknown)` escape hatch ADR 0001
// rules out: a consumer could hand in any model instance and would be coupled
// to that library's major versions through our own types. Reaching a new
// vendor means adding a named factory in `providers.ts`, with its destination
// stated.

export { credentialError, toProviderError, unauthorizedModelError } from './errors.js'

export {
  permitAll,
  staticCredential,
  type CredentialRefusal,
  type CredentialResult,
  type CredentialSource,
  type ModelAuthority,
} from './ports.js'

export {
  ANTHROPIC_PROVIDER_ID,
  GEMINI_PROVIDER_ID,
  GROQ_PROVIDER_ID,
  createAnthropicProvider,
  createGeminiProvider,
  createGroqProvider,
  createOpenAiCompatibleProvider,
  type OpenAiCompatibleOptions,
  type ProviderOptions,
} from './providers.js'
