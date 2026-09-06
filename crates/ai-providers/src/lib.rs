//! HTTP provider adapters for the `ai-core` completion seam.
//!
//! `ai-core` owns the contracts and the run lifecycle. This crate owns what it
//! takes to reach a real provider: endpoints, authentication, framing, and the
//! mapping from provider failures onto the core's typed vocabulary. Nothing
//! here understands a product.
//!
//! ```no_run
//! use std::sync::Arc;
//! use ai_core::{AiComplete, AiCompletionService};
//! use ai_providers::{
//!     AiCredential, AiCredentialError, AiCredentialSource, AiModelAuthority, RemoteAiProvider,
//!     RemoteProviderKind,
//! };
//!
//! struct EnvironmentKey;
//! impl AiCredentialSource for EnvironmentKey {
//!     fn resolve(&self, _provider_id: &str) -> Result<AiCredential, AiCredentialError> {
//!         AiCredential::new(std::env::var("GROQ_API_KEY").unwrap_or_default())
//!     }
//! }
//!
//! struct OneModel;
//! impl AiModelAuthority for OneModel {
//!     fn permits(&self, provider_id: &str, model_id: &str) -> bool {
//!         provider_id == "groq" && model_id == "openai/gpt-oss-20b"
//!     }
//! }
//!
//! let provider = RemoteAiProvider::new(
//!     RemoteProviderKind::Groq,
//!     Arc::new(EnvironmentKey),
//!     Arc::new(OneModel),
//!     "example-app/1.0",
//! )?;
//! let service = AiCompletionService::new([("groq".to_owned(), Arc::new(provider) as Arc<dyn AiComplete>)]);
//! # Ok::<(), ai_providers::RemoteAiSetupError>(())
//! ```
//!
//! # What an adapter guarantees
//!
//! Ordered deltas carrying the request's own id, one returned terminal, the
//! request's output budget respected, and no network access before the request
//! validates, the application authorizes the model, and a credential resolves.
//!
//! # What it does not
//!
//! No retries and no key rotation: a resolver called once cannot react to a
//! later 429, and rotation needs an execution owner the SDK does not have. No
//! fallback between providers, and never from a local endpoint to a remote one.
//! No structured-output strategy. `timeoutMs` is observed by the adapter, not
//! enforced by a watchdog, and cancellation cannot interrupt a blocked read.

mod authority;
mod credentials;
mod listing;

#[cfg(any(feature = "ollama", feature = "remote"))]
mod http;

#[cfg(feature = "ollama")]
mod ollama;
#[cfg(feature = "remote")]
mod remote;

#[cfg(all(test, any(feature = "ollama", feature = "remote")))]
mod fixtures;

pub use authority::AiModelAuthority;
pub use credentials::{
    AiCredential, AiCredentialError, AiCredentialRefusal, AiCredentialSource, MAX_AI_API_KEY_BYTES,
    MIN_AI_API_KEY_BYTES,
};
pub use listing::{
    AiModelListing, AiModelListingError, AiModelSource, MAX_AI_CONTEXT_TOKENS,
    MAX_AI_MODEL_LABEL_BYTES, MAX_AI_MODEL_LISTINGS, MAX_AI_PRICE_MICROS, valid_model_identifier,
};

#[cfg(feature = "ollama")]
pub use ollama::{OLLAMA_PROVIDER_ID, OllamaProvider, OllamaSetupError};

#[cfg(feature = "remote")]
pub use remote::{
    AIMLAPI_PROVIDER_ID, DASHSCOPE_PROVIDER_ID, DEEPSEEK_PROVIDER_ID, GEMINI_PROVIDER_ID,
    GROQ_PROVIDER_ID, MOONSHOT_PROVIDER_ID, RemoteAiProvider, RemoteAiSetupError,
    RemoteProviderKind, ZAI_PROVIDER_ID,
};

#[cfg(feature = "schema-tool")]
pub mod schema;
