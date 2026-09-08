//! Bring-your-own-key remote provider adapters.
//!
//! Every descriptor here implements the core [`AiComplete`] seam and resolves
//! its key through [`AiCredentialSource`] at the moment a request starts. Key
//! bytes never enter a URL, a log line, a contract type, or a provider error.
//!
//! Administration — verification and listing — is not part of the completion
//! trait. It is inherent to this adapter, so a fake or local provider never has
//! to supply a dummy credential or pretend an unsupported listing succeeded.

mod descriptor;

use std::{
    io::{BufRead, BufReader, Read},
    sync::Arc,
    time::{Duration, Instant},
};

use ai_core::{
    AiCancellation, AiComplete, AiCompletionDelta, AiCompletionRequest, AiCompletionTerminal,
    AiEventSink, AiProviderError, AiProviderErrorCategory, AiRecoveryAction, AiUsage,
    MAX_AI_RESPONSE_BYTES,
};
use reqwest::{StatusCode, Url, blocking::Client};

use crate::{
    authority::AiModelAuthority,
    credentials::{AiCredential, AiCredentialSource},
    http::{self, SSE_DONE_PAYLOAD},
    listing::{AiModelListing, MAX_AI_MODEL_LISTINGS, valid_model_identifier},
    transcription::{AiTranscriptionRequest, AiTranscriptionTerminal, MAX_AI_TRANSCRIPT_BYTES},
};

pub use descriptor::{
    AIMLAPI_PROVIDER_ID, DASHSCOPE_PROVIDER_ID, DEEPSEEK_PROVIDER_ID, GEMINI_PROVIDER_ID,
    GROQ_PROVIDER_ID, MOONSHOT_PROVIDER_ID, RemoteProviderKind, ZAI_PROVIDER_ID,
    transcription_models,
};
use descriptor::{ProviderEvent, TranscriptionBody};

const MAX_STREAM_EVENT_BYTES: u64 = 64 * 1024;
const MAX_DISCARDED_RESPONSE_BYTES: u64 = 256 * 1024;
const MAX_MODEL_LIST_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
/// The transcript plus its provider JSON framing. A larger body is cut off and
/// refused as malformed rather than buffered without bound.
const MAX_TRANSCRIPTION_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const ADMINISTRATION_TIMEOUT: Duration = Duration::from_secs(30);
/// One fixed budget rather than a per-request one: a transcription carries no
/// parameters, and the whole recording uploads before any work begins.
const TRANSCRIPTION_TIMEOUT: Duration = Duration::from_secs(120);

/// A remote provider bound to one descriptor, one credential source, and one
/// model authority.
pub struct RemoteAiProvider {
    kind: RemoteProviderKind,
    base_url: Url,
    destination: String,
    client: Client,
    credentials: Arc<dyn AiCredentialSource>,
    models: Arc<dyn AiModelAuthority>,
}

/// Why a provider could not be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RemoteAiSetupError {
    /// The endpoint did not parse, or did not reach the descriptor's host.
    #[error("remote provider endpoint is invalid")]
    InvalidEndpoint,
    /// No usable HTTP client could be built for this configuration.
    #[error("remote provider transport is unavailable")]
    TransportUnavailable,
}

impl RemoteAiProvider {
    /// Binds a descriptor to its shipped endpoint.
    ///
    /// `user_agent` is the calling application's identity; the SDK has none of
    /// its own and will not invent one. The value must be a valid header value.
    ///
    /// # Errors
    ///
    /// [`RemoteAiSetupError`] when the endpoint or the client cannot be built.
    pub fn new(
        kind: RemoteProviderKind,
        credentials: Arc<dyn AiCredentialSource>,
        models: Arc<dyn AiModelAuthority>,
        user_agent: &str,
    ) -> Result<Self, RemoteAiSetupError> {
        let provider = Self::with_base_url(
            kind,
            kind.default_base_url(),
            credentials,
            models,
            user_agent,
        )?;
        if provider.destination != kind.destination() {
            return Err(RemoteAiSetupError::InvalidEndpoint);
        }
        Ok(provider)
    }

    /// Binds a descriptor to an explicit base URL.
    ///
    /// Crate-private on purpose. A public endpoint override would let request
    /// input redirect a saved vendor key to an arbitrary host; ADR 0003 §4
    /// requires an explicitly bound credential source and destination before
    /// any such API exists. Local fixtures use this from inside the crate.
    fn with_base_url(
        kind: RemoteProviderKind,
        base_url: &str,
        credentials: Arc<dyn AiCredentialSource>,
        models: Arc<dyn AiModelAuthority>,
        user_agent: &str,
    ) -> Result<Self, RemoteAiSetupError> {
        let base_url = Url::parse(base_url).map_err(|_| RemoteAiSetupError::InvalidEndpoint)?;
        let destination = base_url
            .host_str()
            .ok_or(RemoteAiSetupError::InvalidEndpoint)?
            .to_owned();
        if !base_url.username().is_empty() || base_url.password().is_some() {
            return Err(RemoteAiSetupError::InvalidEndpoint);
        }
        let client = http::client(user_agent, CONNECT_TIMEOUT, None)
            .ok_or(RemoteAiSetupError::TransportUnavailable)?;
        Ok(Self {
            kind,
            base_url,
            destination,
            client,
            credentials,
            models,
        })
    }

    /// The bound descriptor.
    #[must_use]
    pub fn kind(&self) -> RemoteProviderKind {
        self.kind
    }

    /// Whether the authority permits this model, and the id is path-safe.
    #[must_use]
    pub fn supports_model(&self, model_id: &str) -> bool {
        valid_model_identifier(model_id) && self.models.permits(self.kind.id(), model_id)
    }

    /// Asks the provider which models the stored key can reach.
    ///
    /// This spends no tokens but does send the key, so it belongs behind an
    /// explicit user action. Resolution happens first, so an unconfigured or
    /// refused provider fails before any socket opens. A listing is metadata
    /// only: it never widens what [`AiModelAuthority`] permits.
    ///
    /// # Errors
    ///
    /// `UnavailableProvider` when the descriptor publishes no listing at all —
    /// which is not the same as a successful empty listing — the credential
    /// refusal, the mapped status, or `MalformedResponse`.
    pub fn list_models(&self) -> Result<Vec<AiModelListing>, AiProviderError> {
        let Some(url) = self
            .kind
            .models_endpoint(&self.base_url)
            .filter(|url| self.stays_on_destination(url))
        else {
            return Err(self.error(
                AiProviderErrorCategory::UnavailableProvider,
                "this provider does not publish a model listing; its models come from the catalog",
                AiRecoveryAction::None,
            ));
        };
        let credential = self
            .credentials
            .resolve(self.kind.id())
            .map_err(|error| error.into_provider_error(self.kind.id()))?;
        let response = self
            .kind
            .authorize(self.client.get(url), &credential)
            .timeout(ADMINISTRATION_TIMEOUT)
            .send()
            .map_err(|error| self.transport_error(&error))?;
        let status = response.status();
        let mut body = Vec::new();
        let _ = response
            .take(MAX_MODEL_LIST_RESPONSE_BYTES)
            .read_to_end(&mut body);
        if !status.is_success() {
            return Err(self.status_error(status));
        }
        let payload = String::from_utf8_lossy(&body);
        let mut listings = self.kind.parse_model_listing(&payload).ok_or_else(|| {
            self.error(
                AiProviderErrorCategory::MalformedResponse,
                "the provider returned an unrecognisable model listing",
                AiRecoveryAction::Retry,
            )
        })?;
        listings.truncate(MAX_AI_MODEL_LISTINGS);
        Ok(listings)
    }

    /// Spends one key on the smallest metered request the provider supports and
    /// reports only whether it was accepted.
    ///
    /// Acceptance is evidence about this key and this model at this moment. It
    /// is not a claim that other models are reachable, that quota remains, or
    /// that any capability is supported.
    ///
    /// # Errors
    ///
    /// `RejectedRequest` when the authority does not permit the model, or the
    /// mapped provider status.
    pub fn verify_credential(
        &self,
        model_id: &str,
        credential: &AiCredential,
    ) -> Result<(), AiProviderError> {
        if !self.supports_model(model_id) {
            return Err(self.error(
                AiProviderErrorCategory::RejectedRequest,
                "the selected model is not available for this provider",
                AiRecoveryAction::ChooseDifferentModel,
            ));
        }
        let url = self.endpoint(model_id, false).map_err(|error| *error)?;
        let response = self
            .kind
            .authorize(self.client.post(url), credential)
            .timeout(ADMINISTRATION_TIMEOUT)
            .json(&self.kind.verification_body(model_id))
            .send()
            .map_err(|error| self.transport_error(&error))?;
        let status = response.status();
        // The body is drained under a bound and discarded: a provider error
        // body can contain the echoed request and must not reach the consumer.
        http::discard_body(response, MAX_DISCARDED_RESPONSE_BYTES);
        if status.is_success() {
            Ok(())
        } else {
            Err(self.status_error(status))
        }
    }

    /// Transcribes one recording.
    ///
    /// Inherent rather than part of a trait, for the same reason listing is:
    /// most providers do not transcribe at all, and a fake or local adapter
    /// should not have to pretend otherwise. [`RemoteProviderKind::transcribes`]
    /// answers whether a model is in the catalogue before a request is built.
    ///
    /// One request and one response, never a stream. Cancellation is therefore
    /// cooperative and checked at the two points where it can still avoid work:
    /// before the socket opens and before the transcript is parsed. It cannot
    /// abort an upload already in flight.
    pub fn transcribe(
        &self,
        request: &AiTranscriptionRequest,
        cancellation: &AiCancellation,
    ) -> AiTranscriptionTerminal {
        if request.validate().is_err()
            || request.provider_id != self.kind.id()
            || !self.kind.transcribes(&request.model_id)
        {
            return AiTranscriptionTerminal::ProviderError(self.error(
                AiProviderErrorCategory::RejectedRequest,
                "the transcription request is not valid for this provider",
                AiRecoveryAction::ReduceRequest,
            ));
        }
        if cancellation.is_cancelled() {
            return AiTranscriptionTerminal::Cancelled;
        }
        // Same order as a completion: an unconfigured or refused provider
        // terminalizes before any socket opens, and the resolver may block on a
        // keyring prompt, so cancellation is rechecked after it.
        let credential = match self.credentials.resolve(self.kind.id()) {
            Ok(credential) => credential,
            Err(error) => {
                return AiTranscriptionTerminal::ProviderError(
                    error.into_provider_error(self.kind.id()),
                );
            }
        };
        if cancellation.is_cancelled() {
            return AiTranscriptionTerminal::Cancelled;
        }
        let Some(url) = self
            .kind
            .transcription_endpoint(&self.base_url, &request.model_id)
            .filter(|url| self.stays_on_destination(url))
        else {
            return AiTranscriptionTerminal::ProviderError(self.error(
                AiProviderErrorCategory::InternalFailure,
                "provider transcription endpoint could not be built",
                AiRecoveryAction::None,
            ));
        };
        let Some(body) = self.kind.transcription_body(request) else {
            return AiTranscriptionTerminal::ProviderError(self.error(
                AiProviderErrorCategory::InternalFailure,
                "the recording could not be prepared for this provider",
                AiRecoveryAction::None,
            ));
        };
        let builder = self
            .kind
            .authorize(self.client.post(url), &credential)
            .timeout(TRANSCRIPTION_TIMEOUT);
        let builder = match body {
            TranscriptionBody::Json(body) => builder.json(&body),
            TranscriptionBody::Multipart(form) => builder.multipart(form),
        };
        let response = match builder.send() {
            Ok(response) => response,
            Err(error) if error.is_timeout() => return AiTranscriptionTerminal::Timeout,
            Err(error) => {
                return AiTranscriptionTerminal::ProviderError(self.transport_error(&error));
            }
        };
        let status = response.status();
        let mut payload = Vec::new();
        let _ = response
            .take(MAX_TRANSCRIPTION_RESPONSE_BYTES)
            .read_to_end(&mut payload);
        if !status.is_success() {
            return AiTranscriptionTerminal::ProviderError(self.status_error(status));
        }
        if cancellation.is_cancelled() {
            return AiTranscriptionTerminal::Cancelled;
        }
        let malformed = || {
            AiTranscriptionTerminal::ProviderError(self.error(
                AiProviderErrorCategory::MalformedResponse,
                "the provider returned an unrecognisable transcript",
                AiRecoveryAction::Retry,
            ))
        };
        let Ok(payload) = String::from_utf8(payload) else {
            return malformed();
        };
        let Some(transcript) = self.kind.parse_transcript(&payload) else {
            return malformed();
        };
        if transcript.len() > MAX_AI_TRANSCRIPT_BYTES {
            return AiTranscriptionTerminal::ProviderError(self.error(
                AiProviderErrorCategory::MalformedResponse,
                "the provider returned a transcript larger than the configured limit",
                AiRecoveryAction::ReduceRequest,
            ));
        }
        AiTranscriptionTerminal::Done {
            transcript: transcript.trim().to_owned(),
        }
    }

    fn endpoint(&self, model_id: &str, streaming: bool) -> Result<Url, Box<AiProviderError>> {
        self.kind
            .endpoint(&self.base_url, model_id, streaming)
            .filter(|url| self.stays_on_destination(url))
            .ok_or_else(|| {
                Box::new(self.error(
                    AiProviderErrorCategory::InternalFailure,
                    "provider endpoint could not be built",
                    AiRecoveryAction::None,
                ))
            })
    }

    fn stays_on_destination(&self, url: &Url) -> bool {
        http::stays_on_destination(url, &self.destination)
    }

    fn stream_completion(
        &self,
        request: &AiCompletionRequest,
        credential: &AiCredential,
        cancellation: &AiCancellation,
        sink: &mut dyn AiEventSink,
    ) -> AiCompletionTerminal {
        let url = match self.endpoint(&request.model_id, true) {
            Ok(url) => url,
            Err(error) => return AiCompletionTerminal::ProviderError(*error),
        };
        let timeout = Duration::from_millis(u64::from(request.parameters.timeout_ms));
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        let response = match self
            .kind
            .authorize(self.client.post(url), credential)
            .timeout(timeout)
            .json(&self.kind.completion_body(request, true))
            .send()
        {
            Ok(response) => response,
            Err(error) if error.is_timeout() => return AiCompletionTerminal::Timeout,
            Err(error) => {
                return AiCompletionTerminal::ProviderError(self.transport_error(&error));
            }
        };
        let status = response.status();
        if !status.is_success() {
            http::discard_body(response, MAX_DISCARDED_RESPONSE_BYTES);
            return AiCompletionTerminal::ProviderError(self.status_error(status));
        }

        let mut reader = BufReader::new(response.take(MAX_AI_RESPONSE_BYTES as u64 + 1));
        let mut line = String::new();
        let mut sequence = 0u32;
        let mut response_bytes = 0usize;
        let mut usage: Option<AiUsage> = None;
        let mut saw_event = false;

        loop {
            if cancellation.is_cancelled() {
                return AiCompletionTerminal::Cancelled;
            }
            if Instant::now() >= deadline {
                cancellation.cancel();
                return AiCompletionTerminal::Timeout;
            }
            line.clear();
            let read = match reader
                .by_ref()
                .take(MAX_STREAM_EVENT_BYTES + 1)
                .read_line(&mut line)
            {
                Ok(read) => read,
                Err(_) => {
                    return AiCompletionTerminal::ProviderError(self.error(
                        AiProviderErrorCategory::TransportFailure,
                        "the provider response stream failed",
                        AiRecoveryAction::Retry,
                    ));
                }
            };
            if read == 0 {
                return if saw_event {
                    AiCompletionTerminal::Done { usage }
                } else {
                    AiCompletionTerminal::ProviderError(self.error(
                        AiProviderErrorCategory::MalformedResponse,
                        "the provider closed the stream without producing a response",
                        AiRecoveryAction::Retry,
                    ))
                };
            }
            if read as u64 > MAX_STREAM_EVENT_BYTES {
                cancellation.cancel();
                return AiCompletionTerminal::ProviderError(self.error(
                    AiProviderErrorCategory::MalformedResponse,
                    "the provider returned an oversized stream event",
                    AiRecoveryAction::Retry,
                ));
            }
            let Some(payload) = http::sse_payload(&line) else {
                continue;
            };
            if payload == SSE_DONE_PAYLOAD {
                return AiCompletionTerminal::Done { usage };
            }
            let Some(event) = self.kind.parse_event(payload) else {
                cancellation.cancel();
                return AiCompletionTerminal::ProviderError(self.error(
                    AiProviderErrorCategory::MalformedResponse,
                    "the provider returned malformed completion data",
                    AiRecoveryAction::Retry,
                ));
            };
            saw_event = true;
            if event.usage.is_some() {
                usage = event.usage.clone();
            }
            if !event.text.is_empty() {
                match self.forward(request, &event, sequence, &mut response_bytes, sink) {
                    Ok(()) => {}
                    Err(terminal) => {
                        cancellation.cancel();
                        return terminal;
                    }
                }
                let Some(next) = sequence.checked_add(1) else {
                    cancellation.cancel();
                    return AiCompletionTerminal::ProviderError(self.error(
                        AiProviderErrorCategory::MalformedResponse,
                        "the provider returned too many response deltas",
                        AiRecoveryAction::Retry,
                    ));
                };
                sequence = next;
            }
            if event.finished {
                return AiCompletionTerminal::Done { usage };
            }
        }
    }

    fn forward(
        &self,
        request: &AiCompletionRequest,
        event: &ProviderEvent,
        sequence: u32,
        response_bytes: &mut usize,
        sink: &mut dyn AiEventSink,
    ) -> Result<(), AiCompletionTerminal> {
        *response_bytes = response_bytes.saturating_add(event.text.len());
        if *response_bytes > request.parameters.max_output_bytes as usize
            || *response_bytes > MAX_AI_RESPONSE_BYTES
        {
            return Err(AiCompletionTerminal::ProviderError(self.error(
                AiProviderErrorCategory::MalformedResponse,
                "the provider response exceeded the configured output limit",
                AiRecoveryAction::ReduceRequest,
            )));
        }
        let delta = AiCompletionDelta {
            request_id: request.request_id.clone(),
            sequence,
            text: event.text.clone(),
        };
        if delta.validate().is_err() {
            return Err(AiCompletionTerminal::ProviderError(self.error(
                AiProviderErrorCategory::MalformedResponse,
                "the provider returned an invalid response delta",
                AiRecoveryAction::Retry,
            )));
        }
        if sink.send_delta(delta).is_err() {
            return Err(AiCompletionTerminal::Cancelled);
        }
        Ok(())
    }

    fn error(
        &self,
        category: AiProviderErrorCategory,
        message: &str,
        recovery_action: AiRecoveryAction,
    ) -> AiProviderError {
        AiProviderError::new(self.kind.id(), category, message, recovery_action)
    }

    /// Maps provider HTTP status onto the distinct, actionable states an
    /// application renders. Response bodies stay out: they can echo the request
    /// and, for some providers, the key prefix. Status alone cannot separate
    /// every condition — a 404 may be a wrong endpoint rather than a missing
    /// model — so these are the safe reading, not a diagnosis.
    fn status_error(&self, status: StatusCode) -> AiProviderError {
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => self.error(
                AiProviderErrorCategory::InvalidCredential,
                "the provider rejected this API key",
                AiRecoveryAction::ConfigureCredential,
            ),
            StatusCode::PAYMENT_REQUIRED => self.error(
                AiProviderErrorCategory::QuotaExhausted,
                "this provider account has no remaining credit",
                AiRecoveryAction::ContactProvider,
            ),
            StatusCode::TOO_MANY_REQUESTS => self.error(
                AiProviderErrorCategory::RateLimited,
                "the provider is rate limiting this key",
                AiRecoveryAction::Retry,
            ),
            StatusCode::NOT_FOUND => self.error(
                AiProviderErrorCategory::UnavailableProvider,
                "the provider does not offer this model to this key",
                AiRecoveryAction::ChooseDifferentModel,
            ),
            StatusCode::PAYLOAD_TOO_LARGE => self.error(
                AiProviderErrorCategory::RejectedRequest,
                "the request was larger than the provider accepts",
                AiRecoveryAction::ReduceRequest,
            ),
            status if status.is_server_error() => self.error(
                AiProviderErrorCategory::UnavailableProvider,
                "the provider reported a server-side failure",
                AiRecoveryAction::CheckProviderStatus,
            ),
            _ => self.error(
                AiProviderErrorCategory::RejectedRequest,
                "the provider rejected the request",
                AiRecoveryAction::ReduceRequest,
            ),
        }
    }

    /// A read timeout that surfaces here is reported as a transport failure
    /// rather than the `Timeout` terminal. That is the extracted behavior; the
    /// terminal is reserved for the deadline the adapter observes itself.
    fn transport_error(&self, error: &reqwest::Error) -> AiProviderError {
        if error.is_timeout() {
            return self.error(
                AiProviderErrorCategory::TransportFailure,
                "the provider did not respond in time",
                AiRecoveryAction::Retry,
            );
        }
        if error.is_connect() {
            return self.error(
                AiProviderErrorCategory::TransportFailure,
                "the provider could not be reached. Check your network connection.",
                AiRecoveryAction::Retry,
            );
        }
        self.error(
            AiProviderErrorCategory::TransportFailure,
            "the request to the provider failed",
            AiRecoveryAction::Retry,
        )
    }
}

impl AiComplete for RemoteAiProvider {
    fn complete(
        &self,
        request: &AiCompletionRequest,
        cancellation: &AiCancellation,
        sink: &mut dyn AiEventSink,
    ) -> AiCompletionTerminal {
        if request.validate().is_err()
            || request.provider_id != self.kind.id()
            || !self.supports_model(&request.model_id)
            || crate::unsupported::carries_untransmitted_fields(request)
        {
            return AiCompletionTerminal::ProviderError(self.error(
                AiProviderErrorCategory::RejectedRequest,
                "the completion request is not valid for this provider",
                AiRecoveryAction::ReduceRequest,
            ));
        }
        if cancellation.is_cancelled() {
            return AiCompletionTerminal::Cancelled;
        }
        // Resolving the credential first means an unconfigured or refused
        // provider terminalizes before any socket is opened. The resolver may
        // block — a keyring prompt — so cancellation is rechecked after it.
        let credential = match self.credentials.resolve(self.kind.id()) {
            Ok(credential) => credential,
            Err(error) => {
                return AiCompletionTerminal::ProviderError(
                    error.into_provider_error(self.kind.id()),
                );
            }
        };
        if cancellation.is_cancelled() {
            return AiCompletionTerminal::Cancelled;
        }
        self.stream_completion(request, &credential, cancellation, sink)
    }
}

#[cfg(test)]
mod tests;
