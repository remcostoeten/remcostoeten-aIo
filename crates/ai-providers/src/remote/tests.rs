#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{sync::Arc, time::Duration};

use ai_core::{
    AiCancellation, AiComplete, AiCompletionDelta, AiCompletionParameters, AiCompletionRequest,
    AiCompletionTerminal, AiProviderErrorCategory, AiRecoveryAction, AiSinkError, AiUsage,
};
use reqwest::Url;

use super::{
    GEMINI_PROVIDER_ID, GROQ_PROVIDER_ID, MAX_STREAM_EVENT_BYTES, RemoteAiProvider,
    RemoteAiSetupError, RemoteProviderKind, ZAI_PROVIDER_ID, transcription_models,
};
use crate::{
    authority::AiModelAuthority,
    credentials::{AiCredential, AiCredentialError, AiCredentialRefusal, AiCredentialSource},
    fixtures::{self, Reply},
    listing::AiModelSource,
    transcription::{AiTranscriptionRequest, AiTranscriptionTerminal},
};

const KEY: &str = "sk-test-provider-key";
const USER_AGENT: &str = "ai-sdk-tests/0.1";
const GEMINI_MODEL: &str = "gemini-3.7-flash";
const GROQ_MODEL: &str = "openai/gpt-oss-20b";
const DEEPSEEK_MODEL: &str = "deepseek-v4-flash";
const UNVISITED_WINDOW: Duration = Duration::from_millis(300);

struct StoredKey;

impl AiCredentialSource for StoredKey {
    fn resolve(&self, _provider_id: &str) -> Result<AiCredential, AiCredentialError> {
        AiCredential::new(KEY)
    }
}

struct RefusedKey(AiCredentialRefusal);

impl AiCredentialSource for RefusedKey {
    fn resolve(&self, _provider_id: &str) -> Result<AiCredential, AiCredentialError> {
        Err(AiCredentialError::new(self.0, "the application said no"))
    }
}

/// Permits exactly the models these fixtures address. A real application's
/// authority is its own; the SDK ships none.
struct TestModels;

impl AiModelAuthority for TestModels {
    fn permits(&self, provider_id: &str, model_id: &str) -> bool {
        matches!(
            (provider_id, model_id),
            (GEMINI_PROVIDER_ID, GEMINI_MODEL)
                | (GROQ_PROVIDER_ID, GROQ_MODEL)
                | ("deepseek", DEEPSEEK_MODEL)
                | (ZAI_PROVIDER_ID, "glm-4")
        )
    }
}

#[derive(Default)]
struct RecordingSink {
    deltas: Vec<AiCompletionDelta>,
    close_after: Option<usize>,
    cancel_after: Option<(usize, AiCancellation)>,
}

impl ai_core::AiEventSink for RecordingSink {
    fn send_delta(&mut self, delta: AiCompletionDelta) -> Result<(), AiSinkError> {
        if self.close_after == Some(self.deltas.len()) {
            return Err(AiSinkError::Closed);
        }
        self.deltas.push(delta);
        if let Some((at, cancellation)) = &self.cancel_after
            && self.deltas.len() == *at
        {
            cancellation.cancel();
        }
        Ok(())
    }
}

fn request(provider_id: &str, model_id: &str) -> AiCompletionRequest {
    AiCompletionRequest {
        request_id: "request-1".into(),
        provider_id: provider_id.into(),
        model_id: model_id.into(),
        system_prompt: "Be concise.".into(),
        user_prompt: "Name a colour.".into(),
        prior_messages: Vec::new(),
        parameters: AiCompletionParameters::default(),
    }
}

fn build_provider(
    kind: RemoteProviderKind,
    base_url: &str,
    credentials: Arc<dyn AiCredentialSource>,
) -> RemoteAiProvider {
    RemoteAiProvider::with_base_url(
        kind,
        base_url,
        credentials,
        Arc::new(TestModels),
        USER_AGENT,
    )
    .expect("provider")
}

fn provider_error(terminal: AiCompletionTerminal) -> ai_core::AiProviderError {
    match terminal {
        AiCompletionTerminal::ProviderError(error) => error,
        other => panic!("expected a provider error, got {other:?}"),
    }
}

// Descriptors and endpoint safety.

#[test]
fn resolves_provider_identity_from_a_bounded_identifier() {
    for kind in RemoteProviderKind::ALL {
        assert_eq!(RemoteProviderKind::from_id(kind.id()), Some(kind));
        assert!(!kind.label().is_empty());
    }
    assert_eq!(RemoteProviderKind::from_id("ollama"), None);
    assert_eq!(
        RemoteProviderKind::Gemini.destination(),
        "generativelanguage.googleapis.com"
    );
    assert_eq!(RemoteProviderKind::Groq.destination(), "api.groq.com");
}

/// Every request a shipped descriptor can make must reach the host its
/// disclosure names, and the adapter must enforce that rather than trust it.
#[test]
fn every_endpoint_stays_on_the_disclosed_destination() {
    for kind in RemoteProviderKind::ALL {
        let base = Url::parse(kind.default_base_url()).expect("default base url parses");
        assert_eq!(base.host_str(), Some(kind.destination()));

        let mut endpoints = vec![
            kind.endpoint(&base, "some-model", true)
                .expect("chat endpoint"),
            kind.endpoint(&base, "some-model", false)
                .expect("verify endpoint"),
        ];
        match kind.models_endpoint(&base) {
            Some(url) => endpoints.push(url),
            None => assert!(
                !kind.supports_model_listing(),
                "{} has no listing endpoint but claims listing support",
                kind.id()
            ),
        }
        for model in transcription_models()
            .iter()
            .filter(|model| model.provider_id == kind.id())
        {
            let url = kind
                .transcription_endpoint(&base, &model.model_id)
                .expect("a catalogued transcription model has an endpoint");
            endpoints.push(url);
        }
        for url in endpoints {
            assert_eq!(
                url.host_str(),
                Some(kind.destination()),
                "{} endpoint left the disclosed destination: {url}",
                kind.id()
            );
        }
    }
}

#[test]
fn refuses_an_endpoint_that_carries_userinfo() {
    let refused = RemoteAiProvider::with_base_url(
        RemoteProviderKind::Groq,
        "http://user:secret@127.0.0.1:1/",
        Arc::new(StoredKey),
        Arc::new(TestModels),
        USER_AGENT,
    );

    assert_eq!(refused.err(), Some(RemoteAiSetupError::InvalidEndpoint));
}

/// Z.ai's OpenAI-compatible endpoint is the one dialect speaker whose tolerance
/// of `stream_options` is unverified, so its body omits it.
#[test]
fn stream_usage_option_is_gated_per_provider() {
    let request = request("deepseek", DEEPSEEK_MODEL);

    let deepseek = RemoteProviderKind::DeepSeek.completion_body(&request, true);
    let zai = RemoteProviderKind::Zai.completion_body(&request, true);

    assert_eq!(
        deepseek["stream_options"]["include_usage"],
        serde_json::json!(true)
    );
    assert!(zai.get("stream_options").is_none());
}

// Streaming.

#[test]
fn streams_an_openai_compatible_provider_through_the_shared_path() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"blue\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":1}}\n\n",
        "data: [DONE]\n\n"
    )
    .to_owned();
    let (base, server) = fixtures::serve(fixtures::ok("text/event-stream", body));
    let provider = build_provider(RemoteProviderKind::DeepSeek, &base, Arc::new(StoredKey));
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(
        &request("deepseek", DEEPSEEK_MODEL),
        &AiCancellation::new(),
        &mut sink,
    );

    assert_eq!(
        terminal,
        AiCompletionTerminal::Done {
            usage: Some(AiUsage {
                input_tokens: 4,
                output_tokens: 1,
            }),
        },
        "usage that arrives after the text finished still reaches the terminal"
    );
    assert_eq!(sink.deltas.len(), 1);
    assert_eq!(sink.deltas[0].request_id, "request-1");
    assert_eq!(sink.deltas[0].sequence, 0);
    let captured = server.join().expect("server");
    assert!(captured.contains("POST /chat/completions"));
    assert!(captured.contains("authorization: Bearer"));
    assert!(captured.contains("ai-sdk-tests/0.1"));
    assert!(captured.contains("\"role\":\"system\""));
}

#[test]
fn streams_gemini_deltas_and_reports_usage() {
    let body = concat!(
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"blue\"}]}}]}\n\n",
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" green\"}]}}],",
        "\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":2}}\n\n"
    )
    .to_owned();
    let (base, server) = fixtures::serve(fixtures::ok("text/event-stream", body));
    let provider = build_provider(RemoteProviderKind::Gemini, &base, Arc::new(StoredKey));
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(
        &request(GEMINI_PROVIDER_ID, GEMINI_MODEL),
        &AiCancellation::new(),
        &mut sink,
    );

    assert_eq!(
        terminal,
        AiCompletionTerminal::Done {
            usage: Some(AiUsage {
                input_tokens: 7,
                output_tokens: 2,
            }),
        },
        "an EOF after a parsed event ends the stream as Done"
    );
    assert_eq!(sink.deltas.len(), 2);
    assert_eq!(sink.deltas[0].text, "blue");
    assert_eq!(sink.deltas[1].sequence, 1);
    let captured = server.join().expect("server");
    assert!(
        captured.contains("POST /v1beta/models/gemini-3.7-flash:streamGenerateContent?alt=sse")
    );
    assert!(captured.contains("x-goog-api-key"));
    assert!(!captured.contains("authorization: Bearer"));
}

#[test]
fn stops_at_the_done_sentinel_and_discards_later_text() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"blue\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":1}}\n\n",
        "data: [DONE]\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"late\"}}]}\n\n"
    )
    .to_owned();
    let (base, server) = fixtures::serve(fixtures::ok("text/event-stream", body));
    let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(
        &request(GROQ_PROVIDER_ID, GROQ_MODEL),
        &AiCancellation::new(),
        &mut sink,
    );

    assert_eq!(
        terminal,
        AiCompletionTerminal::Done {
            usage: Some(AiUsage {
                input_tokens: 5,
                output_tokens: 1,
            }),
        }
    );
    assert_eq!(sink.deltas.len(), 1, "text after [DONE] must be discarded");
    let captured = server.join().expect("server");
    assert!(captured.contains("POST /openai/v1/chat/completions"));
    assert!(captured.contains("\"include_usage\":true"));
}

#[test]
fn drops_partial_usage_rather_than_completing_it_with_a_zero() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"blue\"}}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5}}\n\n",
        "data: [DONE]\n\n"
    )
    .to_owned();
    let (base, server) = fixtures::serve(fixtures::ok("text/event-stream", body));
    let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(
        &request(GROQ_PROVIDER_ID, GROQ_MODEL),
        &AiCancellation::new(),
        &mut sink,
    );

    assert_eq!(terminal, AiCompletionTerminal::Done { usage: None });
    server.join().expect("server");
}

// Refusals before the network.

#[test]
fn never_opens_a_socket_without_a_credential() {
    let (base, server) = fixtures::serve_unvisited(UNVISITED_WINDOW);
    let provider = build_provider(
        RemoteProviderKind::Groq,
        &base,
        Arc::new(RefusedKey(AiCredentialRefusal::Missing)),
    );
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(
        &request(GROQ_PROVIDER_ID, GROQ_MODEL),
        &AiCancellation::new(),
        &mut sink,
    );

    let error = provider_error(terminal);
    assert_eq!(error.category, AiProviderErrorCategory::MissingCredential);
    assert_eq!(error.recovery_action, AiRecoveryAction::ConfigureCredential);
    assert_eq!(error.message, "the application said no");
    assert!(sink.deltas.is_empty());
    assert!(server.join().expect("server").is_none(), "no socket opened");
}

#[test]
fn never_opens_a_socket_for_a_model_the_application_did_not_permit() {
    let (base, server) = fixtures::serve_unvisited(UNVISITED_WINDOW);
    let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(
        &request(GROQ_PROVIDER_ID, "some-model-nobody-approved"),
        &AiCancellation::new(),
        &mut sink,
    );

    assert_eq!(
        provider_error(terminal).category,
        AiProviderErrorCategory::RejectedRequest
    );
    assert!(server.join().expect("server").is_none(), "no socket opened");
}

/// These descriptors build a single-turn body and no token ceiling. Until they
/// carry both, a request asking for either is refused rather than answered
/// without the history or the limit the caller specified.
#[test]
fn refuses_rather_than_drops_a_conversation_or_a_token_ceiling() {
    for mutate in [
        (|request: &mut AiCompletionRequest| {
            request.prior_messages = vec![ai_core::AiMessage::user("earlier question")];
        }) as fn(&mut AiCompletionRequest),
        |request: &mut AiCompletionRequest| {
            request.parameters.max_output_tokens = Some(1800);
        },
    ] {
        let (base, server) = fixtures::serve_unvisited(UNVISITED_WINDOW);
        let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));
        let mut sink = RecordingSink::default();
        let mut altered = request(GROQ_PROVIDER_ID, GROQ_MODEL);
        mutate(&mut altered);

        let terminal = provider.complete(&altered, &AiCancellation::new(), &mut sink);

        assert_eq!(
            provider_error(terminal).category,
            AiProviderErrorCategory::RejectedRequest
        );
        assert!(server.join().expect("server").is_none(), "no socket opened");
    }
}

/// An authority that permits a traversing id still cannot make one addressable:
/// the id would otherwise be joined into a Gemini request path.
#[test]
fn refuses_a_path_traversing_model_id_even_when_the_authority_permits_it() {
    struct PermitsAnything;
    impl AiModelAuthority for PermitsAnything {
        fn permits(&self, _provider_id: &str, _model_id: &str) -> bool {
            true
        }
    }
    let (base, server) = fixtures::serve_unvisited(UNVISITED_WINDOW);
    let provider = RemoteAiProvider::with_base_url(
        RemoteProviderKind::Gemini,
        &base,
        Arc::new(StoredKey),
        Arc::new(PermitsAnything),
        USER_AGENT,
    )
    .expect("provider");

    assert!(!provider.supports_model("../../escape"));
    let mut sink = RecordingSink::default();
    let terminal = provider.complete(
        &request(GEMINI_PROVIDER_ID, "../../escape"),
        &AiCancellation::new(),
        &mut sink,
    );

    assert_eq!(
        provider_error(terminal).category,
        AiProviderErrorCategory::RejectedRequest
    );
    assert!(server.join().expect("server").is_none(), "no socket opened");
}

#[test]
fn refuses_a_request_addressed_to_another_provider() {
    let provider = build_provider(
        RemoteProviderKind::Groq,
        "http://127.0.0.1:1/",
        Arc::new(StoredKey),
    );
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(
        &request(GEMINI_PROVIDER_ID, GEMINI_MODEL),
        &AiCancellation::new(),
        &mut sink,
    );

    assert_eq!(
        provider_error(terminal).category,
        AiProviderErrorCategory::RejectedRequest
    );
}

#[test]
fn stops_before_sending_when_the_request_is_already_cancelled() {
    let (base, server) = fixtures::serve_unvisited(UNVISITED_WINDOW);
    let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));
    let cancellation = AiCancellation::new();
    cancellation.cancel();
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(
        &request(GROQ_PROVIDER_ID, GROQ_MODEL),
        &cancellation,
        &mut sink,
    );

    assert_eq!(terminal, AiCompletionTerminal::Cancelled);
    assert!(server.join().expect("server").is_none(), "no socket opened");
}

/// A resolver may block — a keyring prompt — so cancellation is rechecked
/// after it and before the request is sent.
#[test]
fn stops_before_sending_when_cancellation_arrives_during_credential_resolution() {
    struct CancellingKey(AiCancellation);
    impl AiCredentialSource for CancellingKey {
        fn resolve(&self, _provider_id: &str) -> Result<AiCredential, AiCredentialError> {
            self.0.cancel();
            AiCredential::new(KEY)
        }
    }
    let (base, server) = fixtures::serve_unvisited(UNVISITED_WINDOW);
    let cancellation = AiCancellation::new();
    let provider = build_provider(
        RemoteProviderKind::Groq,
        &base,
        Arc::new(CancellingKey(cancellation.clone())),
    );
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(
        &request(GROQ_PROVIDER_ID, GROQ_MODEL),
        &cancellation,
        &mut sink,
    );

    assert_eq!(terminal, AiCompletionTerminal::Cancelled);
    assert!(server.join().expect("server").is_none(), "no socket opened");
}

// Failures during the exchange.

#[test]
fn maps_provider_status_codes_onto_distinct_recoverable_states() {
    for (status_line, category, recovery_action) in [
        (
            "401 Unauthorized",
            AiProviderErrorCategory::InvalidCredential,
            AiRecoveryAction::ConfigureCredential,
        ),
        (
            "403 Forbidden",
            AiProviderErrorCategory::InvalidCredential,
            AiRecoveryAction::ConfigureCredential,
        ),
        (
            "402 Payment Required",
            AiProviderErrorCategory::QuotaExhausted,
            AiRecoveryAction::ContactProvider,
        ),
        (
            "404 Not Found",
            AiProviderErrorCategory::UnavailableProvider,
            AiRecoveryAction::ChooseDifferentModel,
        ),
        (
            "413 Payload Too Large",
            AiProviderErrorCategory::RejectedRequest,
            AiRecoveryAction::ReduceRequest,
        ),
        (
            "429 Too Many Requests",
            AiProviderErrorCategory::RateLimited,
            AiRecoveryAction::Retry,
        ),
        (
            "418 I'm a teapot",
            AiProviderErrorCategory::RejectedRequest,
            AiRecoveryAction::ReduceRequest,
        ),
        (
            "503 Service Unavailable",
            AiProviderErrorCategory::UnavailableProvider,
            AiRecoveryAction::CheckProviderStatus,
        ),
    ] {
        let (base, server) = fixtures::serve(fixtures::status(
            status_line,
            format!("{{\"error\":{{\"message\":\"{KEY} was rejected\"}}}}"),
        ));
        let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));
        let mut sink = RecordingSink::default();

        let terminal = provider.complete(
            &request(GROQ_PROVIDER_ID, GROQ_MODEL),
            &AiCancellation::new(),
            &mut sink,
        );

        let error = provider_error(terminal);
        assert_eq!(error.category, category, "category for {status_line}");
        assert_eq!(
            error.recovery_action, recovery_action,
            "recovery for {status_line}"
        );
        assert!(
            !error.message.contains(KEY),
            "provider error leaked key material for {status_line}"
        );
        server.join().expect("server");
    }
}

/// A redirect is refused rather than followed: `reqwest` strips only the
/// headers it recognizes, so a provider header such as `x-goog-api-key` would
/// otherwise be replayed to whatever origin the 3xx named.
#[test]
fn never_replays_a_credential_to_a_redirect_target() {
    let (elsewhere, elsewhere_server) = fixtures::serve_unvisited(UNVISITED_WINDOW);
    let (base, server) = fixtures::serve(Reply::Redirect {
        location: format!("{elsewhere}v1beta/models/{GEMINI_MODEL}:streamGenerateContent"),
    });
    let provider = build_provider(RemoteProviderKind::Gemini, &base, Arc::new(StoredKey));
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(
        &request(GEMINI_PROVIDER_ID, GEMINI_MODEL),
        &AiCancellation::new(),
        &mut sink,
    );

    assert_eq!(
        provider_error(terminal).category,
        AiProviderErrorCategory::RejectedRequest
    );
    server.join().expect("server");
    assert!(
        elsewhere_server.join().expect("server").is_none(),
        "the redirect target must never be contacted"
    );
}

#[test]
fn fails_visibly_on_malformed_stream_data() {
    let (base, server) = fixtures::serve(fixtures::ok(
        "text/event-stream",
        "data: not json at all\n\n".to_owned(),
    ));
    let provider = build_provider(RemoteProviderKind::Gemini, &base, Arc::new(StoredKey));
    let cancellation = AiCancellation::new();
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(
        &request(GEMINI_PROVIDER_ID, GEMINI_MODEL),
        &cancellation,
        &mut sink,
    );

    assert_eq!(
        provider_error(terminal).category,
        AiProviderErrorCategory::MalformedResponse
    );
    assert!(cancellation.is_cancelled());
    server.join().expect("server");
}

#[test]
fn fails_visibly_when_a_provider_closes_a_stream_with_no_events() {
    let (base, server) = fixtures::serve(fixtures::ok("text/event-stream", String::new()));
    let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(
        &request(GROQ_PROVIDER_ID, GROQ_MODEL),
        &AiCancellation::new(),
        &mut sink,
    );

    assert_eq!(
        provider_error(terminal).category,
        AiProviderErrorCategory::MalformedResponse
    );
    server.join().expect("server");
}

/// A body that stops short of its declared length is a transport failure, not a
/// successful stream: truncation must not be reported as Done.
#[test]
fn fails_visibly_when_a_stream_is_truncated_mid_body() {
    let frame = "data: {\"choices\":[{\"delta\":{\"content\":\"blue\"}}]}\n\n".to_owned();
    let (base, server) = fixtures::serve(Reply::Truncated {
        declared_length: frame.len() + 4_096,
        body: frame,
    });
    let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(
        &request(GROQ_PROVIDER_ID, GROQ_MODEL),
        &AiCancellation::new(),
        &mut sink,
    );

    let error = provider_error(terminal);
    assert_eq!(error.category, AiProviderErrorCategory::TransportFailure);
    assert_eq!(sink.deltas.len(), 1);
    server.join().expect("server");
}

#[test]
fn rejects_stream_bytes_beyond_the_requested_output_limit() {
    let body = format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
        "x".repeat(64)
    );
    let (base, server) = fixtures::serve(fixtures::ok("text/event-stream", body));
    let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));
    let mut bounded = request(GROQ_PROVIDER_ID, GROQ_MODEL);
    bounded.parameters.max_output_bytes = 8;
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(&bounded, &AiCancellation::new(), &mut sink);

    let error = provider_error(terminal);
    assert_eq!(error.category, AiProviderErrorCategory::MalformedResponse);
    assert_eq!(error.recovery_action, AiRecoveryAction::ReduceRequest);
    assert!(sink.deltas.is_empty());
    server.join().expect("server");
}

#[test]
fn rejects_an_oversized_stream_event() {
    let body = format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
        "x".repeat(MAX_STREAM_EVENT_BYTES as usize + 1)
    );
    let (base, server) = fixtures::serve(fixtures::ok("text/event-stream", body));
    let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(
        &request(GROQ_PROVIDER_ID, GROQ_MODEL),
        &AiCancellation::new(),
        &mut sink,
    );

    assert_eq!(
        provider_error(terminal).category,
        AiProviderErrorCategory::MalformedResponse
    );
    assert!(sink.deltas.is_empty());
    server.join().expect("server");
}

#[test]
fn a_closed_consumer_cancels_the_request() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"one\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"two\"}}]}\n\n",
        "data: [DONE]\n\n"
    )
    .to_owned();
    let (base, server) = fixtures::serve(fixtures::ok("text/event-stream", body));
    let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));
    let cancellation = AiCancellation::new();
    let mut sink = RecordingSink {
        close_after: Some(0),
        ..RecordingSink::default()
    };

    let terminal = provider.complete(
        &request(GROQ_PROVIDER_ID, GROQ_MODEL),
        &cancellation,
        &mut sink,
    );

    assert_eq!(terminal, AiCompletionTerminal::Cancelled);
    assert!(cancellation.is_cancelled());
    assert!(sink.deltas.is_empty());
    server.join().expect("server");
}

#[test]
fn cancellation_during_the_read_loop_stops_at_the_next_frame() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"one\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"two\"}}]}\n\n",
        "data: [DONE]\n\n"
    )
    .to_owned();
    let (base, server) = fixtures::serve(fixtures::ok("text/event-stream", body));
    let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));
    let cancellation = AiCancellation::new();
    let mut sink = RecordingSink {
        cancel_after: Some((1, cancellation.clone())),
        ..RecordingSink::default()
    };

    let terminal = provider.complete(
        &request(GROQ_PROVIDER_ID, GROQ_MODEL),
        &cancellation,
        &mut sink,
    );

    assert_eq!(terminal, AiCompletionTerminal::Cancelled);
    assert_eq!(sink.deltas.len(), 1);
    server.join().expect("server");
}

#[test]
fn a_provider_that_never_answers_becomes_the_timeout_terminal() {
    let (base, server) = fixtures::serve(Reply::Silence);
    let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));
    let mut impatient = request(GROQ_PROVIDER_ID, GROQ_MODEL);
    impatient.parameters.timeout_ms = 100;
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(&impatient, &AiCancellation::new(), &mut sink);

    assert_eq!(terminal, AiCompletionTerminal::Timeout);
    server.join().expect("server");
}

// Administration.

#[test]
fn lists_gemini_completion_models_from_the_provider() {
    let body = concat!(
        "{\"models\":[",
        "{\"name\":\"models/gemini-3.0-flash\",\"displayName\":\"Gemini 3.0 Flash\",",
        "\"inputTokenLimit\":1048576,\"supportedGenerationMethods\":[\"generateContent\"]},",
        "{\"name\":\"models/embedding-001\",\"displayName\":\"Embedding\",",
        "\"supportedGenerationMethods\":[\"embedContent\"]},",
        "{\"name\":\"models/../escape\",\"supportedGenerationMethods\":[\"generateContent\"]}",
        "]}"
    )
    .to_owned();
    let (base, server) = fixtures::serve(fixtures::ok("application/json", body));
    let provider = build_provider(RemoteProviderKind::Gemini, &base, Arc::new(StoredKey));

    let listings = provider.list_models().expect("listing");

    assert_eq!(listings.len(), 1, "non-completion and invalid ids drop out");
    assert_eq!(listings[0].model_id, "gemini-3.0-flash");
    assert_eq!(listings[0].label, "Gemini 3.0 Flash");
    assert_eq!(listings[0].context_window_tokens, Some(1_048_576));
    assert_eq!(listings[0].input_price_micros_per_mtok, None);
    assert_eq!(listings[0].source, AiModelSource::Fetched);
    let captured = server.join().expect("server");
    assert!(captured.contains("GET /v1beta/models"));
    assert!(captured.contains("x-goog-api-key"));
}

#[test]
fn lists_openai_style_models_and_skips_inactive_or_non_chat_entries() {
    let body = concat!(
        "{\"object\":\"list\",\"data\":[",
        "{\"id\":\"openai/gpt-oss-20b\",\"context_window\":131072,\"active\":true},",
        "{\"id\":\"whisper-large-v3\",\"active\":false},",
        "{\"id\":\"flux/schnell\",\"type\":\"image\"}",
        "]}"
    )
    .to_owned();
    let (base, server) = fixtures::serve(fixtures::ok("application/json", body));
    let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));

    let listings = provider.list_models().expect("listing");

    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].model_id, "openai/gpt-oss-20b");
    assert_eq!(listings[0].context_window_tokens, Some(131_072));
    let captured = server.join().expect("server");
    assert!(captured.contains("GET /openai/v1/models"));
    assert!(captured.contains("authorization: Bearer"));
}

/// A descriptor with no listing endpoint refuses without a request. That is a
/// different outcome from a listing that succeeded and returned nothing.
#[test]
fn refuses_to_list_models_for_a_provider_that_publishes_none() {
    let (base, server) = fixtures::serve_unvisited(UNVISITED_WINDOW);
    let provider = build_provider(RemoteProviderKind::Zai, &base, Arc::new(StoredKey));

    let error = provider.list_models().expect_err("unsupported listing");

    assert!(!RemoteProviderKind::Zai.supports_model_listing());
    assert_eq!(error.category, AiProviderErrorCategory::UnavailableProvider);
    assert_eq!(error.recovery_action, AiRecoveryAction::None);
    assert!(server.join().expect("server").is_none(), "no socket opened");
}

#[test]
fn model_listing_refuses_before_the_network_and_maps_rejections() {
    let (base, server) = fixtures::serve_unvisited(UNVISITED_WINDOW);
    let refused = build_provider(
        RemoteProviderKind::Groq,
        &base,
        Arc::new(RefusedKey(AiCredentialRefusal::Missing)),
    );
    let error = refused.list_models().expect_err("credential gate");
    assert_eq!(error.category, AiProviderErrorCategory::MissingCredential);
    assert!(server.join().expect("server").is_none(), "no socket opened");

    let (base, server) = fixtures::serve(fixtures::status(
        "401 Unauthorized",
        format!("{{\"error\":\"{KEY} rejected\"}}"),
    ));
    let rejecting = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));
    let error = rejecting.list_models().expect_err("rejection");
    assert_eq!(error.category, AiProviderErrorCategory::InvalidCredential);
    assert!(!error.message.contains(KEY));
    server.join().expect("server");

    let (base, server) = fixtures::serve(fixtures::ok("application/json", "not json".to_owned()));
    let malformed = build_provider(RemoteProviderKind::Gemini, &base, Arc::new(StoredKey));
    let error = malformed.list_models().expect_err("malformed");
    assert_eq!(error.category, AiProviderErrorCategory::MalformedResponse);
    server.join().expect("server");
}

#[test]
fn verification_reports_acceptance_and_rejection_without_echoing_the_key() {
    let (base, server) = fixtures::serve(fixtures::ok("application/json", "{}".to_owned()));
    let provider = build_provider(RemoteProviderKind::Gemini, &base, Arc::new(StoredKey));
    let credential = AiCredential::new(KEY).expect("credential");

    assert_eq!(
        provider.verify_credential(GEMINI_MODEL, &credential),
        Ok(())
    );
    let captured = server.join().expect("server");
    assert!(captured.contains("POST /v1beta/models/gemini-3.7-flash:generateContent"));
    assert!(!captured.contains("?alt=sse"));

    let (base, server) = fixtures::serve(fixtures::status(
        "401 Unauthorized",
        format!("{{\"error\":\"{KEY} is invalid\"}}"),
    ));
    let rejecting = build_provider(RemoteProviderKind::Gemini, &base, Arc::new(StoredKey));

    let error = rejecting
        .verify_credential(GEMINI_MODEL, &credential)
        .expect_err("rejection");

    assert_eq!(error.category, AiProviderErrorCategory::InvalidCredential);
    assert!(!error.message.contains(KEY));
    server.join().expect("server");
}

#[test]
fn verification_refuses_a_model_the_application_did_not_permit() {
    let (base, server) = fixtures::serve_unvisited(UNVISITED_WINDOW);
    let provider = build_provider(RemoteProviderKind::Gemini, &base, Arc::new(StoredKey));
    let credential = AiCredential::new(KEY).expect("credential");

    let error = provider
        .verify_credential("not-permitted", &credential)
        .expect_err("refusal");

    assert_eq!(error.category, AiProviderErrorCategory::RejectedRequest);
    assert_eq!(
        error.recovery_action,
        AiRecoveryAction::ChooseDifferentModel
    );
    assert!(server.join().expect("server").is_none(), "no socket opened");
}

// Transcription.

fn transcription_request(provider_id: &str, model_id: &str) -> AiTranscriptionRequest {
    AiTranscriptionRequest {
        request_id: "request-1".into(),
        provider_id: provider_id.into(),
        model_id: model_id.into(),
        mime_type: "audio/webm".into(),
        language: Some("en".into()),
        audio: vec![0x1a, 0x45, 0xdf, 0xa3, 0x01, 0x02],
    }
}

fn transcription_error(terminal: AiTranscriptionTerminal) -> ai_core::AiProviderError {
    match terminal {
        AiTranscriptionTerminal::ProviderError(error) => error,
        other => panic!("expected a provider error, got {other:?}"),
    }
}

#[test]
fn ships_a_valid_transcription_catalogue_for_shipped_adapters() {
    let models = transcription_models();

    assert!(!models.is_empty());
    for model in &models {
        assert_eq!(model.validate(), Ok(()));
        let kind = RemoteProviderKind::from_id(&model.provider_id).expect("descriptor");
        assert!(kind.transcribes(&model.model_id));
    }
    assert!(!RemoteProviderKind::DeepSeek.transcribes("whisper-large-v3"));
    assert!(!RemoteProviderKind::Groq.transcribes(GROQ_MODEL));
}

#[test]
fn uploads_groq_recordings_as_multipart_and_parses_the_transcript() {
    let (base, server) = fixtures::serve(fixtures::ok(
        "application/json",
        "{\"text\":\" hello world \"}".to_owned(),
    ));
    let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));

    let terminal = provider.transcribe(
        &transcription_request(GROQ_PROVIDER_ID, "whisper-large-v3-turbo"),
        &AiCancellation::new(),
    );

    assert_eq!(
        terminal,
        AiTranscriptionTerminal::Done {
            transcript: "hello world".to_owned()
        }
    );
    let captured = server.join().expect("server");
    assert!(captured.contains("POST /openai/v1/audio/transcriptions"));
    assert!(captured.contains("authorization: Bearer"));
    assert!(captured.contains("multipart/form-data"));
    assert!(captured.contains("whisper-large-v3-turbo"));
    assert!(captured.contains("filename=\"recording.webm\""));
}

#[test]
fn sends_gemini_recordings_inline_and_parses_the_transcript() {
    let (base, server) = fixtures::serve(fixtures::ok(
        "application/json",
        "{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hello \"},{\"text\":\"world\"}]}}]}"
            .to_owned(),
    ));
    let provider = build_provider(RemoteProviderKind::Gemini, &base, Arc::new(StoredKey));

    let terminal = provider.transcribe(
        &transcription_request(GEMINI_PROVIDER_ID, "gemini-2.5-flash"),
        &AiCancellation::new(),
    );

    assert_eq!(
        terminal,
        AiTranscriptionTerminal::Done {
            transcript: "hello world".to_owned()
        }
    );
    let captured = server.join().expect("server");
    assert!(captured.contains("POST /v1beta/models/gemini-2.5-flash:generateContent"));
    assert!(captured.contains("x-goog-api-key"));
    assert!(captured.contains("inlineData"));
    assert!(captured.contains("audio/webm"));
}

#[test]
fn never_opens_a_socket_to_transcribe_without_a_credential() {
    let (base, server) = fixtures::serve_unvisited(UNVISITED_WINDOW);
    let provider = build_provider(
        RemoteProviderKind::Groq,
        &base,
        Arc::new(RefusedKey(AiCredentialRefusal::Missing)),
    );

    let error = transcription_error(provider.transcribe(
        &transcription_request(GROQ_PROVIDER_ID, "whisper-large-v3"),
        &AiCancellation::new(),
    ));

    assert_eq!(error.category, AiProviderErrorCategory::MissingCredential);
    assert_eq!(error.recovery_action, AiRecoveryAction::ConfigureCredential);
    assert!(server.join().expect("server").is_none(), "no socket opened");
}

#[test]
fn refuses_a_model_the_provider_does_not_transcribe_with() {
    let (base, server) = fixtures::serve_unvisited(UNVISITED_WINDOW);
    let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));

    let error = transcription_error(provider.transcribe(
        &transcription_request(GROQ_PROVIDER_ID, GROQ_MODEL),
        &AiCancellation::new(),
    ));

    assert_eq!(error.category, AiProviderErrorCategory::RejectedRequest);
    assert!(server.join().expect("server").is_none(), "no socket opened");
}

#[test]
fn refuses_transcription_on_a_descriptor_that_has_no_adapter() {
    let (base, server) = fixtures::serve_unvisited(UNVISITED_WINDOW);
    let provider = build_provider(RemoteProviderKind::DeepSeek, &base, Arc::new(StoredKey));

    let error = transcription_error(provider.transcribe(
        &transcription_request("deepseek", "whisper-large-v3"),
        &AiCancellation::new(),
    ));

    assert_eq!(error.category, AiProviderErrorCategory::RejectedRequest);
    assert!(server.join().expect("server").is_none(), "no socket opened");
}

#[test]
fn maps_a_rejected_transcription_key_without_echoing_the_body() {
    let (base, server) = fixtures::serve(fixtures::status(
        "401 Unauthorized",
        format!("{{\"error\":\"{KEY} is invalid\"}}"),
    ));
    let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));

    let error = transcription_error(provider.transcribe(
        &transcription_request(GROQ_PROVIDER_ID, "whisper-large-v3"),
        &AiCancellation::new(),
    ));

    assert_eq!(error.category, AiProviderErrorCategory::InvalidCredential);
    assert!(!error.message.contains(KEY));
    let _ = server.join();
}

#[test]
fn fails_visibly_on_malformed_transcription_data() {
    let (base, server) = fixtures::serve(fixtures::ok("application/json", "not json".to_owned()));
    let provider = build_provider(RemoteProviderKind::Gemini, &base, Arc::new(StoredKey));

    let error = transcription_error(provider.transcribe(
        &transcription_request(GEMINI_PROVIDER_ID, "gemini-2.5-flash"),
        &AiCancellation::new(),
    ));

    assert_eq!(error.category, AiProviderErrorCategory::MalformedResponse);
    let _ = server.join();
}

#[test]
fn stops_before_sending_when_the_transcription_is_already_cancelled() {
    let (base, server) = fixtures::serve_unvisited(UNVISITED_WINDOW);
    let provider = build_provider(RemoteProviderKind::Groq, &base, Arc::new(StoredKey));
    let cancellation = AiCancellation::new();
    cancellation.cancel();

    let terminal = provider.transcribe(
        &transcription_request(GROQ_PROVIDER_ID, "whisper-large-v3"),
        &cancellation,
    );

    assert_eq!(terminal, AiTranscriptionTerminal::Cancelled);
    assert!(server.join().expect("server").is_none(), "no socket opened");
}

// Live checks, explicitly opt-in.

#[test]
#[ignore = "live provider: needs a real Groq API key in AI_SDK_GROQ_API_KEY"]
fn streams_a_real_groq_completion() {
    struct EnvironmentKey;
    impl AiCredentialSource for EnvironmentKey {
        fn resolve(&self, _provider_id: &str) -> Result<AiCredential, AiCredentialError> {
            AiCredential::new(std::env::var("AI_SDK_GROQ_API_KEY").unwrap_or_default())
        }
    }
    let provider = RemoteAiProvider::new(
        RemoteProviderKind::Groq,
        Arc::new(EnvironmentKey),
        Arc::new(TestModels),
        USER_AGENT,
    )
    .expect("provider");
    let mut sink = RecordingSink::default();

    let terminal = provider.complete(
        &request(GROQ_PROVIDER_ID, GROQ_MODEL),
        &AiCancellation::new(),
        &mut sink,
    );

    assert!(matches!(terminal, AiCompletionTerminal::Done { .. }));
    assert!(!sink.deltas.is_empty());
}
