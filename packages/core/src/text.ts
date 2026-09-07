/**
 * String measurement that matches Rust's, not JavaScript's.
 *
 * Every bound in the contracts is a UTF-8 byte budget. `String.length` counts
 * UTF-16 code units, so it under-counts every non-ASCII character and would
 * silently accept requests Rust rejects. JSON Schema's `maxLength` has the same
 * problem and cannot express these bounds either.
 */

/** UTF-8 byte length, without allocating an encoded copy. */
export function utf8ByteLength(value: string): number {
  let bytes = 0
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index)
    if (code < 0x80) {
      bytes += 1
    } else if (code < 0x800) {
      bytes += 2
    } else if (code >= 0xd800 && code <= 0xdbff && index + 1 < value.length) {
      const low = value.charCodeAt(index + 1)
      if (low >= 0xdc00 && low <= 0xdfff) {
        bytes += 4
        index += 1
        continue
      }
      bytes += 3
    } else {
      bytes += 3
    }
  }
  return bytes
}

/**
 * True when the string holds a lone surrogate.
 *
 * Such a string has no UTF-8 encoding. Rust `String` cannot hold one, so a
 * boundary that accepted it would produce a value the other language cannot
 * represent — it is rejected rather than replaced with U+FFFD.
 */
export function hasUnpairedSurrogate(value: string): boolean {
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

const IDENTIFIER_BYTE = /^[A-Za-z0-9\-_.:/]+$/

/**
 * The core identifier grammar.
 *
 * It permits `.` and therefore `..`. That is deliberate and matches Rust: this
 * is an identity check, not a safe filesystem path or an encoded URL segment.
 * Provider model authority and URL construction are separate boundaries.
 */
export function isIdentifierGrammar(value: string): boolean {
  return IDENTIFIER_BYTE.test(value)
}

/**
 * Normalizes and bounds a provider error message, exactly as Rust does.
 *
 * Control characters and whitespace runs collapse to single spaces, the result
 * is truncated on a character boundary, and an empty result becomes a generic
 * message so the value always passes validation.
 */
export function boundedMessage(message: string): string {
  let bounded = ''
  let bytes = 0
  let previousWasWhitespace = false

  for (const character of message) {
    const normalized = isControlOrWhitespace(character) ? ' ' : character
    if (normalized === ' ' && previousWasWhitespace) continue
    const width = utf8ByteLength(normalized)
    if (bytes + width > MAX_MESSAGE_BYTES) break
    bounded += normalized
    bytes += width
    previousWasWhitespace = normalized === ' '
  }

  const trimmed = bounded.trim()
  return trimmed === '' ? 'provider request failed' : trimmed
}

const MAX_MESSAGE_BYTES = 1024

function isControlOrWhitespace(character: string): boolean {
  const code = character.codePointAt(0) ?? 0
  return code < 0x20 || code === 0x7f || /\s/u.test(character)
}
