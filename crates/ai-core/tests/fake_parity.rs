//! The fake provider's shared scripts, run through this service.
//!
//! `packages/core/test/fake-parity.test.ts` runs the same files through the
//! TypeScript one. Real providers chunk text however their transport flushes,
//! so delta segmentation is not comparable in general — but a fixed script is,
//! and it is the only place the two implementations can be held to the same
//! output.
//!
//! Per ADR 0002 the comparison is segmentation, identity, sequence, terminal
//! kind, error category and usage. Wall-clock timing is not compared, and a
//! fake's own diagnostic copy is display text rather than contract.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use ai_core::{
    AiComplete, AiCompletionEvent, AiCompletionService, AiProviderError, AiProviderErrorCategory,
    AiRecoveryAction, AiUsage, FakeAiProvider, FakeCompletionOutcome, FakeCompletionScript,
};
use serde_json::Value;

mod support;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../specs/fixtures/fake")
}

fn fixtures() -> Vec<(String, Value)> {
    let mut found: Vec<(String, Value)> = fs::read_dir(fixture_dir())
        .expect("fake fixture directory")
        .map(|entry| entry.expect("fixture entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| {
            let name = path
                .file_name()
                .expect("file name")
                .to_string_lossy()
                .into_owned();
            let document: Value =
                serde_json::from_str(&fs::read_to_string(&path).expect("read fixture"))
                    .expect("fixture parses");
            (name, document)
        })
        .collect();
    found.sort_by(|left, right| left.0.cmp(&right.0));
    found
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().expect("string field").to_owned()
}

fn script_from(document: &Value) -> FakeCompletionScript {
    let tokens = document["script"]["segments"]
        .as_array()
        .expect("segments array")
        .iter()
        .map(|segment| segment.as_str().expect("segment is a string").to_owned())
        .collect();

    let outcome = &document["script"]["outcome"];
    let outcome = match text(outcome, "type").as_str() {
        "done" => FakeCompletionOutcome::Done {
            usage: outcome.get("usage").and_then(usage_from),
        },
        "timeout" => FakeCompletionOutcome::Timeout,
        "malformed_output" => FakeCompletionOutcome::MalformedOutput,
        "provider_error" => FakeCompletionOutcome::ProviderError(AiProviderError::new(
            "fake",
            category_from(&text(outcome, "category")),
            &text(outcome, "message"),
            recovery_from(&text(outcome, "recoveryAction")),
        )),
        other => panic!("unknown scripted outcome {other}"),
    };

    FakeCompletionScript {
        tokens,
        token_delay: Duration::ZERO,
        outcome,
    }
}

fn usage_from(value: &Value) -> Option<AiUsage> {
    let object = value.as_object()?;
    Some(AiUsage {
        input_tokens: object.get("inputTokens")?.as_u64()?,
        output_tokens: object.get("outputTokens")?.as_u64()?,
    })
}

fn category_from(value: &str) -> AiProviderErrorCategory {
    serde_json::from_value(Value::String(value.to_owned())).expect("known error category")
}

fn recovery_from(value: &str) -> AiRecoveryAction {
    serde_json::from_value(Value::String(value.to_owned())).expect("known recovery action")
}

#[test]
fn every_shared_script_is_covered() {
    assert_eq!(fixtures().len(), 8);
}

#[test]
fn every_shared_script_produces_the_agreed_output() {
    for (name, document) in fixtures() {
        let provider: Arc<dyn AiComplete> = Arc::new(FakeAiProvider::new(script_from(&document)));
        let service = AiCompletionService::new([("fake".to_owned(), provider)]);
        let channel = support::RecordingChannel::default();

        service
            .start(
                "parity".to_owned(),
                support::request("request-1"),
                channel.clone(),
            )
            .unwrap_or_else(|error| panic!("{name} should be admitted: {error:?}"));
        let events = channel.wait_for_terminal();

        let expected_deltas: Vec<&str> = document["deltas"]
            .as_array()
            .expect("deltas array")
            .iter()
            .map(|delta| delta.as_str().expect("delta is a string"))
            .collect();

        let deltas: Vec<&AiCompletionEvent> = events
            .iter()
            .filter(|event| matches!(event, AiCompletionEvent::Delta(_)))
            .collect();
        assert_eq!(deltas.len(), expected_deltas.len(), "{name} delta count");

        for (index, (event, expected)) in deltas.iter().zip(&expected_deltas).enumerate() {
            let AiCompletionEvent::Delta(delta) = event else {
                unreachable!("filtered to deltas")
            };
            assert_eq!(delta.text, *expected, "{name} delta {index} text");
            assert_eq!(
                delta.sequence,
                u32::try_from(index).expect("index fits"),
                "{name} delta {index} sequence"
            );
            assert_eq!(delta.request_id, "request-1", "{name} delta {index} id");
        }

        let terminals: Vec<&AiCompletionEvent> = events
            .iter()
            .filter(|event| !matches!(event, AiCompletionEvent::Delta(_)))
            .collect();
        assert_eq!(terminals.len(), 1, "{name} terminal count");

        let expected_terminal = &document["terminal"];
        match (terminals[0], text(expected_terminal, "type").as_str()) {
            (AiCompletionEvent::Done { usage, request_id }, "done") => {
                assert_eq!(request_id, "request-1", "{name} terminal id");
                assert_eq!(
                    *usage,
                    expected_terminal.get("usage").and_then(usage_from),
                    "{name} usage"
                );
            }
            (AiCompletionEvent::Timeout { .. }, "timeout")
            | (AiCompletionEvent::Cancelled { .. }, "cancelled") => {}
            (AiCompletionEvent::ProviderError { error, .. }, "provider_error") => {
                let expected_error = &expected_terminal["error"];
                assert_eq!(
                    error.category,
                    category_from(&text(expected_error, "category")),
                    "{name} error category"
                );
                // Message and recovery action are compared only where the
                // script fixed them; a fake's own diagnostic copy is not
                // contract, and the two languages word it differently.
                if let Some(message) = expected_error.get("message") {
                    assert_eq!(
                        error.message,
                        message.as_str().expect("message is a string"),
                        "{name} error message"
                    );
                    assert_eq!(
                        error.recovery_action,
                        recovery_from(&text(expected_error, "recoveryAction")),
                        "{name} recovery action"
                    );
                    assert_eq!(error.provider_id, "fake", "{name} error provider");
                }
            }
            (actual, expected) => {
                panic!("{name} expected a {expected} terminal, got {actual:?}")
            }
        }
    }
}
