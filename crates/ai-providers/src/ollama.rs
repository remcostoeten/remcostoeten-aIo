//! Ollama generation.
//!
//! Generation only. This adapter never detects, installs, spawns, stops, or
//! auto-starts a process: lifecycle is a separate responsibility that an
//! application composes with this one. An unreachable server is an error here,
//! not a reason to start something.
//!
//! Locality comes from the configured endpoint, never from a model name — and
//! a loopback endpoint proves only where the socket goes, not where inference
//! ultimately happens.

use std::{
    io::{BufRead, BufReader, Read},
    net::IpAddr,
    time::Duration,
};

use ai_core::{
    AiCancellation, AiComplete, AiCompletionDelta, AiCompletionRequest, AiCompletionTerminal,
    AiEventSink, AiProviderError, AiProviderErrorCategory, AiRecoveryAction, AiUsage,
    MAX_AI_RESPONSE_BYTES,
};
use reqwest::{StatusCode, Url, blocking::Client};
use serde::{Deserialize, Serialize};

use crate::http;

/// The registration id this adapter answers to.
pub const OLLAMA_PROVIDER_ID: &str = "ollama";

const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:11434";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(300);

/// A completion provider backed by an Ollama server.
pub struct OllamaProvider {
    endpoint: Url,
    client: Client,
}

/// Why an Ollama provider could not be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OllamaSetupError {
    /// The endpoint did not parse.
    #[error("Ollama endpoint is invalid")]
    InvalidEndpoint,
    /// The endpoint was not loopback HTTP.
    #[error("Ollama endpoint must use localhost or a loopback address")]
    NonLoopbackEndpoint,
    /// No usable HTTP client could be built for this configuration.
    #[error("Ollama transport is unavailable")]
    TransportUnavailable,
}

impl OllamaProvider {
    /// Binds to a loopback Ollama endpoint, defaulting to `127.0.0.1:11434`.
    ///
    /// A non-loopback endpoint is refused rather than accepted quietly. Whether
    /// to allow a remote Ollama is an application privacy decision, and no such
    /// option exists yet.
    ///
    /// # Errors
    ///
    /// [`OllamaSetupError`] when the endpoint is unparseable, is not loopback
    /// HTTP, or the client cannot be built.
    pub fn new(endpoint: Option<&str>, user_agent: &str) -> Result<Self, OllamaSetupError> {
        let endpoint = Url::parse(endpoint.unwrap_or(DEFAULT_ENDPOINT))
            .map_err(|_| OllamaSetupError::InvalidEndpoint)?;
        if !endpoint_is_loopback(&endpoint) {
            return Err(OllamaSetupError::NonLoopbackEndpoint);
        }
        let client = http::client(user_agent, CONNECT_TIMEOUT, Some(CLIENT_TIMEOUT))
            .ok_or(OllamaSetupError::TransportUnavailable)?;
        Ok(Self { endpoint, client })
    }

    /// The bound endpoint.
    #[must_use]
    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    fn error(
        &self,
        category: AiProviderErrorCategory,
        message: &str,
        recovery_action: AiRecoveryAction,
    ) -> AiCompletionTerminal {
        AiCompletionTerminal::ProviderError(AiProviderError::new(
            OLLAMA_PROVIDER_ID,
            category,
            message,
            recovery_action,
        ))
    }
}

impl AiComplete for OllamaProvider {
    fn complete(
        &self,
        request: &AiCompletionRequest,
        cancellation: &AiCancellation,
        sink: &mut dyn AiEventSink,
    ) -> AiCompletionTerminal {
        if request.validate().is_err() || request.provider_id != OLLAMA_PROVIDER_ID {
            return self.error(
                AiProviderErrorCategory::RejectedRequest,
                "Ollama completion request is invalid",
                AiRecoveryAction::ReduceRequest,
            );
        }
        if cancellation.is_cancelled() {
            return AiCompletionTerminal::Cancelled;
        }
        let Ok(url) = self.endpoint.join("/api/generate") else {
            return self.error(
                AiProviderErrorCategory::InternalFailure,
                "Ollama endpoint is invalid",
                AiRecoveryAction::CheckProviderStatus,
            );
        };
        let body = GenerateRequest {
            model: &request.model_id,
            system: &request.system_prompt,
            prompt: &request.user_prompt,
            stream: true,
            options: GenerateOptions {
                temperature: fraction(request.parameters.temperature_millis),
                top_p: fraction(request.parameters.top_p_millis),
            },
        };
        let response = match self
            .client
            .post(url)
            .timeout(Duration::from_millis(u64::from(
                request.parameters.timeout_ms,
            )))
            .json(&body)
            .send()
        {
            Ok(response) if response.status().is_success() => response,
            Ok(response) if response.status() == StatusCode::NOT_FOUND => {
                return self.error(
                    AiProviderErrorCategory::UnavailableProvider,
                    "Ollama model is not installed",
                    AiRecoveryAction::ChooseDifferentModel,
                );
            }
            Ok(_) => {
                return self.error(
                    AiProviderErrorCategory::TransportFailure,
                    "Ollama rejected the completion request",
                    AiRecoveryAction::CheckProviderStatus,
                );
            }
            Err(error) if error.is_timeout() => return AiCompletionTerminal::Timeout,
            Err(_) => {
                return self.error(
                    AiProviderErrorCategory::UnavailableProvider,
                    "Ollama is not reachable",
                    AiRecoveryAction::CheckProviderStatus,
                );
            }
        };
        let mut reader = BufReader::new(response.take(MAX_AI_RESPONSE_BYTES as u64 + 1));
        let mut line = String::new();
        let mut sequence = 0u32;
        let mut response_bytes = 0usize;
        loop {
            if cancellation.is_cancelled() {
                return AiCompletionTerminal::Cancelled;
            }
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    return self.error(
                        AiProviderErrorCategory::MalformedResponse,
                        "Ollama ended the response without a terminal event",
                        AiRecoveryAction::Retry,
                    );
                }
                Ok(_) => {}
                Err(_) => {
                    return self.error(
                        AiProviderErrorCategory::TransportFailure,
                        "Ollama response stream failed",
                        AiRecoveryAction::Retry,
                    );
                }
            }
            let Ok(event) = serde_json::from_str::<GenerateResponse>(line.trim()) else {
                return self.error(
                    AiProviderErrorCategory::MalformedResponse,
                    "Ollama returned malformed completion data",
                    AiRecoveryAction::Retry,
                );
            };
            if !event.response.is_empty() {
                response_bytes = response_bytes.saturating_add(event.response.len());
                if response_bytes > request.parameters.max_output_bytes as usize
                    || response_bytes > MAX_AI_RESPONSE_BYTES
                {
                    cancellation.cancel();
                    return self.error(
                        AiProviderErrorCategory::MalformedResponse,
                        "Ollama response exceeded the output limit",
                        AiRecoveryAction::ReduceRequest,
                    );
                }
                let delta = AiCompletionDelta {
                    request_id: request.request_id.clone(),
                    sequence,
                    text: event.response,
                };
                if delta.validate().is_err() || sink.send_delta(delta).is_err() {
                    cancellation.cancel();
                    return AiCompletionTerminal::Cancelled;
                }
                let Some(next) = sequence.checked_add(1) else {
                    return self.error(
                        AiProviderErrorCategory::MalformedResponse,
                        "Ollama returned too many deltas",
                        AiRecoveryAction::Retry,
                    );
                };
                sequence = next;
            }
            if event.done {
                return AiCompletionTerminal::Done {
                    // Partial counts are no usage at all, never a zero the
                    // application would then price. Out-of-bound counts are
                    // forwarded as reported and rejected by the service's own
                    // validation, rather than being silently dropped here.
                    usage: match (event.prompt_eval_count, event.eval_count) {
                        (Some(input_tokens), Some(output_tokens)) => Some(AiUsage {
                            input_tokens,
                            output_tokens,
                        }),
                        _ => None,
                    },
                };
            }
        }
    }
}

#[derive(Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    system: &'a str,
    prompt: &'a str,
    stream: bool,
    options: GenerateOptions,
}

#[derive(Serialize)]
struct GenerateOptions {
    temperature: Option<f32>,
    top_p: Option<f32>,
}

#[derive(Deserialize)]
struct GenerateResponse {
    #[serde(default)]
    response: String,
    #[serde(default)]
    done: bool,
    prompt_eval_count: Option<u64>,
    eval_count: Option<u64>,
}

fn fraction(millis: Option<u16>) -> Option<f32> {
    millis.map(|value| f32::from(value) / 1_000.0)
}

fn endpoint_is_loopback(endpoint: &Url) -> bool {
    endpoint.scheme() == "http"
        && endpoint.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .parse::<IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        })
}

#[cfg(test)]
mod tests;
