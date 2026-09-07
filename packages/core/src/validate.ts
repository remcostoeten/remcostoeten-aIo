/**
 * The semantic bounds, mirroring Rust's `validate` methods.
 *
 * Structural decoding proves shape; these prove value. The split is the same in
 * both languages, and for the same reason: aggregate budgets (the combined
 * prompt), byte-measured limits, and closed identifier grammars cannot be
 * expressed in a JSON Schema, so a document that a schema accepts can still be
 * an invalid request.
 */

import {
  MAX_DELTA_BYTES,
  MAX_DURATION_MS,
  MAX_ERROR_MESSAGE_BYTES,
  MAX_IDENTIFIER_BYTES,
  MAX_OUTPUT_TOKENS,
  MAX_PRIOR_MESSAGES,
  MAX_PROMPT_BYTES,
  MAX_RESPONSE_BYTES,
  MAX_RETRIES,
  MAX_TOKEN_COUNT,
  type CompletionDelta,
  type CompletionParameters,
  type CompletionRequest,
  type Message,
  type ProviderError,
  type Usage,
} from './contracts.js'
import { isIdentifierGrammar, utf8ByteLength } from './text.js'

/**
 * Why a value was rejected.
 *
 * `field` is a static label from a closed set, never caller-controlled text, so
 * it is safe to surface and safe to branch on.
 */
export type ValidationError =
  | { readonly kind: 'empty'; readonly field: string }
  | { readonly kind: 'too_long'; readonly field: string; readonly maximum: number }
  | { readonly kind: 'invalid_identifier'; readonly field: string }
  | { readonly kind: 'prompt_too_long'; readonly maximum: number }
  | { readonly kind: 'invalid_output_limit'; readonly maximum: number }
  | { readonly kind: 'invalid_timeout'; readonly maximum: number }
  | { readonly kind: 'too_many_retries'; readonly maximum: number }
  | { readonly kind: 'invalid_sampling_parameter'; readonly field: string }
  | { readonly kind: 'token_count_too_large'; readonly field: string; readonly maximum: number }
  | { readonly kind: 'too_many_messages'; readonly maximum: number }
  | { readonly kind: 'invalid_output_token_limit'; readonly maximum: number }

export type ValidationResult = ValidationError | null

export function describeValidationError(error: ValidationError): string {
  switch (error.kind) {
    case 'empty':
      return `${error.field} cannot be empty`
    case 'too_long':
      return `${error.field} exceeds ${error.maximum} bytes`
    case 'invalid_identifier':
      return `${error.field} contains unsupported characters`
    case 'prompt_too_long':
      return `completion prompt exceeds ${error.maximum} bytes`
    case 'invalid_output_limit':
      return `maximum output bytes must be between 1 and ${error.maximum}`
    case 'invalid_timeout':
      return `completion timeout must be between 1 and ${error.maximum} milliseconds`
    case 'too_many_retries':
      return `completion retries exceed ${error.maximum}`
    case 'invalid_sampling_parameter':
      return `${error.field} must be between 0 and 1000`
    case 'token_count_too_large':
      return `${error.field} exceeds ${error.maximum}`
    case 'too_many_messages':
      return `prior messages exceed ${error.maximum} turns`
    case 'invalid_output_token_limit':
      return `maximum output tokens must be between 1 and ${error.maximum}`
  }
}

export function validateIdentifier(field: string, value: string): ValidationResult {
  if (value === '') return { kind: 'empty', field }
  if (utf8ByteLength(value) > MAX_IDENTIFIER_BYTES) {
    return { kind: 'too_long', field, maximum: MAX_IDENTIFIER_BYTES }
  }
  if (!isIdentifierGrammar(value)) return { kind: 'invalid_identifier', field }
  return null
}

export function validateMessage(message: Message): ValidationResult {
  if (message.content === '') return { kind: 'empty', field: 'message content' }
  return null
}

export function validateParameters(parameters: CompletionParameters): ValidationResult {
  if (parameters.maxOutputBytes === 0 || parameters.maxOutputBytes > MAX_RESPONSE_BYTES) {
    return { kind: 'invalid_output_limit', maximum: MAX_RESPONSE_BYTES }
  }
  if (parameters.timeoutMs === 0 || parameters.timeoutMs > MAX_DURATION_MS) {
    return { kind: 'invalid_timeout', maximum: MAX_DURATION_MS }
  }
  if (parameters.retryCount > MAX_RETRIES) {
    return { kind: 'too_many_retries', maximum: MAX_RETRIES }
  }
  const temperature = validateSampling('temperature', parameters.temperatureMillis)
  if (temperature) return temperature
  const topP = validateSampling('top p', parameters.topPMillis)
  if (topP) return topP
  if (parameters.maxOutputTokens !== null && (parameters.maxOutputTokens === 0 || parameters.maxOutputTokens > MAX_OUTPUT_TOKENS)) {
    return { kind: 'invalid_output_token_limit', maximum: MAX_OUTPUT_TOKENS }
  }
  return null
}

/**
 * Checks identifiers, the combined conversation budget, and the parameters.
 *
 * The prompt budget spans the system prompt, every prior message and the final
 * user prompt together, not each of them separately.
 */
export function validateRequest(request: CompletionRequest): ValidationResult {
  const requestId = validateIdentifier('request id', request.requestId)
  if (requestId) return requestId
  const providerId = validateIdentifier('provider id', request.providerId)
  if (providerId) return providerId
  const modelId = validateIdentifier('model id', request.modelId)
  if (modelId) return modelId

  if (request.priorMessages.length > MAX_PRIOR_MESSAGES) {
    return { kind: 'too_many_messages', maximum: MAX_PRIOR_MESSAGES }
  }

  let promptBytes = utf8ByteLength(request.systemPrompt) + utf8ByteLength(request.userPrompt)
  for (const message of request.priorMessages) {
    const invalid = validateMessage(message)
    if (invalid) return invalid
    promptBytes += utf8ByteLength(message.content)
  }
  if (promptBytes > MAX_PROMPT_BYTES) return { kind: 'prompt_too_long', maximum: MAX_PROMPT_BYTES }

  return validateParameters(request.parameters)
}

export function validateDelta(delta: CompletionDelta): ValidationResult {
  const requestId = validateIdentifier('request id', delta.requestId)
  if (requestId) return requestId
  if (utf8ByteLength(delta.text) > MAX_DELTA_BYTES) {
    return { kind: 'too_long', field: 'completion delta', maximum: MAX_DELTA_BYTES }
  }
  return null
}

export function validateUsage(usage: Usage): ValidationResult {
  if (usage.inputTokens > MAX_TOKEN_COUNT) {
    return { kind: 'token_count_too_large', field: 'input tokens', maximum: MAX_TOKEN_COUNT }
  }
  if (usage.outputTokens > MAX_TOKEN_COUNT) {
    return { kind: 'token_count_too_large', field: 'output tokens', maximum: MAX_TOKEN_COUNT }
  }
  return null
}

export function validateProviderError(error: ProviderError): ValidationResult {
  const providerId = validateIdentifier('provider id', error.providerId)
  if (providerId) return providerId
  if (error.message === '') return { kind: 'empty', field: 'provider error message' }
  if (utf8ByteLength(error.message) > MAX_ERROR_MESSAGE_BYTES) {
    return { kind: 'too_long', field: 'provider error message', maximum: MAX_ERROR_MESSAGE_BYTES }
  }
  return null
}

function validateSampling(field: string, value: number | null): ValidationResult {
  if (value !== null && value > 1000) return { kind: 'invalid_sampling_parameter', field }
  return null
}
