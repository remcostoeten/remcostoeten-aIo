/**
 * The typed provider factories a consumer actually calls.
 *
 * Each one names a vendor, a package and a default endpoint, and takes nothing
 * that could smuggle a third-party model instance in. Adding a vendor here is a
 * deliberate act with a disclosed destination — not something a caller can do
 * by passing an options bag.
 *
 * The vendor packages are optional peer dependencies, imported only when their
 * factory is called, so a consumer that uses Groq does not ship Google's SDK.
 */

import type { CompletionProvider } from '@remcostoeten/ai-core'

import { createAdapter, type ModelFactory } from './adapter.js'
import { permitAll, type CredentialSource, type ModelAuthority } from './ports.js'

export type ProviderOptions = {
  readonly credentials: CredentialSource
  /**
   * Which models this application permits.
   *
   * Defaulting to {@link permitAll} would make every id reachable the moment a
   * key exists, so it has no default: naming the policy is the point.
   */
  readonly models: ModelAuthority
  /**
   * Overrides the vendor's endpoint.
   *
   * For a compatible gateway or a local fixture server. Pointing a vendor
   * factory at a different host changes where the conversation and the key are
   * sent, so it is explicit rather than inferred from an environment variable.
   */
  readonly baseURL?: string
  /** Replaces the global fetch, for fixtures and instrumentation. */
  readonly fetch?: typeof globalThis.fetch
}

export const GROQ_PROVIDER_ID = 'groq'
export const GEMINI_PROVIDER_ID = 'gemini'
export const ANTHROPIC_PROVIDER_ID = 'anthropic'

export async function createGroqProvider(options: ProviderOptions): Promise<CompletionProvider> {
  const { createGroq } = await import('@ai-sdk/groq')
  return build(GROQ_PROVIDER_ID, options, (apiKey, modelId) =>
    createGroq({ apiKey, ...endpoint(options) })(modelId),
  )
}

export async function createGeminiProvider(options: ProviderOptions): Promise<CompletionProvider> {
  const { createGoogleGenerativeAI } = await import('@ai-sdk/google')
  return build(GEMINI_PROVIDER_ID, options, (apiKey, modelId) =>
    createGoogleGenerativeAI({ apiKey, ...endpoint(options) })(modelId),
  )
}

export async function createAnthropicProvider(options: ProviderOptions): Promise<CompletionProvider> {
  const { createAnthropic } = await import('@ai-sdk/anthropic')
  return build(ANTHROPIC_PROVIDER_ID, options, (apiKey, modelId) =>
    createAnthropic({ apiKey, ...endpoint(options) })(modelId),
  )
}

export type OpenAiCompatibleOptions = ProviderOptions & {
  /** Identifies this provider instance. Not a vendor name: two accounts are two instances. */
  readonly providerId: string
  /** Required: there is no sensible default host for "compatible". */
  readonly baseURL: string
}

/**
 * A provider speaking the OpenAI-compatible protocol at a caller-named host.
 *
 * `providerId` identifies the instance, not the vendor, so two endpoints or two
 * accounts are two providers — which is what the contract's provider id has
 * always meant.
 */
export async function createOpenAiCompatibleProvider(options: OpenAiCompatibleOptions): Promise<CompletionProvider> {
  const { createOpenAICompatible } = await import('@ai-sdk/openai-compatible')
  return build(options.providerId, options, (apiKey, modelId) =>
    createOpenAICompatible({
      name: options.providerId,
      apiKey,
      baseURL: options.baseURL,
      ...(options.fetch ? { fetch: options.fetch } : {}),
    })(modelId),
  )
}

function build(providerId: string, options: ProviderOptions, createModel: ModelFactory): CompletionProvider {
  return createAdapter({ providerId, credentials: options.credentials, models: options.models, createModel })
}

function endpoint(options: ProviderOptions): { baseURL?: string; fetch?: typeof globalThis.fetch } {
  return {
    ...(options.baseURL === undefined ? {} : { baseURL: options.baseURL }),
    ...(options.fetch === undefined ? {} : { fetch: options.fetch }),
  }
}
