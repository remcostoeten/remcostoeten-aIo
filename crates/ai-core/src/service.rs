//! The completion service: admission, worker lifecycle, terminal delivery.
//!
//! This is a characterization port of Skriuw `crates/skriuw-ai/src/lib.rs` at
//! revision `64827f5e`. It reproduces the source lifecycle exactly, including
//! the edge-case defects documented in `docs/contracts.md` §3.1, so that the
//! approved D1 hardening lands as a separate, visible change on top of a tested
//! baseline rather than being smuggled into the extraction.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    contracts::{
        AiCompletionDelta, AiCompletionEvent, AiCompletionRequest, AiCompletionTerminal,
        AiProviderError, AiProviderErrorCategory, AiRecoveryAction,
    },
    ports::{AiCancellation, AiComplete, AiCompletionChannel, AiEventSink, AiSinkError},
    recording::{
        AiModelPricing, AiRunRecorder, AiRunStatus, AiRunSummary, AiRunTokens, ai_run_cost_micros,
    },
};

/// Why a request was never admitted.
///
/// A start error means no run exists: no terminal is produced and nothing is
/// recorded. It is not a second terminal path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiStartError {
    /// A request with this id is already active.
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

/// Runs completions against registered providers and publishes their events.
///
/// The service is synchronous and thread-per-run. It owns admission,
/// cancellation bookkeeping, terminal publication and the single accounting
/// call; providers own streaming.
#[derive(Default)]
pub struct AiCompletionService {
    providers: BTreeMap<String, Arc<dyn AiComplete>>,
    active: Arc<Mutex<HashMap<String, AiCancellation>>>,
    recorder: Option<Arc<dyn AiRunRecorder>>,
    pricing: Option<Arc<dyn AiModelPricing>>,
}

impl AiCompletionService {
    /// Registers providers by identifier.
    ///
    /// Duplicate identifiers overwrite, matching the source. Rejecting them is
    /// a later hardening decision, not existing behavior.
    #[must_use]
    pub fn new(providers: impl IntoIterator<Item = (String, Arc<dyn AiComplete>)>) -> Self {
        Self {
            providers: providers.into_iter().collect(),
            active: Arc::new(Mutex::new(HashMap::new())),
            recorder: None,
            pricing: None,
        }
    }

    /// Attaches the single place a run is accounted for.
    ///
    /// The recorder is called once per terminalized request, after the terminal
    /// event has already been published, so accounting never delays delivery.
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
    /// `Ok(())`.
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
        let Some(provider) = self.providers.get(&request.provider_id).cloned() else {
            let terminal = AiCompletionTerminal::ProviderError(AiProviderError::new(
                request.provider_id.clone(),
                AiProviderErrorCategory::UnavailableProvider,
                "selected AI provider is unavailable",
                AiRecoveryAction::CheckProviderStatus,
            ));
            let _ = channel.send(terminal.clone().into_event(request.request_id.clone()));
            if let Some(recorder) = &self.recorder {
                let summary = run_summary(
                    &origin,
                    &request,
                    &terminal,
                    now_millis(),
                    Duration::ZERO,
                    0,
                    self.pricing.as_deref(),
                );
                recorder.record(summary, &request);
            }
            return Ok(());
        };

        let cancellation = AiCancellation::new();
        {
            let mut active = lock(&self.active);
            if active.contains_key(&request.request_id) {
                return Err(AiStartError::DuplicateRequest(request.request_id));
            }
            active.insert(request.request_id.clone(), cancellation.clone());
        }

        let active = Arc::clone(&self.active);
        let request_id = request.request_id.clone();
        let cleanup_request_id = request_id.clone();
        let recorder = self.recorder.clone();
        let pricing = self.pricing.clone();
        let worker = thread::Builder::new()
            .name(format!("ai-core-{request_id}"))
            .spawn(move || {
                let mut sink = CompletionServiceSink {
                    channel: Arc::clone(&channel),
                    cancellation: cancellation.clone(),
                    response_bytes: 0,
                };
                let started_at_ms = now_millis();
                let started = Instant::now();
                let mut terminal = provider.complete(&request, &cancellation, &mut sink);
                let elapsed = started.elapsed();
                let response_bytes = sink.response_bytes;
                let was_active = lock(&active).remove(&request_id).is_some();
                if !was_active {
                    return;
                }
                if cancellation.is_cancelled()
                    && matches!(terminal, AiCompletionTerminal::Done { .. })
                {
                    terminal = AiCompletionTerminal::Cancelled;
                }
                let _ = channel.send(terminal.clone().into_event(request_id));
                if let Some(recorder) = recorder {
                    let summary = run_summary(
                        &origin,
                        &request,
                        &terminal,
                        started_at_ms,
                        elapsed,
                        response_bytes,
                        pricing.as_deref(),
                    );
                    recorder.record(summary, &request);
                }
            });

        if worker.is_err() {
            lock(&self.active).remove(&cleanup_request_id);
            return Err(AiStartError::WorkerUnavailable);
        }
        Ok(())
    }

    /// Requests cancellation of an active run.
    ///
    /// Returns whether a run was found. The source removes a run from the
    /// registry before delivering its terminal, so this can return `false`
    /// while a terminal is still in flight.
    pub fn cancel(&self, request_id: &str) -> bool {
        let active = lock(&self.active);
        let Some(cancellation) = active.get(request_id) else {
            return false;
        };
        cancellation.cancel();
        true
    }

    /// Requests cancellation of every currently active run.
    ///
    /// This is cancel-current-runs only. It does not close admission, join
    /// workers, or wait for terminals; a request started immediately afterwards
    /// still runs. A permanent shutdown lifecycle is a separate change.
    pub fn shutdown(&self) {
        let active = lock(&self.active);
        for cancellation in active.values() {
            cancellation.cancel();
        }
    }
}

struct CompletionServiceSink {
    channel: Arc<dyn AiCompletionChannel>,
    cancellation: AiCancellation,
    response_bytes: usize,
}

impl AiEventSink for CompletionServiceSink {
    fn send_delta(&mut self, delta: AiCompletionDelta) -> Result<(), AiSinkError> {
        self.response_bytes = self.response_bytes.saturating_add(delta.text.len());
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

/// Builds the one summary a terminalized run produces.
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
