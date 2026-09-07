/**
 * Our completion provider, implemented over the Vercel AI SDK.
 *
 * The boundary this file defends:
 *
 * ```
 * application -> our contracts -> this adapter -> Vercel AI SDK -> provider
 * ```
 *
 * A `LanguageModel` is created here from typed configuration and never accepted
 * from a caller, so there is no `fromLanguageModel` escape hatch and no version
 * of that library in a consumer's types. `UIMessage`, `TextStreamPart` and
 * `APICallError` stop at this file.
 */

import { streamText, type LanguageModel, type ModelMessage } from 'ai'
import type { CompletionProvider, CompletionRequest, CompletionTerminal, Usage } from '@ai-sdk-local/core'

import { credentialError, toProviderError, unauthorizedModelError } from './errors.js'
import type { CredentialSource, ModelAuthority } from './ports.js'

/** Builds the underlying model once a key has resolved. Never exported. */
export type ModelFactory = (apiKey: string, modelId: string) => LanguageModel

export type AdapterOptions = {
  readonly providerId: string
  readonly credentials: CredentialSource
  readonly models: ModelAuthority
  readonly createModel: ModelFactory
}

/**
 * Builds a provider for the runtime.
 *
 * Ordering is a guarantee, not an implementation detail: the model authority is
 * consulted, then the credential resolves, and only then is a socket opened. An
 * unauthorized model or an unconfigured provider therefore terminalizes without
 * any network access at all.
 */
export function createAdapter(options: AdapterOptions): CompletionProvider {
  const { providerId, credentials, models, createModel } = options

  return {
    providerId,
    async *run(request: CompletionRequest, signal: AbortSignal): AsyncGenerator<string, CompletionTerminal, void> {
      if (request.providerId !== providerId || !models.permits(providerId, request.modelId)) {
        return { type: 'provider_error', error: unauthorizedModelError(providerId) }
      }
      if (signal.aborted) return { type: 'cancelled' }

      const credential = await credentials.resolve(providerId)
      if (!credential.ok) {
        return { type: 'provider_error', error: credentialError(providerId, credential.refusal, credential.message) }
      }
      if (signal.aborted) return { type: 'cancelled' }

      let usage: Usage | null = null
      try {
        const result = streamText({
          model: createModel(credential.apiKey, request.modelId),
          // `ai@7` refuses a system message inside `messages` and takes the
          // system instruction separately. An empty system prompt is omitted
          // rather than sent empty, which some providers reject.
          ...(request.systemPrompt === '' ? {} : { instructions: request.systemPrompt }),
          messages: toModelMessages(request),
          abortSignal: signal,
          // Every retry policy question — deadline accounting, partial output,
          // Retry-After bounds, key identity — is unanswered here, and a hidden
          // retry would silently spend budget and time the caller allotted to
          // one attempt. Retries stay off until this SDK owns a policy.
          maxRetries: 0,
          ...samplingOptions(request),
        })

        for await (const part of result.fullStream) {
          if (part.type === 'text-delta') {
            yield part.text
            continue
          }
          if (part.type === 'error') {
            return { type: 'provider_error', error: toProviderError(providerId, part.error) }
          }
          if (part.type === 'abort') {
            return { type: 'cancelled' }
          }
          if (part.type === 'finish') {
            usage = toUsage(part.totalUsage)
          }
        }

        return { type: 'done', usage }
      } catch (thrown) {
        if (signal.aborted) return { type: 'cancelled' }
        return { type: 'provider_error', error: toProviderError(providerId, thrown) }
      }
    },
  }
}

/**
 * Builds the conversation the contract describes: the prior turns in order,
 * then the final user prompt. The system instruction travels separately.
 */
function toModelMessages(request: CompletionRequest): ModelMessage[] {
  const messages: ModelMessage[] = []
  for (const message of request.priorMessages) {
    messages.push({ role: message.role, content: message.content })
  }
  messages.push({ role: 'user', content: request.userPrompt })
  return messages
}

/**
 * Translates our thousandths into the library's fractions.
 *
 * Absent stays absent: sending an explicit default would override a provider's
 * own, which is a behaviour change disguised as a translation.
 */
function samplingOptions(request: CompletionRequest): {
  temperature?: number
  topP?: number
  maxOutputTokens?: number
} {
  const { temperatureMillis, topPMillis, maxOutputTokens } = request.parameters
  return {
    ...(temperatureMillis === null ? {} : { temperature: temperatureMillis / 1000 }),
    ...(topPMillis === null ? {} : { topP: topPMillis / 1000 }),
    ...(maxOutputTokens === null ? {} : { maxOutputTokens }),
  }
}

/**
 * Reads reported usage, or reports none.
 *
 * A provider that reported nothing must not be recorded as having used zero
 * tokens: that is a real number, and it would be wrong.
 */
function toUsage(reported: { inputTokens?: number | undefined; outputTokens?: number | undefined } | undefined): Usage | null {
  if (!reported) return null
  const { inputTokens, outputTokens } = reported
  if (typeof inputTokens !== 'number' || typeof outputTokens !== 'number') return null
  if (!Number.isFinite(inputTokens) || !Number.isFinite(outputTokens)) return null
  return { inputTokens: Math.trunc(inputTokens), outputTokens: Math.trunc(outputTokens) }
}
