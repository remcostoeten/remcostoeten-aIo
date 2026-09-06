//! The completion service: admission, terminal commitment, delivery.
//!
//! Extracted from Skriuw `crates/skriuw-ai/src/lib.rs` at revision `64827f5e`,
//! then hardened per decision D1 (`docs/architecture.md` §5, specified in
//! `docs/contracts.md` §3.3). Normal completion behavior and the serialized
//! contracts are unchanged; the differences are all at the edges:
//!
//! - ids are reserved before provider lookup, so duplicates are rejected on
//!   every path rather than only the registered-provider one;
//! - a terminal is committed exactly once under the registry lock, and the id
//!   stays reserved through the delivery attempt, so it cannot be reused while
//!   an older terminal is still in flight;
//! - provider output is validated for identity, sequence and bounds before it
//!   reaches the consumer;
//! - a provider that unwinds is cleaned up and reported as `internal_failure`
//!   instead of stranding the run.
//!
//! What none of that buys is listed in `docs/contracts.md` §3.3: no delivery
//! guarantee to a closed channel, no interruption of a provider that blocks
//! forever, no survival of process abort, and no whole-request deadline.

use std::{
    collections::{BTreeMap, HashMap},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    contracts::{
        AiCompletionDelta, AiCompletionEvent, AiCompletionRequest, AiCompletionTerminal,
        AiProviderError, AiProviderErrorCategory, AiRecoveryAction, MAX_AI_RESPONSE_BYTES,
    },
    ports::{AiCancellation, AiComplete, AiCompletionChannel, AiEventSink, AiSinkError},
    recording::{
        AiModelPricing, AiRunRecorder, AiRunStatus, AiRunSummary, AiRunTokens, ai_run_cost_micros,
    },
};

/// Why a request was never admitted.
///
/// A start error means no run exists: no terminal is produced and nothing is
/// recorded. It is never a second terminal path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiStartError {
    /// A request with this id is already active, on any path.
    DuplicateRequest(String),
    /// The request failed [`AiCompletionRequest::validate`].
    InvalidRequest,
    /// A worker thread could not be spawned.
    WorkerUnavailable,
}

impl std::fmt::Display for AiStartError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateRequest(request_id) => {
                write!(formatter, "AI request {request_id} is already active")
            }
            Self::InvalidRequest => formatter.write_str("AI completion request is invalid"),
            Self::WorkerUnavailable => formatter.write_str("AI completion worker is unavailable"),
        }
    }
}

impl std::error::Error for AiStartError {}

/// Distinguishes one reservation of a request id from the next.
///
/// Request ids are caller-chosen and reusable, so cleanup keyed only on the id
/// could remove a newer run's slot. Every slot carries a monotonic run number
/// and every removal checks it.
type RunId = u64;

struct RunSlot {
    run: RunId,
    cancellation: AiCancellation,
    committed: bool,
}

/// Runs completions against registered providers and publishes their events.
///
/// Synchronous and thread-per-run. The service owns admission, cancellation
/// bookkeeping, terminal commitment and the single accounting call; providers
/// own streaming.
#[derive(Default)]
pub struct AiCompletionService {
    providers: BTreeMap<String, Arc<dyn AiComplete>>,
    runs: Arc<Mutex<HashMap<String, RunSlot>>>,
    next_run: Arc<AtomicU64>,
    recorder: Option<Arc<dyn AiRunRecorder>>,
    pricing: Option<Arc<dyn AiModelPricing>>,
}

impl AiCompletionService {
    /// Registers providers by identifier.
    ///
    /// Duplicate identifiers overwrite, matching the source. Rejecting them is
    /// a separate hardening decision that has not been taken.
    #[must_use]
    pub fn new(providers: impl IntoIterator<Item = (String, Arc<dyn AiComplete>)>) -> Self {
        Self {
            providers: providers.into_iter().collect(),
            runs: Arc::new(Mutex::new(HashMap::new())),
            next_run: Arc::new(AtomicU64::new(0)),
            recorder: None,
            pricing: None,
        }
    }

    /// Attaches the single place a run is accounted for.
    ///
    /// The recorder is called once per committed run, after the terminal event
    /// has already been published and with no service lock held, so accounting
    /// never delays or blocks delivery.
    #[must_use]
    pub fn recording(
        mut self,
        recorder: Arc<dyn AiRunRecorder>,
        pricing: Arc<dyn AiModelPricing>,
    ) -> Self {
        self.recorder = Some(recorder);
        self.pricing = Some(pricing);
        self
    }

    /// Admits a request and runs it on its own worker thread.
    ///
    /// # Errors
    ///
    /// See [`AiStartError`]. An unregistered provider is not a start error: it
    /// produces a synchronous `unavailable_provider` terminal and returns
    /// `Ok(())`, preserving the source's contract with its callers.
    pub fn start(
        &self,
        origin: String,
        request: AiCompletionRequest,
        channel: impl AiCompletionChannel,
    ) -> Result<(), AiStartError> {
        let channel: Arc<dyn AiCompletionChannel> = Arc::new(channel);
        if request.validate().is_err() {
            return Err(AiStartError::InvalidRequest);
        }

        let cancellation = AiCancellation::new();
        let Some(run) = self.reserve(&request.request_id, cancellation.clone()) else {
            return Err(AiStartError::DuplicateRequest(request.request_id));
        };

        let Some(provider) = self.providers.get(&request.provider_id).cloned() else {
            let terminal = AiCompletionTerminal::ProviderError(AiProviderError::new(
                request.provider_id.clone(),
                AiProviderErrorCategory::UnavailableProvider,
                "selected AI provider is unavailable",
                AiRecoveryAction::CheckProviderStatus,
            ));
            self.finish(
                &request.request_id,
                run,
                terminal,
                &channel,
                &origin,
                &request,
                now_millis(),
                Duration::ZERO,
                0,
            );
            return Ok(());
        };

        let runs = Arc::clone(&self.runs);
        let recorder = self.recorder.clone();
        let pricing = self.pricing.clone();
        let request_id = request.request_id.clone();
        let cleanup_request_id = request_id.clone();
        let worker = thread::Builder::new()
            .name(format!("ai-core-{request_id}"))
            .spawn(move || {
                let mut sink = ValidatingSink::new(&request, Arc::clone(&channel), &cancellation);
                let started_at_ms = now_millis();
                let started = Instant::now();
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    provider.complete(&request, &cancellation, &mut sink)
                }));
                let elapsed = started.elapsed();
                let response_bytes = sink.response_bytes;

                let terminal = match outcome {
                    // The payload is deliberately dropped: a panic message can
                    // contain prompt text or credentials, and this error is
                    // published to the consumer.
                    Err(_) => AiCompletionTerminal::ProviderError(AiProviderError::new(
                        request.provider_id.clone(),
                        AiProviderErrorCategory::InternalFailure,
                        "provider panicked before returning a terminal",
                        AiRecoveryAction::Retry,
                    )),
                    Ok(terminal) => match sink.violation {
                        Some(violation) => AiCompletionTerminal::ProviderError(violation),
                        None => validated_terminal(&request, terminal),
                    },
                };

                finish_run(
                    &runs,
                    &request_id,
                    run,
                    terminal,
                    &channel,
                    recorder.as_deref(),
                    pricing.as_deref(),
                    &origin,
                    &request,
                    started_at_ms,
                    elapsed,
                    response_bytes,
                );
            });

        if worker.is_err() {
            self.release(&cleanup_request_id, run);
            return Err(AiStartError::WorkerUnavailable);
        }
        Ok(())
    }

    /// Requests cancellation of an active run.
    ///
    /// Returns whether cancellation was accepted. A run whose terminal is
    /// already committed returns `false`: its outcome is decided, even though
    /// the id stays reserved until delivery has been attempted.
    pub fn cancel(&self, request_id: &str) -> bool {
        let runs = lock(&self.runs);
        let Some(slot) = runs.get(request_id) else {
            return false;
        };
        if slot.committed {
            return false;
        }
        slot.cancellation.cancel();
        true
    }

    /// Requests cancellation of every run that has not yet committed a terminal.
    ///
    /// This is cancel-current-runs only. It does not close admission, join
    /// workers, or wait for terminals; a request started immediately afterwards
    /// still runs. A permanent shutdown lifecycle is a separate change.
    pub fn shutdown(&self) {
        let runs = lock(&self.runs);
        for slot in runs.values() {
            if !slot.committed {
                slot.cancellation.cancel();
            }
        }
    }

    fn reserve(&self, request_id: &str, cancellation: AiCancellation) -> Option<RunId> {
        let mut runs = lock(&self.runs);
        if runs.contains_key(request_id) {
            return None;
        }
        let run = self.next_run.fetch_add(1, Ordering::SeqCst);
        runs.insert(
            request_id.to_owned(),
            RunSlot {
                run,
                cancellation,
                committed: false,
            },
        );
        Some(run)
    }

    fn release(&self, request_id: &str, run: RunId) {
        release_run(&self.runs, request_id, run);
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the synchronous unknown-provider path needs the same accounting inputs as a worker"
    )]
    fn finish(
        &self,
        request_id: &str,
        run: RunId,
        terminal: AiCompletionTerminal,
        channel: &Arc<dyn AiCompletionChannel>,
        origin: &str,
        request: &AiCompletionRequest,
        started_at_ms: i64,
        elapsed: Duration,
        response_bytes: usize,
    ) {
        finish_run(
            &self.runs,
            request_id,
            run,
            terminal,
            channel,
            self.recorder.as_deref(),
            self.pricing.as_deref(),
            origin,
            request,
            started_at_ms,
            elapsed,
            response_bytes,
        );
    }
}

/// Commits, delivers, releases, and records — in that order.
///
/// The lock is held only for the commitment decision. Delivery and accounting
/// run without it, so a channel or recorder that calls back into the service
/// cannot deadlock.
#[expect(
    clippy::too_many_arguments,
    reason = "one funnel for both terminal paths beats duplicating the ordering rules"
)]
fn finish_run(
    runs: &Mutex<HashMap<String, RunSlot>>,
    request_id: &str,
    run: RunId,
    terminal: AiCompletionTerminal,
    channel: &Arc<dyn AiCompletionChannel>,
    recorder: Option<&dyn AiRunRecorder>,
    pricing: Option<&dyn AiModelPricing>,
    origin: &str,
    request: &AiCompletionRequest,
    started_at_ms: i64,
    elapsed: Duration,
    response_bytes: usize,
) {
    let Some(terminal) = commit_terminal(runs, request_id, run, terminal) else {
        return;
    };

    let _ = channel.send(terminal.clone().into_event(request_id.to_owned()));
    release_run(runs, request_id, run);

    if let Some(recorder) = recorder {
        let summary = run_summary(
            origin,
            request,
            &terminal,
            started_at_ms,
            elapsed,
            response_bytes,
            pricing,
        );
        recorder.record(summary, request);
    }
}

/// Decides a run's one terminal, under the registry lock.
///
/// Returns `None` when this run no longer owns the slot or has already
/// committed, which is what makes "at most one terminal" true rather than
/// merely likely. Cancellation observed before commitment turns `Done` into
/// `Cancelled` and nothing else: a timeout or a provider error is a real
/// outcome that a cancellation flag must not overwrite.
fn commit_terminal(
    runs: &Mutex<HashMap<String, RunSlot>>,
    request_id: &str,
    run: RunId,
    terminal: AiCompletionTerminal,
) -> Option<AiCompletionTerminal> {
    let mut runs = lock(runs);
    let slot = runs.get_mut(request_id)?;
    if slot.run != run || slot.committed {
        return None;
    }
    slot.committed = true;

    if slot.cancellation.is_cancelled() && matches!(terminal, AiCompletionTerminal::Done { .. }) {
        return Some(AiCompletionTerminal::Cancelled);
    }
    Some(terminal)
}

fn release_run(runs: &Mutex<HashMap<String, RunSlot>>, request_id: &str, run: RunId) {
    let mut runs = lock(runs);
    if runs.get(request_id).is_some_and(|slot| slot.run == run) {
        runs.remove(request_id);
    }
}

/// Rejects a terminal whose payload breaks the contract.
///
/// A provider that reports impossible usage, or an error that fails its own
/// bounds, has malfunctioned. Publishing that unchecked would let a provider
/// put a value on the wire that the consumer's decoder rejects.
fn validated_terminal(
    request: &AiCompletionRequest,
    terminal: AiCompletionTerminal,
) -> AiCompletionTerminal {
    let invalid = match &terminal {
        AiCompletionTerminal::Done { usage: Some(usage) } => usage
            .validate()
            .err()
            .map(|_| "provider reported invalid usage"),
        AiCompletionTerminal::ProviderError(error) => error
            .validate()
            .err()
            .map(|_| "provider returned an invalid error"),
        _ => None,
    };

    match invalid {
        None => terminal,
        Some(message) => AiCompletionTerminal::ProviderError(AiProviderError::new(
            request.provider_id.clone(),
            AiProviderErrorCategory::MalformedResponse,
            message,
            AiRecoveryAction::Retry,
        )),
    }
}

/// Enforces the stream contract between a provider and the consumer.
///
/// Identity, gapless sequence, per-delta size and cumulative size are checked
/// here rather than trusted, because a consumer filtering foreign or
/// out-of-order events proves nothing about what the producer emitted. The
/// first violation cancels the run and is remembered, so the service commits a
/// `malformed_response` terminal rather than the `Cancelled` the provider is
/// likely to return once its sends start failing.
struct ValidatingSink {
    channel: Arc<dyn AiCompletionChannel>,
    cancellation: AiCancellation,
    request_id: String,
    provider_id: String,
    max_output_bytes: usize,
    next_sequence: u32,
    response_bytes: usize,
    violation: Option<AiProviderError>,
}

impl ValidatingSink {
    fn new(
        request: &AiCompletionRequest,
        channel: Arc<dyn AiCompletionChannel>,
        cancellation: &AiCancellation,
    ) -> Self {
        Self {
            channel,
            cancellation: cancellation.clone(),
            request_id: request.request_id.clone(),
            provider_id: request.provider_id.clone(),
            max_output_bytes: usize::try_from(request.parameters.max_output_bytes)
                .unwrap_or(MAX_AI_RESPONSE_BYTES)
                .min(MAX_AI_RESPONSE_BYTES),
            next_sequence: 0,
            response_bytes: 0,
            violation: None,
        }
    }

    fn reject(&mut self, message: &str) -> Result<(), AiSinkError> {
        if self.violation.is_none() {
            self.violation = Some(AiProviderError::new(
                self.provider_id.clone(),
                AiProviderErrorCategory::MalformedResponse,
                message,
                AiRecoveryAction::Retry,
            ));
        }
        self.cancellation.cancel();
        Err(AiSinkError::Closed)
    }
}

impl AiEventSink for ValidatingSink {
    fn send_delta(&mut self, delta: AiCompletionDelta) -> Result<(), AiSinkError> {
        if self.violation.is_some() {
            return Err(AiSinkError::Closed);
        }
        if delta.request_id != self.request_id {
            return self.reject("provider emitted a delta for another request");
        }
        if delta.sequence != self.next_sequence {
            return self.reject("provider emitted an out-of-order delta");
        }
        if delta.validate().is_err() {
            return self.reject("provider emitted an invalid delta");
        }
        let total = self.response_bytes.saturating_add(delta.text.len());
        if total > self.max_output_bytes {
            return self.reject("provider response exceeded the configured output limit");
        }

        self.next_sequence = self.next_sequence.saturating_add(1);
        self.response_bytes = total;
        self.channel
            .send(AiCompletionEvent::Delta(delta))
            .inspect_err(|_| self.cancellation.cancel())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Builds the one summary a committed run produces.
///
/// Providers that report their own token counts win; everything else is derived
/// from transferred bytes and carried as an estimate, including every cancelled,
/// timed-out and failed run.
fn run_summary(
    origin: &str,
    request: &AiCompletionRequest,
    terminal: &AiCompletionTerminal,
    started_at_ms: i64,
    elapsed: Duration,
    response_bytes: usize,
    pricing: Option<&dyn AiModelPricing>,
) -> AiRunSummary {
    let status = match terminal {
        AiCompletionTerminal::Done { .. } => AiRunStatus::Done,
        AiCompletionTerminal::Cancelled => AiRunStatus::Cancelled,
        AiCompletionTerminal::Timeout => AiRunStatus::Timeout,
        AiCompletionTerminal::ProviderError(error) => AiRunStatus::ProviderError {
            category: error.category,
        },
    };
    let prompt_bytes = request
        .system_prompt
        .len()
        .saturating_add(request.user_prompt.len());
    let tokens = match terminal {
        AiCompletionTerminal::Done { usage: Some(usage) } => AiRunTokens::reported(usage),
        _ => AiRunTokens::estimated(prompt_bytes, response_bytes),
    };
    let cost_micros = pricing
        .and_then(|pricing| pricing.price(&request.provider_id, &request.model_id))
        .map(|price| ai_run_cost_micros(&tokens, price));

    AiRunSummary {
        run_id: request.request_id.clone(),
        started_at_ms,
        origin: origin.to_owned(),
        provider_id: request.provider_id.clone(),
        model_id: request.model_id.clone(),
        status,
        duration_ms: u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX),
        tokens,
        cost_micros,
    }
}
