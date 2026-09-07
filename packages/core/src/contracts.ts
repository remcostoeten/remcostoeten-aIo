/**
 * The serialized completion contracts, in TypeScript.
 *
 * These are the same semantic contracts `crates/ai-core` defines in Rust, not a
 * generated translation of them: field names, wire tags, accepted values and
 * null behaviour are the compatibility surface, and `specs/schemas` plus
 * `specs/fixtures` are the shared authority both languages are tested against.
 * A change here is a wire change even when both languages still compile.
 *
 * Spec 0.2.0. See `docs/contracts.md` and
 * `docs/decisions/0004-history-and-token-limit.md`.
 */

/** Maximum length of any identifier field, in UTF-8 bytes. */
export const MAX_IDENTIFIER_BYTES = 128
/** Maximum combined system, history and user prompt size, in UTF-8 bytes. */
export const MAX_PROMPT_BYTES = 1024 * 1024
/** Hard ceiling on accumulated response bytes, independent of the request. */
export const MAX_RESPONSE_BYTES = 4 * 1024 * 1024
/** Maximum size of a single delta, in UTF-8 bytes. */
export const MAX_DELTA_BYTES = 64 * 1024
/** Maximum accepted request timeout, in milliseconds. */
export const MAX_DURATION_MS = 5 * 60 * 1000
/** Accepted, but never acted upon: there is no retry engine. */
export const MAX_RETRIES = 2
/** Maximum length of a provider error message, in UTF-8 bytes. */
export const MAX_ERROR_MESSAGE_BYTES = 1024
/** Maximum accepted value for a single token counter. */
export const MAX_TOKEN_COUNT = 1_000_000_000
/** Maximum number of turns preceding the final user prompt. */
export const MAX_PRIOR_MESSAGES = 64
/** Maximum accepted value for a requested output token limit. */
export const MAX_OUTPUT_TOKENS = 1_000_000

/**
 * Who produced a conversation turn.
 *
 * Deliberately without `system`: the system instruction is
 * {@link CompletionRequest.systemPrompt}, so a conversation cannot carry a
 * second, competing instruction channel.
 */
export type MessageRole = 'user' | 'assistant'

/** One turn of conversation preceding the final user prompt. Text only. */
export type Message = {
  readonly role: MessageRole
  /** Never empty. */
  readonly content: string
}

export type CompletionParameters = {
  /** Response byte budget, enforced on this side of the boundary. */
  readonly maxOutputBytes: number
  /** Provider-observed timeout, not a service-wide deadline. */
  readonly timeoutMs: number
  /** Accepted but inert. Present for wire compatibility only. */
  readonly retryCount: number
  /** Sampling temperature in thousandths, or null. */
  readonly temperatureMillis: number | null
  /** Nucleus sampling threshold in thousandths, or null. */
  readonly topPMillis: number | null
  /**
   * Output token ceiling asked of the provider, or null for its default.
   *
   * Distinct from {@link CompletionParameters.maxOutputBytes}, which is the
   * local accumulation cap. A provider that ignores this is not a contract
   * violation; the byte cap still applies.
   */
  readonly maxOutputTokens: number | null
}

/** The values a request omitting optional fields decodes to. */
export const DEFAULT_PARAMETERS: CompletionParameters = {
  maxOutputBytes: 262_144,
  timeoutMs: 60_000,
  retryCount: 0,
  temperatureMillis: null,
  topPMillis: null,
  maxOutputTokens: null,
}

/**
 * One completion request.
 *
 * The conversation a provider receives is exactly `systemPrompt`, then
 * `priorMessages` in order, then `userPrompt` as the final user turn.
 * `userPrompt` is the sole authority for that final turn, so a consumer holding
 * a `messages` array sends everything but its last entry as `priorMessages`.
 *
 * `origin` is deliberately absent: it is an application concept passed
 * separately to the runtime and never sent to a provider.
 */
export type CompletionRequest = {
  readonly requestId: string
  /** Identifies a registered provider instance, not a vendor globally. */
  readonly providerId: string
  readonly modelId: string
  /** May be empty. */
  readonly systemPrompt: string
  /** The final user turn. May be empty. */
  readonly userPrompt: string
  /** Turns preceding `userPrompt`, oldest first. */
  readonly priorMessages: readonly Message[]
  readonly parameters: CompletionParameters
}

/** Token counts a provider reported for itself. Never fabricated as zero. */
export type Usage = {
  readonly inputTokens: number
  readonly outputTokens: number
}

/**
 * Semantic, provider-independent failure category.
 *
 * Closed on the wire. Timeout and cancellation are terminal variants rather
 * than error categories, and adding a value here breaks every decoder.
 */
export type ProviderErrorCategory =
  | 'unavailable_provider'
  | 'missing_credential'
  | 'invalid_credential'
  | 'quota_exhausted'
  | 'rate_limited'
  | 'rejected_request'
  | 'transport_failure'
  | 'malformed_response'
  | 'internal_failure'

export const PROVIDER_ERROR_CATEGORIES: readonly ProviderErrorCategory[] = [
  'unavailable_provider',
  'missing_credential',
  'invalid_credential',
  'quota_exhausted',
  'rate_limited',
  'rejected_request',
  'transport_failure',
  'malformed_response',
  'internal_failure',
]

/**
 * A hint about what a person could do next.
 *
 * Presentation advice. Never permission for the SDK to retry, switch model, or
 * fall back on its own.
 */
export type RecoveryAction =
  | 'configure_credential'
  | 'retry'
  | 'choose_different_model'
  | 'check_provider_status'
  | 'reduce_request'
  | 'contact_provider'
  | 'none'

export const RECOVERY_ACTIONS: readonly RecoveryAction[] = [
  'configure_credential',
  'retry',
  'choose_different_model',
  'check_provider_status',
  'reduce_request',
  'contact_provider',
  'none',
]

/**
 * A typed provider failure.
 *
 * Carries no HTTP status, response body, or diagnostics bag: provider bodies
 * can contain prompts or secrets and stay inside the adapter.
 */
export type ProviderError = {
  readonly providerId: string
  readonly category: ProviderErrorCategory
  /** Bounded, whitespace-normalized display text. Never empty. */
  readonly message: string
  readonly recoveryAction: RecoveryAction
}

export type CompletionDelta = {
  readonly requestId: string
  /** Zero-based, increasing by one. */
  readonly sequence: number
  /** May be empty. */
  readonly text: string
}

/** Everything a consumer observes for one request. */
export type CompletionEvent =
  | { readonly type: 'delta'; readonly requestId: string; readonly sequence: number; readonly text: string }
  | { readonly type: 'done'; readonly requestId: string; readonly usage: Usage | null }
  | { readonly type: 'cancelled'; readonly requestId: string }
  | { readonly type: 'timeout'; readonly requestId: string }
  | { readonly type: 'provider_error'; readonly requestId: string; readonly error: ProviderError }

/** How a completion ended, before the request identity is attached. */
export type CompletionTerminal =
  | { readonly type: 'done'; readonly usage: Usage | null }
  | { readonly type: 'cancelled' }
  | { readonly type: 'timeout' }
  | { readonly type: 'provider_error'; readonly error: ProviderError }

export function terminalToEvent(terminal: CompletionTerminal, requestId: string): CompletionEvent {
  switch (terminal.type) {
    case 'done':
      return { type: 'done', requestId, usage: terminal.usage }
    case 'cancelled':
      return { type: 'cancelled', requestId }
    case 'timeout':
      return { type: 'timeout', requestId }
    case 'provider_error':
      return { type: 'provider_error', requestId, error: terminal.error }
  }
}

/** True when the event ends its run. Exactly one such event is emitted. */
export function isTerminalEvent(event: CompletionEvent): boolean {
  return event.type !== 'delta'
}
