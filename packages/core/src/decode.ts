/**
 * The trust boundary: `unknown` in, a typed contract or a typed rejection out.
 *
 * Hand-written rather than Zod, for two reasons that are not style. The bounds
 * are UTF-8 byte budgets and unpaired-surrogate rules, which a schema library
 * measuring `String.length` cannot express; and `core` stays dependency-free so
 * a browser consumer bundling it pulls in nothing else.
 *
 * Structural decoding is separate from semantic validation, exactly as it is in
 * Rust: a document that decodes has the right shape, not necessarily an
 * acceptable value. Callers run {@link validateRequest} too — or use
 * {@link decodeRequest}, which does both.
 */

import {
  DEFAULT_PARAMETERS,
  MAX_PRIOR_MESSAGES,
  PROVIDER_ERROR_CATEGORIES,
  RECOVERY_ACTIONS,
  type CompletionDelta,
  type CompletionEvent,
  type CompletionParameters,
  type CompletionRequest,
  type Message,
  type MessageRole,
  type ProviderError,
  type ProviderErrorCategory,
  type RecoveryAction,
  type Usage,
} from './contracts.js'

/**
 * Why a document was rejected.
 *
 * `path` is a dotted location inside the document. `code` is a closed
 * vocabulary, never a free-form string a caller could branch on by spelling.
 */
export type DecodeIssue = {
  readonly path: string
  readonly code:
    | 'not_an_object'
    | 'missing_field'
    | 'unknown_field'
    | 'wrong_type'
    | 'unknown_variant'
    | 'not_an_integer'
    | 'out_of_range'
    | 'lone_surrogate'
  readonly detail: string
}

export type DecodeResult<T> = { readonly ok: true; readonly value: T } | { readonly ok: false; readonly issue: DecodeIssue }

function fail<T>(path: string, code: DecodeIssue['code'], detail: string): DecodeResult<T> {
  return { ok: false, issue: { path, code, detail } }
}

function succeed<T>(value: T): DecodeResult<T> {
  return { ok: true, value }
}

type Fields = Record<string, unknown>

function asObject(value: unknown, path: string): DecodeResult<Fields> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    return fail(path, 'not_an_object', 'expected a JSON object')
  }
  return succeed(value as Fields)
}

/** Rejects any field the contract does not declare, mirroring serde's `deny_unknown_fields`. */
function rejectUnknownFields(fields: Fields, known: readonly string[], path: string): DecodeIssue | null {
  for (const key of Object.keys(fields)) {
    if (!known.includes(key)) {
      return { path: path === '' ? key : `${path}.${key}`, code: 'unknown_field', detail: `unknown field ${key}` }
    }
  }
  return null
}

function requiredString(fields: Fields, key: string, path: string): DecodeResult<string> {
  const at = path === '' ? key : `${path}.${key}`
  if (!(key in fields)) return fail(at, 'missing_field', `missing field ${key}`)
  const value = fields[key]
  if (typeof value !== 'string') return fail(at, 'wrong_type', 'expected a string')
  if (hasLoneSurrogate(value)) return fail(at, 'lone_surrogate', 'string holds an unpaired surrogate')
  return succeed(value)
}

function requiredInteger(fields: Fields, key: string, path: string, minimum: number, maximum: number): DecodeResult<number> {
  const at = path === '' ? key : `${path}.${key}`
  if (!(key in fields)) return fail(at, 'missing_field', `missing field ${key}`)
  return integerAt(fields[key], at, minimum, maximum)
}

/**
 * Reads a `T | null` field that may also be omitted.
 *
 * Rust emits an explicit `null` and its serde accepts omission; both decode to
 * `null` here rather than to `undefined`, so the value re-encodes to the
 * canonical form.
 */
function nullableInteger(fields: Fields, key: string, path: string, minimum: number, maximum: number): DecodeResult<number | null> {
  const at = path === '' ? key : `${path}.${key}`
  if (!(key in fields) || fields[key] === null || fields[key] === undefined) return succeed(null)
  return integerAt(fields[key], at, minimum, maximum)
}

function integerAt(value: unknown, at: string, minimum: number, maximum: number): DecodeResult<number> {
  if (typeof value !== 'number') return fail(at, 'wrong_type', 'expected a number')
  if (!Number.isInteger(value)) return fail(at, 'not_an_integer', 'expected an integer')
  if (value < minimum || value > maximum) {
    return fail(at, 'out_of_range', `expected an integer between ${minimum} and ${maximum}`)
  }
  return succeed(value)
}

function hasLoneSurrogate(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index)
    if (code >= 0xd800 && code <= 0xdbff) {
      const low = index + 1 < value.length ? value.charCodeAt(index + 1) : 0
      if (low < 0xdc00 || low > 0xdfff) return true
      index += 1
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      return true
    }
  }
  return false
}

const U8_MAX = 255
const U16_MAX = 65_535
const U32_MAX = 4_294_967_295
/** JavaScript's exact-integer ceiling. Rust `u64` exceeds it; token bounds do not. */
const SAFE_INTEGER_MAX = Number.MAX_SAFE_INTEGER

const MESSAGE_FIELDS = ['role', 'content'] as const
const MESSAGE_ROLES: readonly MessageRole[] = ['user', 'assistant']

export function decodeMessage(document: unknown, path = ''): DecodeResult<Message> {
  const object = asObject(document, path)
  if (!object.ok) return object
  const unknownField = rejectUnknownFields(object.value, MESSAGE_FIELDS, path)
  if (unknownField) return { ok: false, issue: unknownField }

  const role = requiredString(object.value, 'role', path)
  if (!role.ok) return role
  if (!MESSAGE_ROLES.includes(role.value as MessageRole)) {
    return fail(path === '' ? 'role' : `${path}.role`, 'unknown_variant', `unknown role ${role.value}`)
  }
  const content = requiredString(object.value, 'content', path)
  if (!content.ok) return content

  return succeed({ role: role.value as MessageRole, content: content.value })
}

const PARAMETER_FIELDS = [
  'maxOutputBytes',
  'timeoutMs',
  'retryCount',
  'temperatureMillis',
  'topPMillis',
  'maxOutputTokens',
] as const

export function decodeParameters(document: unknown, path = ''): DecodeResult<CompletionParameters> {
  const object = asObject(document, path)
  if (!object.ok) return object
  const unknownField = rejectUnknownFields(object.value, PARAMETER_FIELDS, path)
  if (unknownField) return { ok: false, issue: unknownField }

  const maxOutputBytes = requiredInteger(object.value, 'maxOutputBytes', path, 0, U32_MAX)
  if (!maxOutputBytes.ok) return maxOutputBytes
  const timeoutMs = requiredInteger(object.value, 'timeoutMs', path, 0, U32_MAX)
  if (!timeoutMs.ok) return timeoutMs
  const retryCount = requiredInteger(object.value, 'retryCount', path, 0, U8_MAX)
  if (!retryCount.ok) return retryCount
  const temperatureMillis = nullableInteger(object.value, 'temperatureMillis', path, 0, U16_MAX)
  if (!temperatureMillis.ok) return temperatureMillis
  const topPMillis = nullableInteger(object.value, 'topPMillis', path, 0, U16_MAX)
  if (!topPMillis.ok) return topPMillis
  const maxOutputTokens = nullableInteger(object.value, 'maxOutputTokens', path, 0, U32_MAX)
  if (!maxOutputTokens.ok) return maxOutputTokens

  return succeed({
    maxOutputBytes: maxOutputBytes.value,
    timeoutMs: timeoutMs.value,
    retryCount: retryCount.value,
    temperatureMillis: temperatureMillis.value,
    topPMillis: topPMillis.value,
    maxOutputTokens: maxOutputTokens.value,
  })
}

const REQUEST_FIELDS = [
  'requestId',
  'providerId',
  'modelId',
  'systemPrompt',
  'userPrompt',
  'priorMessages',
  'parameters',
] as const

/**
 * Decodes a request's structure.
 *
 * `priorMessages` and `parameters.maxOutputTokens` may be omitted: a spec 0.1.0
 * producer's document decodes to the empty history and no token ceiling. Every
 * other field is required.
 */
export function decodeRequestShape(document: unknown, path = ''): DecodeResult<CompletionRequest> {
  const object = asObject(document, path)
  if (!object.ok) return object
  const unknownField = rejectUnknownFields(object.value, REQUEST_FIELDS, path)
  if (unknownField) return { ok: false, issue: unknownField }

  const requestId = requiredString(object.value, 'requestId', path)
  if (!requestId.ok) return requestId
  const providerId = requiredString(object.value, 'providerId', path)
  if (!providerId.ok) return providerId
  const modelId = requiredString(object.value, 'modelId', path)
  if (!modelId.ok) return modelId
  const systemPrompt = requiredString(object.value, 'systemPrompt', path)
  if (!systemPrompt.ok) return systemPrompt
  const userPrompt = requiredString(object.value, 'userPrompt', path)
  if (!userPrompt.ok) return userPrompt

  const messagesPath = path === '' ? 'priorMessages' : `${path}.priorMessages`
  const rawMessages = object.value['priorMessages']
  let priorMessages: Message[] = []
  if (rawMessages !== undefined) {
    if (!Array.isArray(rawMessages)) return fail(messagesPath, 'wrong_type', 'expected an array')
    if (rawMessages.length > MAX_PRIOR_MESSAGES) {
      return fail(messagesPath, 'out_of_range', `at most ${MAX_PRIOR_MESSAGES} prior messages`)
    }
    priorMessages = []
    for (const [index, entry] of rawMessages.entries()) {
      const message = decodeMessage(entry, `${messagesPath}[${index}]`)
      if (!message.ok) return message
      priorMessages.push(message.value)
    }
  }

  const parametersPath = path === '' ? 'parameters' : `${path}.parameters`
  if (!('parameters' in object.value)) return fail(parametersPath, 'missing_field', 'missing field parameters')
  const parameters = decodeParameters(object.value['parameters'], parametersPath)
  if (!parameters.ok) return parameters

  return succeed({
    requestId: requestId.value,
    providerId: providerId.value,
    modelId: modelId.value,
    systemPrompt: systemPrompt.value,
    userPrompt: userPrompt.value,
    priorMessages,
    parameters: parameters.value,
  })
}

const USAGE_FIELDS = ['inputTokens', 'outputTokens'] as const

export function decodeUsage(document: unknown, path = ''): DecodeResult<Usage> {
  const object = asObject(document, path)
  if (!object.ok) return object
  const unknownField = rejectUnknownFields(object.value, USAGE_FIELDS, path)
  if (unknownField) return { ok: false, issue: unknownField }

  const inputTokens = requiredInteger(object.value, 'inputTokens', path, 0, SAFE_INTEGER_MAX)
  if (!inputTokens.ok) return inputTokens
  const outputTokens = requiredInteger(object.value, 'outputTokens', path, 0, SAFE_INTEGER_MAX)
  if (!outputTokens.ok) return outputTokens

  return succeed({ inputTokens: inputTokens.value, outputTokens: outputTokens.value })
}

const PROVIDER_ERROR_FIELDS = ['providerId', 'category', 'message', 'recoveryAction'] as const

export function decodeProviderError(document: unknown, path = ''): DecodeResult<ProviderError> {
  const object = asObject(document, path)
  if (!object.ok) return object
  const unknownField = rejectUnknownFields(object.value, PROVIDER_ERROR_FIELDS, path)
  if (unknownField) return { ok: false, issue: unknownField }

  const providerId = requiredString(object.value, 'providerId', path)
  if (!providerId.ok) return providerId
  const category = requiredString(object.value, 'category', path)
  if (!category.ok) return category
  if (!PROVIDER_ERROR_CATEGORIES.includes(category.value as ProviderErrorCategory)) {
    return fail(`${path === '' ? '' : `${path}.`}category`, 'unknown_variant', `unknown category ${category.value}`)
  }
  const message = requiredString(object.value, 'message', path)
  if (!message.ok) return message
  const recoveryAction = requiredString(object.value, 'recoveryAction', path)
  if (!recoveryAction.ok) return recoveryAction
  if (!RECOVERY_ACTIONS.includes(recoveryAction.value as RecoveryAction)) {
    return fail(
      `${path === '' ? '' : `${path}.`}recoveryAction`,
      'unknown_variant',
      `unknown recovery action ${recoveryAction.value}`,
    )
  }

  return succeed({
    providerId: providerId.value,
    category: category.value as ProviderErrorCategory,
    message: message.value,
    recoveryAction: recoveryAction.value as RecoveryAction,
  })
}

const DELTA_FIELDS = ['requestId', 'sequence', 'text'] as const

export function decodeDelta(document: unknown, path = ''): DecodeResult<CompletionDelta> {
  const object = asObject(document, path)
  if (!object.ok) return object
  const unknownField = rejectUnknownFields(object.value, DELTA_FIELDS, path)
  if (unknownField) return { ok: false, issue: unknownField }

  const requestId = requiredString(object.value, 'requestId', path)
  if (!requestId.ok) return requestId
  const sequence = requiredInteger(object.value, 'sequence', path, 0, U32_MAX)
  if (!sequence.ok) return sequence
  const text = requiredString(object.value, 'text', path)
  if (!text.ok) return text

  return succeed({ requestId: requestId.value, sequence: sequence.value, text: text.value })
}

const EVENT_FIELDS: Record<CompletionEvent['type'], readonly string[]> = {
  delta: ['type', 'requestId', 'sequence', 'text'],
  done: ['type', 'requestId', 'usage'],
  cancelled: ['type', 'requestId'],
  timeout: ['type', 'requestId'],
  provider_error: ['type', 'requestId', 'error'],
}

/**
 * Decodes one event.
 *
 * Unknown tags, unknown fields and unknown enum values are rejected. There is
 * no fallback variant on purpose: silently mapping an unrecognized terminal
 * onto `internal_failure` would erase a real state and make a closed enum look
 * forward-compatible when it is not.
 */
export function decodeEvent(document: unknown, path = ''): DecodeResult<CompletionEvent> {
  const object = asObject(document, path)
  if (!object.ok) return object

  const tag = requiredString(object.value, 'type', path)
  if (!tag.ok) return tag
  const known = EVENT_FIELDS[tag.value as CompletionEvent['type']]
  if (known === undefined) {
    return fail(path === '' ? 'type' : `${path}.type`, 'unknown_variant', `unknown event type ${tag.value}`)
  }
  const unknownField = rejectUnknownFields(object.value, known, path)
  if (unknownField) return { ok: false, issue: unknownField }

  const requestId = requiredString(object.value, 'requestId', path)
  if (!requestId.ok) return requestId

  switch (tag.value) {
    case 'delta': {
      const sequence = requiredInteger(object.value, 'sequence', path, 0, U32_MAX)
      if (!sequence.ok) return sequence
      const text = requiredString(object.value, 'text', path)
      if (!text.ok) return text
      return succeed({ type: 'delta', requestId: requestId.value, sequence: sequence.value, text: text.value })
    }
    case 'done': {
      const raw = object.value['usage']
      if (raw === null || raw === undefined) {
        return succeed({ type: 'done', requestId: requestId.value, usage: null })
      }
      const usage = decodeUsage(raw, path === '' ? 'usage' : `${path}.usage`)
      if (!usage.ok) return usage
      return succeed({ type: 'done', requestId: requestId.value, usage: usage.value })
    }
    case 'cancelled':
      return succeed({ type: 'cancelled', requestId: requestId.value })
    case 'timeout':
      return succeed({ type: 'timeout', requestId: requestId.value })
    case 'provider_error': {
      if (!('error' in object.value)) {
        return fail(path === '' ? 'error' : `${path}.error`, 'missing_field', 'missing field error')
      }
      const error = decodeProviderError(object.value['error'], path === '' ? 'error' : `${path}.error`)
      if (!error.ok) return error
      return succeed({ type: 'provider_error', requestId: requestId.value, error: error.value })
    }
    default:
      return fail(path === '' ? 'type' : `${path}.type`, 'unknown_variant', `unknown event type ${tag.value}`)
  }
}

/**
 * Builds the canonical wire form of a request.
 *
 * Nullable fields are emitted as explicit `null` and `priorMessages` is always
 * present, so a round trip through this function is byte-stable and matches
 * what Rust serializes.
 */
export function encodeRequest(request: CompletionRequest): Record<string, unknown> {
  return {
    requestId: request.requestId,
    providerId: request.providerId,
    modelId: request.modelId,
    systemPrompt: request.systemPrompt,
    userPrompt: request.userPrompt,
    priorMessages: request.priorMessages.map((message) => ({ role: message.role, content: message.content })),
    parameters: {
      maxOutputBytes: request.parameters.maxOutputBytes,
      timeoutMs: request.parameters.timeoutMs,
      retryCount: request.parameters.retryCount,
      temperatureMillis: request.parameters.temperatureMillis,
      topPMillis: request.parameters.topPMillis,
      maxOutputTokens: request.parameters.maxOutputTokens,
    },
  }
}

/** The parameters a request omitting every optional field decodes to. */
export function defaultParameters(): CompletionParameters {
  return { ...DEFAULT_PARAMETERS }
}
