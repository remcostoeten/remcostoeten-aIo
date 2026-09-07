/**
 * The adapter against a fixture `fetch`.
 *
 * Nothing here opens a socket or needs a key: every provider response is a
 * canned SSE body, which is what makes these runnable in CI and on a laptop
 * with no configuration. Live-provider checks are a separate, opt-in thing.
 */

import { describe, expect, test } from 'bun:test'

import { consumeEvents, createRuntime, defaultParameters, type CompletionRequest } from '@remcostoeten/ai-core'

import { createGroqProvider, permitAll, staticCredential, type CredentialSource, type ModelAuthority } from '../src/index.ts'
import { createAdapter } from '../src/adapter.ts'

const MODEL = 'llama-3.3-70b-versatile'

function request(overrides: Partial<CompletionRequest> = {}): CompletionRequest {
  return {
    requestId: 'request-1',
    providerId: 'groq',
    modelId: MODEL,
    systemPrompt: 'Be brief.',
    userPrompt: 'Name a colour.',
    priorMessages: [],
    parameters: defaultParameters(),
    ...overrides,
  }
}

type Captured = { body: unknown; url: string; headers: Record<string, string> }

/**
 * An SSE body in the shape Groq streams.
 *
 * Groq reports usage under `x_groq.usage` rather than the OpenAI-compatible
 * top-level `usage` field, and its own parser reads only that. A fixture using
 * the generic shape would silently report no usage and the test would be
 * asserting nothing.
 */
function sseBody(chunks: readonly string[], usage?: { prompt_tokens: number; completion_tokens: number }): string {
  const lines = chunks.map(
    (text) => `data: ${JSON.stringify({ choices: [{ index: 0, delta: { content: text }, finish_reason: null }] })}\n\n`,
  )
  const final: Record<string, unknown> = { choices: [{ index: 0, delta: {}, finish_reason: 'stop' }] }
  if (usage) final['x_groq'] = { usage }
  lines.push(`data: ${JSON.stringify(final)}\n\n`)
  lines.push('data: [DONE]\n\n')
  return lines.join('')
}

function fixtureFetch(reply: () => Response, captured: Captured[] = []): { fetch: typeof globalThis.fetch; captured: Captured[] } {
  const fetchImpl = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const headers: Record<string, string> = {}
    new Headers(init?.headers).forEach((value, key) => {
      headers[key] = key === 'authorization' ? 'redacted' : value
    })
    captured.push({
      url: String(input),
      body: typeof init?.body === 'string' ? JSON.parse(init.body) : null,
      headers,
    })
    return reply()
  }) as typeof globalThis.fetch
  return { fetch: fetchImpl, captured }
}

function sseResponse(body: string): Response {
  return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } })
}

async function runGroq(
  options: { credentials?: CredentialSource; models?: ModelAuthority; reply: () => Response },
  overrides: Partial<CompletionRequest> = {},
) {
  const { fetch, captured } = fixtureFetch(options.reply)
  const provider = await createGroqProvider({
    credentials: options.credentials ?? staticCredential('sk-test-key'),
    models: options.models ?? { permits: (_, modelId) => modelId === MODEL },
    baseURL: 'https://api.groq.test/openai/v1',
    fetch,
  })
  const runtime = createRuntime({ providers: [provider] })
  const started = runtime.stream(request(overrides))
  if (!started.ok) throw new Error(`expected admission, got ${started.error.reason}`)
  return { outcome: await consumeEvents(started.events), captured }
}

describe('a normal run', () => {
  test('streams text and reports usage', async () => {
    const { outcome } = await runGroq({
      reply: () => sseResponse(sseBody(['Blue', '.'], { prompt_tokens: 9, completion_tokens: 2 })),
    })

    expect(outcome.status).toBe('done')
    expect(outcome.text).toBe('Blue.')
    if (outcome.status !== 'done') return
    expect(outcome.usage).toEqual({ inputTokens: 9, outputTokens: 2 })
  })

  test('a provider reporting no usage is not recorded as having used zero', async () => {
    const { outcome } = await runGroq({ reply: () => sseResponse(sseBody(['Blue.'])) })

    expect(outcome.status).toBe('done')
    if (outcome.status !== 'done') return
    expect(outcome.usage).toBeNull()
  })
})

describe('the conversation reaches the provider in contract order', () => {
  test('system, then prior turns, then the user prompt', async () => {
    const { captured } = await runGroq(
      { reply: () => sseResponse(sseBody(['ok'])) },
      {
        priorMessages: [
          { role: 'user', content: 'Name a colour.' },
          { role: 'assistant', content: 'Blue.' },
        ],
        userPrompt: 'And in Dutch?',
      },
    )

    const body = captured[0]?.body as { messages: { role: string; content: string }[] }
    expect(body.messages).toEqual([
      { role: 'system', content: 'Be brief.' },
      { role: 'user', content: 'Name a colour.' },
      { role: 'assistant', content: 'Blue.' },
      { role: 'user', content: 'And in Dutch?' },
    ])
  })

  test('an empty system prompt is omitted rather than sent empty', async () => {
    const { captured } = await runGroq({ reply: () => sseResponse(sseBody(['ok'])) }, { systemPrompt: '' })

    const body = captured[0]?.body as { messages: { role: string }[] }
    expect(body.messages.map((message) => message.role)).toEqual(['user'])
  })

  test('the token ceiling and sampling parameters are translated, not invented', async () => {
    const { captured } = await runGroq(
      { reply: () => sseResponse(sseBody(['ok'])) },
      { parameters: { ...defaultParameters(), maxOutputTokens: 1800, temperatureMillis: 200 } },
    )

    const body = captured[0]?.body as Record<string, unknown>
    expect(body['max_tokens'] ?? body['max_completion_tokens']).toBe(1800)
    expect(body['temperature']).toBe(0.2)
    expect(body['top_p']).toBeUndefined()
  })
})

describe('nothing reaches the network before the application has approved it', () => {
  test('an unpermitted model is refused without a request', async () => {
    const { outcome, captured } = await runGroq({
      models: { permits: () => false },
      reply: () => sseResponse(sseBody(['should not happen'])),
    })

    expect(captured).toHaveLength(0)
    expect(outcome.status).toBe('provider_error')
    if (outcome.status !== 'provider_error') return
    expect(outcome.error.category).toBe('rejected_request')
  })

  test('a missing credential is refused without a request', async () => {
    const { outcome, captured } = await runGroq({
      credentials: staticCredential(''),
      reply: () => sseResponse(sseBody(['should not happen'])),
    })

    expect(captured).toHaveLength(0)
    expect(outcome.status).toBe('provider_error')
    if (outcome.status !== 'provider_error') return
    expect(outcome.error.category).toBe('missing_credential')
    expect(outcome.error.recoveryAction).toBe('configure_credential')
  })

  test('a withheld credential is distinguishable from a missing one', async () => {
    const { outcome } = await runGroq({
      credentials: { resolve: () => ({ ok: false, refusal: 'withheld', message: 'the vault is locked' }) },
      reply: () => sseResponse(sseBody(['should not happen'])),
    })

    expect(outcome.status).toBe('provider_error')
    if (outcome.status !== 'provider_error') return
    expect(outcome.error.category).toBe('invalid_credential')
  })
})

describe('provider failures become typed categories, not status codes', () => {
  const cases = [
    { status: 401, category: 'invalid_credential', recovery: 'configure_credential' },
    { status: 404, category: 'unavailable_provider', recovery: 'choose_different_model' },
    { status: 429, category: 'rate_limited', recovery: 'retry' },
    { status: 400, category: 'rejected_request', recovery: 'reduce_request' },
    { status: 402, category: 'quota_exhausted', recovery: 'contact_provider' },
    { status: 503, category: 'transport_failure', recovery: 'check_provider_status' },
  ] as const

  for (const { status, category, recovery } of cases) {
    test(`${status} becomes ${category}`, async () => {
      const { outcome } = await runGroq({
        reply: () => new Response(JSON.stringify({ error: { message: 'Be brief. Name a colour.' } }), { status }),
      })

      expect(outcome.status).toBe('provider_error')
      if (outcome.status !== 'provider_error') return
      expect(outcome.error.category).toBe(category)
      expect(outcome.error.recoveryAction).toBe(recovery)
    })
  }

  test("the provider's response body never reaches the caller", async () => {
    const { outcome } = await runGroq({
      reply: () =>
        new Response(JSON.stringify({ error: { message: 'rejected prompt: Be brief. Name a colour. key sk-live-abc' } }), {
          status: 400,
        }),
    })

    expect(outcome.status).toBe('provider_error')
    if (outcome.status !== 'provider_error') return
    expect(outcome.error.message).not.toContain('sk-live-abc')
    expect(outcome.error.message).not.toContain('Name a colour')
  })

  test('an unreachable host becomes a transport failure', async () => {
    const { outcome } = await runGroq({
      reply: () => {
        throw new TypeError('fetch failed')
      },
    })

    expect(outcome.status).toBe('provider_error')
    if (outcome.status !== 'provider_error') return
    expect(outcome.error.category).toBe('transport_failure')
  })
})

describe('retries stay disabled', () => {
  test('a 429 is attempted exactly once', async () => {
    const { captured } = await runGroq({ reply: () => new Response('{}', { status: 429 }) })

    expect(captured).toHaveLength(1)
  })
})

describe('cancellation', () => {
  test('aborting mid-stream commits a cancelled terminal', async () => {
    const { fetch } = fixtureFetch(() => sseResponse(sseBody(['one', 'two', 'three'])))
    const provider = await createGroqProvider({
      credentials: staticCredential('sk-test-key'),
      models: permitAll(),
      baseURL: 'https://api.groq.test/openai/v1',
      fetch,
    })
    const runtime = createRuntime({ providers: [provider] })
    const started = runtime.stream(request())
    if (!started.ok) throw new Error('expected admission')

    const outcome = await consumeEvents(started.events, {
      onText: () => {
        runtime.cancel('request-1')
      },
    })

    expect(outcome.status).toBe('cancelled')
  })
})

describe('the adapter refuses a foreign request', () => {
  test('a request addressed to another provider is rejected', async () => {
    const adapter = createAdapter({
      providerId: 'groq',
      credentials: staticCredential('sk-test-key'),
      models: permitAll(),
      createModel: () => {
        throw new Error('the model should never be built')
      },
    })
    const runtime = createRuntime({ providers: [adapter] })
    const started = runtime.stream(request({ providerId: 'groq', modelId: MODEL }))
    if (!started.ok) throw new Error('expected admission')

    // The adapter accepts its own id; a foreign one never routes here at all,
    // so the runtime answers with unavailable_provider instead.
    const foreign = createRuntime({ providers: [adapter] }).stream(request({ providerId: 'deepseek' }))
    if (!foreign.ok) throw new Error('expected admission')
    const outcome = await consumeEvents(foreign.events)

    expect(outcome.status).toBe('provider_error')
    if (outcome.status !== 'provider_error') return
    expect(outcome.error.category).toBe('unavailable_provider')

    await consumeEvents(started.events)
  })
})
