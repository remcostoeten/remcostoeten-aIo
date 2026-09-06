#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use ai_core::{
    AiCancellation, AiComplete, AiCompletionDelta, AiCompletionParameters, AiCompletionRequest,
    AiCompletionTerminal, AiEventSink, AiProviderErrorCategory, AiRecoveryAction, AiSinkError,
    AiUsage,
};

use super::{OLLAMA_PROVIDER_ID, OllamaProvider, OllamaSetupError};
use crate::fixtures::{self, Reply};

const USER_AGENT: &str = "ai-sdk-tests/0.1";
const UNVISITED_WINDOW: Duration = Duration::from_millis(300);

#[derive(Default)]
struct RecordingSink {
    deltas: Vec<AiCompletionDelta>,
    close_after: Option<usize>,
}

impl AiEventSink for RecordingSink {
    fn send_delta(&mut self, delta: AiCompletionDelta) -> Result<(), AiSinkError> {
        if self.close_after == Some(self.deltas.len()) {
            return Err(AiSinkError::Closed);
        }
        self.deltas.push(delta);
        Ok(())
    }
}

fn request(model_id: &str) -> AiCompletionRequest {
    AiCompletionRequest {
        request_id: "request-1".into(),
        provider_id: OLLAMA_PROVIDER_ID.into(),
        model_id: model_id.into(),
        system_prompt: "Be concise.".into(),
        user_prompt: "Answer locally.".into(),
        parameters: AiCompletionParameters::default(),
    }
}

fn provider(base: &str) -> OllamaProvider {
    OllamaProvider::new(Some(base), USER_AGENT).expect("provider")
}

fn provider_error(terminal: AiCompletionTerminal) -> ai_core::AiProviderError {
    match terminal {
        AiCompletionTerminal::ProviderError(error) => error,
        other => panic!("expected a provider error, got {other:?}"),
    }
}

#[test]
fn accepts_only_loopback_http_endpoints() {
    assert!(OllamaProvider::new(Some("http://127.0.0.1:11434"), USER_AGENT).is_ok());
    assert!(OllamaProvider::new(Some("http://localhost:11434"), USER_AGENT).is_ok());
    assert!(OllamaProvider::new(Some("http://[::1]:11434"), USER_AGENT).is_ok());
    assert_eq!(
        OllamaProvider::new(Some("http://198.51.100.7:11434"), USER_AGENT).err(),
        Some(OllamaSetupError::NonLoopbackEndpoint)
    );
    assert_eq!(
        OllamaProvider::new(Some("https://127.0.0.1:11434"), USER_AGENT).err(),
        Some(OllamaSetupError::NonLoopbackEndpoint)
    );
    assert_eq!(
        OllamaProvider::new(Some("not a url"), USER_AGENT).err(),
        Some(OllamaSetupError::InvalidEndpoint)
    );
}

#[test]
fn defaults_to_the_local_ollama_port() {
    let provider = OllamaProvider::new(None, USER_AGENT).expect("provider");

    assert_eq!(provider.endpoint().as_str(), "http://127.0.0.1:11434/");
}

#[test]
fn streams_ndjson_deltas_and_reports_usage() {
    let body = concat!(
        "{\"response\":\"local \",\"done\":false}\n",
        "{\"response\":\"answer\",\"done\":true,\"prompt_eval_count\":3,\"eval_count\":2}\n"
    )
    .to_owned();
    let (base, server) = fixtures::serve(fixtures::ok("application/x-ndjson", body));
    let mut sink = RecordingSink::default();

    let terminal =
        provider(&base).complete(&request("gemma3:4b"), &AiCancellation::new(), &mut sink);

    assert_eq!(
        terminal,
        AiCompletionTerminal::Done {
            usage: Some(AiUsage {
                input_tokens: 3,
                output_tokens: 2,
            }),
        }
    );
    assert_eq!(sink.deltas.len(), 2);
    assert_eq!(sink.deltas[0].text, "local ");
    assert_eq!(sink.deltas[1].sequence, 1);
    let captured = server.join().expect("server");
    assert!(captured.contains("POST /api/generate"));
    assert!(captured.contains("\"stream\":true"));
    assert!(captured.contains("\"system\":\"Be concise.\""));
}

#[test]
fn reports_no_usage_when_only_one_counter_arrives() {
    let body = "{\"response\":\"hi\",\"done\":true,\"eval_count\":2}\n".to_owned();
    let (base, server) = fixtures::serve(fixtures::ok("application/x-ndjson", body));
    let mut sink = RecordingSink::default();

    let terminal =
        provider(&base).complete(&request("gemma3:4b"), &AiCancellation::new(), &mut sink);

    assert_eq!(terminal, AiCompletionTerminal::Done { usage: None });
    server.join().expect("server");
}

#[test]
fn refuses_a_request_addressed_to_another_provider() {
    let mut foreign = request("gemma3:4b");
    foreign.provider_id = "groq".into();
    let mut sink = RecordingSink::default();

    let terminal =
        provider("http://127.0.0.1:1/").complete(&foreign, &AiCancellation::new(), &mut sink);

    assert_eq!(
        provider_error(terminal).category,
        AiProviderErrorCategory::RejectedRequest
    );
}

#[test]
fn stops_before_sending_when_the_request_is_already_cancelled() {
    let (base, server) = fixtures::serve_unvisited(UNVISITED_WINDOW);
    let cancellation = AiCancellation::new();
    cancellation.cancel();
    let mut sink = RecordingSink::default();

    let terminal = provider(&base).complete(&request("gemma3:4b"), &cancellation, &mut sink);

    assert_eq!(terminal, AiCompletionTerminal::Cancelled);
    assert!(server.join().expect("server").is_none(), "no socket opened");
}

#[test]
fn reports_a_missing_model_separately_from_a_rejected_request() {
    for (status_line, category, recovery_action) in [
        (
            "404 Not Found",
            AiProviderErrorCategory::UnavailableProvider,
            AiRecoveryAction::ChooseDifferentModel,
        ),
        (
            "500 Internal Server Error",
            AiProviderErrorCategory::TransportFailure,
            AiRecoveryAction::CheckProviderStatus,
        ),
    ] {
        let (base, server) = fixtures::serve(fixtures::status(status_line, "{}".to_owned()));
        let mut sink = RecordingSink::default();

        let terminal =
            provider(&base).complete(&request("gemma3:4b"), &AiCancellation::new(), &mut sink);

        let error = provider_error(terminal);
        assert_eq!(error.category, category, "category for {status_line}");
        assert_eq!(
            error.recovery_action, recovery_action,
            "recovery for {status_line}"
        );
        server.join().expect("server");
    }
}

/// Nothing here starts, installs, or waits for a server: lifecycle belongs to
/// the application that composes it with this adapter.
#[test]
fn an_unreachable_server_is_an_error_rather_than_a_reason_to_start_one() {
    let mut sink = RecordingSink::default();

    let terminal = provider("http://127.0.0.1:1/").complete(
        &request("gemma3:4b"),
        &AiCancellation::new(),
        &mut sink,
    );

    let error = provider_error(terminal);
    assert_eq!(error.category, AiProviderErrorCategory::UnavailableProvider);
    assert_eq!(error.recovery_action, AiRecoveryAction::CheckProviderStatus);
}

#[test]
fn fails_visibly_when_the_stream_ends_without_a_terminal_event() {
    let body = "{\"response\":\"partial\",\"done\":false}\n".to_owned();
    let (base, server) = fixtures::serve(fixtures::ok("application/x-ndjson", body));
    let mut sink = RecordingSink::default();

    let terminal =
        provider(&base).complete(&request("gemma3:4b"), &AiCancellation::new(), &mut sink);

    assert_eq!(
        provider_error(terminal).category,
        AiProviderErrorCategory::MalformedResponse
    );
    assert_eq!(sink.deltas.len(), 1);
    server.join().expect("server");
}

#[test]
fn fails_visibly_on_malformed_stream_data() {
    let (base, server) = fixtures::serve(fixtures::ok(
        "application/x-ndjson",
        "not json at all\n".to_owned(),
    ));
    let mut sink = RecordingSink::default();

    let terminal =
        provider(&base).complete(&request("gemma3:4b"), &AiCancellation::new(), &mut sink);

    assert_eq!(
        provider_error(terminal).category,
        AiProviderErrorCategory::MalformedResponse
    );
    server.join().expect("server");
}

#[test]
fn fails_visibly_when_a_stream_is_truncated_mid_body() {
    let frame = "{\"response\":\"partial\",\"done\":false}\n".to_owned();
    let (base, server) = fixtures::serve(Reply::Truncated {
        declared_length: frame.len() + 4_096,
        body: frame,
    });
    let mut sink = RecordingSink::default();

    let terminal =
        provider(&base).complete(&request("gemma3:4b"), &AiCancellation::new(), &mut sink);

    assert_eq!(
        provider_error(terminal).category,
        AiProviderErrorCategory::TransportFailure
    );
    server.join().expect("server");
}

#[test]
fn rejects_stream_bytes_beyond_the_requested_output_limit() {
    let body = format!("{{\"response\":\"{}\",\"done\":false}}\n", "x".repeat(64));
    let (base, server) = fixtures::serve(fixtures::ok("application/x-ndjson", body));
    let cancellation = AiCancellation::new();
    let mut bounded = request("gemma3:4b");
    bounded.parameters.max_output_bytes = 8;
    let mut sink = RecordingSink::default();

    let terminal = provider(&base).complete(&bounded, &cancellation, &mut sink);

    let error = provider_error(terminal);
    assert_eq!(error.category, AiProviderErrorCategory::MalformedResponse);
    assert_eq!(error.recovery_action, AiRecoveryAction::ReduceRequest);
    assert!(sink.deltas.is_empty());
    assert!(cancellation.is_cancelled());
    server.join().expect("server");
}

#[test]
fn a_closed_consumer_cancels_the_request() {
    let body = concat!(
        "{\"response\":\"one\",\"done\":false}\n",
        "{\"response\":\"two\",\"done\":true}\n"
    )
    .to_owned();
    let (base, server) = fixtures::serve(fixtures::ok("application/x-ndjson", body));
    let cancellation = AiCancellation::new();
    let mut sink = RecordingSink {
        deltas: Vec::new(),
        close_after: Some(0),
    };

    let terminal = provider(&base).complete(&request("gemma3:4b"), &cancellation, &mut sink);

    assert_eq!(terminal, AiCompletionTerminal::Cancelled);
    assert!(cancellation.is_cancelled());
    assert!(sink.deltas.is_empty());
    server.join().expect("server");
}

#[test]
fn a_server_that_never_answers_becomes_the_timeout_terminal() {
    let (base, server) = fixtures::serve(Reply::Silence);
    let mut impatient = request("gemma3:4b");
    impatient.parameters.timeout_ms = 100;
    let mut sink = RecordingSink::default();

    let terminal = provider(&base).complete(&impatient, &AiCancellation::new(), &mut sink);

    assert_eq!(terminal, AiCompletionTerminal::Timeout);
    server.join().expect("server");
}

#[test]
#[ignore = "live runtime: needs a local Ollama server with gemma3:4b installed"]
fn streams_a_real_local_completion() {
    let mut sink = RecordingSink::default();

    let terminal = OllamaProvider::new(None, USER_AGENT)
        .expect("provider")
        .complete(&request("gemma3:4b"), &AiCancellation::new(), &mut sink);

    assert!(matches!(terminal, AiCompletionTerminal::Done { .. }));
    assert!(!sink.deltas.is_empty());
}
