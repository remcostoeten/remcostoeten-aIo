//! The credential port: how an adapter obtains a secret for one request.
//!
//! Resolution happens at the provider boundary, after request validation and
//! after the application's model authorization, and before any socket opens.
//! The SDK stores nothing and knows no vault, keyring, or consent rule.

use std::fmt;

use ai_core::{AiProviderError, AiProviderErrorCategory, AiRecoveryAction};
use thiserror::Error;

/// Minimum accepted API key length, in bytes.
pub const MIN_AI_API_KEY_BYTES: usize = 8;
/// Maximum accepted API key length, in bytes.
pub const MAX_AI_API_KEY_BYTES: usize = 4_096;

/// A provider credential in transit from an application store to the adapter
/// that spends it.
///
/// It is not `Clone`, not `Serialize`, and has a redacted `Debug`, so it cannot
/// reach a log line, a contract type, or a serialized configuration. `Drop`
/// overwrites the buffer in place: an overwrite attempt, not a proof of
/// compiler-resistant zeroization. See `docs/contracts.md` §4.2.
pub struct AiCredential(Vec<u8>);

impl AiCredential {
    /// Accepts printable ASCII within the length bounds.
    ///
    /// # Errors
    ///
    /// [`AiCredentialRefusal::Invalid`] when the value is too short, too long,
    /// or contains anything but printable ASCII.
    pub fn new(value: impl AsRef<str>) -> Result<Self, AiCredentialError> {
        let value = value.as_ref();
        if value.len() < MIN_AI_API_KEY_BYTES
            || value.len() > MAX_AI_API_KEY_BYTES
            || !value.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(AiCredentialError::invalid());
        }
        Ok(Self(value.as_bytes().to_vec()))
    }

    /// Reveals the secret for one outbound request.
    ///
    /// Construction accepts only printable ASCII, so the stored bytes are always
    /// valid UTF-8. This is the single accessor, which keeps every read
    /// greppable.
    #[must_use]
    pub fn expose(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or_default()
    }
}

impl fmt::Debug for AiCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AiCredential(<redacted>)")
    }
}

impl Drop for AiCredential {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

/// Why a credential could not be supplied.
///
/// Deliberately small and closed. Application policy — consent, disclosure
/// versions, keyring state — decides *which* refusal applies; it never becomes a
/// machine-readable state of its own here. See `docs/contracts.md` §4.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiCredentialRefusal {
    /// Nothing is configured, or policy withholds what is configured.
    Missing,
    /// Something is configured but is not a usable key.
    Invalid,
    /// The store exists but cannot answer right now.
    Unavailable,
}

/// A typed refusal carrying bounded display copy.
///
/// The message is the application's, so consent wording stays in the
/// application that owns the disclosure; the category and recovery action are
/// the SDK's, so every provider failure keeps one vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct AiCredentialError {
    refusal: AiCredentialRefusal,
    message: String,
}

impl AiCredentialError {
    /// Builds a refusal with application-supplied display copy.
    ///
    /// The message is normalized and truncated by [`AiProviderError::new`] when
    /// it crosses the boundary; supply safe copy, never a provider body.
    #[must_use]
    pub fn new(refusal: AiCredentialRefusal, message: impl Into<String>) -> Self {
        Self {
            refusal,
            message: message.into(),
        }
    }

    /// Nothing is configured, with default copy.
    #[must_use]
    pub fn missing() -> Self {
        Self::new(
            AiCredentialRefusal::Missing,
            "no credential is configured for this provider",
        )
    }

    /// Something is configured but unusable, with default copy.
    #[must_use]
    pub fn invalid() -> Self {
        Self::new(
            AiCredentialRefusal::Invalid,
            "the stored credential is not a usable API key",
        )
    }

    /// The store cannot answer, with default copy.
    #[must_use]
    pub fn unavailable() -> Self {
        Self::new(
            AiCredentialRefusal::Unavailable,
            "the credential store is unavailable",
        )
    }

    /// The typed reason.
    #[must_use]
    pub fn refusal(&self) -> AiCredentialRefusal {
        self.refusal
    }

    /// Maps the refusal onto the completion error vocabulary.
    ///
    /// `Missing` and `Unavailable` both surface as `MissingCredential`: from the
    /// caller's side there is nothing to spend either way, and the difference is
    /// carried by the message rather than by a category a consumer would have to
    /// branch on.
    #[must_use]
    pub fn into_provider_error(self, provider_id: &str) -> AiProviderError {
        let category = match self.refusal {
            AiCredentialRefusal::Missing | AiCredentialRefusal::Unavailable => {
                AiProviderErrorCategory::MissingCredential
            }
            AiCredentialRefusal::Invalid => AiProviderErrorCategory::InvalidCredential,
        };
        AiProviderError::new(
            provider_id,
            category,
            &self.message,
            AiRecoveryAction::ConfigureCredential,
        )
    }
}

/// The narrow capability an adapter uses to obtain a key for one request.
///
/// Implementations own every policy decision that precedes handing over a
/// secret. An adapter calls `resolve` once per request and never caches the
/// result, so revoking or withholding a key takes effect on the next request.
pub trait AiCredentialSource: Send + Sync {
    /// Resolves the credential for `provider_id`.
    ///
    /// # Errors
    ///
    /// Any [`AiCredentialError`]; the adapter terminalizes without opening a
    /// socket.
    fn resolve(&self, provider_id: &str) -> Result<AiCredential, AiCredentialError>;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{
        AiCredential, AiCredentialError, AiCredentialRefusal, MAX_AI_API_KEY_BYTES,
        MIN_AI_API_KEY_BYTES,
    };
    use ai_core::{AiProviderErrorCategory, AiRecoveryAction};

    #[test]
    fn accepts_only_bounded_printable_ascii_keys() {
        assert!(AiCredential::new("sk-test-provider-key").is_ok());
        for rejected in [
            "x".repeat(MIN_AI_API_KEY_BYTES - 1),
            "x".repeat(MAX_AI_API_KEY_BYTES + 1),
            "sk-test key".to_owned(),
            "sk-test\nkey".to_owned(),
            String::new(),
        ] {
            assert_eq!(
                AiCredential::new(&rejected).err(),
                Some(AiCredentialError::invalid())
            );
        }
    }

    #[test]
    fn never_reveals_the_secret_through_debug() {
        let credential = AiCredential::new("sk-test-provider-key").expect("credential");

        let rendered = format!("{credential:?}");

        assert_eq!(rendered, "AiCredential(<redacted>)");
        assert!(!rendered.contains("sk-test"));
    }

    #[test]
    fn maps_every_refusal_onto_the_completion_error_vocabulary() {
        for (error, category) in [
            (
                AiCredentialError::missing(),
                AiProviderErrorCategory::MissingCredential,
            ),
            (
                AiCredentialError::unavailable(),
                AiProviderErrorCategory::MissingCredential,
            ),
            (
                AiCredentialError::invalid(),
                AiProviderErrorCategory::InvalidCredential,
            ),
        ] {
            let message = error.to_string();

            let provider_error = error.into_provider_error("groq");

            assert_eq!(provider_error.category, category);
            assert_eq!(
                provider_error.recovery_action,
                AiRecoveryAction::ConfigureCredential
            );
            assert_eq!(provider_error.message, message);
            assert_eq!(provider_error.validate(), Ok(()));
        }
    }

    /// Skriuw's consent refusals reach the seam as `Missing` with the
    /// application's own copy, which reproduces its current provider error
    /// exactly without moving disclosure policy into the SDK.
    #[test]
    fn application_copy_survives_the_mapping() {
        let error = AiCredentialError::new(
            AiCredentialRefusal::Missing,
            "this provider's privacy disclosure changed and needs review",
        );

        let provider_error = error.into_provider_error("gemini");

        assert_eq!(
            provider_error.category,
            AiProviderErrorCategory::MissingCredential
        );
        assert_eq!(
            provider_error.message,
            "this provider's privacy disclosure changed and needs review"
        );
    }

    #[test]
    fn bounds_hostile_application_copy_at_the_boundary() {
        let error =
            AiCredentialError::new(AiCredentialRefusal::Missing, "a\n\n  b\u{0}".to_owned());

        let provider_error = error.into_provider_error("groq");

        assert_eq!(provider_error.message, "a b");
        assert_eq!(provider_error.validate(), Ok(()));
    }
}
