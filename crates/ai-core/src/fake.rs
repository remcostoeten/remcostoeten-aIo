//! Deterministic fake provider.
//!
//! Intentionally part of core. "No provider names in core" excludes real vendor
//! dispatch, endpoints and credentials — not a scripted provider whose whole
//! purpose is offline tests and a playground.

use std::{thread, time::Duration, time::Instant};

use crate::{
    contracts::{
        AiCompletionDelta, AiCompletionRequest, AiCompletionTerminal, AiProviderError,
        AiProviderErrorCategory, AiRecoveryAction, AiUsage, MAX_AI_RESPONSE_BYTES,
    },
    ports::{AiCancellation, AiComplete, AiEventSink},
};

/// How a scripted run ends after its tokens are emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeCompletionOutcome {
    /// Normal completion, optionally reporting usage.
    Done {
        /// Usage to report. Invalid usage is turned into a malformed-response
        /// failure, exercising the validation path.
        usage: Option<AiUsage>,
    },
    /// Ends in a timeout terminal.
    Timeout,
    /// Ends in a `malformed_response` provider error.
    MalformedOutput,
    /// Ends in the given provider error. An invalid error is itself turned
    /// into a malformed-response failure.
    ProviderError(AiProviderError),
}

/// A fixed sequence of output tokens and a terminal outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeCompletionScript {
    /// Emitted in order, one delta each.
    pub tokens: Vec<String>,
    /// Simulated pause before each token. Cancellation and the deadline are
    /// polled while waiting.
    pub token_delay: Duration,
    /// How the run ends once every token has been emitted.
    pub outcome: FakeCompletionOutcome,
}

impl FakeCompletionScript {
    /// A script that emits the given tokens and completes normally.
    #[must_use]
    pub fn success(tokens: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            tokens: tokens.into_iter().map(Into::into).collect(),
            token_delay: Duration::ZERO,
            outcome: FakeCompletionOutcome::Done { usage: None },
        }
    }
}

/// A provider that replays a [`FakeCompletionScript`].
///
/// Deterministic in output and outcome. Only elapsed wall-clock time varies, so
/// tests may compare text, sequence and terminal but never durations.
#[derive(Debug, Clone)]
pub struct FakeAiProvider {
    script: FakeCompletionScript,
}

impl FakeAiProvider {
    /// Builds a provider from a script.
    #[must_use]
    pub fn new(script: FakeCompletionScript) -> Self {
        Self { script }
    }

    fn wait_for_token(
        &self,
        cancellation: &AiCancellation,
        remaining: Duration,
        deadline: Instant,
    ) -> Result<(), AiCompletionTerminal> {
        let mut waited = Duration::ZERO;
        while waited < remaining {
            if cancellation.is_cancelled() {
                return Err(AiCompletionTerminal::Cancelled);
            }
            let until_deadline = deadline.saturating_duration_since(Instant::now());
            if until_deadline.is_zero() {
                cancellation.cancel();
                return Err(AiCompletionTerminal::Timeout);
            }
            let interval = (remaining - waited)
                .min(Duration::from_millis(1))
                .min(until_deadline);
            thread::sleep(interval);
            waited += interval;
        }
        Ok(())
    }

    fn rejected_request(&self, request: &AiCompletionRequest) -> AiCompletionTerminal {
        AiCompletionTerminal::ProviderError(AiProviderError::new(
            request.provider_id.clone(),
            AiProviderErrorCategory::RejectedRequest,
            "completion request is invalid",
            AiRecoveryAction::ReduceRequest,
        ))
    }

    fn malformed_response(
        &self,
        request: &AiCompletionRequest,
        message: &str,
        recovery_action: AiRecoveryAction,
    ) -> AiCompletionTerminal {
        AiCompletionTerminal::ProviderError(AiProviderError::new(
            request.provider_id.clone(),
            AiProviderErrorCategory::MalformedResponse,
            message,
            recovery_action,
        ))
    }
}

impl AiComplete for FakeAiProvider {
    fn complete(
        &self,
        request: &AiCompletionRequest,
        cancellation: &AiCancellation,
        sink: &mut dyn AiEventSink,
    ) -> AiCompletionTerminal {
        if request.validate().is_err() {
            return self.rejected_request(request);
        }

        let deadline = Instant::now()
            .checked_add(Duration::from_millis(u64::from(
                request.parameters.timeout_ms,
            )))
            .unwrap_or_else(Instant::now);
        let mut response_bytes = 0usize;

        for (sequence, token) in self.script.tokens.iter().enumerate() {
            if cancellation.is_cancelled() {
                return AiCompletionTerminal::Cancelled;
            }
            if let Err(terminal) =
                self.wait_for_token(cancellation, self.script.token_delay, deadline)
            {
                return terminal;
            }
            if Instant::now() >= deadline {
                cancellation.cancel();
                return AiCompletionTerminal::Timeout;
            }

            response_bytes = response_bytes.saturating_add(token.len());
            if response_bytes > usize::try_from(request.parameters.max_output_bytes).unwrap_or(0)
                || response_bytes > MAX_AI_RESPONSE_BYTES
            {
                cancellation.cancel();
                return self.malformed_response(
                    request,
                    "provider response exceeded the configured output limit",
                    AiRecoveryAction::ReduceRequest,
                );
            }

            let Ok(sequence) = u32::try_from(sequence) else {
                cancellation.cancel();
                return self.malformed_response(
                    request,
                    "provider returned too many response deltas",
                    AiRecoveryAction::Retry,
                );
            };
            let delta = AiCompletionDelta {
                request_id: request.request_id.clone(),
                sequence,
                text: token.clone(),
            };
            if delta.validate().is_err() {
                cancellation.cancel();
                return self.malformed_response(
                    request,
                    "provider returned an invalid delta",
                    AiRecoveryAction::Retry,
                );
            }
            if sink.send_delta(delta).is_err() {
                cancellation.cancel();
                return AiCompletionTerminal::Cancelled;
            }
        }

        if cancellation.is_cancelled() {
            return AiCompletionTerminal::Cancelled;
        }

        match &self.script.outcome {
            FakeCompletionOutcome::Done { usage } => {
                if usage
                    .as_ref()
                    .is_some_and(|usage| usage.validate().is_err())
                {
                    cancellation.cancel();
                    return self.malformed_response(
                        request,
                        "provider returned invalid usage",
                        AiRecoveryAction::Retry,
                    );
                }
                AiCompletionTerminal::Done {
                    usage: usage.clone(),
                }
            }
            FakeCompletionOutcome::Timeout => {
                cancellation.cancel();
                AiCompletionTerminal::Timeout
            }
            FakeCompletionOutcome::MalformedOutput => {
                cancellation.cancel();
                self.malformed_response(
                    request,
                    "provider returned malformed output",
                    AiRecoveryAction::Retry,
                )
            }
            FakeCompletionOutcome::ProviderError(error) => {
                if error.validate().is_err() {
                    cancellation.cancel();
                    return self.malformed_response(
                        request,
                        "provider returned an invalid error",
                        AiRecoveryAction::Retry,
                    );
                }
                AiCompletionTerminal::ProviderError(error.clone())
            }
        }
    }
}
