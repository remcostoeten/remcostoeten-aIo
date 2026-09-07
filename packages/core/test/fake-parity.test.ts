/**
 * The fake provider's shared scripts, run through this runtime.
 *
 * `crates/ai-core/tests/fake_parity.rs` runs the same files through the Rust
 * one. Real providers chunk text however their transport flushes, so delta
 * segmentation is not comparable in general — but a fixed script is, and it is
 * the only place the two implementations can be held to the same output.
 *
 * Per ADR 0002 the comparison is segmentation, identity, sequence, terminal
 * kind, error category and usage. Wall-clock timing is not compared, and a
 * fake's own diagnostic copy is display text rather than contract.
 */

import { describe, expect, test } from 'bun:test'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

import {
  createFakeProvider,
  createRuntime,
  defaultParameters,
  type CompletionEvent,
  type FakeOutcome,
  type ProviderError,
  type Usage,
} from '../src/index.ts'

const FIXTURES = join(import.meta.dir, '../../../specs/fixtures/fake')

type Fixture = {
  description: string
  script: { segments: string[]; outcome: FakeOutcome }
  deltas: string[]
  terminal:
    | { type: 'done'; usage: Usage | null }
    | { type: 'timeout' }
    | { type: 'cancelled' }
    | { type: 'provider_error'; error: Partial<ProviderError> & { category: ProviderError['category'] } }
}

const files = readdirSync(FIXTURES).filter((name) => name.endsWith('.json')).sort()

test('every shared script is covered', () => {
  expect(files.length).toBe(8)
})

describe('shared fake scripts', () => {
  for (const file of files) {
    const fixture = JSON.parse(readFileSync(join(FIXTURES, file), 'utf8')) as Fixture

    test(`${file}: ${fixture.description}`, async () => {
      const runtime = createRuntime({ providers: [createFakeProvider(fixture.script)] })
      const started = runtime.stream({
        requestId: 'request-1',
        providerId: 'fake',
        modelId: 'model',
        systemPrompt: '',
        userPrompt: 'go',
        priorMessages: [],
        parameters: defaultParameters(),
      })
      if (!started.ok) throw new Error(`expected admission, got ${started.error.reason}`)

      const events: CompletionEvent[] = []
      for await (const event of started.events) events.push(event)

      const deltas = events.filter((event) => event.type === 'delta')
      expect(deltas.map((delta) => delta.text)).toEqual(fixture.deltas)
      expect(deltas.map((delta) => delta.sequence)).toEqual(fixture.deltas.map((_, index) => index))
      expect(events.every((event) => event.requestId === 'request-1')).toBe(true)

      const terminal = events.at(-1)
      expect(terminal).toBeDefined()
      if (!terminal) return
      expect(events.filter((event) => event.type !== 'delta')).toHaveLength(1)
      expect(terminal.type).toBe(fixture.terminal.type)

      if (terminal.type === 'done' && fixture.terminal.type === 'done') {
        expect(terminal.usage).toEqual(fixture.terminal.usage)
      }
      if (terminal.type === 'provider_error' && fixture.terminal.type === 'provider_error') {
        expect(terminal.error.category).toBe(fixture.terminal.error.category)
        // Only compared when the script fixed them.
        if (fixture.terminal.error.message !== undefined) {
          expect(terminal.error).toEqual(fixture.terminal.error as ProviderError)
        }
      }
    })
  }
})
