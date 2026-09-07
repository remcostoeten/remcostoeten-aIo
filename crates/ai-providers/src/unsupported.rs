//! Request features the Rust adapters do not yet transmit.
//!
//! Spec 0.2.0 added `priorMessages` and `parameters.maxOutputTokens` to the
//! completion request for the TypeScript consumer. No Rust consumer asks for
//! them yet, so no adapter here builds them into a provider body.
//!
//! Silently dropping either one would be worse than refusing: a dropped
//! conversation makes the model answer without context that the caller
//! believed it had, and a dropped token limit removes a spend and latency
//! control. Both therefore terminalize as `rejected_request` until the
//! adapters carry them. See `docs/decisions/0004-history-and-token-limit.md`.

use ai_core::AiCompletionRequest;

pub(crate) fn carries_untransmitted_fields(request: &AiCompletionRequest) -> bool {
    !request.prior_messages.is_empty() || request.parameters.max_output_tokens.is_some()
}
