//! Speech-to-text: the audio a provider accepts and the transcript it returns.
//!
//! A transcription is one request and one response, never a stream, so there is
//! no sink and no delta here. Nothing in this module is a serialized contract:
//! the recording must not reach a schema, a fixture, or an event, and the
//! catalogue is descriptor data rather than something a consumer exchanges. An
//! application that renders these values owns its own wire types and projects
//! onto them, exactly as it does for [`AiModelListing`].
//!
//! [`AiModelListing`]: crate::AiModelListing

use std::fmt;

use ai_core::AiProviderError;
use thiserror::Error;

use crate::listing::{valid_label, valid_model_identifier};

/// Maximum accepted recording, in bytes.
///
/// The smallest cap any shipped transcribing provider publishes, applied to all
/// of them: a recording either transcribes everywhere or is refused everywhere,
/// rather than succeeding on one provider and failing on the next.
pub const MAX_AI_AUDIO_BYTES: usize = 25 * 1024 * 1024;
/// Maximum accepted transcript, in UTF-8 bytes.
pub const MAX_AI_TRANSCRIPT_BYTES: usize = 512 * 1024;
/// Maximum length of a language hint, in UTF-8 bytes.
pub const MAX_AI_LANGUAGE_BYTES: usize = 16;

/// The container formats the adapters forward.
///
/// The bytes are opaque to the SDK: this list is what the adapters will put a
/// content type on, not a claim that the payload was decoded or sniffed.
pub const AI_AUDIO_MIME_TYPES: [&str; 5] = [
    "audio/webm",
    "audio/ogg",
    "audio/mp4",
    "audio/mpeg",
    "audio/wav",
];

/// One recording to transcribe.
///
/// Deliberately not serializable. Audio is the most sensitive payload the SDK
/// handles, and a `Serialize` impl is all it takes for a recording to end up in
/// a log line, a history record, or a crash report.
#[derive(Clone, PartialEq, Eq)]
pub struct AiTranscriptionRequest {
    /// Application-scoped identity for this transcription.
    pub request_id: String,
    /// The registered provider this request is bound to.
    pub provider_id: String,
    /// Provider-scoped transcription model identity.
    pub model_id: String,
    /// Container type of `audio`, one of [`AI_AUDIO_MIME_TYPES`].
    pub mime_type: String,
    /// BCP-47 style hint such as `en` or `nl`. `None` lets the model detect.
    pub language: Option<String>,
    /// The recording itself.
    pub audio: Vec<u8>,
}

/// Prints the recording's length instead of its bytes, so a debug line can
/// never carry what someone said.
impl fmt::Debug for AiTranscriptionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiTranscriptionRequest")
            .field("request_id", &self.request_id)
            .field("provider_id", &self.provider_id)
            .field("model_id", &self.model_id)
            .field("mime_type", &self.mime_type)
            .field("language", &self.language)
            .field("audio", &format_args!("<{} bytes>", self.audio.len()))
            .finish()
    }
}

/// A transcription field that failed validation.
///
/// The field label is static and closed, never provider or application text, so
/// it is safe to surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("transcription request has an invalid {field}")]
pub struct AiTranscriptionError {
    /// The rejected field.
    pub field: &'static str,
}

impl AiTranscriptionRequest {
    /// Checks identities, container, hint and bounds.
    ///
    /// # Errors
    ///
    /// Returns the first violated rule.
    pub fn validate(&self) -> Result<(), AiTranscriptionError> {
        let invalid = |field| AiTranscriptionError { field };
        if !valid_model_identifier(&self.request_id) {
            return Err(invalid("request id"));
        }
        if !valid_model_identifier(&self.provider_id) {
            return Err(invalid("provider id"));
        }
        if !valid_model_identifier(&self.model_id) {
            return Err(invalid("model id"));
        }
        if !AI_AUDIO_MIME_TYPES.contains(&self.mime_type.as_str()) {
            return Err(invalid("audio mime type"));
        }
        if let Some(language) = &self.language
            && !valid_language(language)
        {
            return Err(invalid("language hint"));
        }
        if self.audio.is_empty() || self.audio.len() > MAX_AI_AUDIO_BYTES {
            return Err(invalid("audio"));
        }
        Ok(())
    }
}

/// How a transcription ended.
///
/// `Timeout` is the deadline the adapter observed itself; a read timeout that
/// surfaces from the transport is a `ProviderError` instead, matching the
/// completion path.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AiTranscriptionTerminal {
    /// The provider returned a transcript.
    Done {
        /// The transcript, trimmed.
        transcript: String,
    },
    /// The caller cancelled before the transcript was produced.
    Cancelled,
    /// The adapter's own deadline passed.
    Timeout,
    /// The provider, the credential, or the transport failed.
    ProviderError(AiProviderError),
}

/// One model a provider transcribes with.
///
/// There is no discovery request behind this: an entry exists exactly when an
/// adapter mapping for it exists, so the catalogue is a property of the SDK
/// rather than of a credential. Prices are absent because speech-to-text is
/// metered per audio-minute, which no completion listing field describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiTranscriptionModel {
    /// The registered provider this model belongs to.
    pub provider_id: String,
    /// Provider-scoped model identity.
    pub model_id: String,
    /// Display text.
    pub label: String,
}

impl AiTranscriptionModel {
    /// Checks identities and label.
    ///
    /// # Errors
    ///
    /// Returns the first violated rule.
    pub fn validate(&self) -> Result<(), AiTranscriptionError> {
        let invalid = |field| AiTranscriptionError { field };
        if !valid_model_identifier(&self.provider_id) {
            return Err(invalid("provider id"));
        }
        if !valid_model_identifier(&self.model_id) {
            return Err(invalid("model id"));
        }
        if !valid_label(&self.label) {
            return Err(invalid("label"));
        }
        Ok(())
    }
}

fn valid_language(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_AI_LANGUAGE_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{
        AI_AUDIO_MIME_TYPES, AiTranscriptionError, AiTranscriptionModel, AiTranscriptionRequest,
        MAX_AI_AUDIO_BYTES,
    };

    fn request() -> AiTranscriptionRequest {
        AiTranscriptionRequest {
            request_id: "request-1".into(),
            provider_id: "groq".into(),
            model_id: "whisper-large-v3-turbo".into(),
            mime_type: "audio/webm".into(),
            language: Some("en".into()),
            audio: vec![0x1a, 0x45, 0xdf, 0xa3],
        }
    }

    #[test]
    fn accepts_a_bounded_recording_with_or_without_a_hint() {
        assert_eq!(request().validate(), Ok(()));

        let mut detected = request();
        detected.language = None;
        assert_eq!(detected.validate(), Ok(()));
    }

    #[test]
    fn rejects_unbounded_or_empty_audio() {
        let invalid = |field| Err(AiTranscriptionError { field });

        let mut empty = request();
        empty.audio = Vec::new();
        assert_eq!(empty.validate(), invalid("audio"));

        let mut oversized = request();
        oversized.audio = vec![0; MAX_AI_AUDIO_BYTES + 1];
        assert_eq!(oversized.validate(), invalid("audio"));
    }

    #[test]
    fn refuses_unsupported_containers_and_malformed_hints() {
        let invalid = |field| Err(AiTranscriptionError { field });

        let mut video = request();
        video.mime_type = "video/webm".into();
        assert_eq!(video.validate(), invalid("audio mime type"));

        for mime_type in AI_AUDIO_MIME_TYPES {
            let mut accepted = request();
            accepted.mime_type = mime_type.into();
            assert_eq!(accepted.validate(), Ok(()));
        }

        let mut spaced = request();
        spaced.language = Some("en US".into());
        assert_eq!(spaced.validate(), invalid("language hint"));

        let mut long = request();
        long.language = Some("a".repeat(17));
        assert_eq!(long.validate(), invalid("language hint"));

        let mut empty = request();
        empty.language = Some(String::new());
        assert_eq!(empty.validate(), invalid("language hint"));
    }

    #[test]
    fn never_debug_prints_the_recording() {
        let printed = format!("{:?}", request());

        assert!(printed.contains("audio: <4 bytes>"), "{printed}");
        assert!(!printed.contains("26"), "{printed}");
    }

    #[test]
    fn validates_catalogue_entries() {
        let model = |label: &str| AiTranscriptionModel {
            provider_id: "groq".into(),
            model_id: "whisper-large-v3".into(),
            label: label.into(),
        };

        assert_eq!(model("Whisper Large v3").validate(), Ok(()));
        assert_eq!(
            model("bad\nlabel").validate(),
            Err(AiTranscriptionError { field: "label" })
        );
    }
}
