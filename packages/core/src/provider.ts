/**
 * The provider port.
 *
 * A provider produces text and returns how the run ended. It does not number
 * deltas, enforce the byte cap, decide cancellation, or emit events — the
 * runtime owns those, so every provider gets the same guarantees without
 * re-implementing them, and a provider that gets them wrong is caught rather
 * than trusted.
 *
 * The signature is an async generator so that a provider can be written as a
 * loop over its own stream: `yield` a chunk of text, `return` a terminal. The
 * runtime drives it by hand and always calls `return()` on the way out, so a
 * provider's `finally` runs even when the consumer stops reading early.
 */

import type { CompletionRequest, CompletionTerminal, ProviderError, ProviderErrorCategory, RecoveryAction } from './contracts.js'
import { boundedMessage } from './text.js'

export type CompletionProvider = {
  /** Must equal the `providerId` of requests routed here. */
  readonly providerId: string
  /**
   * Runs one completion.
   *
   * `signal` aborts for cancellation and for timeout alike; the terminal the
   * runtime commits distinguishes them, so a provider need not.
   */
  run(request: CompletionRequest, signal: AbortSignal): AsyncGenerator<string, CompletionTerminal, void>
}

/** Builds an error with a normalized, bounded message. */
export function providerError(
  providerId: string,
  category: ProviderErrorCategory,
  message: string,
  recoveryAction: RecoveryAction,
): ProviderError {
  return { providerId, category, message: boundedMessage(message), recoveryAction }
}

export function errorTerminal(
  providerId: string,
  category: ProviderErrorCategory,
  message: string,
  recoveryAction: RecoveryAction,
): CompletionTerminal {
  return { type: 'provider_error', error: providerError(providerId, category, message, recoveryAction) }
}
