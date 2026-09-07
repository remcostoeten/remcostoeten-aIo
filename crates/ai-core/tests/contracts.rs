//! Characterization of the extracted contracts: bounds, accepted inputs, and
//! wire compatibility with Skriuw's committed fixtures.
//!
//! These tests exist to catch a silent narrowing of the contract during
//! extraction. Where a bound looks surprising — empty prompts and deltas being
//! accepted, `retryCount` doing nothing, `..` being a legal identifier — the
//! surprise is the point.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use ai_core::{
    AiCompletionDelta, AiCompletionEvent, AiCompletionParameters, AiCompletionRequest,
    AiCompletionTerminal, AiMessage, AiMessageRole, AiProviderError, AiProviderErrorCategory,
    AiRecoveryAction, AiUsage, AiValidationError, MAX_AI_DELTA_BYTES, MAX_AI_ERROR_MESSAGE_BYTES,
    MAX_AI_IDENTIFIER_BYTES, MAX_AI_OUTPUT_TOKENS, MAX_AI_PRIOR_MESSAGES, MAX_AI_PROMPT_BYTES,
};
use serde_json::{Value, json};

fn request() -> AiCompletionRequest {
    AiCompletionRequest {
        request_id: "request-1".into(),
        provider_id: "provider.local/v1".into(),
        model_id: "model:small".into(),
        system_prompt: "system".into(),
        user_prompt: "user".into(),
        prior_messages: Vec::new(),
        parameters: AiCompletionParameters::default(),
    }
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../specs/fixtures")
}

fn fixture(relative: &str) -> Value {
    let path = fixture_dir().join(relative);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

#[test]
fn default_parameters_match_the_source_defaults() {
    assert_eq!(
        AiCompletionParameters::default(),
        AiCompletionParameters {
            max_output_bytes: 262_144,
            timeout_ms: 60_000,
            retry_count: 0,
            temperature_millis: None,
            top_p_millis: None,
            max_output_tokens: None,
        }
    );
}

#[test]
fn accepts_a_valid_request() {
    assert_eq!(request().validate(), Ok(()));
}

#[test]
fn accepts_empty_prompts() {
    let mut empty = request();
    empty.system_prompt = String::new();
    empty.user_prompt = String::new();
    assert_eq!(empty.validate(), Ok(()));
}

#[test]
fn accepts_identifier_punctuation_including_traversal_sequences() {
    for identifier in ["provider.local/v1", "a_b-c.d:e/f", "..", "a/../b"] {
        let mut named = request();
        named.provider_id = identifier.into();
        assert_eq!(
            named.validate(),
            Ok(()),
            "{identifier} should be an accepted identifier"
        );
    }
}

#[test]
fn rejects_identifier_bytes_outside_the_grammar() {
    for identifier in ["provider id", "provider+1", "provïder", ""] {
        let mut named = request();
        named.provider_id = identifier.into();
        assert!(named.validate().is_err(), "{identifier} should be rejected");
    }
}

#[test]
fn rejects_an_identifier_over_the_byte_bound() {
    let mut long = request();
    long.model_id = "m".repeat(MAX_AI_IDENTIFIER_BYTES + 1);
    assert_eq!(
        long.validate(),
        Err(AiValidationError::TooLong {
            field: "model id",
            maximum: MAX_AI_IDENTIFIER_BYTES,
        })
    );
}

#[test]
fn bounds_the_combined_prompt_not_each_prompt() {
    let mut split = request();
    split.system_prompt = "x".repeat(MAX_AI_PROMPT_BYTES);
    split.user_prompt = "x".to_owned();
    assert_eq!(
        split.validate(),
        Err(AiValidationError::PromptTooLong {
            maximum: MAX_AI_PROMPT_BYTES,
        })
    );
}

#[test]
fn accepts_the_full_sampling_range_in_thousandths() {
    let mut sampling = request();
    sampling.parameters.temperature_millis = Some(1_000);
    sampling.parameters.top_p_millis = Some(0);
    assert_eq!(sampling.validate(), Ok(()));

    sampling.parameters.temperature_millis = Some(1_001);
    assert!(matches!(
        sampling.validate(),
        Err(AiValidationError::InvalidSamplingParameter { .. })
    ));
}

#[test]
fn accepts_retry_count_without_performing_retries() {
    let mut retries = request();
    retries.parameters.retry_count = 2;
    assert_eq!(retries.validate(), Ok(()));

    retries.parameters.retry_count = 3;
    assert!(matches!(
        retries.validate(),
        Err(AiValidationError::TooManyRetries { .. })
    ));
}

#[test]
fn rejects_zero_and_oversized_output_limits() {
    let mut output = request();
    output.parameters.max_output_bytes = 0;
    assert!(matches!(
        output.validate(),
        Err(AiValidationError::InvalidOutputLimit { .. })
    ));

    output.parameters.max_output_bytes = 4 * 1024 * 1024 + 1;
    assert!(matches!(
        output.validate(),
        Err(AiValidationError::InvalidOutputLimit { .. })
    ));
}

#[test]
fn rejects_zero_and_oversized_timeouts() {
    let mut timeout = request();
    timeout.parameters.timeout_ms = 0;
    assert!(matches!(
        timeout.validate(),
        Err(AiValidationError::InvalidTimeout { .. })
    ));

    timeout.parameters.timeout_ms = 300_001;
    assert!(matches!(
        timeout.validate(),
        Err(AiValidationError::InvalidTimeout { .. })
    ));
}

#[test]
fn accepts_an_empty_delta_and_rejects_an_oversized_one() {
    let empty = AiCompletionDelta {
        request_id: "request-1".into(),
        sequence: 0,
        text: String::new(),
    };
    assert_eq!(empty.validate(), Ok(()));

    let oversized = AiCompletionDelta {
        request_id: "request-1".into(),
        sequence: 0,
        text: "x".repeat(MAX_AI_DELTA_BYTES + 1),
    };
    assert!(matches!(
        oversized.validate(),
        Err(AiValidationError::TooLong { .. })
    ));
}

#[test]
fn bounds_token_counts() {
    let usage = AiUsage {
        input_tokens: 1_000_000_000,
        output_tokens: 1_000_000_001,
    };
    assert!(matches!(
        usage.validate(),
        Err(AiValidationError::TokenCountTooLarge { .. })
    ));
}

#[test]
fn normalizes_and_bounds_provider_error_messages() {
    let source = format!("  failed\n\t{}", "é".repeat(MAX_AI_ERROR_MESSAGE_BYTES));
    let error = AiProviderError::new(
        "fake",
        AiProviderErrorCategory::TransportFailure,
        &source,
        AiRecoveryAction::Retry,
    );

    assert!(error.message.len() <= MAX_AI_ERROR_MESSAGE_BYTES);
    assert!(!error.message.contains('\n'));
    assert!(error.message.starts_with("failed "));
    assert_eq!(error.validate(), Ok(()));
}

#[test]
fn replaces_an_all_whitespace_message_with_a_valid_default() {
    let error = AiProviderError::new(
        "fake",
        AiProviderErrorCategory::InternalFailure,
        " \n\t ",
        AiRecoveryAction::None,
    );
    assert_eq!(error.message, "provider request failed");
    assert_eq!(error.validate(), Ok(()));
}

#[test]
fn terminal_conversion_attaches_the_request_id() {
    let event = AiCompletionTerminal::Timeout.into_event("request-1".into());
    assert_eq!(
        event,
        AiCompletionEvent::Timeout {
            request_id: "request-1".into()
        }
    );
}

#[test]
fn serializes_events_with_the_source_tags_and_field_names() {
    let event = AiCompletionEvent::Delta(AiCompletionDelta {
        request_id: "request-1".into(),
        sequence: 3,
        text: "hi".into(),
    });
    assert_eq!(
        serde_json::to_value(event).unwrap(),
        json!({"type": "delta", "requestId": "request-1", "sequence": 3, "text": "hi"})
    );
}

#[test]
fn emits_null_usage_rather_than_omitting_it() {
    let event = AiCompletionEvent::Done {
        request_id: "request-1".into(),
        usage: None,
    };
    assert_eq!(
        serde_json::to_value(event).unwrap(),
        json!({"type": "done", "requestId": "request-1", "usage": null})
    );
}

#[test]
fn accepts_omitted_usage_on_decode() {
    let decoded: AiCompletionEvent =
        serde_json::from_value(json!({"type": "done", "requestId": "request-1"})).unwrap();
    assert_eq!(
        decoded,
        AiCompletionEvent::Done {
            request_id: "request-1".into(),
            usage: None,
        }
    );
}

#[test]
fn accepts_omitted_sampling_parameters_on_decode() {
    let decoded: AiCompletionParameters =
        serde_json::from_value(json!({"maxOutputBytes": 1024, "timeoutMs": 1000, "retryCount": 0}))
            .unwrap();
    assert_eq!(decoded.temperature_millis, None);
    assert_eq!(decoded.top_p_millis, None);
}

#[test]
fn every_valid_request_fixture_decodes_and_validates() {
    for name in [
        "valid/completion-request.json",
        "valid/completion-request-empty-prompts.json",
        "valid/completion-request-conversation.json",
    ] {
        let value = fixture(name);
        let decoded: AiCompletionRequest = serde_json::from_value(value.clone())
            .unwrap_or_else(|error| panic!("{name} should decode: {error}"));
        assert_eq!(decoded.validate(), Ok(()), "{name} should validate");
        assert_eq!(
            serde_json::to_value(&decoded).unwrap(),
            value,
            "{name} should round-trip unchanged"
        );
    }
}

#[test]
fn every_valid_event_fixture_decodes_validates_and_round_trips() {
    for name in [
        "valid/event-delta.json",
        "valid/event-done-with-usage.json",
        "valid/event-done-null-usage.json",
        "valid/event-cancelled.json",
        "valid/event-timeout.json",
        "valid/event-provider-error.json",
    ] {
        let value = fixture(name);
        let decoded: AiCompletionEvent = serde_json::from_value(value.clone())
            .unwrap_or_else(|error| panic!("{name} should decode: {error}"));
        assert_eq!(decoded.validate(), Ok(()), "{name} should validate");
        assert_eq!(
            serde_json::to_value(&decoded).unwrap(),
            value,
            "{name} should round-trip unchanged"
        );
    }
}

#[test]
fn every_invalid_request_fixture_is_rejected_on_decode() {
    for name in [
        "invalid/request-unknown-field.json",
        "invalid/request-unknown-nested-field.json",
        "invalid/request-missing-field.json",
        "invalid/request-snake-case-field.json",
        "invalid/request-message-unknown-role.json",
        "invalid/request-message-unknown-field.json",
        "invalid/request-message-missing-content.json",
    ] {
        let value = fixture(name);
        assert!(
            serde_json::from_value::<AiCompletionRequest>(value).is_err(),
            "{name} should be rejected"
        );
    }
}

#[test]
fn a_pre_delta_request_still_decodes_to_the_defaults() {
    let value = fixture("valid/completion-request-legacy-omitted.json");
    assert!(!value.as_object().unwrap().contains_key("priorMessages"));
    let decoded: AiCompletionRequest = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.prior_messages, Vec::new());
    assert_eq!(decoded.parameters.max_output_tokens, None);
    assert_eq!(decoded.validate(), Ok(()));
}

#[test]
fn emits_the_delta_fields_rather_than_omitting_them() {
    let value = serde_json::to_value(request()).unwrap();
    let object = value.as_object().unwrap();
    assert_eq!(object["priorMessages"], json!([]));
    assert_eq!(object["parameters"]["maxOutputTokens"], Value::Null);
}

#[test]
fn conversation_order_is_prior_messages_then_the_user_prompt() {
    let decoded: AiCompletionRequest =
        serde_json::from_value(fixture("valid/completion-request-conversation.json")).unwrap();
    assert_eq!(
        decoded.prior_messages,
        vec![
            AiMessage::user("Name a colour."),
            AiMessage::assistant("Blue."),
        ]
    );
    assert_eq!(decoded.user_prompt, "And in Dutch?");
    assert_eq!(decoded.parameters.max_output_tokens, Some(1800));
}

#[test]
fn message_roles_serialize_in_snake_case_and_exclude_system() {
    assert_eq!(
        serde_json::to_value(AiMessageRole::Assistant).unwrap(),
        json!("assistant")
    );
    assert!(serde_json::from_value::<AiMessageRole>(json!("system")).is_err());
}

#[test]
fn rejects_an_empty_message_but_accepts_an_empty_user_prompt() {
    let mut request = request();
    request.user_prompt = String::new();
    request.prior_messages = vec![AiMessage::user(String::new())];
    assert_eq!(
        request.validate(),
        Err(AiValidationError::Empty {
            field: "message content"
        })
    );
    request.prior_messages = vec![AiMessage::user("still here")];
    assert_eq!(request.validate(), Ok(()));
}

#[test]
fn the_prompt_budget_spans_the_conversation_not_each_turn() {
    let half = MAX_AI_PROMPT_BYTES / 2;
    let mut request = request();
    request.system_prompt = String::new();
    request.user_prompt = "x".repeat(half);
    request.prior_messages = vec![AiMessage::user("y".repeat(half))];
    assert_eq!(request.validate(), Ok(()));

    request.prior_messages.push(AiMessage::assistant("z"));
    assert_eq!(
        request.validate(),
        Err(AiValidationError::PromptTooLong {
            maximum: MAX_AI_PROMPT_BYTES
        })
    );
}

#[test]
fn bounds_the_number_of_prior_messages() {
    let mut request = request();
    request.prior_messages = vec![AiMessage::user("turn"); MAX_AI_PRIOR_MESSAGES];
    assert_eq!(request.validate(), Ok(()));

    request.prior_messages.push(AiMessage::user("turn"));
    assert_eq!(
        request.validate(),
        Err(AiValidationError::TooManyMessages {
            maximum: MAX_AI_PRIOR_MESSAGES
        })
    );
}

#[test]
fn accepts_consecutive_turns_from_the_same_role() {
    let mut request = request();
    request.prior_messages = vec![AiMessage::user("first"), AiMessage::user("second")];
    assert_eq!(request.validate(), Ok(()));
}

#[test]
fn rejects_zero_and_oversized_output_token_limits() {
    let mut parameters = AiCompletionParameters::default();
    for tokens in [0, MAX_AI_OUTPUT_TOKENS + 1] {
        parameters.max_output_tokens = Some(tokens);
        assert_eq!(
            parameters.validate(),
            Err(AiValidationError::InvalidOutputTokenLimit {
                maximum: MAX_AI_OUTPUT_TOKENS
            })
        );
    }
    parameters.max_output_tokens = Some(MAX_AI_OUTPUT_TOKENS);
    assert_eq!(parameters.validate(), Ok(()));
}

#[test]
fn every_invalid_event_fixture_is_rejected_on_decode() {
    for name in [
        "invalid/event-unknown-tag.json",
        "invalid/event-unknown-field.json",
        "invalid/event-unknown-error-category.json",
        "invalid/event-delta-negative-sequence.json",
    ] {
        let value = fixture(name);
        assert!(
            serde_json::from_value::<AiCompletionEvent>(value).is_err(),
            "{name} should be rejected"
        );
    }
}

#[test]
fn decoding_does_not_imply_validation() {
    let decoded: AiCompletionRequest = serde_json::from_value(json!({
        "requestId": "not a valid identifier",
        "providerId": "fake",
        "modelId": "model",
        "systemPrompt": "",
        "userPrompt": "",
        "parameters": {
            "maxOutputBytes": 0,
            "timeoutMs": 0,
            "retryCount": 0,
            "temperatureMillis": null,
            "topPMillis": null
        }
    }))
    .expect("structurally valid documents decode");

    assert!(
        decoded.validate().is_err(),
        "a decoded request must still be validated explicitly"
    );
}
