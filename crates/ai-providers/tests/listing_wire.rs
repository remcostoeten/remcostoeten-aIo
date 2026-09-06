//! Wire compatibility for the one contract this crate serializes.
//!
//! A listing crosses an IPC or HTTP boundary into an application's settings
//! surface, so its field names, absent-value meaning, and enum vocabulary are a
//! compatibility surface. Structural decoding and semantic validation are
//! separate checks: a document can decode and still be an invalid listing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use ai_providers::{AiModelListing, AiModelListingError, AiModelSource};
use serde_json::Value;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../specs/fixtures")
}

fn fixture(relative: &str) -> String {
    let path = fixture_dir().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn decoded(relative: &str) -> AiModelListing {
    serde_json::from_str(&fixture(relative)).unwrap_or_else(|error| panic!("{relative}: {error}"))
}

#[test]
fn decodes_a_fetched_listing_and_validates_it() {
    let listing = decoded("valid/model-listing-fetched.json");

    assert_eq!(listing.provider_id, "groq");
    assert_eq!(listing.model_id, "openai/gpt-oss-20b");
    assert_eq!(listing.context_window_tokens, Some(131_072));
    assert_eq!(listing.input_price_micros_per_mtok, None);
    assert_eq!(listing.source, AiModelSource::Fetched);
    assert_eq!(listing.validate(), Ok(()));
}

#[test]
fn decodes_an_application_catalog_listing_with_prices() {
    let listing = decoded("valid/model-listing-catalog-priced.json");

    assert_eq!(listing.source, AiModelSource::Catalog);
    assert_eq!(listing.input_price_micros_per_mtok, Some(75_000));
    assert_eq!(listing.validate(), Ok(()));
}

/// Omitted metadata decodes as absent, not as zero. A consumer that treated a
/// missing price as free, or a missing window as unlimited, would be reading a
/// number the provider never sent.
#[test]
fn omitted_metadata_decodes_as_absent_rather_than_zero() {
    let listing = decoded("valid/model-listing-absent-metadata.json");

    assert_eq!(listing.context_window_tokens, None);
    assert_eq!(listing.input_price_micros_per_mtok, None);
    assert_eq!(listing.output_price_micros_per_mtok, None);
    assert_eq!(listing.validate(), Ok(()));
}

#[test]
fn rejects_unknown_fields_snake_case_and_unknown_enum_values() {
    for relative in [
        "invalid/model-listing-unknown-field.json",
        "invalid/model-listing-snake-case-field.json",
        "invalid/model-listing-unknown-source.json",
    ] {
        let decoded: Result<AiModelListing, _> = serde_json::from_str(&fixture(relative));

        assert!(decoded.is_err(), "{relative} should not decode");
    }
}

#[test]
fn round_trips_every_valid_fixture_byte_for_byte_in_meaning() {
    for relative in [
        "valid/model-listing-fetched.json",
        "valid/model-listing-catalog-priced.json",
        "valid/model-listing-absent-metadata.json",
    ] {
        let committed: Value = serde_json::from_str(&fixture(relative)).expect("fixture parses");
        let listing: AiModelListing =
            serde_json::from_value(committed.clone()).expect("fixture decodes");
        let reserialized = serde_json::to_value(&listing).expect("serializes");

        for (field, value) in committed.as_object().expect("object") {
            assert_eq!(
                reserialized.get(field),
                Some(value),
                "{relative} changed field {field}"
            );
        }
    }
}

/// Decoding is not validation: the structural schema cannot express the
/// path-safety rule that keeps a model id out of an endpoint's parent
/// directory.
#[test]
fn a_document_that_decodes_can_still_be_an_invalid_listing() {
    let mut traversing = decoded("valid/model-listing-fetched.json");
    traversing.model_id = "../escape".into();

    assert_eq!(
        traversing.validate(),
        Err(AiModelListingError { field: "model id" })
    );
}
