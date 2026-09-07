/**
 * Mapping the underlying AI SDK's failures onto our closed vocabulary.
 *
 * This is the whole reason the adapter exists. A consumer that branched on
 * `APICallError` and an HTTP status would be coupled to that library's major
 * versions and to each provider's status conventions; a consumer that branches
 * on `rate_limited` is coupled to a meaning that survives both.
 *
 * Nothing from the provider's response body crosses this boundary. Bodies carry
 * echoed prompts and, from some providers, the key prefix. Only a bounded,
 * normalized message does.
 */

import { APICallError, InvalidPromptError, LoadAPIKeyError, NoSuchModelError, TypeValidationError } from 'ai'
import { providerError, type ProviderError, type ProviderErrorCategory, type RecoveryAction } from '@ai-sdk-local/core'

import type { CredentialRefusal } from './ports.js'

/**
 * The status codes with a meaning every provider agrees on.
 *
 * A code absent here is not guessed at. `transport_failure` says "the exchange
 * failed", which is true and useful; inventing a category from a 418 would not
 * be.
 */
function fromStatus(status: number | undefined): { category: ProviderErrorCategory; recovery: RecoveryAction } | null {
  switch (status) {
    case 401:
    case 403:
      return { category: 'invalid_credential', recovery: 'configure_credential' }
    case 404:
      return { category: 'unavailable_provider', recovery: 'choose_different_model' }
    case 400:
    case 422:
      return { category: 'rejected_request', recovery: 'reduce_request' }
    case 402:
      return { category: 'quota_exhausted', recovery: 'contact_provider' }
    case 413:
      return { category: 'rejected_request', recovery: 'reduce_request' }
    case 429:
      return { category: 'rate_limited', recovery: 'retry' }
    case 500:
    case 502:
    case 503:
    case 504:
      return { category: 'transport_failure', recovery: 'check_provider_status' }
    default:
      return null
  }
}

export function credentialError(providerId: string, refusal: CredentialRefusal, message: string): ProviderError {
  return providerError(
    providerId,
    refusal === 'missing' ? 'missing_credential' : 'invalid_credential',
    message,
    'configure_credential',
  )
}

export function unauthorizedModelError(providerId: string): ProviderError {
  return providerError(providerId, 'rejected_request', 'this model is not permitted for this application', 'choose_different_model')
}

/** Translates whatever the underlying library threw into our typed failure. */
export function toProviderError(providerId: string, thrown: unknown): ProviderError {
  if (APICallError.isInstance(thrown)) {
    const mapped = fromStatus(thrown.statusCode)
    if (mapped) return providerError(providerId, mapped.category, statusMessage(thrown.statusCode), mapped.recovery)
    return providerError(providerId, 'transport_failure', 'the provider call failed', 'check_provider_status')
  }

  if (LoadAPIKeyError.isInstance(thrown)) {
    return providerError(providerId, 'missing_credential', 'no usable api key was available', 'configure_credential')
  }

  if (NoSuchModelError.isInstance(thrown)) {
    return providerError(providerId, 'unavailable_provider', 'the provider does not serve this model', 'choose_different_model')
  }

  if (InvalidPromptError.isInstance(thrown)) {
    return providerError(providerId, 'rejected_request', 'the provider rejected the conversation', 'reduce_request')
  }

  if (TypeValidationError.isInstance(thrown)) {
    return providerError(providerId, 'malformed_response', "the provider's response did not match its protocol", 'none')
  }

  if (isNetworkFailure(thrown)) {
    return providerError(providerId, 'transport_failure', 'the provider could not be reached', 'check_provider_status')
  }

  return providerError(providerId, 'internal_failure', 'the completion failed inside the adapter', 'none')
}

/**
 * A status-only sentence.
 *
 * Deliberately not the provider's own text: several providers echo part of the
 * request back in it, and this string is destined for a UI.
 */
function statusMessage(status: number | undefined): string {
  return status === undefined ? 'the provider call failed' : `the provider responded with status ${status}`
}

function isNetworkFailure(thrown: unknown): boolean {
  if (!(thrown instanceof Error)) return false
  return thrown.name === 'TypeError' && /fetch|network/i.test(thrown.message)
}
