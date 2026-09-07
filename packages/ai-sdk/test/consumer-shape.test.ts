/**
 * The shape the first TypeScript consumer actually needs.
 *
 * Betalingen streams a multi-turn conversation with a system prompt and a token
 * ceiling over NDJSON, from a Hono route, with the key on the server. Phase 4
 * does not migrate it — that is Phase 5 and needs its own authorization — but
 * the delta was specified for it, so the packages are checked against its
 * actual shape rather than a shape invented here.
 *
 * Nothing framework-specific appears below: the point is that a Web `Response`
 * is all a route needs, which is what makes the same code work under Hono,
 * Next.js, a bare Worker, or Bun.serve.
 */

import { describe, expect, test } from 'bun:test'

import {
  buildRequest,
  consumeEvents,
  createRuntime,
  defaultParameters,
  fromNdjsonStream,
  toNdjsonStream,
  type CompletionEvent,
  type Message,
} from '@remcostoeten/ai-core'

import { createGroqProvider, staticCredential } from '../src/index.ts'

const MODEL = 'llama-3.3-70b-versatile'

function sseBody(chunks: readonly string[]): string {
  return [
    ...chunks.map(
      (text) => `data: ${JSON.stringify({ choices: [{ index: 0, delta: { content: text }, finish_reason: null }] })}\n\n`,
    ),
    `data: ${JSON.stringify({ choices: [{ index: 0, delta: {}, finish_reason: 'stop' }], x_groq: { usage: { prompt_tokens: 42, completion_tokens: 6 } } })}\n\n`,
    'data: [DONE]\n\n',
  ].join('')
}

/** What a route hands back. No framework, just a Web `Response`. */
async function chatRoute(messages: readonly Message[], captured: unknown[]): Promise<Response> {
  const provider = await createGroqProvider({
    credentials: staticCredential('sk-server-side-only'),
    models: { permits: (_, modelId) => modelId === MODEL },
    baseURL: 'https://api.groq.test/openai/v1',
    fetch: (async (_input: RequestInfo | URL, init?: RequestInit) => {
      captured.push(typeof init?.body === 'string' ? JSON.parse(init.body) : null)
      return new Response(sseBody(['Dat ', 'klopt.']), {
        status: 200,
        headers: { 'content-type': 'text/event-stream' },
      })
    }) as typeof globalThis.fetch,
  })

  const built = buildRequest({
    requestId: 'request-1',
    providerId: 'groq',
    modelId: MODEL,
    systemPrompt: 'Je bent een data-assistent.',
    messages,
    parameters: { ...defaultParameters(), maxOutputTokens: 1800, temperatureMillis: 200 },
  })
  if (!built.ok) return new Response(JSON.stringify({ error: built.error.reason }), { status: 400 })

  const started = createRuntime({ providers: [provider] }).stream(built.request)
  if (!started.ok) return new Response(JSON.stringify({ error: started.error.reason }), { status: 400 })

  return new Response(toNdjsonStream(started.events), {
    status: 200,
    headers: {
      'content-type': 'application/x-ndjson; charset=utf-8',
      'cache-control': 'no-store',
      'x-content-type-options': 'nosniff',
    },
  })
}

describe('a conversation over an ndjson route', () => {
  const conversation: Message[] = [
    { role: 'user', content: 'Wat waren mijn uitgaven in maart?' },
    { role: 'assistant', content: 'In maart gaf je 812 euro uit.' },
    { role: 'user', content: 'En in april?' },
  ]

  test('the whole conversation reaches the provider, and the answer comes back', async () => {
    const captured: unknown[] = []
    const response = await chatRoute(conversation, captured)

    expect(response.status).toBe(200)
    expect(response.headers.get('content-type')).toContain('application/x-ndjson')
    expect(response.body).not.toBeNull()
    if (!response.body) return

    const outcome = await consumeEvents(fromNdjsonStream(response.body))

    expect(outcome.status).toBe('done')
    expect(outcome.text).toBe('Dat klopt.')
    if (outcome.status !== 'done') return
    expect(outcome.usage).toEqual({ inputTokens: 42, outputTokens: 6 })

    const body = captured[0] as { messages: { role: string; content: string }[] }
    expect(body.messages).toEqual([
      { role: 'system', content: 'Je bent een data-assistent.' },
      { role: 'user', content: 'Wat waren mijn uitgaven in maart?' },
      { role: 'assistant', content: 'In maart gaf je 812 euro uit.' },
      { role: 'user', content: 'En in april?' },
    ])
  })

  test('nothing reaches the provider until the response body is read', async () => {
    const captured: unknown[] = []
    const response = await chatRoute(conversation, captured)

    expect(captured).toHaveLength(0)

    await response.text()
    expect(captured).toHaveLength(1)
    // Four turns, not one flattened prompt: the delta exists so that a
    // conversation stays a conversation all the way to the provider.
    expect((captured[0] as { messages: unknown[] }).messages).toHaveLength(4)
  })

  test('a conversation ending in an assistant turn is refused before any request', async () => {
    const captured: unknown[] = []
    const response = await chatRoute([{ role: 'assistant', content: 'unprompted' }], captured)

    expect(response.status).toBe(400)
    expect(await response.json()).toEqual({ error: 'last_turn_is_not_from_the_user' })
  })

  test('the ndjson body is one JSON document per line', async () => {
    const response = await chatRoute(conversation, [])
    const text = await response.text()
    const lines = text.split('\n').filter((line) => line !== '')

    expect(lines.length).toBeGreaterThan(1)
    const parsed = lines.map((line) => JSON.parse(line) as CompletionEvent)
    expect(parsed.at(-1)?.type).toBe('done')
    expect(parsed.every((event) => event.requestId === 'request-1')).toBe(true)
  })

  test('a browser consumer can read the stream with core alone', async () => {
    const response = await chatRoute(conversation, [])
    if (!response.body) throw new Error('expected a body')

    const seen: string[] = []
    const outcome = await consumeEvents(fromNdjsonStream(response.body), {
      requestId: 'request-1',
      onText: (text) => seen.push(text),
    })

    expect(seen).toEqual(['Dat ', 'klopt.'])
    expect(outcome.status).toBe('done')
  })
})
