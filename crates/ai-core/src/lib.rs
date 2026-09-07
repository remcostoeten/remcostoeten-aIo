//! Provider-neutral AI completion contracts and execution.
//!
//! `ai-core` owns one narrow seam: a validated completion request goes in,
//! ordered deltas and exactly one terminal come out. It knows nothing about
//! HTTP, credentials, prompts, products, or persistence.
//!
//! ```
//! use std::sync::Arc;
//! use ai_core::{
//!     AiComplete, AiCompletionEvent, AiCompletionParameters, AiCompletionRequest,
//!     AiCompletionService, AiSinkError, FakeAiProvider, FakeCompletionScript,
//! };
//!
//! struct Collector(std::sync::mpsc::Sender<AiCompletionEvent>);
//! impl ai_core::AiCompletionChannel for Collector {
//!     fn send(&self, event: AiCompletionEvent) -> Result<(), AiSinkError> {
//!         self.0.send(event).map_err(|_| AiSinkError::Closed)
//!     }
//! }
//!
//! let provider: Arc<dyn AiComplete> =
//!     Arc::new(FakeAiProvider::new(FakeCompletionScript::success(["hi"])));
//! let service = AiCompletionService::new([("fake".to_owned(), provider)]);
//! let (sender, events) = std::sync::mpsc::channel();
//!
//! service.start(
//!     "playground".to_owned(),
//!     AiCompletionRequest {
//!         request_id: "request-1".to_owned(),
//!         provider_id: "fake".to_owned(),
//!         model_id: "model".to_owned(),
//!         system_prompt: String::new(),
//!         user_prompt: "hello".to_owned(),
//!         prior_messages: Vec::new(),
//!         parameters: AiCompletionParameters::default(),
//!     },
//!     Collector(sender),
//! )?;
//!
//! let collected: Vec<_> = events.iter().collect();
//! assert!(matches!(collected.last(), Some(AiCompletionEvent::Done { .. })));
//! # Ok::<(), ai_core::AiStartError>(())
//! ```
//!
//! # Guarantees, and their limits
//!
//! A committed run produces exactly one terminal and attempts to deliver it
//! once. That is not a delivery guarantee: a closed channel cannot receive it,
//! cancellation cannot interrupt a provider that blocks or never returns, and
//! nothing survives process abort. `timeout_ms` is observed by the provider,
//! not enforced by a service watchdog. See `docs/contracts.md` §3.

mod contracts;
mod fake;
mod ports;
mod recording;
mod service;

pub use contracts::{
    AiCompletionDelta, AiCompletionEvent, AiCompletionParameters, AiCompletionRequest,
    AiCompletionTerminal, AiMessage, AiMessageRole, AiProviderError, AiProviderErrorCategory,
    AiRecoveryAction, AiUsage, AiValidationError, MAX_AI_DELTA_BYTES, MAX_AI_DURATION_MS,
    MAX_AI_ERROR_MESSAGE_BYTES, MAX_AI_IDENTIFIER_BYTES, MAX_AI_OUTPUT_TOKENS,
    MAX_AI_PRIOR_MESSAGES, MAX_AI_PROMPT_BYTES, MAX_AI_RESPONSE_BYTES, MAX_AI_RETRIES,
    MAX_AI_TOKEN_COUNT,
};
pub use fake::{FakeAiProvider, FakeCompletionOutcome, FakeCompletionScript};
pub use ports::{AiCancellation, AiComplete, AiCompletionChannel, AiEventSink, AiSinkError};
pub use recording::{
    AI_TOKEN_ESTIMATE_BYTES, AiModelPrice, AiModelPricing, AiRunRecorder, AiRunStatus,
    AiRunSummary, AiRunTokens, AiTokenSource, ai_run_cost_micros, estimate_ai_tokens,
};
pub use service::{AiCompletionService, AiStartError};

#[cfg(feature = "schema-tool")]
pub mod schema;
