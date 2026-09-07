/**
 * Cross-language conformance: the committed fixtures are the shared authority.
 *
 * Rust runs the same files in `crates/ai-core/tests/contracts.rs`. Anything
 * these two suites disagree about is a contract divergence, whichever language
 * is "right", which is the point of pointing both at one directory.
 */

import { describe, expect, test } from 'bun:test'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

import { decodeEvent, decodeRequestShape, encodeRequest, validateRequest, validateProviderError, validateUsage } from '../src/index.ts'

const FIXTURES = join(import.meta.dir, '../../../specs/fixtures')
const SPEC_VERSION = readFileSync(join(FIXTURES, '../VERSION'), 'utf8').trim()

function fixture(name: string): unknown {
  return JSON.parse(readFileSync(join(FIXTURES, name), 'utf8'))
}

function names(directory: string, prefix: string): string[] {
  return readdirSync(join(FIXTURES, directory))
    .filter((name) => name.startsWith(prefix) && name.endsWith('.json'))
    .map((name) => `${directory}/${name}`)
    .sort()
}

test('the fixtures are the version this package implements', () => {
  expect(SPEC_VERSION).toBe('0.2.0')
})

describe('valid request fixtures', () => {
  const canonical = names('valid', 'completion-request').filter((name) => !name.includes('legacy-omitted'))

  test('every file is covered, so a new fixture cannot be silently ignored', () => {
    expect(canonical.length).toBeGreaterThanOrEqual(3)
  })

  for (const name of canonical) {
    test(`${name} decodes, validates and round-trips`, () => {
      const document = fixture(name)
      const decoded = decodeRequestShape(document)
      expect(decoded.ok).toBe(true)
      if (!decoded.ok) return
      expect(validateRequest(decoded.value)).toBeNull()
      expect(encodeRequest(decoded.value)).toEqual(document as Record<string, unknown>)
    })
  }

  test('a spec 0.1.0 producer still decodes, to the documented defaults', () => {
    const decoded = decodeRequestShape(fixture('valid/completion-request-legacy-omitted.json'))
    expect(decoded.ok).toBe(true)
    if (!decoded.ok) return
    expect(decoded.value.priorMessages).toEqual([])
    expect(decoded.value.parameters.maxOutputTokens).toBeNull()
    expect(validateRequest(decoded.value)).toBeNull()
  })

  test('the conversation fixture keeps its order and its token ceiling', () => {
    const decoded = decodeRequestShape(fixture('valid/completion-request-conversation.json'))
    expect(decoded.ok).toBe(true)
    if (!decoded.ok) return
    expect(decoded.value.priorMessages).toEqual([
      { role: 'user', content: 'Name a colour.' },
      { role: 'assistant', content: 'Blue.' },
    ])
    expect(decoded.value.userPrompt).toBe('And in Dutch?')
    expect(decoded.value.parameters.maxOutputTokens).toBe(1800)
  })
})

describe('invalid request fixtures', () => {
  for (const name of names('invalid', 'request-')) {
    test(`${name} is rejected`, () => {
      expect(decodeRequestShape(fixture(name)).ok).toBe(false)
    })
  }
})

describe('valid event fixtures', () => {
  for (const name of names('valid', 'event-')) {
    test(`${name} decodes and round-trips`, () => {
      const document = fixture(name)
      const decoded = decodeEvent(document)
      expect(decoded.ok).toBe(true)
      if (!decoded.ok) return
      expect(JSON.parse(JSON.stringify(decoded.value))).toEqual(document as Record<string, unknown>)
      if (decoded.value.type === 'done' && decoded.value.usage) {
        expect(validateUsage(decoded.value.usage)).toBeNull()
      }
      if (decoded.value.type === 'provider_error') {
        expect(validateProviderError(decoded.value.error)).toBeNull()
      }
    })
  }
})

describe('invalid event fixtures', () => {
  for (const name of names('invalid', 'event-')) {
    test(`${name} is rejected`, () => {
      expect(decodeEvent(fixture(name)).ok).toBe(false)
    })
  }
})

describe('the generated schemas describe the same contracts', () => {
  const SCHEMAS = join(FIXTURES, '../schemas')

  test('every committed schema pins the spec version', () => {
    const files = readdirSync(SCHEMAS).filter((name) => name.endsWith('.json'))
    expect(files.length).toBeGreaterThan(0)
    for (const file of files) {
      const schema = JSON.parse(readFileSync(join(SCHEMAS, file), 'utf8')) as { $id?: string }
      expect(schema.$id).toContain(SPEC_VERSION)
    }
  })

  test('the request schema declares the fields this package decodes', () => {
    const schema = JSON.parse(readFileSync(join(SCHEMAS, 'completion-request.json'), 'utf8')) as {
      properties: Record<string, unknown>
      additionalProperties: boolean
    }
    expect(Object.keys(schema.properties).sort()).toEqual([
      'modelId',
      'parameters',
      'priorMessages',
      'providerId',
      'requestId',
      'systemPrompt',
      'userPrompt',
    ])
    expect(schema.additionalProperties).toBe(false)
  })

  test('the parameter schema declares the token ceiling', () => {
    const schema = JSON.parse(readFileSync(join(SCHEMAS, 'completion-parameters.json'), 'utf8')) as {
      properties: Record<string, unknown>
    }
    expect(Object.keys(schema.properties)).toContain('maxOutputTokens')
  })
})
