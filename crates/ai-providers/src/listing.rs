//! What a provider reports about the models a credential can reach.
//!
//! A listing is metadata, never authorization: see [`AiModelAuthority`]. It is
//! also not a capability claim — nothing here says a model streams, accepts a
//! schema, or will actually serve this key.
//!
//! [`AiModelAuthority`]: crate::AiModelAuthority

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Maximum number of listings an adapter returns from one call.
pub const MAX_AI_MODEL_LISTINGS: usize = 256;
/// Maximum length of a model label, in UTF-8 bytes.
pub const MAX_AI_MODEL_LABEL_BYTES: usize = 128;
/// Maximum accepted context window, in tokens.
pub const MAX_AI_CONTEXT_TOKENS: u32 = 16 * 1_024 * 1_024;
/// Maximum accepted price, in micro-units per million tokens.
pub const MAX_AI_PRICE_MICROS: u64 = 1_000_000_000;

/// Where a listing's information came from.
///
/// Provenance, not trust: `Catalog` describes application-owned shipped data,
/// which the SDK never produces. Adapters only ever emit `Fetched`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AiModelSource {
    /// Supplied by the application's own catalog.
    Catalog,
    /// Reported by the provider for a specific credential.
    Fetched,
}

/// One model a provider reports.
///
/// Prices and context windows are absent rather than guessed. Absent means the
/// listing said nothing, never zero, and a consumer must not fabricate a
/// default that would silently price or truncate a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiModelListing {
    /// The registered provider this model belongs to.
    pub provider_id: String,
    /// Provider-scoped model identity.
    pub model_id: String,
    /// Display text.
    pub label: String,
    /// Reported context window, in tokens.
    pub context_window_tokens: Option<u32>,
    /// Reported prompt price, in micro-units per million tokens.
    pub input_price_micros_per_mtok: Option<u64>,
    /// Reported response price, in micro-units per million tokens.
    pub output_price_micros_per_mtok: Option<u64>,
    /// Where this information came from.
    pub source: AiModelSource,
}

/// A listing field that failed validation.
///
/// The field label is static and closed, never provider text, so it is safe to
/// surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("model listing has an invalid {field}")]
pub struct AiModelListingError {
    /// The rejected field.
    pub field: &'static str,
}

impl AiModelListing {
    /// Checks identities, label and bounds.
    ///
    /// Identifiers use the path-safe grammar in [`valid_model_identifier`],
    /// because a model id becomes part of a request URL for some providers.
    ///
    /// # Errors
    ///
    /// Returns the first violated rule.
    pub fn validate(&self) -> Result<(), AiModelListingError> {
        let invalid = |field| AiModelListingError { field };
        if !valid_model_identifier(&self.provider_id) {
            return Err(invalid("provider id"));
        }
        if !valid_model_identifier(&self.model_id) {
            return Err(invalid("model id"));
        }
        if !valid_label(&self.label) {
            return Err(invalid("label"));
        }
        if self
            .context_window_tokens
            .is_some_and(|window| window == 0 || window > MAX_AI_CONTEXT_TOKENS)
        {
            return Err(invalid("context window"));
        }
        for price in [
            self.input_price_micros_per_mtok,
            self.output_price_micros_per_mtok,
        ]
        .into_iter()
        .flatten()
        {
            if price > MAX_AI_PRICE_MICROS {
                return Err(invalid("price"));
            }
        }
        Ok(())
    }
}

/// Whether an identifier is safe to place in a request path.
///
/// Stricter than the core request grammar: it additionally rejects a leading
/// `/`, empty segments, and `.`/`..` segments, so no provider- or
/// application-supplied model id can traverse out of the endpoint it is joined
/// onto.
#[must_use]
pub fn valid_model_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= ai_core::MAX_AI_IDENTIFIER_BYTES
        && !value.starts_with('/')
        && value
            .split('/')
            .all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."))
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}

pub(crate) fn valid_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_AI_MODEL_LABEL_BYTES
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{
        AiModelListing, AiModelListingError, AiModelSource, MAX_AI_CONTEXT_TOKENS,
        MAX_AI_PRICE_MICROS, valid_model_identifier,
    };

    fn listing() -> AiModelListing {
        AiModelListing {
            provider_id: "groq".into(),
            model_id: "openai/gpt-oss-20b".into(),
            label: "openai/gpt-oss-20b".into(),
            context_window_tokens: Some(131_072),
            input_price_micros_per_mtok: None,
            output_price_micros_per_mtok: None,
            source: AiModelSource::Fetched,
        }
    }

    #[test]
    fn accepts_a_reported_listing_without_prices() {
        assert_eq!(listing().validate(), Ok(()));
    }

    #[test]
    fn rejects_identifiers_that_could_traverse_an_endpoint_path() {
        assert!(valid_model_identifier("gemini-3.7-flash"));
        assert!(valid_model_identifier("openai/gpt-oss-20b"));
        assert!(!valid_model_identifier("../escape"));
        assert!(!valid_model_identifier("models/../escape"));
        assert!(!valid_model_identifier("/leading"));
        assert!(!valid_model_identifier("double//segment"));
        assert!(!valid_model_identifier("space model"));
        assert!(!valid_model_identifier(""));
    }

    #[test]
    fn rejects_out_of_bound_metadata() {
        let invalid = |field| Err(AiModelListingError { field });

        let mut zero_window = listing();
        zero_window.context_window_tokens = Some(0);
        assert_eq!(zero_window.validate(), invalid("context window"));

        let mut huge_window = listing();
        huge_window.context_window_tokens = Some(MAX_AI_CONTEXT_TOKENS + 1);
        assert_eq!(huge_window.validate(), invalid("context window"));

        let mut priced = listing();
        priced.output_price_micros_per_mtok = Some(MAX_AI_PRICE_MICROS + 1);
        assert_eq!(priced.validate(), invalid("price"));

        let mut unlabelled = listing();
        unlabelled.label = String::new();
        assert_eq!(unlabelled.validate(), invalid("label"));

        let mut controlled = listing();
        controlled.label = "line\nbreak".into();
        assert_eq!(controlled.validate(), invalid("label"));
    }
}
