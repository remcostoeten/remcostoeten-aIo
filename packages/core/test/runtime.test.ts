/**
 * The run lifecycle, and the guarantees it makes about a stream.
 *
 * Every test here has a Rust counterpart in `crates/ai-core/tests/service.rs`,
 * because the two runtimes are supposed to be the same seam in two languages.
 */

import { describe, expect, test } from 'bun:test'

import {
  MAX_RESPONSE_BYTES,
  consumeEvents,
  createFakeProvider,
  createRuntime,
  defaultParameters,
  successScript,
  type CompletionEvent,
  type CompletionProvider,
  type CompletionRequest,
  type CompletionTerminal,
} from '../src/index.ts'

function request(overrides: Partial<CompletionRequest> = {}): CompletionRequest {
  return {
    requestId: 'request-1',
    providerId: 'fake',
    modelId: 'model',
    systemPrompt: 'system',
    userPrompt: 'user',
    priorMessages: [],
    parameters: defaultParameters(),
    ...overrides,
  }
}

async function collect(events: AsyncIterable<CompletionEvent>): Promise<CompletionEvent[]> {
  const collected: CompletionEvent[] = []
  for await (const event of events) collected.push(event)
  return collected
}

function runtimeWith(provider: CompletionProvider) {
  return createRuntime({ providers: [provider] })
}

describe('admission', () => {
  test('an invalid request is a start rejection, and produces no terminal', () => {
    const runtime = runtimeWith(createFakeProvider(successScript(['hi'])))
    const started = runtime.stream(request({ requestId: 'not a valid identifier' }))

    expect(started.ok).toBe(false)
    if (started.ok) return
    expect(started.error.reason).toBe('invalid_request')
  })

  test('a duplicate id is refused while the first run is in flight', async () => {
    const runtime = runtimeWith(createFakeProvider(successScript(['hi'])))
    const first = runtime.stream(request())
    expect(first.ok).toBe(true)

    const second = runtime.stream(request())
    expect(second.ok).toBe(false)
    if (second.ok) return
    expect(second.error.reason).toBe('duplicate_request')

    if (first.ok) await collect(first.events)
  })

  test('an unknown provider refuses a duplicate id on the same path', async () => {
    const runtime = createRuntime({ providers: [] })
    const first = runtime.stream(request())
    expect(first.ok).toBe(true)

    const second = runtime.stream(request())
    expect(second.ok).toBe(false)

    if (!first.ok) return
    const events = await collect(first.events)
    expect(events).toHaveLength(1)
    expect(events[0]?.type).toBe('provider_error')
    if (events[0]?.type !== 'provider_error') return
    expect(events[0].error.category).toBe('unavailable_provider')
  })

  test('an id is reusable once its run has ended', async () => {
    const runtime = runtimeWith(createFakeProvider(successScript(['hi'])))
    const first = runtime.stream(request())
    if (!first.ok) throw new Error('expected admission')
    await collect(first.events)

    expect(runtime.stream(request()).ok).toBe(true)
  })
})

describe('stream invariants', () => {
  test('deltas carry the request id and gapless sequences from zero', async () => {
    const runtime = runtimeWith(createFakeProvider(successScript(['a', 'b', 'c'])))
    const started = runtime.stream(request())
    if (!started.ok) throw new Error('expected admission')

    const events = await collect(started.events)

    expect(events.map((event) => event.type)).toEqual(['delta', 'delta', 'delta', 'done'])
    expect(events.filter((event) => event.type === 'delta').map((event) => event.sequence)).toEqual([0, 1, 2])
    expect(events.every((event) => event.requestId === 'request-1')).toBe(true)
  })

  test('exactly one terminal ends the stream', async () => {
    const runtime = runtimeWith(createFakeProvider(successScript(['a'])))
    const started = runtime.stream(request())
    if (!started.ok) throw new Error('expected admission')

    const events = await collect(started.events)
    const terminals = events.filter((event) => event.type !== 'delta')
    expect(terminals).toHaveLength(1)
    expect(events.at(-1)).toBe(terminals[0]!)
  })

  test('a fixed script reproduces its segmentation exactly', async () => {
    const runtime = runtimeWith(createFakeProvider(successScript(['hel', 'lo ', 'world'])))
    const started = runtime.stream(request())
    if (!started.ok) throw new Error('expected admission')

    const outcome = await consumeEvents(started.events)
    expect(outcome.status).toBe('done')
    expect(outcome.text).toBe('hello world')
  })

  test('provider-reported usage reaches the terminal', async () => {
    const runtime = runtimeWith(createFakeProvider(successScript(['hi'], { inputTokens: 4, outputTokens: 1 })))
    const started = runtime.stream(request())
    if (!started.ok) throw new Error('expected admission')

    const outcome = await consumeEvents(started.events)
    expect(outcome.status).toBe('done')
    if (outcome.status !== 'done') return
    expect(outcome.usage).toEqual({ inputTokens: 4, outputTokens: 1 })
  })
})

describe('the runtime does not trust its provider', () => {
  test('output beyond the requested budget becomes a malformed response', async () => {
    const runtime = runtimeWith(createFakeProvider(successScript(['12345678', '90'])))
    const started = runtime.stream(request({ parameters: { ...defaultParameters(), maxOutputBytes: 8 } }))
    if (!started.ok) throw new Error('expected admission')

    const outcome = await consumeEvents(started.events)
    expect(outcome.status).toBe('provider_error')
    if (outcome.status !== 'provider_error') return
    expect(outcome.error.category).toBe('malformed_response')
    expect(outcome.text).toBe('12345678')
  })

  test('the request budget cannot exceed the hard ceiling', async () => {
    const runtime = runtimeWith(createFakeProvider(successScript(['ok'])))
    const started = runtime.stream(request({ parameters: { ...defaultParameters(), maxOutputBytes: MAX_RESPONSE_BYTES } }))
    if (!started.ok) throw new Error('expected admission')
    expect((await consumeEvents(started.events)).status).toBe('done')
  })

  test('a non-text chunk is caught rather than forwarded', async () => {
    const runtime = runtimeWith(createFakeProvider({ segments: ['ok'], outcome: { type: 'malformed_output' } }))
    const started = runtime.stream(request())
    if (!started.ok) throw new Error('expected admission')

    const outcome = await consumeEvents(started.events)
    expect(outcome.status).toBe('provider_error')
    if (outcome.status !== 'provider_error') return
    expect(outcome.error.category).toBe('malformed_response')
  })

  test('out-of-range usage is rejected rather than sent onward', async () => {
    const provider: CompletionProvider = {
      providerId: 'fake',
      // eslint-disable-next-line require-yield
      async *run(): AsyncGenerator<string, CompletionTerminal, void> {
        return { type: 'done', usage: { inputTokens: 5_000_000_000, outputTokens: 0 } }
      },
    }
    const started = runtimeWith(provider).stream(request())
    if (!started.ok) throw new Error('expected admission')

    const outcome = await consumeEvents(started.events)
    expect(outcome.status).toBe('provider_error')
    if (outcome.status !== 'provider_error') return
    expect(outcome.error.category).toBe('malformed_response')
  })

  test('a provider that throws becomes a typed internal failure, not a leaked stack', async () => {
    const provider: CompletionProvider = {
      providerId: 'fake',
      // eslint-disable-next-line require-yield
      async *run(): AsyncGenerator<string, CompletionTerminal, void> {
        throw new Error('provider exploded')
      },
    }
    const started = runtimeWith(provider).stream(request())
    if (!started.ok) throw new Error('expected admission')

    const outcome = await consumeEvents(started.events)
    expect(outcome.status).toBe('provider_error')
    if (outcome.status !== 'provider_error') return
    expect(outcome.error.category).toBe('internal_failure')
    expect(outcome.error.message).toBe('provider exploded')
  })
})

describe('cancellation and timeout stay distinguishable', () => {
  test('cancelling mid-run commits a cancelled terminal', async () => {
    const runtime = runtimeWith(createFakeProvider({ segments: ['a', 'b', 'c'], outcome: { type: 'done' }, delayMs: 5 }))
    const started = runtime.stream(request())
    if (!started.ok) throw new Error('expected admission')

    const collected: CompletionEvent[] = []
    for await (const event of started.events) {
      collected.push(event)
      if (event.type === 'delta' && event.sequence === 0) runtime.cancel('request-1')
    }

    expect(collected.at(-1)?.type).toBe('cancelled')
  })

  test('a caller signal cancels the run', async () => {
    const controller = new AbortController()
    const runtime = runtimeWith(createFakeProvider({ segments: ['a', 'b'], outcome: { type: 'done' }, delayMs: 5 }))
    const started = runtime.stream(request(), { signal: controller.signal })
    if (!started.ok) throw new Error('expected admission')

    const collected: CompletionEvent[] = []
    for await (const event of started.events) {
      collected.push(event)
      controller.abort()
    }

    expect(collected.at(-1)?.type).toBe('cancelled')
  })

  test('an already-aborted signal terminalizes before the provider runs', async () => {
    let entered = false
    const provider: CompletionProvider = {
      providerId: 'fake',
      // eslint-disable-next-line require-yield
      async *run(): AsyncGenerator<string, CompletionTerminal, void> {
        entered = true
        return { type: 'done', usage: null }
      },
    }
    const started = runtimeWith(provider).stream(request(), { signal: AbortSignal.abort() })
    if (!started.ok) throw new Error('expected admission')

    const events = await collect(started.events)
    expect(events).toEqual([{ type: 'cancelled', requestId: 'request-1' }])
    expect(entered).toBe(false)
  })

  test('the deadline produces a timeout, not a cancellation', async () => {
    const runtime = runtimeWith(createFakeProvider({ segments: ['a', 'b'], outcome: { type: 'done' }, delayMs: 40 }))
    const started = runtime.stream(request({ parameters: { ...defaultParameters(), timeoutMs: 10 } }))
    if (!started.ok) throw new Error('expected admission')

    expect((await consumeEvents(started.events)).status).toBe('timeout')
  })

  test('a provider error survives a cancellation flag rather than being relabelled', async () => {
    // A provider that reports its own failure even though it was asked to stop:
    // the failure is the real outcome, and cancellation must not hide it.
    const provider: CompletionProvider = {
      providerId: 'fake',
      async *run(): AsyncGenerator<string, CompletionTerminal, void> {
        yield 'partial'
        return {
          type: 'provider_error',
          error: { providerId: 'fake', category: 'rate_limited', message: 'slow down', recoveryAction: 'retry' },
        }
      },
    }
    const runtime = runtimeWith(provider)
    const started = runtime.stream(request())
    if (!started.ok) throw new Error('expected admission')

    const outcome = await consumeEvents(started.events, {
      onText: () => {
        runtime.cancel('request-1')
      },
    })
    expect(outcome.status).toBe('provider_error')
    if (outcome.status !== 'provider_error') return
    expect(outcome.error.category).toBe('rate_limited')
  })

  test('cancelling an unknown run reports that there was nothing to cancel', () => {
    expect(createRuntime({ providers: [] }).cancel('request-1')).toBe(false)
  })

  test('shutdown cancels active runs without closing admission', async () => {
    const runtime = runtimeWith(createFakeProvider({ segments: ['a', 'b'], outcome: { type: 'done' }, delayMs: 5 }))
    const started = runtime.stream(request())
    if (!started.ok) throw new Error('expected admission')
    runtime.shutdown()

    expect((await consumeEvents(started.events)).status).toBe('cancelled')
    expect(runtime.stream(request({ requestId: 'request-2' })).ok).toBe(true)
  })
})

describe('closing the iterator early', () => {
  test("runs the provider's cleanup and frees the id", async () => {
    let cleaned = false
    const provider: CompletionProvider = {
      providerId: 'fake',
      async *run(): AsyncGenerator<string, CompletionTerminal, void> {
        try {
          yield 'first'
          yield 'second'
          return { type: 'done', usage: null }
        } finally {
          cleaned = true
        }
      },
    }
    const runtime = runtimeWith(provider)
    const started = runtime.stream(request())
    if (!started.ok) throw new Error('expected admission')

    for await (const event of started.events) {
      if (event.type === 'delta') break
    }

    expect(cleaned).toBe(true)
    expect(runtime.stream(request()).ok).toBe(true)
  })
})
