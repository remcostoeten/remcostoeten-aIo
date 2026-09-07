/**
 * Building a request from what a consumer actually holds.
 *
 * Applications carry a `messages` array; the contract carries a system prompt,
 * the turns before the last one, and the final user prompt separately. Doing
 * that split by hand at every call site is how a conversation quietly loses its
 * last turn, so it is done once, here.
 */

import { DEFAULT_PARAMETERS, type CompletionParameters, type CompletionRequest, type Message } from './contracts.js'
import { decodeRequestShape, type DecodeIssue } from './decode.js'
import { validateRequest, type ValidationError } from './validate.js'

export type ConversationInput = {
  readonly requestId: string
  readonly providerId: string
  readonly modelId: string
  readonly systemPrompt?: string
  /** Oldest first. The last entry must be a user turn: it becomes `userPrompt`. */
  readonly messages: readonly Message[]
  readonly parameters?: Partial<CompletionParameters>
}

export type ConversationError = { readonly reason: 'empty_conversation' } | { readonly reason: 'last_turn_is_not_from_the_user' }

export type BuildResult =
  | { readonly ok: true; readonly request: CompletionRequest }
  | { readonly ok: false; readonly error: ConversationError | { readonly reason: 'invalid_request'; readonly error: ValidationError } }

/**
 * Splits a conversation into the contract's shape.
 *
 * The last turn must come from the user. That is not a universal rule about
 * conversations — it is what this contract's `userPrompt` means, and refusing
 * is better than silently promoting an assistant turn into the question.
 */
export function buildRequest(input: ConversationInput): BuildResult {
  const last = input.messages.at(-1)
  if (!last) return { ok: false, error: { reason: 'empty_conversation' } }
  if (last.role !== 'user') return { ok: false, error: { reason: 'last_turn_is_not_from_the_user' } }

  const request: CompletionRequest = {
    requestId: input.requestId,
    providerId: input.providerId,
    modelId: input.modelId,
    systemPrompt: input.systemPrompt ?? '',
    userPrompt: last.content,
    priorMessages: input.messages.slice(0, -1),
    parameters: { ...DEFAULT_PARAMETERS, ...input.parameters },
  }

  const invalid = validateRequest(request)
  if (invalid) return { ok: false, error: { reason: 'invalid_request', error: invalid } }
  return { ok: true, request }
}

/** Why an untrusted request document was refused: its shape, or its values. */
export type RequestRejection =
  | { readonly stage: 'decode'; readonly issue: DecodeIssue }
  | { readonly stage: 'validate'; readonly error: ValidationError }

export type RequestResult =
  | { readonly ok: true; readonly request: CompletionRequest }
  | { readonly ok: false; readonly rejection: RequestRejection }

/**
 * Decodes and validates an untrusted request document in one step.
 *
 * Use this at an HTTP or IPC boundary. `decodeRequestShape` alone proves shape
 * only, which is why the two stages stay distinguishable in the rejection.
 */
export function decodeRequest(document: unknown): RequestResult {
  const decoded = decodeRequestShape(document)
  if (!decoded.ok) return { ok: false, rejection: { stage: 'decode', issue: decoded.issue } }
  const invalid = validateRequest(decoded.value)
  if (invalid) return { ok: false, rejection: { stage: 'validate', error: invalid } }
  return { ok: true, request: decoded.value }
}
