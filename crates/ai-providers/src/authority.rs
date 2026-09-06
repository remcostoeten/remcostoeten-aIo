//! The model authorization port.

/// Which models an adapter may address.
///
/// This is authorization, not syntax. An id that parses, that a provider
/// listing mentioned, or that a model name resembles is still not permitted
/// until the application says so. Adapters consult the authority before
/// resolving a credential, so an unpermitted model never reaches the network.
pub trait AiModelAuthority: Send + Sync {
    /// Whether `model_id` may be addressed on `provider_id`.
    fn permits(&self, provider_id: &str, model_id: &str) -> bool;
}
