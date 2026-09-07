/**
 * The consuming end: invariant checking, and the NDJSON round trip that carries
 * events over a byte stream.
 */

import { describe, expect, test } from 'bun:test'

import {
  MAX_NDJSON_LINE_BYTES,
  NdjsonError,
  accumulate,
  consumeEvents,
  createFakeProvider,
  createRuntime,
  defaultParameters,
  fromNdjsonStream,
  successScript,
  toNdjsonStream,
  type CompletionEvent,
} from '../src/index.ts'

async function* iterate(events: readonly CompletionEvent[]): AsyncGenerator<CompletionEvent, void, void> {
  for (const event of events) yield event
}

function bytes(text: string): ReadableStream<Uint8Array> {
  const encoded = new TextEncoder().encode(text)
  return new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(encoded)
      controller.close()
    },
  })
}

describe('the consumer checks rather than repairs', () => {
  test('accumulates a well-formed stream', async () => {
    const outcome = await consumeEvents(
      iterate([
        { type: 'delta', requestId: 'r', sequence: 0, text: 'he' },
        { type: 'delta', requestId: 'r', sequence: 1, text: 'llo' },
        { type: 'done', requestId: 'r', usage: null },
      ]),
    )

    expect(outcome.status).toBe('done')
    expect(outcome.text).toBe('hello')
  })

  test('refuses a foreign request id instead of adopting it', async () => {
    const outcome = await consumeEvents(
      iterate([
        { type: 'delta', requestId: 'mine', sequence: 0, text: 'a' },
        { type: 'delta', requestId: 'someone-elses', sequence: 1, text: 'b' },
      ]),
      { requestId: 'mine' },
    )

    expect(outcome.status).toBe('violated')
    if (outcome.status !== 'violated') return
    expect(outcome.violation.kind).toBe('foreign_request_id')
    expect(outcome.text).toBe('a')
  })

  test('reports a sequence gap rather than filling it', async () => {
    const outcome = await consumeEvents(
      iterate([
        { type: 'delta', requestId: 'r', sequence: 0, text: 'a' },
        { type: 'delta', requestId: 'r', sequence: 2, text: 'c' },
      ]),
    )

    expect(outcome.status).toBe('violated')
    if (outcome.status !== 'violated') return
    expect(outcome.violation).toEqual({ kind: 'out_of_order_sequence', expected: 1, received: 2 })
  })

  test('rejects anything after a terminal', async () => {
    const outcome = await consumeEvents(
      iterate([
        { type: 'done', requestId: 'r', usage: null },
        { type: 'delta', requestId: 'r', sequence: 0, text: 'late' },
      ]),
    )

    expect(outcome.status).toBe('violated')
    if (outcome.status !== 'violated') return
    expect(outcome.violation.kind).toBe('event_after_terminal')
  })

  test('a stream that just stops is a violation, not a silent success', async () => {
    const outcome = await consumeEvents(iterate([{ type: 'delta', requestId: 'r', sequence: 0, text: 'a' }]))

    expect(outcome.status).toBe('violated')
    if (outcome.status !== 'violated') return
    expect(outcome.violation.kind).toBe('no_terminal')
  })

  test('accumulate ignores terminals', () => {
    expect(
      accumulate([
        { type: 'delta', requestId: 'r', sequence: 0, text: 'a' },
        { type: 'timeout', requestId: 'r' },
      ]),
    ).toBe('a')
  })
})

describe('ndjson', () => {
  test('round-trips a real run through bytes', async () => {
    const runtime = createRuntime({ providers: [createFakeProvider(successScript(['hel', 'lo'], { inputTokens: 2, outputTokens: 2 }))] })
    const started = runtime.stream({
      requestId: 'request-1',
      providerId: 'fake',
      modelId: 'model',
      systemPrompt: '',
      userPrompt: 'hi',
      priorMessages: [],
      parameters: defaultParameters(),
    })
    if (!started.ok) throw new Error('expected admission')

    const outcome = await consumeEvents(fromNdjsonStream(toNdjsonStream(started.events)))

    expect(outcome.status).toBe('done')
    expect(outcome.text).toBe('hello')
    if (outcome.status !== 'done') return
    expect(outcome.usage).toEqual({ inputTokens: 2, outputTokens: 2 })
  })

  test('skips blank lines and decodes a final line without a newline', async () => {
    const body = '\n{"type":"delta","requestId":"r","sequence":0,"text":"a"}\n\n{"type":"done","requestId":"r","usage":null}'
    const collected: CompletionEvent[] = []
    for await (const event of fromNdjsonStream(bytes(body))) collected.push(event)

    expect(collected.map((event) => event.type)).toEqual(['delta', 'done'])
  })

  test('decodes a multi-byte character split across chunks', async () => {
    const encoded = new TextEncoder().encode('{"type":"delta","requestId":"r","sequence":0,"text":"🙂"}\n')
    const split = 55
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(encoded.slice(0, split))
        controller.enqueue(encoded.slice(split))
        controller.close()
      },
    })

    const collected: CompletionEvent[] = []
    for await (const event of fromNdjsonStream(body)) collected.push(event)

    expect(collected).toHaveLength(1)
    expect(collected[0]?.type === 'delta' && collected[0].text).toBe('🙂')
  })

  test('refuses a line that is not JSON', async () => {
    const iterator = fromNdjsonStream(bytes('not json at all\n'))
    await expect(iterator.next()).rejects.toBeInstanceOf(NdjsonError)
  })

  test('refuses a well-formed line that is not a valid event', async () => {
    const iterator = fromNdjsonStream(bytes('{"type":"finished","requestId":"r"}\n'))
    await expect(iterator.next()).rejects.toBeInstanceOf(NdjsonError)
  })

  test('refuses an unbounded line rather than buffering it', async () => {
    const iterator = fromNdjsonStream(bytes('x'.repeat(2048)), 1024)
    await expect(iterator.next()).rejects.toBeInstanceOf(NdjsonError)
    expect(MAX_NDJSON_LINE_BYTES).toBe(1024 * 1024)
  })

  test('cancelling the reader stops the producer', async () => {
    let closed = false
    const events = (async function* (): AsyncGenerator<CompletionEvent, void, void> {
      try {
        yield { type: 'delta', requestId: 'r', sequence: 0, text: 'a' }
        yield { type: 'delta', requestId: 'r', sequence: 1, text: 'b' }
      } finally {
        closed = true
      }
    })()

    const reader = toNdjsonStream(events).getReader()
    await reader.read()
    await reader.cancel('done reading')

    expect(closed).toBe(true)
  })
})
