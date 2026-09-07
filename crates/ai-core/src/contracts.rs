//! Serialized completion contracts.
//!
//! These types and their bounds are extracted unchanged from Skriuw
//! `crates/skriuw-domain/src/ai.rs` at revision `64827f5e`. Field names, wire
//! tags, accepted values, and null behavior are a compatibility surface: a
//! change here is a wire change even when both languages still compile. See
//! `docs/contracts.md` §§1–2.
//!
//! Deserialization alone does not enforce every rule. Each type carries an
//! explicit `validate`, and callers must invoke it; `deny_unknown_fields` and
//! the derived schemas describe structure only.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Maximum length of any identifier field, in UTF-8 bytes.
pub const MAX_AI_IDENTIFIER_BYTES: usize = 128;
/// Maximum combined prompt size, in UTF-8 bytes.
///
/// Since spec 0.2.0 this budget spans the system prompt, every prior message's
/// content, and the final user prompt together. Before the history delta it
/// covered only the two prompts.
pub const MAX_AI_PROMPT_BYTES: usize = 1024 * 1024;
/// Maximum number of conversation turns preceding the final user prompt.
pub const MAX_AI_PRIOR_MESSAGES: usize = 64;
/// Maximum accepted value for a requested output token limit.
pub const MAX_AI_OUTPUT_TOKENS: u32 = 1_000_000;
/// Hard ceiling on accumulated response bytes, independent of the request.
pub const MAX_AI_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// Maximum size of a single delta, in UTF-8 bytes.
pub const MAX_AI_DELTA_BYTES: usize = 64 * 1024;
/// Maximum accepted request timeout, in milliseconds.
pub const MAX_AI_DURATION_MS: u32 = 5 * 60 * 1_000;
/// Maximum accepted retry count. Accepted, but never acted upon: the core has
/// no retry engine, and requesting retries does not produce any.
pub const MAX_AI_RETRIES: u8 = 2;
/// Maximum length of a provider error message, in UTF-8 bytes.
pub const MAX_AI_ERROR_MESSAGE_BYTES: usize = 1_024;
/// Maximum accepted value for a single token counter.
pub const MAX_AI_TOKEN_COUNT: u64 = 1_000_000_000;

/// Why a contract value was rejected.
///
/// Field labels are static and closed. They are diagnostic identity, never a
/// caller-supplied string, so this stays safe to surface.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AiValidationError {
    /// A required field held an empty string.
    #[error("{field} cannot be empty")]
    Empty {
        /// The rejected field.
        field: &'static str,
    },
    /// A field exceeded its byte budget.
    #[error("{field} exceeds {maximum} bytes")]
    TooLong {
        /// The rejected field.
        field: &'static str,
        /// The byte budget that was exceeded.
        maximum: usize,
    },
    /// An identifier contained bytes outside the accepted grammar.
    #[error("{field} contains unsupported characters")]
    InvalidIdentifier {
        /// The rejected field.
        field: &'static str,
    },
    /// The combined prompt exceeded [`MAX_AI_PROMPT_BYTES`].
    #[error("completion prompt exceeds {maximum} bytes")]
    PromptTooLong {
        /// The byte budget that was exceeded.
        maximum: usize,
    },
    /// The requested output limit was zero or above [`MAX_AI_RESPONSE_BYTES`].
    #[error("maximum output bytes must be between 1 and {maximum}")]
    InvalidOutputLimit {
        /// The byte budget that was exceeded.
        maximum: usize,
    },
    /// The requested timeout was zero or above [`MAX_AI_DURATION_MS`].
    #[error("completion timeout must be between 1 and {maximum} milliseconds")]
    InvalidTimeout {
        /// The millisecond budget that was exceeded.
        maximum: u32,
    },
    /// The requested retry count was above [`MAX_AI_RETRIES`].
    #[error("completion retries exceed {maximum}")]
    TooManyRetries {
        /// The retry budget that was exceeded.
        maximum: u8,
    },
    /// A sampling parameter was outside 0 through 1000.
    #[error("{field} must be between 0 and 1000")]
    InvalidSamplingParameter {
        /// The rejected field.
        field: &'static str,
    },
    /// A token counter was above [`MAX_AI_TOKEN_COUNT`].
    #[error("{field} exceeds {maximum}")]
    TokenCountTooLarge {
        /// The rejected field.
        field: &'static str,
        /// The count budget that was exceeded.
        maximum: u64,
    },
    /// The conversation carried more turns than [`MAX_AI_PRIOR_MESSAGES`].
    #[error("prior messages exceed {maximum} turns")]
    TooManyMessages {
        /// The turn budget that was exceeded.
        maximum: usize,
    },
    /// A requested output token limit was zero or above
    /// [`MAX_AI_OUTPUT_TOKENS`].
    #[error("maximum output tokens must be between 1 and {maximum}")]
    InvalidOutputTokenLimit {
        /// The token budget that was exceeded.
        maximum: u32,
    },
}

/// Who produced a conversation turn.
///
/// Closed on the wire, and deliberately without a `system` value: the system
/// instruction is [`AiCompletionRequest::system_prompt`], so a conversation
/// cannot carry a second, competing instruction channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AiMessageRole {
    /// A turn from the person.
    User,
    /// A turn the model previously produced.
    Assistant,
}

/// One turn of conversation preceding the final user prompt.
///
/// Text only. Content parts, tool calls, attachments and names are absent
/// until a consumer needs them; see `docs/contracts.md` §4.1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiMessage {
    /// Who produced this turn.
    pub role: AiMessageRole,
    /// Turn text. Never empty.
    pub content: String,
}

impl AiMessage {
    /// Builds a user turn.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: AiMessageRole::User,
            content: content.into(),
        }
    }

    /// Builds an assistant turn.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: AiMessageRole::Assistant,
            content: content.into(),
        }
    }

    /// Checks that the turn carries text.
    ///
    /// The combined prompt budget spans every turn together and is therefore
    /// checked by [`AiCompletionRequest::validate`], not here.
    ///
    /// # Errors
    ///
    /// Returns [`AiValidationError::Empty`] for empty content.
    pub fn validate(&self) -> Result<(), AiValidationError> {
        if self.content.is_empty() {
            return Err(AiValidationError::Empty {
                field: "message content",
            });
        }
        Ok(())
    }
}

/// Execution parameters for one completion.
///
/// `retry_count` is accepted and validated but inert: no retry is performed.
/// Sampling parameters are thousandths, so 0 through 1000, and may be omitted
/// on the wire as well as sent explicitly as `null`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiCompletionParameters {
    /// Response byte budget the provider must not exceed.
    pub max_output_bytes: u32,
    /// Provider-observed timeout. This is not a service-wide deadline; see
    /// `docs/contracts.md` §3.2.
    pub timeout_ms: u32,
    /// Accepted but inert. Present for wire compatibility only.
    pub retry_count: u8,
    /// Sampling temperature in thousandths.
    pub temperature_millis: Option<u16>,
    /// Nucleus sampling threshold in thousandths.
    pub top_p_millis: Option<u16>,
    /// Output token ceiling asked of the provider, or `null` for its default.
    ///
    /// Distinct from `max_output_bytes`, which is this side's accumulation cap
    /// and is enforced locally. A provider that ignores this field is not a
    /// contract violation; the byte cap still applies.
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
}

impl Default for AiCompletionParameters {
    fn default() -> Self {
        Self {
            max_output_bytes: 256 * 1024,
            timeout_ms: 60_000,
            retry_count: 0,
            temperature_millis: None,
            top_p_millis: None,
            max_output_tokens: None,
        }
    }
}

impl AiCompletionParameters {
    /// Checks every bound this type does not encode in its field types.
    ///
    /// # Errors
    ///
    /// Returns the first violated bound.
    pub fn validate(&self) -> Result<(), AiValidationError> {
        if self.max_output_bytes == 0
            || usize::try_from(self.max_output_bytes).unwrap_or(usize::MAX) > MAX_AI_RESPONSE_BYTES
        {
            return Err(AiValidationError::InvalidOutputLimit {
                maximum: MAX_AI_RESPONSE_BYTES,
            });
        }
        if self.timeout_ms == 0 || self.timeout_ms > MAX_AI_DURATION_MS {
            return Err(AiValidationError::InvalidTimeout {
                maximum: MAX_AI_DURATION_MS,
            });
        }
        if self.retry_count > MAX_AI_RETRIES {
            return Err(AiValidationError::TooManyRetries {
                maximum: MAX_AI_RETRIES,
            });
        }
        validate_sampling_parameter("temperature", self.temperature_millis)?;
        validate_sampling_parameter("top p", self.top_p_millis)?;
        if self
            .max_output_tokens
            .is_some_and(|tokens| tokens == 0 || tokens > MAX_AI_OUTPUT_TOKENS)
        {
            return Err(AiValidationError::InvalidOutputTokenLimit {
                maximum: MAX_AI_OUTPUT_TOKENS,
            });
        }
        Ok(())
    }
}

/// One completion request.
///
/// Provider and model identity are flat by design; a nested model reference is
/// a later, breaking wire change. `origin` is deliberately absent: it is an
/// application concept passed separately to the service and never sent to a
/// provider.
///
/// The conversation a provider receives is exactly
/// `system_prompt`, then `prior_messages` in order, then `user_prompt` as the
/// final user turn. `user_prompt` is the sole authority for that final turn, so
/// a consumer holding a `messages` array sends everything but its last entry as
/// `prior_messages`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiCompletionRequest {
    /// Caller-chosen identity, unique among the caller's active requests.
    pub request_id: String,
    /// Identifies a registered provider instance, not a vendor globally.
    pub provider_id: String,
    /// Provider-scoped model identity.
    pub model_id: String,
    /// May be empty.
    pub system_prompt: String,
    /// The final user turn. May be empty.
    pub user_prompt: String,
    /// Turns preceding `user_prompt`, oldest first. Omission means none.
    ///
    /// No alternation rule is imposed: whether consecutive same-role turns are
    /// meaningful is the consuming application's judgement, and providers
    /// differ on it.
    #[serde(default)]
    pub prior_messages: Vec<AiMessage>,
    /// Execution parameters.
    pub parameters: AiCompletionParameters,
}

impl AiCompletionRequest {
    /// Checks identifiers, the combined prompt budget, and the parameters.
    ///
    /// # Errors
    ///
    /// Returns the first violated bound.
    pub fn validate(&self) -> Result<(), AiValidationError> {
        validate_identifier("request id", &self.request_id)?;
        validate_identifier("provider id", &self.provider_id)?;
        validate_identifier("model id", &self.model_id)?;
        if self.prior_messages.len() > MAX_AI_PRIOR_MESSAGES {
            return Err(AiValidationError::TooManyMessages {
                maximum: MAX_AI_PRIOR_MESSAGES,
            });
        }
        let mut prompt_bytes = self
            .system_prompt
            .len()
            .saturating_add(self.user_prompt.len());
        for message in &self.prior_messages {
            message.validate()?;
            prompt_bytes = prompt_bytes.saturating_add(message.content.len());
        }
        if prompt_bytes > MAX_AI_PROMPT_BYTES {
            return Err(AiValidationError::PromptTooLong {
                maximum: MAX_AI_PROMPT_BYTES,
            });
        }
        self.parameters.validate()
    }
}

/// One chunk of streamed output.
///
/// Sequence numbers start at zero and increase by one. Empty text is accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiCompletionDelta {
    /// The request this delta belongs to.
    pub request_id: String,
    /// Zero-based position in the stream.
    pub sequence: u32,
    /// Output text. May be empty.
    pub text: String,
}

impl AiCompletionDelta {
    /// Checks the request identity and the delta byte budget.
    ///
    /// # Errors
    ///
    /// Returns the first violated bound.
    pub fn validate(&self) -> Result<(), AiValidationError> {
        validate_identifier("request id", &self.request_id)?;
        if self.text.len() > MAX_AI_DELTA_BYTES {
            return Err(AiValidationError::TooLong {
                field: "completion delta",
                maximum: MAX_AI_DELTA_BYTES,
            });
        }
        Ok(())
    }
}

/// Token counts a provider reported for itself.
///
/// Absent usage means the provider reported none. It never means zero, and
/// must not be fabricated as zero. Estimation lives in run accounting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiUsage {
    /// Prompt tokens the provider reported.
    pub input_tokens: u64,
    /// Response tokens the provider reported.
    pub output_tokens: u64,
}

impl AiUsage {
    /// Checks both counters against [`MAX_AI_TOKEN_COUNT`].
    ///
    /// # Errors
    ///
    /// Returns the first violated bound.
    pub fn validate(&self) -> Result<(), AiValidationError> {
        validate_token_count("input tokens", self.input_tokens)?;
        validate_token_count("output tokens", self.output_tokens)
    }
}

/// Semantic, provider-independent failure category.
///
/// This enum is closed on the wire. Timeout and cancellation are terminal
/// variants rather than error categories, and adding a value here is a
/// breaking change for every decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AiProviderErrorCategory {
    /// No such provider is registered, or it cannot serve requests.
    UnavailableProvider,
    /// No credential was available.
    MissingCredential,
    /// The credential was rejected.
    InvalidCredential,
    /// The account's quota is exhausted.
    QuotaExhausted,
    /// The provider applied rate limiting.
    RateLimited,
    /// The provider rejected the request itself.
    RejectedRequest,
    /// The exchange failed below the protocol.
    TransportFailure,
    /// The provider's output could not be interpreted, or broke the contract.
    MalformedResponse,
    /// A defect on this side of the boundary.
    InternalFailure,
}

/// A hint about what a person could do next.
///
/// This is presentation advice. It is never permission for the SDK to retry,
/// switch model, or fall back on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AiRecoveryAction {
    /// Supply or correct a credential.
    ConfigureCredential,
    /// Try again later.
    Retry,
    /// Pick another model.
    ChooseDifferentModel,
    /// Check whether the provider is up.
    CheckProviderStatus,
    /// Send less input.
    ReduceRequest,
    /// Escalate to the provider.
    ContactProvider,
    /// Nothing actionable.
    None,
}

/// A typed provider failure.
///
/// Carries no HTTP status, response body, or diagnostics bag: provider bodies
/// can contain prompts or secrets and stay internal to the adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiProviderError {
    /// The provider that failed.
    pub provider_id: String,
    /// Semantic category.
    pub category: AiProviderErrorCategory,
    /// Bounded, whitespace-normalized display text.
    pub message: String,
    /// Presentation hint.
    pub recovery_action: AiRecoveryAction,
}

impl AiProviderError {
    /// Builds an error, normalizing and bounding the message.
    ///
    /// Control characters and runs of whitespace collapse to single spaces, the
    /// result is truncated on a character boundary to
    /// [`MAX_AI_ERROR_MESSAGE_BYTES`], and an empty result becomes a generic
    /// message so the value always satisfies [`Self::validate`].
    #[must_use]
    pub fn new(
        provider_id: impl Into<String>,
        category: AiProviderErrorCategory,
        message: &str,
        recovery_action: AiRecoveryAction,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            category,
            message: bounded_message(message),
            recovery_action,
        }
    }

    /// Checks the provider identity and the message bounds.
    ///
    /// # Errors
    ///
    /// Returns the first violated bound. A value built by [`Self::new`] always
    /// passes; a deserialized one need not.
    pub fn validate(&self) -> Result<(), AiValidationError> {
        validate_identifier("provider id", &self.provider_id)?;
        if self.message.is_empty() {
            return Err(AiValidationError::Empty {
                field: "provider error message",
            });
        }
        if self.message.len() > MAX_AI_ERROR_MESSAGE_BYTES {
            return Err(AiValidationError::TooLong {
                field: "provider error message",
                maximum: MAX_AI_ERROR_MESSAGE_BYTES,
            });
        }
        Ok(())
    }
}

/// Everything a consumer observes for one request.
///
/// Exactly one of the four terminal variants ends a committed run. Unknown
/// tags, unknown fields, and unknown enum values are rejected on decode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AiCompletionEvent {
    /// Streamed output.
    Delta(AiCompletionDelta),
    /// Normal completion.
    Done {
        /// The request this event belongs to.
        request_id: String,
        /// Provider-reported usage, or `null` when the provider reported none.
        usage: Option<AiUsage>,
    },
    /// The run was cancelled before completing.
    Cancelled {
        /// The request this event belongs to.
        request_id: String,
    },
    /// The provider's deadline elapsed.
    Timeout {
        /// The request this event belongs to.
        request_id: String,
    },
    /// The run failed with a typed provider error.
    ProviderError {
        /// The request this event belongs to.
        request_id: String,
        /// The failure.
        error: AiProviderError,
    },
}

impl AiCompletionEvent {
    /// Checks the event's identity and payload.
    ///
    /// # Errors
    ///
    /// Returns the first violated bound.
    pub fn validate(&self) -> Result<(), AiValidationError> {
        match self {
            Self::Delta(delta) => delta.validate(),
            Self::Done { request_id, usage } => {
                validate_identifier("request id", request_id)?;
                if let Some(usage) = usage {
                    usage.validate()?;
                }
                Ok(())
            }
            Self::Cancelled { request_id } | Self::Timeout { request_id } => {
                validate_identifier("request id", request_id)
            }
            Self::ProviderError { request_id, error } => {
                validate_identifier("request id", request_id)?;
                error.validate()
            }
        }
    }
}

/// How a completion ended, before the request identity is attached.
///
/// In-process only: this is what a provider returns and what the service
/// commits. The serialized form is [`AiCompletionEvent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiCompletionTerminal {
    /// Normal completion.
    Done {
        /// Provider-reported usage, if any.
        usage: Option<AiUsage>,
    },
    /// Cancelled before completing.
    Cancelled,
    /// The provider's deadline elapsed.
    Timeout,
    /// A typed provider failure.
    ProviderError(AiProviderError),
}

impl AiCompletionTerminal {
    /// Attaches a request identity, producing the deliverable event.
    #[must_use]
    pub fn into_event(self, request_id: String) -> AiCompletionEvent {
        match self {
            Self::Done { usage } => AiCompletionEvent::Done { request_id, usage },
            Self::Cancelled => AiCompletionEvent::Cancelled { request_id },
            Self::Timeout => AiCompletionEvent::Timeout { request_id },
            Self::ProviderError(error) => AiCompletionEvent::ProviderError { request_id, error },
        }
    }
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), AiValidationError> {
    if value.is_empty() {
        return Err(AiValidationError::Empty { field });
    }
    if value.len() > MAX_AI_IDENTIFIER_BYTES {
        return Err(AiValidationError::TooLong {
            field,
            maximum: MAX_AI_IDENTIFIER_BYTES,
        });
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
    }) {
        return Err(AiValidationError::InvalidIdentifier { field });
    }
    Ok(())
}

fn validate_sampling_parameter(
    field: &'static str,
    value: Option<u16>,
) -> Result<(), AiValidationError> {
    if value.is_some_and(|value| value > 1_000) {
        return Err(AiValidationError::InvalidSamplingParameter { field });
    }
    Ok(())
}

fn validate_token_count(field: &'static str, value: u64) -> Result<(), AiValidationError> {
    if value > MAX_AI_TOKEN_COUNT {
        return Err(AiValidationError::TokenCountTooLarge {
            field,
            maximum: MAX_AI_TOKEN_COUNT,
        });
    }
    Ok(())
}

fn bounded_message(message: &str) -> String {
    let mut bounded = String::with_capacity(message.len().min(MAX_AI_ERROR_MESSAGE_BYTES));
    let mut previous_was_whitespace = false;

    for character in message.chars() {
        let character = if character.is_control() || character.is_whitespace() {
            ' '
        } else {
            character
        };
        if character == ' ' && previous_was_whitespace {
            continue;
        }
        if bounded.len() + character.len_utf8() > MAX_AI_ERROR_MESSAGE_BYTES {
            break;
        }
        bounded.push(character);
        previous_was_whitespace = character == ' ';
    }

    let bounded = bounded.trim().to_owned();
    if bounded.is_empty() {
        "provider request failed".to_owned()
    } else {
        bounded
    }
}
