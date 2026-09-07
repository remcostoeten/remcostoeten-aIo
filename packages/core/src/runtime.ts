/**
 * The run lifecycle: the TypeScript counterpart of `ai-core`'s completion
 * service, with the same invariants and the same stated limits.
 *
 * What a committed run guarantees: deltas carry its own request id, sequence
 * numbers start at zero and increase by one, the accumulated byte budget is
 * enforced here rather than trusted to the provider, and exactly one terminal
 * event is emitted. Cancellation observed before commitment turns Done into
 * Cancelled; a provider error and a timeout stay the typed outcome they are.
 *
 * What it does not: there is no retry, no fallback, and no provider switch.
 * Aborting cannot interrupt a provider wedged inside an un-abortable await —
 * the run terminalizes as cancelled and the generator is returned, but a
 * provider that ignores its signal keeps its own work alive.
 *
 * Start rejection is a result, not an exception: `stream` hands back a
 * discriminated union so a caller cannot mistake a request that never ran for a
 * run that produced no events.
 */

import {
  MAX_RESPONSE_BYTES,
  terminalToEvent,
  type CompletionEvent,
  type CompletionRequest,
  type CompletionTerminal,
} from './contracts.js'
import { errorTerminal, type CompletionProvider } from './provider.js'
import { utf8ByteLength } from './text.js'
import { describeValidationError, validateRequest, validateUsage, type ValidationError } from './validate.js'

/** Why a request was never admitted. No terminal follows a start rejection. */
export type StartError =
  | { readonly reason: 'duplicate_request'; readonly requestId: string }
  | { readonly reason: 'invalid_request'; readonly error: ValidationError }

export type StreamStart =
  | { readonly ok: true; readonly events: AsyncIterable<CompletionEvent> }
  | { readonly ok: false; readonly error: StartError }

export type StreamOptions = {
  /**
   * Caller-owned cancellation. Aborting it produces a `cancelled` terminal.
   *
   * A run also has its own timeout from `parameters.timeoutMs`; the two are
   * distinguished in the terminal, so a consumer can tell "the person stopped
   * it" from "it ran out of time".
   */
  readonly signal?: AbortSignal
}

export type Runtime = {
  /** Admits a request, or explains why it was not admitted. */
  stream(request: CompletionRequest, options?: StreamOptions): StreamStart
  /** Requests cancellation of an active run. False when there is none to cancel. */
  cancel(requestId: string): boolean
  /** Requests cancellation of every active run. Admission stays open. */
  shutdown(): void
}

export type RuntimeOptions = {
  readonly providers: readonly CompletionProvider[]
}

type ActiveRun = {
  readonly controller: AbortController
  cancelled: boolean
}

export function createRuntime(options: RuntimeOptions): Runtime {
  const providers = new Map<string, CompletionProvider>()
  for (const provider of options.providers) {
    providers.set(provider.providerId, provider)
  }
  const active = new Map<string, ActiveRun>()

  function stream(request: CompletionRequest, streamOptions: StreamOptions = {}): StreamStart {
    const invalid = validateRequest(request)
    if (invalid) return { ok: false, error: { reason: 'invalid_request', error: invalid } }

    // Reserved before the provider is looked up, so an unknown provider and a
    // registered one reject a duplicate id identically. The id stays reserved
    // through the terminal, so it cannot be reused while one is in flight.
    if (active.has(request.requestId)) {
      return { ok: false, error: { reason: 'duplicate_request', requestId: request.requestId } }
    }

    const controller = new AbortController()
    const run: ActiveRun = { controller, cancelled: false }
    active.set(request.requestId, run)

    return { ok: true, events: execute(request, run, providers.get(request.providerId), streamOptions.signal, active) }
  }

  function cancel(requestId: string): boolean {
    const run = active.get(requestId)
    if (!run || run.cancelled) return false
    run.cancelled = true
    run.controller.abort()
    return true
  }

  function shutdown(): void {
    for (const requestId of [...active.keys()]) cancel(requestId)
  }

  return { stream, cancel, shutdown }
}

async function* execute(
  request: CompletionRequest,
  run: ActiveRun,
  provider: CompletionProvider | undefined,
  callerSignal: AbortSignal | undefined,
  active: Map<string, ActiveRun>,
): AsyncGenerator<CompletionEvent, void, void> {
  const { requestId } = request
  let timer: ReturnType<typeof setTimeout> | undefined
  let timedOut = false
  const onCallerAbort = () => {
    run.cancelled = true
    run.controller.abort()
  }

  try {
    if (!provider) {
      yield terminalToEvent(
        errorTerminal(request.providerId, 'unavailable_provider', 'no provider is registered under this id', 'choose_different_model'),
        requestId,
      )
      return
    }

    if (callerSignal) {
      if (callerSignal.aborted) onCallerAbort()
      else callerSignal.addEventListener('abort', onCallerAbort, { once: true })
    }
    if (run.controller.signal.aborted) {
      yield terminalToEvent({ type: 'cancelled' }, requestId)
      return
    }

    timer = setTimeout(() => {
      timedOut = true
      run.controller.abort()
    }, request.parameters.timeoutMs)

    const byteBudget = Math.min(request.parameters.maxOutputBytes, MAX_RESPONSE_BYTES)
    const iterator = provider.run(request, run.controller.signal)
    let sequence = 0
    let accumulated = 0
    let terminal: CompletionTerminal | undefined

    try {
      let step = await iterator.next()
      while (!step.done) {
        const text = step.value
        if (typeof text !== 'string') {
          terminal = errorTerminal(request.providerId, 'malformed_response', 'the provider produced a non-text chunk', 'none')
          break
        }
        accumulated += utf8ByteLength(text)
        if (accumulated > byteBudget) {
          terminal = errorTerminal(
            request.providerId,
            'malformed_response',
            'the provider exceeded the requested output budget',
            'reduce_request',
          )
          break
        }
        yield { type: 'delta', requestId, sequence, text }
        sequence += 1
        step = await iterator.next()
      }
      if (!terminal && step.done) terminal = normalizeTerminal(step.value, request.providerId)
    } catch (error) {
      terminal = fromThrown(error, request.providerId, timedOut, run.cancelled)
    } finally {
      await iterator.return({ type: 'cancelled' }).catch(() => undefined)
    }

    terminal ??= errorTerminal(request.providerId, 'malformed_response', 'the provider ended without a terminal', 'none')

    // Precedence matches Rust: a provider error stays a provider error, because
    // relabelling it would erase the failure the caller needs to see.
    //
    // The deadline outranks cancellation, and it must: the runtime aborts the
    // signal to enforce the timeout, so a cooperative provider reports back
    // `cancelled` for what was in fact a timeout. Only the runtime knows which
    // it was.
    if (timedOut && (terminal.type === 'done' || terminal.type === 'cancelled')) terminal = { type: 'timeout' }
    else if (run.cancelled && terminal.type === 'done') terminal = { type: 'cancelled' }

    yield terminalToEvent(terminal, requestId)
  } finally {
    if (timer !== undefined) clearTimeout(timer)
    callerSignal?.removeEventListener('abort', onCallerAbort)
    active.delete(requestId)
  }
}

/**
 * Checks what a provider returned before it becomes the run's terminal.
 *
 * A provider is not trusted to have validated its own usage: an out-of-range
 * count would otherwise cross the wire and fail on the far side, where nothing
 * can attribute it.
 */
function normalizeTerminal(terminal: CompletionTerminal, providerId: string): CompletionTerminal {
  if (terminal.type === 'done' && terminal.usage) {
    const invalid = validateUsage(terminal.usage)
    if (invalid) {
      return errorTerminal(providerId, 'malformed_response', `provider usage is out of range: ${describeValidationError(invalid)}`, 'none')
    }
  }
  return terminal
}

function fromThrown(error: unknown, providerId: string, timedOut: boolean, cancelled: boolean): CompletionTerminal {
  if (timedOut) return { type: 'timeout' }
  if (cancelled || isAbort(error)) return { type: 'cancelled' }
  return errorTerminal(providerId, 'internal_failure', describeThrown(error), 'none')
}

function isAbort(error: unknown): boolean {
  return error instanceof Error && error.name === 'AbortError'
}

/**
 * Renders a thrown value without letting a provider body escape.
 *
 * Only the message reaches the caller, and `boundedMessage` truncates it. A
 * stack, a cause chain, or a raw response could carry prompts or a key.
 */
function describeThrown(error: unknown): string {
  if (error instanceof Error && error.message !== '') return error.message
  return 'the provider failed'
}
