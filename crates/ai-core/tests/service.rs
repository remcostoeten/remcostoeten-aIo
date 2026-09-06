//! Characterization of the service lifecycle as extracted.
//!
//! Tests named `defect_*` pin behavior that is wrong on purpose: they record
//! what Skriuw does today so the approved D1 hardening shows up as a visible
//! change to these expectations rather than as an unnoticed drift. Do not
//! "fix" one by editing the assertion alone.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use ai_core::{
    AiComplete, AiCompletionEvent, AiCompletionService, AiCompletionTerminal, AiProviderError,
    AiProviderErrorCategory, AiRecoveryAction, AiRunStatus, AiStartError, AiTokenSource, AiUsage,
    FakeAiProvider, FakeCompletionOutcome, FakeCompletionScript,
};
use support::{
    FixedPricing, GatedChannel, NoPricing, RecordingChannel, RecordingRecorder, ScriptedProvider,
    Step, request,
};

fn fake_service(script: FakeCompletionScript) -> AiCompletionService {
    let provider: Arc<dyn AiComplete> = Arc::new(FakeAiProvider::new(script));
    AiCompletionService::new([("fake".to_owned(), provider)])
}

fn scripted_service(
    steps: Vec<Step>,
    terminal: AiCompletionTerminal,
) -> (
    AiCompletionService,
    support::ProviderControl,
    Arc<ScriptedProvider>,
) {
    let (provider, control) = ScriptedProvider::new(steps, terminal);
    let registered: Arc<dyn AiComplete> = provider.clone();
    (
        AiCompletionService::new([("fake".to_owned(), registered)]),
        control,
        provider,
    )
}

#[test]
fn streams_ordered_deltas_then_one_terminal() {
    let service = fake_service(FakeCompletionScript::success(["one ", "two"]));
    let channel = RecordingChannel::default();

    service
        .start("playground".into(), request("request-1"), channel.clone())
        .expect("start");

    let events = channel.wait_for_terminal();
    assert_eq!(events.len(), 3);
    match (&events[0], &events[1], &events[2]) {
        (
            AiCompletionEvent::Delta(first),
            AiCompletionEvent::Delta(second),
            AiCompletionEvent::Done { request_id, usage },
        ) => {
            assert_eq!((first.sequence, first.text.as_str()), (0, "one "));
            assert_eq!((second.sequence, second.text.as_str()), (1, "two"));
            assert_eq!(request_id, "request-1");
            assert_eq!(*usage, None);
        }
        other => panic!("unexpected event sequence: {other:?}"),
    }
}

#[test]
fn rejects_an_invalid_request_without_producing_a_terminal() {
    let service = fake_service(FakeCompletionScript::success(["x"]));
    let channel = RecordingChannel::default();
    let mut invalid = request("request-1");
    invalid.parameters.timeout_ms = 0;

    assert_eq!(
        service.start("playground".into(), invalid, channel.clone()),
        Err(AiStartError::InvalidRequest)
    );
    assert!(channel.events().is_empty());
}

#[test]
fn rejects_a_duplicate_request_id_on_the_registered_provider_path() {
    let (service, control, _provider) = scripted_service(
        vec![Step::Await],
        AiCompletionTerminal::Done { usage: None },
    );
    let channel = RecordingChannel::default();

    service
        .start("playground".into(), request("request-1"), channel.clone())
        .expect("first start");
    control.await_entry();

    assert_eq!(
        service.start("playground".into(), request("request-1"), channel.clone()),
        Err(AiStartError::DuplicateRequest("request-1".into()))
    );

    control.release();
    channel.wait_for_terminal();
}

#[test]
fn an_unknown_provider_terminalizes_synchronously_and_returns_success() {
    let service = fake_service(FakeCompletionScript::success(["x"]));
    let channel = RecordingChannel::default();
    let mut unknown = request("request-1");
    unknown.provider_id = "ghost".into();

    assert_eq!(
        service.start("playground".into(), unknown, channel.clone()),
        Ok(())
    );

    let events = channel.events();
    assert_eq!(events.len(), 1, "the terminal is sent before start returns");
    match &events[0] {
        AiCompletionEvent::ProviderError { request_id, error } => {
            assert_eq!(request_id, "request-1");
            assert_eq!(error.category, AiProviderErrorCategory::UnavailableProvider);
            assert_eq!(error.recovery_action, AiRecoveryAction::CheckProviderStatus);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn cancellation_before_completion_produces_a_cancelled_terminal() {
    let (service, control, _provider) = scripted_service(
        vec![Step::Await],
        AiCompletionTerminal::Done { usage: None },
    );
    let channel = RecordingChannel::default();

    service
        .start("playground".into(), request("request-1"), channel.clone())
        .expect("start");
    control.await_entry();

    assert!(service.cancel("request-1"));
    control.release();

    let events = channel.wait_for_terminal();
    assert!(matches!(
        events.last(),
        Some(AiCompletionEvent::Cancelled { .. })
    ));
}

#[test]
fn cancellation_does_not_erase_a_timeout_outcome() {
    let mut script = FakeCompletionScript::success(["x"]);
    script.outcome = FakeCompletionOutcome::Timeout;
    let service = fake_service(script);
    let channel = RecordingChannel::default();

    service
        .start("playground".into(), request("request-1"), channel.clone())
        .expect("start");

    let events = channel.wait_for_terminal();
    assert!(
        matches!(events.last(), Some(AiCompletionEvent::Timeout { .. })),
        "the fake sets the cancellation flag before returning Timeout, and the \
         service must not relabel it Cancelled"
    );
}

#[test]
fn cancellation_does_not_erase_a_provider_error_outcome() {
    let mut script = FakeCompletionScript::success(["x"]);
    script.outcome = FakeCompletionOutcome::MalformedOutput;
    let service = fake_service(script);
    let channel = RecordingChannel::default();

    service
        .start("playground".into(), request("request-1"), channel.clone())
        .expect("start");

    let events = channel.wait_for_terminal();
    match events.last() {
        Some(AiCompletionEvent::ProviderError { error, .. }) => {
            assert_eq!(error.category, AiProviderErrorCategory::MalformedResponse);
        }
        other => panic!("unexpected terminal: {other:?}"),
    }
}

#[test]
fn a_closed_channel_cancels_the_run() {
    // A failed delta send sets cancellation, so the run terminalizes as
    // Cancelled. Nothing reaches the consumer, including the terminal: a closed
    // channel cannot be guaranteed anything. The run is still accounted for.
    let recorder = Arc::new(RecordingRecorder::default());
    let service = fake_service(FakeCompletionScript::success(["one", "two"]))
        .recording(recorder.clone(), Arc::new(NoPricing));
    let channel = RecordingChannel::default();
    channel.close();

    service
        .start("playground".into(), request("request-1"), channel.clone())
        .expect("start");

    let (summary, _) = recorder.wait_for_one();
    assert_eq!(summary.status, AiRunStatus::Cancelled);
    assert!(channel.events().is_empty());
}

#[test]
fn shutdown_cancels_current_runs_but_does_not_close_admission() {
    let (service, control, _provider) = scripted_service(
        vec![Step::Await],
        AiCompletionTerminal::Done { usage: None },
    );
    let channel = RecordingChannel::default();

    service
        .start("playground".into(), request("request-1"), channel.clone())
        .expect("start");
    control.await_entry();

    service.shutdown();
    control.release();

    let events = channel.wait_for_terminal();
    assert!(matches!(
        events.last(),
        Some(AiCompletionEvent::Cancelled { .. })
    ));

    let after = RecordingChannel::default();
    assert_eq!(
        fake_service(FakeCompletionScript::success(["x"])).start(
            "playground".into(),
            request("request-2"),
            after.clone()
        ),
        Ok(()),
        "shutdown is cancel-current-runs only; admission stays open"
    );
}

#[test]
fn cancel_reports_false_for_an_unknown_request() {
    let service = fake_service(FakeCompletionScript::success(["x"]));
    assert!(!service.cancel("never-started"));
}

#[test]
fn records_provider_reported_usage_and_catalogue_cost() {
    let mut script = FakeCompletionScript::success(["hello"]);
    script.outcome = FakeCompletionOutcome::Done {
        usage: Some(AiUsage {
            input_tokens: 1_000_000,
            output_tokens: 500_000,
        }),
    };
    let recorder = Arc::new(RecordingRecorder::default());
    let pricing = Arc::new(FixedPricing {
        provider_id: "fake".into(),
        model_id: "model:small".into(),
        price: ai_core::AiModelPrice {
            input_price_micros_per_mtok: 150_000,
            output_price_micros_per_mtok: 600_000,
        },
    });
    let service = fake_service(script).recording(recorder.clone(), pricing);

    service
        .start(
            "playground".into(),
            request("request-1"),
            RecordingChannel::default(),
        )
        .expect("start");

    let (summary, borrowed) = recorder.wait_for_one();
    assert_eq!(summary.status, AiRunStatus::Done);
    assert_eq!(summary.run_id, "request-1");
    assert_eq!(summary.origin, "playground");
    assert_eq!(summary.tokens.source, AiTokenSource::Provider);
    assert_eq!(summary.tokens.input_tokens, 1_000_000);
    assert_eq!(summary.cost_micros, Some(450_000));
    assert_eq!(
        borrowed.user_prompt, "user",
        "the borrowed request carries the prompts the SDK summary omits"
    );
}

#[test]
fn estimates_usage_for_runs_that_report_none() {
    let recorder = Arc::new(RecordingRecorder::default());
    let service = fake_service(FakeCompletionScript::success(["hello"]))
        .recording(recorder.clone(), Arc::new(NoPricing));

    service
        .start(
            "playground".into(),
            request("request-1"),
            RecordingChannel::default(),
        )
        .expect("start");

    let (summary, _) = recorder.wait_for_one();
    assert_eq!(summary.tokens.source, AiTokenSource::Estimated);
    // "system" + "user" = 10 bytes, "hello" = 5 bytes, both ceil-divided by 4.
    assert_eq!(summary.tokens.input_tokens, 3);
    assert_eq!(summary.tokens.output_tokens, 2);
    assert_eq!(summary.cost_micros, None);
}

#[test]
fn records_an_unknown_provider_run_without_starting_a_worker() {
    let recorder = Arc::new(RecordingRecorder::default());
    let service = fake_service(FakeCompletionScript::success(["x"]))
        .recording(recorder.clone(), Arc::new(NoPricing));
    let mut unknown = request("request-1");
    unknown.provider_id = "ghost".into();

    service
        .start("playground".into(), unknown, RecordingChannel::default())
        .expect("start");

    let calls = recorder.calls();
    assert_eq!(
        calls.len(),
        1,
        "recorded synchronously, before start returned"
    );
    assert_eq!(
        calls[0].0.status,
        AiRunStatus::ProviderError {
            category: AiProviderErrorCategory::UnavailableProvider
        }
    );
}

#[test]
fn an_invalid_request_is_never_recorded() {
    let recorder = Arc::new(RecordingRecorder::default());
    let service = fake_service(FakeCompletionScript::success(["x"]))
        .recording(recorder.clone(), Arc::new(NoPricing));
    let mut invalid = request("request-1");
    invalid.parameters.max_output_bytes = 0;

    assert_eq!(
        service.start("playground".into(), invalid, RecordingChannel::default()),
        Err(AiStartError::InvalidRequest)
    );
    assert!(recorder.calls().is_empty());
}

#[test]
fn defect_an_unknown_provider_bypasses_duplicate_detection() {
    let (service, control, _provider) = scripted_service(
        vec![Step::Await],
        AiCompletionTerminal::Done { usage: None },
    );
    let channel = RecordingChannel::default();

    service
        .start("playground".into(), request("request-1"), channel.clone())
        .expect("first start");
    control.await_entry();

    let mut unknown = request("request-1");
    unknown.provider_id = "ghost".into();
    assert_eq!(
        service.start("playground".into(), unknown, channel.clone()),
        Ok(()),
        "source looks up the provider before checking for a duplicate id"
    );

    assert!(
        channel.events().iter().any(|event| matches!(
            event,
            AiCompletionEvent::ProviderError { request_id, .. } if request_id == "request-1"
        )),
        "so an active request id receives a second, foreign terminal"
    );

    control.release();
}

#[test]
fn defect_the_registry_entry_is_released_before_the_terminal_is_delivered() {
    let (provider, control) = ScriptedProvider::new(
        vec![Step::Await],
        AiCompletionTerminal::Done { usage: None },
    );
    let registered: Arc<dyn AiComplete> = provider;
    let service = AiCompletionService::new([("fake".to_owned(), registered)]);
    let (channel, channel_control) = GatedChannel::new();

    service
        .start("playground".into(), request("request-1"), channel.clone())
        .expect("start");
    control.await_entry();
    control.release();
    channel_control.await_terminal_send();

    assert!(
        !service.cancel("request-1"),
        "the run is already untracked while its terminal is still in flight"
    );

    let second = RecordingChannel::default();
    assert_eq!(
        service.start("playground".into(), request("request-1"), second),
        Ok(()),
        "so the same id can be admitted again before the old terminal lands"
    );

    channel_control.release();
}

#[test]
fn defect_the_sink_forwards_provider_output_without_validating_it() {
    let (provider, control) = ScriptedProvider::new(
        vec![
            Step::ForeignDelta("someone-elses-request".into()),
            Step::Delta(9, "out of order".into()),
        ],
        AiCompletionTerminal::Done { usage: None },
    );
    let registered: Arc<dyn AiComplete> = provider;
    let service = AiCompletionService::new([("fake".to_owned(), registered)]);
    let channel = RecordingChannel::default();

    service
        .start("playground".into(), request("request-1"), channel.clone())
        .expect("start");
    control.await_entry();

    let events = channel.wait_for_terminal();
    assert!(
        events.iter().any(|event| matches!(
            event,
            AiCompletionEvent::Delta(delta) if delta.request_id == "someone-elses-request"
        )),
        "a foreign request id reaches the consumer unchallenged"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            AiCompletionEvent::Delta(delta) if delta.sequence == 9
        )),
        "so does a gap in the sequence"
    );
}

#[test]
fn defect_a_provider_panic_strands_the_run_with_no_terminal() {
    // The worker thread's panic message on stderr is expected output for this
    // test; there is no service-level catch_unwind to absorb it yet.
    let (provider, control) = ScriptedProvider::new(
        vec![Step::Signal, Step::Panic],
        AiCompletionTerminal::Done { usage: None },
    );
    let registered: Arc<dyn AiComplete> = provider;
    let recorder = Arc::new(RecordingRecorder::default());
    let service = AiCompletionService::new([("fake".to_owned(), registered)])
        .recording(recorder.clone(), Arc::new(NoPricing));
    let channel = RecordingChannel::default();

    service
        .start("playground".into(), request("request-1"), channel.clone())
        .expect("start");
    control.await_entry();
    control.await_signal();

    assert!(
        service.cancel("request-1"),
        "the registry entry outlives the panicking worker forever"
    );
    assert!(channel.events().is_empty(), "and no terminal is ever sent");
    assert!(recorder.calls().is_empty(), "and nothing is recorded");
}

#[test]
fn provider_errors_carry_the_registered_provider_id() {
    let error = AiProviderError::new(
        "fake",
        AiProviderErrorCategory::RateLimited,
        "slow down",
        AiRecoveryAction::Retry,
    );
    let mut script = FakeCompletionScript::success(Vec::<String>::new());
    script.outcome = FakeCompletionOutcome::ProviderError(error.clone());
    let service = fake_service(script);
    let channel = RecordingChannel::default();

    service
        .start("playground".into(), request("request-1"), channel.clone())
        .expect("start");

    let events = channel.wait_for_terminal();
    assert_eq!(
        events.last(),
        Some(&AiCompletionEvent::ProviderError {
            request_id: "request-1".into(),
            error,
        })
    );
}
