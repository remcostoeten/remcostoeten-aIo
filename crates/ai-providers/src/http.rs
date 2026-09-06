//! The small amount of HTTP behavior every adapter shares.
//!
//! Deliberately not a dialect framework: framing, request bodies, and error
//! mapping differ per protocol and stay with their adapter. What lives here is
//! what would otherwise be re-implemented inconsistently — client construction,
//! bounded body draining, and the SSE data-line helper.

use std::time::Duration;

#[cfg(feature = "remote")]
use std::io::Read;

#[cfg(feature = "remote")]
use reqwest::{Url, blocking::Response};
use reqwest::{blocking::Client, redirect};

#[cfg(feature = "remote")]
const SSE_DATA_PREFIX: &str = "data:";
/// The sentinel an OpenAI-compatible stream sends instead of a final frame.
#[cfg(feature = "remote")]
pub(crate) const SSE_DONE_PAYLOAD: &str = "[DONE]";

/// Builds a client that never follows a redirect.
///
/// Redirects are refused rather than followed because credentials travel in
/// headers and `reqwest` only strips the ones it recognizes: a provider header
/// such as `x-goog-api-key` would be replayed verbatim to whatever origin a
/// 3xx names. A redirect therefore surfaces as its status code instead. See
/// `docs/extraction-inventory-providers.md` P1.
pub(crate) fn client(
    user_agent: &str,
    connect_timeout: Duration,
    request_timeout: Option<Duration>,
) -> Option<Client> {
    let mut builder = Client::builder()
        .connect_timeout(connect_timeout)
        .redirect(redirect::Policy::none())
        .user_agent(user_agent.to_owned());
    if let Some(request_timeout) = request_timeout {
        builder = builder.timeout(request_timeout);
    }
    builder.build().ok()
}

#[cfg(feature = "remote")]
/// Reads and discards a body under a byte cap.
///
/// Error bodies can echo the request and, for some providers, a key prefix, so
/// they are drained rather than read: draining lets the connection be reused
/// and guarantees nothing from the body reaches a contract type.
pub(crate) fn discard_body(response: Response, cap: u64) {
    let mut sink = Vec::new();
    let _ = response.take(cap).read_to_end(&mut sink);
}

#[cfg(feature = "remote")]
/// Extracts the payload of an SSE `data:` line.
///
/// This is a line helper, not an SSE decoder: it does not join multi-line data
/// fields, track event names, or treat a blank line as a dispatch boundary. It
/// is what the extracted providers rely on, and it is correct for their
/// single-line frames only. See ADR 0003 §2.
pub(crate) fn sse_payload(line: &str) -> Option<&str> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let payload = trimmed.strip_prefix(SSE_DATA_PREFIX)?.trim_start();
    (!payload.is_empty()).then_some(payload)
}

#[cfg(feature = "remote")]
/// Whether a built URL still points at the host the adapter promised.
///
/// Every request URL is checked at construction time, not only in tests: path
/// joining takes a model id that ultimately came from an application, and a
/// disclosure that names one destination must not be able to reach another.
pub(crate) fn stays_on_destination(url: &Url, destination: &str) -> bool {
    url.host_str() == Some(destination) && url.username().is_empty() && url.password().is_none()
}

#[cfg(all(test, feature = "remote"))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{sse_payload, stays_on_destination};
    use reqwest::Url;

    #[test]
    fn parses_only_sse_data_frames() {
        assert_eq!(sse_payload("data: {\"a\":1}\n"), Some("{\"a\":1}"));
        assert_eq!(sse_payload("data:[DONE]\r\n"), Some("[DONE]"));
        assert_eq!(sse_payload("event: message\n"), None);
        assert_eq!(sse_payload("\n"), None);
        assert_eq!(sse_payload("data: \n"), None);
    }

    #[test]
    fn refuses_a_url_that_left_the_destination_or_carries_userinfo() {
        let parse = |value: &str| Url::parse(value).expect("url");

        assert!(stays_on_destination(
            &parse("https://api.groq.com/openai/v1/models"),
            "api.groq.com"
        ));
        assert!(!stays_on_destination(
            &parse("https://elsewhere.example/openai/v1/models"),
            "api.groq.com"
        ));
        assert!(!stays_on_destination(
            &parse("https://user:secret@api.groq.com/"),
            "api.groq.com"
        ));
    }
}
