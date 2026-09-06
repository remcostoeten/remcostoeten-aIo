//! Schema drift and fixture conformance.
//!
//! Runs only with the `schema-tool` feature, the same gate that keeps
//! `serde_json` out of the runtime dependency graph.

#![cfg(feature = "schema-tool")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use ai_core::schema::{SPEC_VERSION, generate_all};

fn specs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../specs")
}

#[test]
fn committed_schemas_match_the_generated_ones() {
    for generated in generate_all() {
        let path = specs()
            .join("schemas")
            .join(format!("{}.json", generated.name));
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{} is missing: {error}", path.display()));
        let mut rendered = serde_json::to_string_pretty(&generated.schema).unwrap();
        rendered.push('\n');
        assert_eq!(
            committed, rendered,
            "{}.json is stale; regenerate it with the ai-schema binary",
            generated.name
        );
    }
}

#[test]
fn every_schema_id_carries_the_spec_version() {
    let version = SPEC_VERSION.trim();
    assert!(!version.is_empty(), "specs/VERSION must not be empty");

    for generated in generate_all() {
        let id = generated.schema["$id"]
            .as_str()
            .unwrap_or_else(|| panic!("{} has no $id", generated.name));
        assert!(
            id.contains(version),
            "{} should pin the spec version, got {id}",
            generated.name
        );
    }
}

#[test]
fn schema_generation_covers_every_serialized_contract() {
    let names: Vec<&str> = generate_all()
        .iter()
        .map(|generated| generated.name)
        .collect();
    assert_eq!(
        names,
        vec![
            "completion-parameters",
            "completion-request",
            "completion-delta",
            "usage",
            "provider-error",
            "completion-event",
        ],
        "adding a serialized contract without a schema is a silent contract gap"
    );
}
