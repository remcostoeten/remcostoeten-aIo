//! Decision D2's compatibility proof.
//!
//! The SDK hands out a metadata-only [`AiRunSummary`] plus the request borrowed
//! for the callback. This harness models Skriuw's side of that seam — its
//! `AiRunRecord`, `AiRunPrompts`, `AiRunState` and its storage-time retention
//! switch — and shows the record can be rebuilt in full, with prompts retained
//! or redacted, without the SDK owning a prompt field or a retention setting.
//!
//! The types below are a local model of the consumer, not an extraction target.
//! Nothing here may migrate into `ai-core`, and no test in this file touches
//! the Skriuw checkout: proving compatibility must not require editing the
//! application. Actual Skriuw integration is Phase 3.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::{Arc, Mutex};

use ai_core::{
    AiComplete, AiCompletionRequest, AiCompletionService, AiCompletionTerminal, AiModelPrice,
    AiProviderError, AiProviderErrorCategory, AiRecoveryAction, AiRunRecorder, AiRunStatus,
    AiRunSummary, AiRunTokens, AiTokenSource, AiUsage, FakeAiProvider, FakeCompletionOutcome,
    FakeCompletionScript,
};
use support::{FixedPricing, NoPricing, RecordingChannel, request};

// --- the consumer's existing history contract, modelled locally ---------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AiRunState {
    Done,
    Cancelled,
    TimedOut,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AiRunPrompts {
    system_prompt: String,
    user_prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AiRunRecord {
    run_id: String,
    started_at_ms: i64,
    origin: String,
    provider_id: String,
    model_id: String,
    prompts: Option<AiRunPrompts>,
    state: AiRunState,
    error_category: Option<AiProviderErrorCategory>,
    duration_ms: u32,
    tokens: AiRunTokens,
    cost_micros: Option<u64>,
}

/// The application's storage-time retention switch, unchanged and still its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HistorySettings {
    retain_prompts: bool,
}

/// The whole of the adapter Skriuw needs in order to consume the SDK's port.
///
/// It copies the prompts out of the borrowed request while the borrow is live,
/// maps the discriminated status onto the legacy state/category pair, and then
/// applies its own retention setting exactly where it applies it today.
struct HistoryRecorderAdapter {
    settings: HistorySettings,
    stored: Arc<Mutex<Vec<AiRunRecord>>>,
}

impl AiRunRecorder for HistoryRecorderAdapter {
    fn record(&self, summary: AiRunSummary, request: &AiCompletionRequest) {
        let prompts = self.settings.retain_prompts.then(|| AiRunPrompts {
            system_prompt: request.system_prompt.clone(),
            user_prompt: request.user_prompt.clone(),
        });
        let (state, error_category) = match summary.status {
            AiRunStatus::Done => (AiRunState::Done, None),
            AiRunStatus::Cancelled => (AiRunState::Cancelled, None),
            AiRunStatus::Timeout => (AiRunState::TimedOut, None),
            AiRunStatus::ProviderError { category } => (AiRunState::Failed, Some(category)),
        };

        self.stored.lock().unwrap().push(AiRunRecord {
            run_id: summary.run_id,
            started_at_ms: summary.started_at_ms,
            origin: summary.origin,
            provider_id: summary.provider_id,
            model_id: summary.model_id,
            prompts,
            state,
            error_category,
            duration_ms: summary.duration_ms,
            tokens: summary.tokens,
            cost_micros: summary.cost_micros,
        });
    }
}

// --- harness -----------------------------------------------------------------

struct Harness {
    stored: Arc<Mutex<Vec<AiRunRecord>>>,
}

impl Harness {
    fn records(&self) -> Vec<AiRunRecord> {
        self.stored.lock().unwrap().clone()
    }

    fn wait_for_one(&self) -> AiRunRecord {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Some(record) = self.records().into_iter().next() {
                return record;
            }
            std::thread::yield_now();
        }
        panic!("no history record was written within 5s");
    }
}

fn run(
    script: FakeCompletionScript,
    retain_prompts: bool,
    pricing: Option<AiModelPrice>,
) -> Harness {
    run_with(request("request-1"), script, retain_prompts, pricing)
}

fn run_with(
    request: AiCompletionRequest,
    script: FakeCompletionScript,
    retain_prompts: bool,
    price: Option<AiModelPrice>,
) -> Harness {
    let stored = Arc::new(Mutex::new(Vec::new()));
    let adapter = Arc::new(HistoryRecorderAdapter {
        settings: HistorySettings { retain_prompts },
        stored: Arc::clone(&stored),
    });
    let provider: Arc<dyn AiComplete> = Arc::new(FakeAiProvider::new(script));
    let service = match price {
        Some(price) => AiCompletionService::new([("fake".to_owned(), provider)]).recording(
            adapter,
            Arc::new(FixedPricing {
                provider_id: "fake".into(),
                model_id: "model:small".into(),
                price,
            }),
        ),
        None => AiCompletionService::new([("fake".to_owned(), provider)])
            .recording(adapter, Arc::new(NoPricing)),
    };

    service
        .start("playground".into(), request, RecordingChannel::default())
        .expect("start");

    Harness { stored }
}

// --- tests -------------------------------------------------------------------

#[test]
fn reconstructs_a_full_history_record_when_prompts_are_retained() {
    let mut script = FakeCompletionScript::success(["hello"]);
    script.outcome = FakeCompletionOutcome::Done {
        usage: Some(AiUsage {
            input_tokens: 1_000_000,
            output_tokens: 500_000,
        }),
    };
    let harness = run(
        script,
        true,
        Some(AiModelPrice {
            input_price_micros_per_mtok: 150_000,
            output_price_micros_per_mtok: 600_000,
        }),
    );

    let record = harness.wait_for_one();
    assert_eq!(record.run_id, "request-1");
    assert_eq!(record.origin, "playground");
    assert_eq!(record.provider_id, "fake");
    assert_eq!(record.model_id, "model:small");
    assert_eq!(record.state, AiRunState::Done);
    assert_eq!(record.error_category, None);
    assert_eq!(
        record.prompts,
        Some(AiRunPrompts {
            system_prompt: "system".into(),
            user_prompt: "user".into(),
        }),
        "the borrowed request carries everything the summary deliberately omits"
    );
    assert_eq!(record.tokens.source, AiTokenSource::Provider);
    assert_eq!(record.cost_micros, Some(450_000));
    assert!(record.started_at_ms > 0);
}

#[test]
fn reconstructs_a_metadata_only_record_when_retention_is_off() {
    let harness = run(FakeCompletionScript::success(["hello"]), false, None);

    let record = harness.wait_for_one();
    assert_eq!(
        record.prompts, None,
        "retention stays a storage-time application decision"
    );
    assert_eq!(record.state, AiRunState::Done);
    assert_eq!(record.tokens.source, AiTokenSource::Estimated);
    assert_eq!(record.tokens.input_tokens, 3);
    assert_eq!(record.tokens.output_tokens, 2);
    assert_eq!(record.cost_micros, None);
}

#[test]
fn maps_a_timeout_to_timed_out_without_an_error_category() {
    let mut script = FakeCompletionScript::success(["hello"]);
    script.outcome = FakeCompletionOutcome::Timeout;
    let harness = run(script, true, None);

    let record = harness.wait_for_one();
    assert_eq!(record.state, AiRunState::TimedOut);
    assert_eq!(record.error_category, None);
    assert_eq!(record.tokens.source, AiTokenSource::Estimated);
}

#[test]
fn maps_a_provider_error_to_failed_with_its_category() {
    let mut script = FakeCompletionScript::success(["hello"]);
    script.outcome = FakeCompletionOutcome::ProviderError(AiProviderError::new(
        "fake",
        AiProviderErrorCategory::RateLimited,
        "slow down",
        AiRecoveryAction::Retry,
    ));
    let harness = run(script, true, None);

    let record = harness.wait_for_one();
    assert_eq!(record.state, AiRunState::Failed);
    assert_eq!(
        record.error_category,
        Some(AiProviderErrorCategory::RateLimited),
        "only a failed run carries a category, and it always carries one"
    );
}

#[test]
fn maps_a_cancelled_run_to_cancelled_without_an_error_category() {
    // A closed consumer cancels the run, which is the cheapest deterministic
    // route to a Cancelled terminal.
    let stored = Arc::new(Mutex::new(Vec::new()));
    let adapter = Arc::new(HistoryRecorderAdapter {
        settings: HistorySettings {
            retain_prompts: true,
        },
        stored: Arc::clone(&stored),
    });
    let provider: Arc<dyn AiComplete> =
        Arc::new(FakeAiProvider::new(FakeCompletionScript::success([
            "a", "b",
        ])));
    let service = AiCompletionService::new([("fake".to_owned(), provider)])
        .recording(adapter, Arc::new(NoPricing));
    let channel = RecordingChannel::default();
    channel.close();

    service
        .start("playground".into(), request("request-1"), channel)
        .expect("start");

    let record = Harness { stored }.wait_for_one();
    assert_eq!(record.state, AiRunState::Cancelled);
    assert_eq!(record.error_category, None);
}

#[test]
fn records_an_unknown_provider_run_synchronously() {
    let stored = Arc::new(Mutex::new(Vec::new()));
    let adapter = Arc::new(HistoryRecorderAdapter {
        settings: HistorySettings {
            retain_prompts: true,
        },
        stored: Arc::clone(&stored),
    });
    let provider: Arc<dyn AiComplete> =
        Arc::new(FakeAiProvider::new(FakeCompletionScript::success(["x"])));
    let service = AiCompletionService::new([("fake".to_owned(), provider)])
        .recording(adapter, Arc::new(NoPricing));
    let mut unknown = request("request-1");
    unknown.provider_id = "ghost".into();

    service
        .start("playground".into(), unknown, RecordingChannel::default())
        .expect("start");

    let records = stored.lock().unwrap().clone();
    assert_eq!(
        records.len(),
        1,
        "the unknown-provider path records before start returns, on the caller's thread"
    );
    assert_eq!(records[0].state, AiRunState::Failed);
    assert_eq!(
        records[0].error_category,
        Some(AiProviderErrorCategory::UnavailableProvider)
    );
    assert_eq!(records[0].duration_ms, 0);
}

#[test]
fn a_failed_start_writes_no_history_at_all() {
    let stored = Arc::new(Mutex::new(Vec::new()));
    let adapter = Arc::new(HistoryRecorderAdapter {
        settings: HistorySettings {
            retain_prompts: true,
        },
        stored: Arc::clone(&stored),
    });
    let provider: Arc<dyn AiComplete> =
        Arc::new(FakeAiProvider::new(FakeCompletionScript::success(["x"])));
    let service = AiCompletionService::new([("fake".to_owned(), provider)])
        .recording(adapter, Arc::new(NoPricing));
    let mut invalid = request("request-1");
    invalid.parameters.timeout_ms = 0;

    assert!(
        service
            .start("playground".into(), invalid, RecordingChannel::default())
            .is_err()
    );
    assert!(
        stored.lock().unwrap().is_empty(),
        "a rejected start is not a run, so it has no history"
    );
}

#[test]
fn an_empty_prompt_is_retained_as_empty_not_as_absent() {
    let mut empty = request("request-1");
    empty.system_prompt = String::new();
    empty.user_prompt = String::new();
    let harness = run_with(empty, FakeCompletionScript::success(["hello"]), true, None);

    let record = harness.wait_for_one();
    assert_eq!(
        record.prompts,
        Some(AiRunPrompts {
            system_prompt: String::new(),
            user_prompt: String::new(),
        }),
        "absent prompts mean retention was off, never that the prompt was empty"
    );
}

#[test]
fn the_summary_carries_no_prompt_of_its_own() {
    // A structural guard, not a behavioral one: if a prompt field is ever added
    // to the summary, this stops compiling and the reviewer has to justify it.
    fn assert_fields(summary: AiRunSummary) {
        let AiRunSummary {
            run_id: _,
            started_at_ms: _,
            origin: _,
            provider_id: _,
            model_id: _,
            status: _,
            duration_ms: _,
            tokens: _,
            cost_micros: _,
        } = summary;
    }

    assert_fields(AiRunSummary {
        run_id: "request-1".into(),
        started_at_ms: 0,
        origin: "playground".into(),
        provider_id: "fake".into(),
        model_id: "model".into(),
        status: AiRunStatus::Done,
        duration_ms: 0,
        tokens: AiRunTokens::estimated(0, 0),
        cost_micros: None,
    });
}

#[test]
fn the_terminal_is_delivered_before_the_recorder_runs() {
    // Accounting must never sit in front of delivery. The adapter blocks; the
    // consumer must already hold its terminal by then.
    struct SlowRecorder {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl AiRunRecorder for SlowRecorder {
        fn record(&self, _summary: AiRunSummary, _request: &AiCompletionRequest) {
            self.events.lock().unwrap().push("recorded".into());
        }
    }

    let events = Arc::new(Mutex::new(Vec::new()));
    let channel = RecordingChannel::default();
    let provider: Arc<dyn AiComplete> =
        Arc::new(FakeAiProvider::new(FakeCompletionScript::success(["hi"])));
    let service = AiCompletionService::new([("fake".to_owned(), provider)]).recording(
        Arc::new(SlowRecorder {
            events: Arc::clone(&events),
        }),
        Arc::new(NoPricing),
    );

    service
        .start("playground".into(), request("request-1"), channel.clone())
        .expect("start");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while events.lock().unwrap().is_empty() {
        assert!(std::time::Instant::now() < deadline, "recorder never ran");
        std::thread::yield_now();
    }
    assert!(
        matches!(
            channel.events().last(),
            Some(ai_core::AiCompletionEvent::Done { .. })
        ),
        "the terminal is already delivered by the time the recorder is called"
    );
}

#[test]
fn a_recorder_that_calls_back_into_the_service_does_not_deadlock() {
    // The registry lock must not be held across the recorder call. If it were,
    // this test would hang rather than fail.
    struct ReentrantRecorder {
        service: Mutex<Option<Arc<AiCompletionService>>>,
        observed: Arc<Mutex<Vec<bool>>>,
    }

    impl AiRunRecorder for ReentrantRecorder {
        fn record(&self, summary: AiRunSummary, _request: &AiCompletionRequest) {
            let service = self.service.lock().unwrap();
            if let Some(service) = service.as_ref() {
                self.observed
                    .lock()
                    .unwrap()
                    .push(service.cancel(&summary.run_id));
            }
        }
    }

    let observed = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::new(ReentrantRecorder {
        service: Mutex::new(None),
        observed: Arc::clone(&observed),
    });
    let provider: Arc<dyn AiComplete> =
        Arc::new(FakeAiProvider::new(FakeCompletionScript::success(["hi"])));
    let service = Arc::new(
        AiCompletionService::new([("fake".to_owned(), provider)])
            .recording(recorder.clone(), Arc::new(NoPricing)),
    );
    *recorder.service.lock().unwrap() = Some(Arc::clone(&service));

    service
        .start(
            "playground".into(),
            request("request-1"),
            RecordingChannel::default(),
        )
        .expect("start");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while observed.lock().unwrap().is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "reentrant recorder deadlocked or never ran"
        );
        std::thread::yield_now();
    }
    assert_eq!(
        observed.lock().unwrap().as_slice(),
        &[false],
        "the run is finished by then, so cancelling it reports false"
    );
}

#[test]
fn the_borrowed_request_matches_the_summary_it_accompanies() {
    struct PairRecorder {
        pairs: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl AiRunRecorder for PairRecorder {
        fn record(&self, summary: AiRunSummary, request: &AiCompletionRequest) {
            self.pairs
                .lock()
                .unwrap()
                .push((summary.run_id, request.request_id.clone()));
        }
    }

    let pairs = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn AiComplete> =
        Arc::new(FakeAiProvider::new(FakeCompletionScript::success(["hi"])));
    let service = AiCompletionService::new([("fake".to_owned(), provider)]).recording(
        Arc::new(PairRecorder {
            pairs: Arc::clone(&pairs),
        }),
        Arc::new(NoPricing),
    );

    for index in 0..8 {
        let id = format!("request-{index}");
        service
            .start(
                "playground".into(),
                request(&id),
                RecordingChannel::default(),
            )
            .expect("start");
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while pairs.lock().unwrap().len() < 8 {
        assert!(std::time::Instant::now() < deadline, "runs did not finish");
        std::thread::yield_now();
    }
    for (summary_id, request_id) in pairs.lock().unwrap().iter() {
        assert_eq!(
            summary_id, request_id,
            "correlation must survive concurrent runs and reused ids"
        );
    }
}

#[test]
fn a_scripted_terminal_maps_to_exactly_one_state() {
    // Enumerates the mapping table in docs/contracts.md 2.8 so a new status
    // variant cannot be added without deciding what it stores as.
    let cases = [
        (AiCompletionTerminal::Done { usage: None }, AiRunState::Done),
        (AiCompletionTerminal::Cancelled, AiRunState::Cancelled),
        (AiCompletionTerminal::Timeout, AiRunState::TimedOut),
        (
            AiCompletionTerminal::ProviderError(AiProviderError::new(
                "fake",
                AiProviderErrorCategory::TransportFailure,
                "gone",
                AiRecoveryAction::Retry,
            )),
            AiRunState::Failed,
        ),
    ];

    for (terminal, expected) in cases {
        let status = match &terminal {
            AiCompletionTerminal::Done { .. } => AiRunStatus::Done,
            AiCompletionTerminal::Cancelled => AiRunStatus::Cancelled,
            AiCompletionTerminal::Timeout => AiRunStatus::Timeout,
            AiCompletionTerminal::ProviderError(error) => AiRunStatus::ProviderError {
                category: error.category,
            },
        };
        let (state, category) = match status {
            AiRunStatus::Done => (AiRunState::Done, None),
            AiRunStatus::Cancelled => (AiRunState::Cancelled, None),
            AiRunStatus::Timeout => (AiRunState::TimedOut, None),
            AiRunStatus::ProviderError { category } => (AiRunState::Failed, Some(category)),
        };
        assert_eq!(state, expected);
        assert_eq!(
            category.is_some(),
            expected == AiRunState::Failed,
            "a category accompanies failure and nothing else"
        );
    }
}
