//! Descriptor rows and per-dialect request/response shapes.
//!
//! One row per provider the SDK actually ships. A row describes an endpoint,
//! an authentication mode, and the listing behavior the provider documents —
//! nothing that would let arbitrary runtime input name a destination. Adding a
//! row is only sufficient when its fixtures prove the shared path fits.

use ai_core::{AiCompletionRequest, AiUsage};
use reqwest::{Url, blocking::RequestBuilder};
use serde_json::{Value, json};

use crate::{
    credentials::AiCredential,
    listing::{AiModelListing, AiModelSource, MAX_AI_CONTEXT_TOKENS},
};

/// Identifier of the shipped Google Gemini registration.
pub const GEMINI_PROVIDER_ID: &str = "gemini";
/// Identifier of the shipped Groq registration.
pub const GROQ_PROVIDER_ID: &str = "groq";
/// Identifier of the shipped DeepSeek registration.
pub const DEEPSEEK_PROVIDER_ID: &str = "deepseek";
/// Identifier of the shipped Moonshot registration.
pub const MOONSHOT_PROVIDER_ID: &str = "moonshot";
/// Identifier of the shipped Z.ai registration.
pub const ZAI_PROVIDER_ID: &str = "zai";
/// Identifier of the shipped Alibaba DashScope registration.
pub const DASHSCOPE_PROVIDER_ID: &str = "dashscope";
/// Identifier of the shipped AI/ML API registration.
pub const AIMLAPI_PROVIDER_ID: &str = "aimlapi";

/// One parsed stream frame, normalized away from provider syntax.
///
/// The shape permits states the protocols cannot produce together — text with
/// `finished`, usage on an otherwise empty frame — and is kept as extracted so
/// the adapter's behavior is unchanged. ADR 0003 §2 records the typed
/// replacement as a later, separately approved change.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ProviderEvent {
    pub(crate) text: String,
    pub(crate) usage: Option<AiUsage>,
    pub(crate) finished: bool,
}

/// Every provider except Gemini speaks the OpenAI chat-completions dialect, so
/// each is one row here and the request, stream, and model-listing code is
/// shared. The paths are relative to the base URL, which always ends in `/`.
struct OpenAiCompatible {
    id: &'static str,
    label: &'static str,
    destination: &'static str,
    base_url: &'static str,
    chat_path: &'static str,
    /// `None` when the provider publishes no model-listing endpoint (Z.ai);
    /// its models then come only from the application's catalog.
    models_path: Option<&'static str>,
    /// Whether the provider accepts `stream_options.include_usage`. Providers
    /// that reject unknown parameters would fail the whole request over it.
    stream_usage_option: bool,
}

/// A shipped provider descriptor.
///
/// Vendor identity, not registration identity: an application may register the
/// same vendor twice under different ids with different credential bindings.
/// [`Self::id`] is the default registration id, not a uniqueness rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RemoteProviderKind {
    /// Google Gemini, `generateContent` dialect.
    Gemini,
    /// Groq.
    Groq,
    /// DeepSeek.
    DeepSeek,
    /// Moonshot Kimi.
    Moonshot,
    /// Z.ai GLM.
    Zai,
    /// Alibaba Qwen via DashScope.
    DashScope,
    /// AI/ML API.
    AimlApi,
}

impl RemoteProviderKind {
    /// Every shipped descriptor.
    pub const ALL: [Self; 7] = [
        Self::Gemini,
        Self::Groq,
        Self::DeepSeek,
        Self::Moonshot,
        Self::Zai,
        Self::DashScope,
        Self::AimlApi,
    ];

    fn openai_compatible(self) -> Option<&'static OpenAiCompatible> {
        match self {
            Self::Gemini => None,
            Self::Groq => Some(&OpenAiCompatible {
                id: GROQ_PROVIDER_ID,
                label: "Groq",
                destination: "api.groq.com",
                base_url: "https://api.groq.com/",
                chat_path: "openai/v1/chat/completions",
                models_path: Some("openai/v1/models"),
                stream_usage_option: true,
            }),
            Self::DeepSeek => Some(&OpenAiCompatible {
                id: DEEPSEEK_PROVIDER_ID,
                label: "DeepSeek",
                destination: "api.deepseek.com",
                base_url: "https://api.deepseek.com/",
                chat_path: "chat/completions",
                models_path: Some("models"),
                stream_usage_option: true,
            }),
            Self::Moonshot => Some(&OpenAiCompatible {
                id: MOONSHOT_PROVIDER_ID,
                label: "Moonshot Kimi",
                destination: "api.moonshot.ai",
                base_url: "https://api.moonshot.ai/",
                chat_path: "v1/chat/completions",
                models_path: Some("v1/models"),
                stream_usage_option: true,
            }),
            Self::Zai => Some(&OpenAiCompatible {
                id: ZAI_PROVIDER_ID,
                label: "Z.ai GLM",
                destination: "api.z.ai",
                base_url: "https://api.z.ai/",
                chat_path: "api/paas/v4/chat/completions",
                models_path: None,
                stream_usage_option: false,
            }),
            Self::DashScope => Some(&OpenAiCompatible {
                id: DASHSCOPE_PROVIDER_ID,
                label: "Alibaba Qwen",
                destination: "dashscope-intl.aliyuncs.com",
                base_url: "https://dashscope-intl.aliyuncs.com/",
                chat_path: "compatible-mode/v1/chat/completions",
                models_path: Some("compatible-mode/v1/models"),
                stream_usage_option: true,
            }),
            Self::AimlApi => Some(&OpenAiCompatible {
                id: AIMLAPI_PROVIDER_ID,
                label: "AI/ML API",
                destination: "api.aimlapi.com",
                base_url: "https://api.aimlapi.com/",
                chat_path: "v1/chat/completions",
                models_path: Some("v1/models"),
                stream_usage_option: true,
            }),
        }
    }

    /// The default registration id.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self.openai_compatible() {
            Some(provider) => provider.id,
            None => GEMINI_PROVIDER_ID,
        }
    }

    /// Display name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self.openai_compatible() {
            Some(provider) => provider.label,
            None => "Google Gemini",
        }
    }

    /// The host every request reaches on the default endpoint.
    ///
    /// An application's privacy disclosure names this. It must stay in step
    /// with the host requests actually reach, which the adapter enforces per
    /// request rather than trusting.
    #[must_use]
    pub fn destination(self) -> &'static str {
        match self.openai_compatible() {
            Some(provider) => provider.destination,
            None => "generativelanguage.googleapis.com",
        }
    }

    /// Resolves a descriptor from its default registration id.
    #[must_use]
    pub fn from_id(provider_id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == provider_id)
    }

    /// Whether the provider publishes a model listing at all.
    ///
    /// `false` is not an empty listing: it means asking is unsupported, and
    /// [`RemoteAiProvider::list_models`] refuses without a request.
    ///
    /// [`RemoteAiProvider::list_models`]: super::RemoteAiProvider::list_models
    #[must_use]
    pub fn supports_model_listing(self) -> bool {
        match self.openai_compatible() {
            Some(provider) => provider.models_path.is_some(),
            None => true,
        }
    }

    #[must_use]
    pub(crate) fn default_base_url(self) -> &'static str {
        match self.openai_compatible() {
            Some(provider) => provider.base_url,
            None => "https://generativelanguage.googleapis.com/",
        }
    }

    pub(crate) fn endpoint(self, base: &Url, model_id: &str, streaming: bool) -> Option<Url> {
        match self.openai_compatible() {
            Some(provider) => base.join(provider.chat_path).ok(),
            None => {
                let method = if streaming {
                    "streamGenerateContent?alt=sse"
                } else {
                    "generateContent"
                };
                base.join(&format!("v1beta/models/{model_id}:{method}"))
                    .ok()
            }
        }
    }

    pub(crate) fn models_endpoint(self, base: &Url) -> Option<Url> {
        match self.openai_compatible() {
            Some(provider) => base.join(provider.models_path?).ok(),
            None => base.join("v1beta/models?pageSize=1000").ok(),
        }
    }

    /// Attaches the key to a single outbound request. The credential is never
    /// stored on the builder beyond this call and never enters a URL, so it
    /// cannot reach a redirect target, a log line, or an error message.
    pub(crate) fn authorize(
        self,
        builder: RequestBuilder,
        credential: &AiCredential,
    ) -> RequestBuilder {
        match self {
            Self::Gemini => builder.header("x-goog-api-key", credential.expose()),
            _ => builder.bearer_auth(credential.expose()),
        }
    }

    pub(crate) fn completion_body(self, request: &AiCompletionRequest, streaming: bool) -> Value {
        let temperature = fraction(request.parameters.temperature_millis);
        let top_p = fraction(request.parameters.top_p_millis);
        match self.openai_compatible() {
            Some(provider) => {
                let mut messages = Vec::with_capacity(2);
                if !request.system_prompt.is_empty() {
                    messages.push(json!({"role": "system", "content": request.system_prompt}));
                }
                messages.push(json!({"role": "user", "content": request.user_prompt}));
                let mut body = json!({
                    "model": request.model_id,
                    "messages": messages,
                    "stream": streaming,
                });
                if streaming && provider.stream_usage_option {
                    body["stream_options"] = json!({"include_usage": true});
                }
                if let Some(temperature) = temperature {
                    body["temperature"] = json!(temperature);
                }
                if let Some(top_p) = top_p {
                    body["top_p"] = json!(top_p);
                }
                body
            }
            None => {
                let mut generation_config = json!({});
                if let Some(temperature) = temperature {
                    generation_config["temperature"] = json!(temperature);
                }
                if let Some(top_p) = top_p {
                    generation_config["topP"] = json!(top_p);
                }
                let mut body = json!({
                    "contents": [{"role": "user", "parts": [{"text": request.user_prompt}]}],
                    "generationConfig": generation_config,
                });
                if !request.system_prompt.is_empty() {
                    body["systemInstruction"] = json!({"parts": [{"text": request.system_prompt}]});
                }
                body
            }
        }
    }

    pub(crate) fn verification_body(self, model_id: &str) -> Value {
        match self {
            Self::Gemini => json!({
                "contents": [{"role": "user", "parts": [{"text": "ping"}]}],
                "generationConfig": {"maxOutputTokens": 1},
            }),
            _ => json!({
                "model": model_id,
                "messages": [{"role": "user", "content": "ping"}],
                "max_tokens": 1,
                "stream": false,
            }),
        }
    }

    /// Translates one provider model-listing response into validated fetched
    /// listings. `None` means the payload was not the documented shape;
    /// individual entries that fail validation are dropped instead of failing
    /// the whole listing, so one odd model id cannot hide the rest.
    pub(crate) fn parse_model_listing(self, payload: &str) -> Option<Vec<AiModelListing>> {
        let value: Value = serde_json::from_str(payload).ok()?;
        if value.get("error").is_some() {
            return None;
        }
        let entries = match self {
            Self::Gemini => value.get("models")?.as_array()?,
            _ => value.get("data")?.as_array()?,
        };
        let listings = entries
            .iter()
            .filter_map(|entry| self.parse_model_entry(entry))
            .filter(|listing| listing.validate().is_ok())
            .collect();
        Some(listings)
    }

    fn parse_model_entry(self, entry: &Value) -> Option<AiModelListing> {
        let (model_id, label, context_window_tokens) = match self {
            Self::Gemini => {
                let supports_completion = entry
                    .get("supportedGenerationMethods")
                    .and_then(Value::as_array)
                    .is_some_and(|methods| {
                        methods
                            .iter()
                            .any(|method| method.as_str() == Some("generateContent"))
                    });
                if !supports_completion {
                    return None;
                }
                let name = entry.get("name").and_then(Value::as_str)?;
                let model_id = name.strip_prefix("models/").unwrap_or(name);
                let label = entry
                    .get("displayName")
                    .and_then(Value::as_str)
                    .unwrap_or(model_id);
                let window = entry.get("inputTokenLimit").and_then(Value::as_u64);
                (model_id.to_owned(), label.to_owned(), window)
            }
            _ => {
                if entry.get("active").and_then(Value::as_bool) == Some(false) {
                    return None;
                }
                // Aggregator listings mix chat with image, audio, and video
                // models under a `type` field; plain OpenAI-style listings
                // carry no `type` and pass through.
                if entry
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind != "openai/chat-completions")
                {
                    return None;
                }
                let model_id = entry.get("id").and_then(Value::as_str)?.to_owned();
                let window = entry
                    .get("context_window")
                    .or_else(|| entry.get("context_length"))
                    .or_else(|| entry.get("info").and_then(|info| info.get("contextLength")))
                    .and_then(Value::as_u64);
                (model_id.clone(), model_id, window)
            }
        };
        Some(AiModelListing {
            provider_id: self.id().to_owned(),
            model_id,
            label,
            context_window_tokens: context_window_tokens
                .filter(|window| *window <= u64::from(MAX_AI_CONTEXT_TOKENS))
                .and_then(|window| u32::try_from(window).ok()),
            input_price_micros_per_mtok: None,
            output_price_micros_per_mtok: None,
            source: AiModelSource::Fetched,
        })
    }

    /// Translates one provider payload into the neutral event shape. `None`
    /// means the payload was not the documented shape and the stream is
    /// malformed; an empty event means "nothing to forward yet".
    pub(crate) fn parse_event(self, payload: &str) -> Option<ProviderEvent> {
        let value: Value = serde_json::from_str(payload).ok()?;
        if value.get("error").is_some() {
            return None;
        }
        match self {
            Self::Gemini => {
                let mut event = ProviderEvent::default();
                if let Some(parts) = value
                    .get("candidates")
                    .and_then(Value::as_array)
                    .and_then(|candidates| candidates.first())
                    .and_then(|candidate| candidate.get("content"))
                    .and_then(|content| content.get("parts"))
                    .and_then(Value::as_array)
                {
                    for part in parts {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            event.text.push_str(text);
                        }
                    }
                }
                event.usage = value.get("usageMetadata").and_then(|usage| {
                    bounded_usage(
                        usage.get("promptTokenCount").and_then(Value::as_u64),
                        usage.get("candidatesTokenCount").and_then(Value::as_u64),
                    )
                });
                Some(event)
            }
            _ => {
                let mut event = ProviderEvent::default();
                if let Some(choice) = value
                    .get("choices")
                    .and_then(Value::as_array)
                    .and_then(|choices| choices.first())
                {
                    if let Some(text) = choice
                        .get("delta")
                        .and_then(|delta| delta.get("content"))
                        .and_then(Value::as_str)
                    {
                        event.text.push_str(text);
                    }
                    event.finished = choice
                        .get("finish_reason")
                        .is_some_and(|reason| !reason.is_null());
                }
                event.usage = value.get("usage").and_then(|usage| {
                    bounded_usage(
                        usage.get("prompt_tokens").and_then(Value::as_u64),
                        usage.get("completion_tokens").and_then(Value::as_u64),
                    )
                });
                Some(event)
            }
        }
    }
}

fn fraction(millis: Option<u16>) -> Option<f32> {
    millis.map(|value| f32::from(value) / 1_000.0)
}

/// Partial usage is no usage: a provider that reports only one counter has not
/// reported a total, and completing it with a zero would invent a number the
/// application would then price.
fn bounded_usage(input_tokens: Option<u64>, output_tokens: Option<u64>) -> Option<AiUsage> {
    let usage = AiUsage {
        input_tokens: input_tokens?,
        output_tokens: output_tokens?,
    };
    usage.validate().ok().map(|()| usage)
}
