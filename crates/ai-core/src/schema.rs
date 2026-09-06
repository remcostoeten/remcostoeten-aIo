//! Structural JSON Schema generation for the serialized contracts.
//!
//! Development tooling, behind the `schema-tool` feature, so no `serde_json`
//! type reaches the library's public API. `schemars` depends on `serde_json`
//! transitively either way; the gate is about the contract surface, not the
//! dependency count.
//!
//! A generated schema describes structure only. Passing it proves a document
//! decodes, never that the value satisfies its `validate` rules — combined
//! prompt budgets, UTF-8 byte limits and aggregate stream invariants are not
//! expressible here. Cross-language conformance must compare validators
//! separately.

use schemars::{JsonSchema, SchemaGenerator, generate::SchemaSettings};
use serde_json::Value;

use crate::contracts::{
    AiCompletionDelta, AiCompletionEvent, AiCompletionParameters, AiCompletionRequest,
    AiProviderError, AiUsage,
};

/// The contract spec version stamped into every generated `$id`.
pub const SPEC_VERSION: &str = include_str!("../../../specs/VERSION");

/// One generated schema, ready to write.
pub struct GeneratedSchema {
    /// File stem, e.g. `completion-request`.
    pub name: &'static str,
    /// The schema document.
    pub schema: Value,
}

/// Generates a schema for every serialized contract.
///
/// In-process ports and the run summary are deliberately absent: they have no
/// wire form, and giving them one would export the recorder's unencoded
/// timestamp and cost widths as if they were portable.
#[must_use]
pub fn generate_all() -> Vec<GeneratedSchema> {
    vec![
        generated::<AiCompletionParameters>("completion-parameters"),
        generated::<AiCompletionRequest>("completion-request"),
        generated::<AiCompletionDelta>("completion-delta"),
        generated::<AiUsage>("usage"),
        generated::<AiProviderError>("provider-error"),
        generated::<AiCompletionEvent>("completion-event"),
    ]
}

fn generated<T: JsonSchema>(name: &'static str) -> GeneratedSchema {
    let mut generator = SchemaGenerator::new(SchemaSettings::draft2020_12());
    let mut schema = generator.root_schema_for::<T>().to_value();
    let version = SPEC_VERSION.trim();
    if let Value::Object(object) = &mut schema {
        object.insert(
            "$id".to_owned(),
            Value::String(format!(
                "https://schemas.ai-sdk.local/{version}/{name}.json"
            )),
        );
    }
    GeneratedSchema { name, schema }
}
