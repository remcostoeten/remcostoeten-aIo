/**
 * Provider-neutral AI completion contracts and execution, in TypeScript.
 *
 * Same seam as `crates/ai-core`: a validated request goes in, ordered deltas
 * and exactly one terminal come out. No HTTP client, no credentials, no
 * prompts, no product, no framework, and no dependencies — a browser consumer
 * that imports this pulls in nothing else.
 *
 * ```ts
 * import { createFakeProvider, createRuntime, consumeEvents, buildRequest, successScript } from '@ai-sdk-local/core'
 *
 * const runtime = createRuntime({ providers: [createFakeProvider(successScript(['hel', 'lo']))] })
 * const built = buildRequest({
 *   requestId: 'request-1',
 *   providerId: 'fake',
 *   modelId: 'model',
 *   messages: [{ role: 'user', content: 'hello' }],
 * })
 * if (!built.ok) throw new Error(built.error.reason)
 *
 * const started = runtime.stream(built.request)
 * if (!started.ok) throw new Error(started.error.reason)
 *
 * const outcome = await consumeEvents(started.events)
 * // outcome.status === 'done', outcome.text === 'hello'
 * ```
 */

export {
  DEFAULT_PARAMETERS,
  MAX_DELTA_BYTES,
  MAX_DURATION_MS,
  MAX_ERROR_MESSAGE_BYTES,
  MAX_IDENTIFIER_BYTES,
  MAX_OUTPUT_TOKENS,
  MAX_PRIOR_MESSAGES,
  MAX_PROMPT_BYTES,
  MAX_RESPONSE_BYTES,
  MAX_RETRIES,
  MAX_TOKEN_COUNT,
  PROVIDER_ERROR_CATEGORIES,
  RECOVERY_ACTIONS,
  isTerminalEvent,
  terminalToEvent,
  type CompletionDelta,
  type CompletionEvent,
  type CompletionParameters,
  type CompletionRequest,
  type CompletionTerminal,
  type Message,
  type MessageRole,
  type ProviderError,
  type ProviderErrorCategory,
  type RecoveryAction,
  type Usage,
} from './contracts.js'

export {
  decodeDelta,
  decodeEvent,
  decodeMessage,
  decodeParameters,
  decodeProviderError,
  decodeRequestShape,
  decodeUsage,
  defaultParameters,
  encodeRequest,
  type DecodeIssue,
  type DecodeResult,
} from './decode.js'

export {
  describeValidationError,
  validateDelta,
  validateIdentifier,
  validateMessage,
  validateParameters,
  validateProviderError,
  validateRequest,
  validateUsage,
  type ValidationError,
  type ValidationResult,
} from './validate.js'

export {
  buildRequest,
  decodeRequest,
  type BuildResult,
  type ConversationError,
  type ConversationInput,
  type RequestRejection,
  type RequestResult,
} from './request.js'

export { errorTerminal, providerError, type CompletionProvider } from './provider.js'

export {
  createRuntime,
  type Runtime,
  type RuntimeOptions,
  type StartError,
  type StreamOptions,
  type StreamStart,
} from './runtime.js'

export {
  accumulate,
  consumeEvents,
  type CompletionOutcome,
  type ConsumeOptions,
  type StreamViolation,
} from './consumer.js'

export {
  MAX_NDJSON_LINE_BYTES,
  NdjsonError,
  describeNdjsonFailure,
  encodeEventLine,
  fromNdjsonStream,
  toNdjsonStream,
  type NdjsonFailure,
} from './ndjson.js'

export {
  FAKE_PROVIDER_ID,
  createFakeProvider,
  successScript,
  type FakeOutcome,
  type FakeScript,
} from './fake.js'

export { boundedMessage, hasUnpairedSurrogate, isIdentifierGrammar, utf8ByteLength } from './text.js'
