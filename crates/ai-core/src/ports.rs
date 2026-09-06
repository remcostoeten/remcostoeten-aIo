//! In-process ports: cancellation, delta sink, provider, delivery channel.
//!
//! None of these are serializable, and none should acquire a serialized form.
//! A TypeScript implementation expresses the same semantics with `AsyncIterable`
//! and `AbortSignal` rather than trying to serialize a callback or a trait
//! object. See `docs/contracts.md` §2.6.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use thiserror::Error;

use crate::contracts::{
    AiCompletionDelta, AiCompletionEvent, AiCompletionRequest, AiCompletionTerminal,
};

/// A shared, clonable cancellation flag.
///
/// Cancellation is cooperative. Setting the flag asks a provider to stop at its
/// next check; it cannot interrupt a blocked read or a provider that never
/// returns.
#[derive(Debug, Clone, Default)]
pub struct AiCancellation {
    cancelled: Arc<AtomicBool>,
}

impl AiCancellation {
    /// Creates an uncancelled flag.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Requests cancellation. Idempotent, and never un-set.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

/// The consumer is gone and no further event can be delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AiSinkError {
    /// The event consumer is closed.
    #[error("completion event consumer is closed")]
    Closed,
}

/// Where a provider writes its output deltas.
///
/// Providers receive deltas only. The terminal is published by the service, so
/// a provider cannot emit two terminals or bypass commitment.
pub trait AiEventSink: Send {
    /// Publishes one delta.
    ///
    /// # Errors
    ///
    /// [`AiSinkError::Closed`] when the consumer is gone, or when the delta
    /// broke the stream contract. Either way the provider should stop and
    /// return; the service decides which terminal that becomes.
    fn send_delta(&mut self, delta: AiCompletionDelta) -> Result<(), AiSinkError>;
}

/// The one completion seam.
///
/// Implementations are registered by the application under an identifier of its
/// choosing. The trait deliberately has no `id`, no verification, no model
/// listing, and no credential dependency: administration is a separate typed
/// capability, so a fake or local provider never has to supply dummy
/// credentials or pretend an unsupported listing succeeded.
pub trait AiComplete: Send + Sync {
    /// Runs one completion to a terminal, streaming deltas into `sink`.
    ///
    /// Implementations must emit deltas carrying the request's own id with
    /// sequence numbers starting at zero and increasing by one, must respect
    /// the request's output budget, and should poll `cancellation` often enough
    /// to stop promptly.
    fn complete(
        &self,
        request: &AiCompletionRequest,
        cancellation: &AiCancellation,
        sink: &mut dyn AiEventSink,
    ) -> AiCompletionTerminal;
}

/// The application's delivery port for completion events.
///
/// This is where events leave the SDK — a Tauri channel, a queue, a websocket,
/// a test collector. The core has no opinion beyond the send result.
pub trait AiCompletionChannel: Send + Sync + 'static {
    /// Delivers one event.
    ///
    /// # Errors
    ///
    /// [`AiSinkError::Closed`] when the consumer is gone. A failed delta send
    /// requests cancellation; a failed terminal send is not retried.
    fn send(&self, event: AiCompletionEvent) -> Result<(), AiSinkError>;
}
