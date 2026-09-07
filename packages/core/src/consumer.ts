/**
 * The consuming end of an event stream.
 *
 * A consumer that trusts its producer cannot tell a correct stream from a
 * broken one, and the failure shows up as garbled text rather than an error. So
 * this checks what the invariants promise: one request id, sequences starting
 * at zero and increasing by one, at most one terminal, and nothing after it.
 *
 * Checking is not repair. A gap is reported, never filled; text is not
 * reordered; a foreign id is refused rather than adopted.
 */

import type { CompletionEvent, ProviderError, Usage } from './contracts.js'

export type StreamViolation =
  | { readonly kind: 'foreign_request_id'; readonly expected: string; readonly received: string }
  | { readonly kind: 'out_of_order_sequence'; readonly expected: number; readonly received: number }
  | { readonly kind: 'event_after_terminal'; readonly received: CompletionEvent['type'] }
  | { readonly kind: 'no_terminal' }

export type CompletionOutcome =
  | { readonly status: 'done'; readonly text: string; readonly usage: Usage | null }
  | { readonly status: 'cancelled'; readonly text: string }
  | { readonly status: 'timeout'; readonly text: string }
  | { readonly status: 'provider_error'; readonly text: string; readonly error: ProviderError }
  | { readonly status: 'violated'; readonly text: string; readonly violation: StreamViolation }

export type ConsumeOptions = {
  /** Rejects any event carrying a different id. Omit to adopt the first one seen. */
  readonly requestId?: string
  /** Called for each accepted delta, before the outcome resolves. */
  readonly onText?: (text: string) => void
}

/**
 * Folds a stream into one outcome, or the first invariant it broke.
 *
 * The accumulated text is returned in every case, including a violation: a
 * partial answer is often still worth showing, and hiding it would be the
 * caller's decision to make, not this function's.
 */
export async function consumeEvents(
  events: AsyncIterable<CompletionEvent>,
  options: ConsumeOptions = {},
): Promise<CompletionOutcome> {
  let requestId = options.requestId
  let expectedSequence = 0
  let text = ''
  let terminal: CompletionOutcome | null = null

  for await (const event of events) {
    if (requestId === undefined) requestId = event.requestId
    if (event.requestId !== requestId) {
      return { status: 'violated', text, violation: { kind: 'foreign_request_id', expected: requestId, received: event.requestId } }
    }
    if (terminal) {
      return { status: 'violated', text, violation: { kind: 'event_after_terminal', received: event.type } }
    }

    switch (event.type) {
      case 'delta': {
        if (event.sequence !== expectedSequence) {
          return {
            status: 'violated',
            text,
            violation: { kind: 'out_of_order_sequence', expected: expectedSequence, received: event.sequence },
          }
        }
        expectedSequence += 1
        text += event.text
        options.onText?.(event.text)
        break
      }
      case 'done':
        terminal = { status: 'done', text, usage: event.usage }
        break
      case 'cancelled':
        terminal = { status: 'cancelled', text }
        break
      case 'timeout':
        terminal = { status: 'timeout', text }
        break
      case 'provider_error':
        terminal = { status: 'provider_error', text, error: event.error }
        break
    }
  }

  if (!terminal) return { status: 'violated', text, violation: { kind: 'no_terminal' } }
  return terminal.status === 'done' ? { ...terminal, text } : { ...terminal, text }
}

/** Accumulates deltas into text, without interpreting the terminal. */
export function accumulate(events: readonly CompletionEvent[]): string {
  let text = ''
  for (const event of events) {
    if (event.type === 'delta') text += event.text
  }
  return text
}
