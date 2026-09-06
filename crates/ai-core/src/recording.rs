//! Run accounting and the recorder port.
//!
//! This is decision D2 (`docs/architecture.md` §5, specified in
//! `docs/contracts.md` §2.8). The SDK hands out a metadata-only summary with a
//! discriminated status, plus the request borrowed for the duration of the
//! callback. It owns no prompt field, no retention policy, and no storage.
//!
//! An application that retains prompts copies them out of the borrowed request
//! inside the callback, before queueing anything. That keeps retention and
//! redaction where they already live — in the application's storage layer —
//! without the SDK caching prompts or growing a persistence dependency.

use crate::contracts::{AiProviderErrorCategory, AiUsage, MAX_AI_TOKEN_COUNT};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Bytes of text assumed to make one token when a provider reports no usage.
///
/// Deliberately coarse. Every count derived from it is carried as
/// [`AiTokenSource::Estimated`] and must be presented as an estimate.
pub const AI_TOKEN_ESTIMATE_BYTES: u64 = 4;

/// How a run ended, as accounting sees it.
///
/// One discriminated status, so a failure always carries its category and a
/// success can never carry one. This is the correction to the source record,
/// where `state` and `error_category` were independent fields that could
/// contradict each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiRunStatus {
    /// Normal completion.
    Done,
    /// Cancelled before completing.
    Cancelled,
    /// The provider's deadline elapsed.
    Timeout,
    /// Failed with a typed provider error.
    ProviderError {
        /// The failure category.
        category: AiProviderErrorCategory,
    },
}

/// Whether token counts came from the provider or from the byte heuristic.
///
/// Nothing may present an estimate as an exact count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AiTokenSource {
    /// The provider reported these counts.
    Provider,
    /// Derived from transferred bytes.
    Estimated,
}

/// Token counts for one run, tagged with their provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiRunTokens {
    /// Prompt tokens.
    pub input_tokens: u64,
    /// Response tokens.
    pub output_tokens: u64,
    /// Where the counts came from.
    pub source: AiTokenSource,
}

impl AiRunTokens {
    /// Counts a provider reported for itself.
    #[must_use]
    pub fn reported(usage: &AiUsage) -> Self {
        Self {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            source: AiTokenSource::Provider,
        }
    }

    /// Counts derived from transferred bytes because the provider reported
    /// none. Cancelled, timed-out and failed runs always land here.
    #[must_use]
    pub fn estimated(prompt_bytes: usize, response_bytes: usize) -> Self {
        Self {
            input_tokens: estimate_ai_tokens(prompt_bytes),
            output_tokens: estimate_ai_tokens(response_bytes),
            source: AiTokenSource::Estimated,
        }
    }
}

/// Estimates tokens from bytes as `ceil(bytes / 4)`, capped at the token bound.
#[must_use]
pub fn estimate_ai_tokens(bytes: usize) -> u64 {
    let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
    bytes
        .div_ceil(AI_TOKEN_ESTIMATE_BYTES)
        .min(MAX_AI_TOKEN_COUNT)
}

/// Per-million-token prices in micro-units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiModelPrice {
    /// Price per million prompt tokens.
    pub input_price_micros_per_mtok: u64,
    /// Price per million response tokens.
    pub output_price_micros_per_mtok: u64,
}

/// The narrow lookup the service uses to price a finished run.
///
/// The core ships no catalog. An application supplies one, so pricing data
/// stays a single application-owned source of truth.
pub trait AiModelPricing: Send + Sync {
    /// Returns the price for a provider/model pair, if it is priced at all.
    fn price(&self, provider_id: &str, model_id: &str) -> Option<AiModelPrice>;
}

/// Computes the cost of a run in micro-units, rounding half up.
///
/// Intermediate products are computed in `u128`: the per-million multiplication
/// overflows a `u64` — and JavaScript's exact-integer range — long before the
/// final amount does. Any port of this arithmetic must use an equally exact
/// intermediate type.
#[must_use]
pub fn ai_run_cost_micros(tokens: &AiRunTokens, price: AiModelPrice) -> u64 {
    let input = u128::from(tokens.input_tokens)
        .saturating_mul(u128::from(price.input_price_micros_per_mtok));
    let output = u128::from(tokens.output_tokens)
        .saturating_mul(u128::from(price.output_price_micros_per_mtok));
    let total = input.saturating_add(output).saturating_add(500_000) / 1_000_000;
    u64::try_from(total).unwrap_or(u64::MAX)
}

/// Everything the SDK knows about a finished run.
///
/// Deliberately not serializable: this is an in-process port argument, not a
/// shared wire contract. It has no prompt field, and its timestamp and cost
/// widths have not been given a cross-language encoding, so publishing it as a
/// DTO would export both problems at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiRunSummary {
    /// The request id this run used.
    pub run_id: String,
    /// Wall-clock start, in milliseconds since the Unix epoch.
    pub started_at_ms: i64,
    /// The application-supplied origin for this run.
    pub origin: String,
    /// The provider that served it.
    pub provider_id: String,
    /// The model it named.
    pub model_id: String,
    /// How it ended.
    pub status: AiRunStatus,
    /// Elapsed provider time, saturating at `u32::MAX`.
    pub duration_ms: u32,
    /// Token counts and their provenance.
    pub tokens: AiRunTokens,
    /// Cost in micro-units when the model was priced; `None` otherwise, which
    /// means unpriced, never free.
    pub cost_micros: Option<u64>,
}

/// The capability the service calls once per committed run.
///
/// Called after the terminal send attempt, on the thread that ran the request,
/// with no service lock held. Implementations must not block delivery: the
/// expected shape is a bounded queue, and dropping under pressure is the
/// application's choice.
///
/// `request` is borrowed for the duration of the call only. Copy out whatever
/// must outlive it — prompts included — before returning.
///
/// One invocation is not durable storage, and a failure inside it cannot
/// produce another terminal.
pub trait AiRunRecorder: Send + Sync {
    /// Records one finished run.
    fn record(&self, summary: AiRunSummary, request: &crate::contracts::AiCompletionRequest);
}
