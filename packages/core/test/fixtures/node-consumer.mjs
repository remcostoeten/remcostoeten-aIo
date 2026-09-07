// Run under real Node, from the emitted JavaScript, by
// `portability.test.ts`. Bundling for a target proves it compiles; this proves
// it runs, including the Web Streams path and the async-iterator lifecycle.
import {
  buildRequest, consumeEvents, createFakeProvider, createRuntime,
  decodeRequestShape, encodeRequest, fromNdjsonStream, successScript, toNdjsonStream,
} from '../../dist/index.js'

const runtime = createRuntime({
  providers: [createFakeProvider(successScript(['hel', 'lo'], { inputTokens: 2, outputTokens: 2 }))],
})

const built = buildRequest({
  requestId: 'request-1',
  providerId: 'fake',
  modelId: 'model',
  systemPrompt: 'be brief',
  messages: [
    { role: 'user', content: 'a' },
    { role: 'assistant', content: 'b' },
    { role: 'user', content: 'c' },
  ],
  parameters: { maxOutputTokens: 1800 },
})
if (!built.ok) throw new Error(`build failed: ${built.error.reason}`)
if (built.request.priorMessages.length !== 2) throw new Error('history was lost')
if (built.request.userPrompt !== 'c') throw new Error('final turn was lost')

const started = runtime.stream(built.request)
if (!started.ok) throw new Error(`start failed: ${started.error.reason}`)

const outcome = await consumeEvents(fromNdjsonStream(toNdjsonStream(started.events)))
if (outcome.status !== 'done') throw new Error(`unexpected outcome ${outcome.status}`)
if (outcome.text !== 'hello') throw new Error(`unexpected text ${outcome.text}`)
if (outcome.usage?.inputTokens !== 2) throw new Error('usage was lost')

const round = decodeRequestShape(encodeRequest(built.request))
if (!round.ok) throw new Error(`round trip failed: ${round.issue.code}`)

process.stdout.write('ok')
