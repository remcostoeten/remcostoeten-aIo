//! Structural JSON Schema generation for this crate's serialized contract.
//!
//! Only [`AiModelListing`] has a wire form. Credentials deliberately have none,
//! descriptors are code rather than data a consumer exchanges, and the
//! completion contracts belong to `ai-core`'s generator.

use schemars::{JsonSchema, SchemaGenerator, generate::SchemaSettings};
use serde_json::Value;

use crate::listing::AiModelListing;

/// The contract spec version stamped into every generated `$id`.
pub const SPEC_VERSION: &str = include_str!("../../../specs/VERSION");

/// One generated schema, ready to write.
pub struct GeneratedSchema {
    /// File stem, e.g. `model-listing`.
    pub name: &'static str,
    /// The schema document.
    pub schema: Value,
}

/// Generates a schema for every serialized contract this crate owns.
#[must_use]
pub fn generate_all() -> Vec<GeneratedSchema> {
    vec![generated::<AiModelListing>("model-listing")]
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
