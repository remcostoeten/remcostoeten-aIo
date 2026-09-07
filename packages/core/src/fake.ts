/**
 * A provider that does exactly what its script says.
 *
 * Its purpose is that the same script produces the same segmentation and the
 * same outcome in Rust and in TypeScript, which is what makes cross-language
 * conformance checkable without a network or a key. Real providers chunk text
 * however their transport happens to flush; the fake does not.
 *
 * It lives in core deliberately. "No provider names in core" excludes vendor
 * dispatch and URLs, not a deterministic test double.
 */

import type { CompletionRequest, CompletionTerminal, ProviderErrorCategory, RecoveryAction, Usage } from './contracts.js'
import { providerError, type CompletionProvider } from './provider.js'

export type FakeOutcome =
  | { readonly type: 'done'; readonly usage?: Usage }
  | { readonly type: 'timeout' }
  /** Emits its segments, then breaks the contract by yielding a non-string. */
  | { readonly type: 'malformed_output' }
  | {
      readonly type: 'provider_error'
      readonly category: ProviderErrorCategory
      readonly message: string
      readonly recoveryAction: RecoveryAction
    }

export type FakeScript = {
  /** Emitted in order, one delta each. */
  readonly segments: readonly string[]
  readonly outcome: FakeOutcome
  /** Milliseconds to wait before each segment. Cancellation is polled across it. */
  readonly delayMs?: number
}

export function successScript(segments: readonly string[], usage?: Usage): FakeScript {
  return { segments, outcome: usage ? { type: 'done', usage } : { type: 'done' } }
}

export const FAKE_PROVIDER_ID = 'fake'

export function createFakeProvider(script: FakeScript, providerId = FAKE_PROVIDER_ID): CompletionProvider {
  return {
    providerId,
    async *run(_request: CompletionRequest, signal: AbortSignal): AsyncGenerator<string, CompletionTerminal, void> {
      for (const segment of script.segments) {
        if (signal.aborted) return { type: 'cancelled' }
        if (script.delayMs) await sleep(script.delayMs, signal)
        if (signal.aborted) return { type: 'cancelled' }
        yield segment
      }

      if (signal.aborted) return { type: 'cancelled' }

      switch (script.outcome.type) {
        case 'done':
          return { type: 'done', usage: script.outcome.usage ?? null }
        case 'timeout':
          return { type: 'timeout' }
        case 'malformed_output':
          // Deliberately off-contract, to prove the runtime catches it rather
          // than forwarding it to the consumer.
          yield 42 as unknown as string
          return { type: 'done', usage: null }
        case 'provider_error':
          return {
            type: 'provider_error',
            error: providerError(providerId, script.outcome.category, script.outcome.message, script.outcome.recoveryAction),
          }
      }
    },
  }
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(finish, ms)
    function finish() {
      clearTimeout(timer)
      signal.removeEventListener('abort', finish)
      resolve()
    }
    signal.addEventListener('abort', finish, { once: true })
  })
}
