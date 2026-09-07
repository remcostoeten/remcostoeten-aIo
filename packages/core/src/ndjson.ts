/**
 * Carrying events over a byte stream, as newline-delimited JSON.
 *
 * Built on Web Streams and `TextDecoder` only, so the same code runs in a
 * browser, in Node, in Bun, and in a serverless runtime. Nothing here is
 * HTTP-aware: an application owns its status codes, headers and route policy.
 *
 * The reader is bounded. A producer that never emits a newline would otherwise
 * grow one line until the process runs out of memory, so an over-long line is a
 * typed failure instead.
 */

import type { CompletionEvent } from './contracts.js'
import { decodeEvent, type DecodeIssue } from './decode.js'

/** A line longer than this is refused rather than buffered. */
export const MAX_NDJSON_LINE_BYTES = 1024 * 1024

export type NdjsonFailure =
  | { readonly kind: 'line_too_long'; readonly maximum: number }
  | { readonly kind: 'not_json'; readonly line: string }
  | { readonly kind: 'invalid_event'; readonly issue: DecodeIssue }

export class NdjsonError extends Error {
  readonly failure: NdjsonFailure

  constructor(failure: NdjsonFailure) {
    super(describeNdjsonFailure(failure))
    this.name = 'NdjsonError'
    this.failure = failure
  }
}

export function describeNdjsonFailure(failure: NdjsonFailure): string {
  switch (failure.kind) {
    case 'line_too_long':
      return `an ndjson line exceeded ${failure.maximum} bytes`
    case 'not_json':
      return 'an ndjson line was not valid JSON'
    case 'invalid_event':
      return `an ndjson line was not a valid event: ${failure.issue.path} ${failure.issue.code}`
  }
}

/** Serializes one event as a single NDJSON line, newline included. */
export function encodeEventLine(event: CompletionEvent): string {
  return `${JSON.stringify(event)}\n`
}

/** Streams events as NDJSON bytes, ready to become a response body. */
export function toNdjsonStream(events: AsyncIterable<CompletionEvent>): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder()
  const iterator = events[Symbol.asyncIterator]()
  return new ReadableStream<Uint8Array>({
    async pull(controller) {
      const step = await iterator.next()
      if (step.done) {
        controller.close()
        return
      }
      controller.enqueue(encoder.encode(encodeEventLine(step.value)))
    },
    async cancel(reason) {
      await iterator.return?.(reason)
    },
  })
}

/**
 * Decodes NDJSON bytes back into events.
 *
 * Throws {@link NdjsonError} on a malformed or over-long line: the consumer is
 * mid-stream, so there is no result value left to return a failure in. Blank
 * lines are skipped, and a trailing line without a newline is still decoded.
 */
export async function* fromNdjsonStream(
  body: ReadableStream<Uint8Array>,
  maximumLineBytes = MAX_NDJSON_LINE_BYTES,
): AsyncGenerator<CompletionEvent, void, void> {
  const reader = body.getReader()
  const decoder = new TextDecoder('utf-8')
  let buffered = ''

  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      buffered += decoder.decode(value, { stream: true })

      let newline = buffered.indexOf('\n')
      while (newline !== -1) {
        const line = buffered.slice(0, newline)
        buffered = buffered.slice(newline + 1)
        const event = parseLine(line)
        if (event) yield event
        newline = buffered.indexOf('\n')
      }
      if (buffered.length > maximumLineBytes) {
        throw new NdjsonError({ kind: 'line_too_long', maximum: maximumLineBytes })
      }
    }
    buffered += decoder.decode()
    const event = parseLine(buffered)
    if (event) yield event
  } finally {
    reader.releaseLock()
    await body.cancel().catch(() => undefined)
  }
}

function parseLine(line: string): CompletionEvent | null {
  const trimmed = line.trim()
  if (trimmed === '') return null

  let document: unknown
  try {
    document = JSON.parse(trimmed)
  } catch {
    throw new NdjsonError({ kind: 'not_json', line: trimmed.slice(0, 200) })
  }

  const event = decodeEvent(document)
  if (!event.ok) throw new NdjsonError({ kind: 'invalid_event', issue: event.issue })
  return event.value
}
