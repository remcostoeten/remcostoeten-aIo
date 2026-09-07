//! Shared test doubles.
//!
//! Everything here is barrier- or channel-controlled. No test in this crate may
//! synchronize by sleeping and hoping: a race that only reproduces on a slow
//! machine is worse than no test.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{Receiver, Sender, channel},
};
use std::time::{Duration, Instant};

use ai_core::{
    AiCancellation, AiComplete, AiCompletionChannel, AiCompletionDelta, AiCompletionEvent,
    AiCompletionParameters, AiCompletionRequest, AiCompletionTerminal, AiEventSink, AiModelPrice,
    AiModelPricing, AiRunRecorder, AiRunSummary, AiSinkError,
};

/// A request that validates, aimed at the `fake` provider.
pub fn request(request_id: &str) -> AiCompletionRequest {
    AiCompletionRequest {
        request_id: request_id.to_owned(),
        provider_id: "fake".into(),
        model_id: "model:small".into(),
        system_prompt: "system".into(),
        user_prompt: "user".into(),
        prior_messages: Vec::new(),
        parameters: AiCompletionParameters::default(),
    }
}

/// Collects every delivered event, and can be closed to simulate a gone
/// consumer.
#[derive(Clone, Default)]
pub struct RecordingChannel {
    events: Arc<Mutex<Vec<AiCompletionEvent>>>,
    closed: Arc<AtomicBool>,
}

impl RecordingChannel {
    pub fn events(&self) -> Vec<AiCompletionEvent> {
        self.events.lock().unwrap().clone()
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// Waits until a terminal event arrives, then returns everything delivered.
    ///
    /// Panics on timeout rather than returning a partial stream, so a hang
    /// fails loudly instead of silently asserting on nothing.
    pub fn wait_for_terminal(&self) -> Vec<AiCompletionEvent> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let events = self.events();
            if events
                .iter()
                .any(|event| !matches!(event, AiCompletionEvent::Delta(_)))
            {
                return events;
            }
            std::thread::yield_now();
        }
        panic!("no terminal event arrived within 5s: {:?}", self.events());
    }
}

impl AiCompletionChannel for RecordingChannel {
    fn send(&self, event: AiCompletionEvent) -> Result<(), AiSinkError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(AiSinkError::Closed);
        }
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

/// A channel that blocks inside the terminal send until the test releases it.
///
/// This is how the window between "the service stopped tracking the run" and
/// "the consumer received its terminal" is made observable without sleeping.
#[derive(Clone)]
pub struct GatedChannel {
    events: Arc<Mutex<Vec<AiCompletionEvent>>>,
    inside_terminal: Sender<()>,
    gate: Arc<Mutex<Receiver<()>>>,
}

/// The test-side handle to a [`GatedChannel`].
pub struct ChannelControl {
    inside_terminal: Receiver<()>,
    release: Sender<()>,
}

impl ChannelControl {
    /// Blocks until the service is inside the terminal send.
    pub fn await_terminal_send(&self) {
        self.inside_terminal
            .recv_timeout(Duration::from_secs(5))
            .expect("service should attempt a terminal send within 5s");
    }

    /// Lets the terminal send complete.
    pub fn release(&self) {
        let _ = self.release.send(());
    }
}

impl GatedChannel {
    pub fn new() -> (Self, ChannelControl) {
        let (inside_tx, inside_rx) = channel();
        let (release_tx, release_rx) = channel();
        let channel = Self {
            events: Arc::new(Mutex::new(Vec::new())),
            inside_terminal: inside_tx,
            gate: Arc::new(Mutex::new(release_rx)),
        };
        let control = ChannelControl {
            inside_terminal: inside_rx,
            release: release_tx,
        };
        (channel, control)
    }

    pub fn events(&self) -> Vec<AiCompletionEvent> {
        self.events.lock().unwrap().clone()
    }
}

impl AiCompletionChannel for GatedChannel {
    fn send(&self, event: AiCompletionEvent) -> Result<(), AiSinkError> {
        let terminal = !matches!(event, AiCompletionEvent::Delta(_));
        if terminal {
            let _ = self.inside_terminal.send(());
            let _ = self
                .gate
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5));
        }
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

/// Captures recorder calls, including a copy of the borrowed request, so a test
/// can assert the borrow really did carry the prompts.
#[derive(Clone, Default)]
pub struct RecordingRecorder {
    calls: Arc<Mutex<Vec<(AiRunSummary, AiCompletionRequest)>>>,
}

impl RecordingRecorder {
    pub fn calls(&self) -> Vec<(AiRunSummary, AiCompletionRequest)> {
        self.calls.lock().unwrap().clone()
    }

    pub fn wait_for_one(&self) -> (AiRunSummary, AiCompletionRequest) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(call) = self.calls().into_iter().next() {
                return call;
            }
            std::thread::yield_now();
        }
        panic!("recorder was not called within 5s");
    }
}

impl AiRunRecorder for RecordingRecorder {
    fn record(&self, summary: AiRunSummary, request: &AiCompletionRequest) {
        self.calls.lock().unwrap().push((summary, request.clone()));
    }
}

/// Prices exactly one provider/model pair.
pub struct FixedPricing {
    pub provider_id: String,
    pub model_id: String,
    pub price: AiModelPrice,
}

impl AiModelPricing for FixedPricing {
    fn price(&self, provider_id: &str, model_id: &str) -> Option<AiModelPrice> {
        (provider_id == self.provider_id && model_id == self.model_id).then_some(self.price)
    }
}

/// Prices nothing, so cost is always absent.
pub struct NoPricing;

impl AiModelPricing for NoPricing {
    fn price(&self, _provider_id: &str, _model_id: &str) -> Option<AiModelPrice> {
        None
    }
}

/// What a [`ScriptedProvider`] should do at each step.
pub enum Step {
    /// Emit a delta with the given sequence and text.
    Delta(u32, String),
    /// Emit a delta carrying a foreign request id.
    ForeignDelta(String),
    /// Emit a delta far larger than the delta bound.
    OversizedDelta,
    /// Block until the test releases it.
    Await,
    /// Signal the test that this point was reached.
    Signal,
    /// Panic, unwinding out of the provider.
    Panic,
}

/// A provider driven step by step under the test's control.
///
/// `entered` fires when `complete` begins; `Await` blocks until `release` is
/// called. That makes cancel-versus-commit races deterministic.
pub struct ScriptedProvider {
    steps: Mutex<Vec<Step>>,
    terminal: Mutex<Option<AiCompletionTerminal>>,
    entered: Sender<()>,
    signals: Sender<()>,
    gate: Mutex<Receiver<()>>,
    calls: AtomicUsize,
}

/// The test-side handle to a [`ScriptedProvider`].
pub struct ProviderControl {
    pub entered: Receiver<()>,
    pub signals: Receiver<()>,
    release: Sender<()>,
}

impl ProviderControl {
    /// Blocks until the provider has entered `complete`.
    pub fn await_entry(&self) {
        self.entered
            .recv_timeout(Duration::from_secs(5))
            .expect("provider should enter complete within 5s");
    }

    /// Blocks until the provider reaches its next `Signal` step.
    pub fn await_signal(&self) {
        self.signals
            .recv_timeout(Duration::from_secs(5))
            .expect("provider should signal within 5s");
    }

    /// Unblocks the provider's next `Await` step.
    pub fn release(&self) {
        let _ = self.release.send(());
    }
}

impl ScriptedProvider {
    pub fn new(steps: Vec<Step>, terminal: AiCompletionTerminal) -> (Arc<Self>, ProviderControl) {
        let (entered_tx, entered_rx) = channel();
        let (signal_tx, signal_rx) = channel();
        let (release_tx, release_rx) = channel();
        let provider = Arc::new(Self {
            steps: Mutex::new(steps),
            terminal: Mutex::new(Some(terminal)),
            entered: entered_tx,
            signals: signal_tx,
            gate: Mutex::new(release_rx),
            calls: AtomicUsize::new(0),
        });
        let control = ProviderControl {
            entered: entered_rx,
            signals: signal_rx,
            release: release_tx,
        };
        (provider, control)
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl AiComplete for ScriptedProvider {
    fn complete(
        &self,
        request: &AiCompletionRequest,
        cancellation: &AiCancellation,
        sink: &mut dyn AiEventSink,
    ) -> AiCompletionTerminal {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _ = self.entered.send(());
        let steps = std::mem::take(&mut *self.steps.lock().unwrap());

        for step in steps {
            match step {
                Step::Delta(sequence, text) => {
                    let delta = AiCompletionDelta {
                        request_id: request.request_id.clone(),
                        sequence,
                        text,
                    };
                    if sink.send_delta(delta).is_err() {
                        cancellation.cancel();
                        return AiCompletionTerminal::Cancelled;
                    }
                }
                Step::ForeignDelta(request_id) => {
                    let delta = AiCompletionDelta {
                        request_id,
                        sequence: 0,
                        text: "leaked".into(),
                    };
                    if sink.send_delta(delta).is_err() {
                        cancellation.cancel();
                        return AiCompletionTerminal::Cancelled;
                    }
                }
                Step::OversizedDelta => {
                    let delta = AiCompletionDelta {
                        request_id: request.request_id.clone(),
                        sequence: 0,
                        text: "x".repeat(ai_core::MAX_AI_DELTA_BYTES + 1),
                    };
                    if sink.send_delta(delta).is_err() {
                        cancellation.cancel();
                        return AiCompletionTerminal::Cancelled;
                    }
                }
                Step::Await => {
                    self.gate
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .expect("test should release the provider within 5s");
                }
                Step::Signal => {
                    let _ = self.signals.send(());
                }
                Step::Panic => panic!("scripted provider panic"),
            }
        }

        self.terminal
            .lock()
            .unwrap()
            .take()
            .unwrap_or(AiCompletionTerminal::Cancelled)
    }
}
