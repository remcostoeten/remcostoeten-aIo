# Phase 2 extraction inventory

Source: Skriuw at revision `64827f5e81d097321715e789c9fcd795303c1595`, read-only
and unmodified. Destination: `crates/ai-providers`. This file records what
moved, what changed, what deliberately stayed behind, and which source defects
were preserved rather than quietly fixed, so Phase 3 can diff intent against
reality.

The Phase 1 inventory for `crates/ai-core` is `docs/extraction-inventory.md`.

## Symbols

### From `crates/skriuw-ai-remote/src/lib.rs`

| Source symbol | Destination | Change |
| --- | --- | --- |
| `RemoteAiProvider` + `new` | `remote::RemoteAiProvider` | authority and user agent are now required arguments (P2, P7) |
| `with_model_authority` | — | folded into `new`; there is no default authority to override |
| `with_base_url` | `remote` (private) | stays crate-private: a public endpoint override would let request input redirect a stored key (ADR 0003 §4) |
| `kind`, `supports_model` | `remote` | `supports_model` additionally requires a path-safe id (P5) |
| `list_models` | `remote` | destination re-checked before the request (P5) |
| `verify_credential` | `remote` | destination re-checked before the request (P5) |
| `stream_completion`, `forward` | `remote` | unchanged apart from P1/P4/P5 |
| `error`, `status_error`, `transport_error` | `remote` | messages neutralized of the product name |
| `RemoteAiSetupError` | `remote` | now `thiserror`, same variants |
| `sse_payload`, `SSE_DONE_PAYLOAD` | `http` | none |
| `MAX_STREAM_EVENT_BYTES`, `MAX_MODEL_LIST_RESPONSE_BYTES`, `CONNECT_TIMEOUT` | `remote` | none |
| `MAX_VERIFICATION_RESPONSE_BYTES` | `remote::MAX_DISCARDED_RESPONSE_BYTES` | renamed; it bounds every discarded body, not only verification |
| `VERIFICATION_TIMEOUT` | `remote::ADMINISTRATION_TIMEOUT` | renamed; it covers listing too |
| `RemoteAiModelAuthority` | `authority::AiModelAuthority` | renamed; unchanged shape |
| `CatalogModelAuthority`, `remote_ai_catalog`, `CATALOG_SOURCE`, `models.json` | — | the catalog is application-owned product data |

### From `crates/skriuw-ai-remote/src/provider.rs`

| Source symbol | Destination | Change |
| --- | --- | --- |
| `RemoteProviderKind` + all seven rows | `remote::descriptor` | `#[non_exhaustive]`; row data byte-identical |
| `OpenAiCompatible` | `remote::descriptor` | none |
| `ProviderEvent` | `remote::descriptor` | none — see H5 |
| `id`, `label`, `destination`, `from_id`, `ALL`, `default_base_url` | `remote::descriptor` | none |
| `endpoint`, `models_endpoint`, `supports_model_listing` | `remote::descriptor` | none |
| `authorize`, `completion_body`, `verification_body` | `remote::descriptor` | none |
| `parse_model_listing`, `parse_model_entry`, `parse_event` | `remote::descriptor` | none |
| `fraction`, `bounded_usage` | `remote::descriptor` | none |
| provider id constants (7) | `remote::descriptor` | none |

### From `crates/skriuw-ai-ollama/src/lib.rs`

Only generation. Detection, installation, spawning, stopping, status, model
pulls, removals, progress, and shutdown are Phase 6 and stay in Skriuw.

| Source symbol | Destination | Change |
| --- | --- | --- |
| `impl AiComplete for OllamaRuntime` | `ollama::OllamaProvider` | P4; usage handling unchanged, see H7 |
| `GenerateRequest`, `GenerateOptions`, `GenerateResponse` | `ollama` | none |
| `endpoint_is_loopback`, `DEFAULT_ENDPOINT` | `ollama` | none |
| `api_url` | inlined as `endpoint.join` | none |
| `OllamaRuntime::new` client settings | `ollama::OllamaProvider::new` | typed `OllamaSetupError` instead of `LocalAiError` (P2, P6) |
| `with_endpoint_override`, `notice`, `detail` | — | status-surface degradation, a lifecycle concern (P6) |
| `LocalAiRuntime` and everything it needs | — | Phase 6 |

### From `crates/skriuw-domain/src/remote_ai.rs`

| Source symbol | Destination | Change |
| --- | --- | --- |
| `AiCredential` + `new` + `expose` + `Debug` + `Drop` | `credentials` | none |
| `MIN_AI_API_KEY_BYTES`, `MAX_AI_API_KEY_BYTES` | `credentials` | none |
| `AiCredentialError` (6 variants) | `credentials::AiCredentialRefusal` (3) + message | P3 |
| `into_provider_error` | `credentials` | same categories and recovery actions (P3) |
| `AiCredentialSource` | `credentials` | none |
| `RemoteAiModelListing` + `validate` | `listing::AiModelListing` | renamed; same fields and rules |
| `RemoteAiModelSource` | `listing::AiModelSource` | renamed; same wire values |
| `valid_provider_identifier` | `listing::valid_model_identifier` | renamed and made public; same grammar |
| `valid_label`, `MAX_REMOTE_AI_LABEL_BYTES`, `MAX_REMOTE_AI_CONTEXT_TOKENS`, `MAX_REMOTE_AI_PRICE_MICROS`, `MAX_REMOTE_AI_CATALOG_MODELS` | `listing` | renamed to `MAX_AI_*`; same values |
| `RemoteAiCatalog`, `RemoteAiModel`, `RemoteAiModelDirectory`, `RemoteAiCatalogError` | — | application-owned catalog data |
| `RemoteAiConsent`, `REMOTE_AI_DISCLOSURE_VERSION`, `CredentialVault*`, `RemoteAiKeyTier`, `RemoteAiProviderState` | — | consent and vault policy stay in Skriuw |

## Behavior changes

Seven, each deliberate and each with a test. Everything else — request bodies,
endpoint paths, headers, framing, status mapping, error categories, recovery
actions, stream bounds, usage handling — is unchanged from the source.

### P1 — redirects are refused, not followed

The source used `reqwest`'s default redirect policy. `reqwest` strips only the
headers it recognizes as sensitive on a cross-origin redirect, so Gemini's
`x-goog-api-key` would have been replayed verbatim to whatever origin a 3xx
named. The client now uses `redirect::Policy::none()`, and a 3xx surfaces
through the ordinary status mapping as `RejectedRequest`. No shipped provider
redirects its API endpoints.

Test: `never_replays_a_credential_to_a_redirect_target`, which asserts the
redirect target is never contacted.

### P2 — the user agent is a construction parameter

The source hard-coded `Skriuw` and `Skriuw local AI`. Product identity is not
the SDK's to send. Skriuw preserves its current user agent by passing it, so no
observable request changes for it.

### P3 — the credential vocabulary is generic, the copy is the application's

`AiCredentialError`'s six variants collapse to three refusals — `Missing`,
`Invalid`, `Unavailable` — carrying an application-supplied message. Consent and
keyring wording, which are Skriuw's disclosure policy, stay in Skriuw. The
resulting `AiProviderError` is identical for every source variant: same
category, same recovery action, same message text when the application passes
its own copy.

Tests: `maps_every_refusal_onto_the_completion_error_vocabulary`,
`application_copy_survives_the_mapping`, `bounds_hostile_application_copy_at_the_boundary`.

### P4 — cancellation is rechecked before the request is sent

Remote credential resolution may block on a keyring prompt; the source did not
recheck cancellation after it. The Ollama adapter checked cancellation only
after the request had been sent, so an already-cancelled run still reached the
server. Both now terminalize as `Cancelled` without opening a socket.

Tests: `stops_before_sending_when_cancellation_arrives_during_credential_resolution`,
`stops_before_sending_when_the_request_is_already_cancelled` (both adapters).

### P5 — endpoint construction is validated, not trusted

The source proved in a test that its shipped rows stay on their disclosed hosts,
and relied on the catalog to keep model ids path-safe. With an
application-supplied authority that is no longer a given, so: a model id must
pass `valid_model_identifier` before it can be addressed, every built URL is
checked against the configured destination, and a base URL carrying userinfo is
refused at construction.

Tests: `refuses_a_path_traversing_model_id_even_when_the_authority_permits_it`,
`refuses_an_endpoint_that_carries_userinfo`,
`every_endpoint_stays_on_the_disclosed_destination`.

### P6 — a non-loopback Ollama endpoint is refused at construction

`OllamaRuntime::new` refused it; `with_endpoint_override` then swallowed the
refusal, fell back to the default endpoint, and reported it in a status detail —
a lifecycle-surface behavior with no completion equivalent. The adapter returns
`OllamaSetupError::NonLoopbackEndpoint` and leaves the fallback decision to the
application. Whether a remote Ollama is ever permitted is an application privacy
decision that has not been made.

### P7 — there is no default model authority

`RemoteAiProvider::new` defaulted to the shipped catalog. The SDK ships no
catalog, so the authority is a required argument. This preserves the source's
ordering — authorization before credential resolution before network — without
importing product data.

## Preserved defects and known holes

Characterized, not fixed. Correcting any of these is a separate approved change
with its own fixtures, per ADR 0003 §6.

- **H1 — inconsistent stream completion.** The remote adapter treats EOF after
  at least one parsed event as `Done`; Ollama requires an explicit `done` flag
  and reports EOF as `MalformedResponse`. Tests:
  `streams_gemini_deltas_and_reports_usage`,
  `fails_visibly_when_the_stream_ends_without_a_terminal_event`.
- **H2 — a read timeout is a transport failure, not the `Timeout` terminal.**
  The terminal is reserved for the deadline the adapter observes itself.
- **H3 — the response cap can disguise truncation.** Both adapters read through
  `take(MAX_AI_RESPONSE_BYTES + 1)`. A stream that reaches the cap ends at EOF,
  which the remote adapter reports as `Done` once any event has parsed. No
  fixture covers this — it needs a body over 4 MiB — so it is recorded rather
  than claimed. ADR 0003 §2 names the fix.
- **H4 — `sse_payload` is a line helper, not an SSE decoder.** No multi-line
  `data` joining, no event names, no comment handling, no blank-line dispatch,
  no split-UTF-8 handling. It is correct for the single-line frames these
  providers send and nothing more.
- **H5 — `ProviderEvent` permits contradictory states.** Text with `finished`,
  usage on an empty frame. ADR 0003 §2 records the typed replacement.
- **H6 — no deadline inside the Ollama read loop.** The remote adapter checks
  its deadline each iteration; Ollama relies on the request timeout alone.
  Neither cancellation flag interrupts a blocked read, and there is still no
  service watchdog.
- **H7 — divergent usage validation.** The remote adapter drops out-of-bound
  usage to `None`; Ollama forwards it and lets the service's own validation
  reject it. Both are as extracted.

## Tests

54 unit tests in the crate, plus 6 wire tests in `tests/listing_wire.rs`. Every
one runs against a local fixture server; the two live tests are `#[ignore]`d and
need an environment variable or a local server.

### Ported from Skriuw

| Source test | Here |
| --- | --- |
| `parses_only_sse_data_frames` | `http::tests::parses_only_sse_data_frames` |
| `stream_usage_option_is_gated_per_provider` | same name |
| `streams_an_openai_compatible_first_party_provider_through_the_shared_path` | `streams_an_openai_compatible_provider_through_the_shared_path` |
| `streams_gemini_deltas_and_reports_usage` | same name |
| `streams_groq_deltas_and_stops_at_the_done_sentinel` | `stops_at_the_done_sentinel_and_discards_later_text` |
| `never_opens_a_socket_without_a_credential` | same name, now proving no connection was made |
| `refuses_a_request_whose_disclosure_consent_is_stale` | folded into `credentials::tests::application_copy_survives_the_mapping` (P3) |
| `maps_provider_status_codes_onto_distinct_recoverable_states` | same name, three more statuses |
| `fails_visibly_on_malformed_stream_data` | same name |
| `fails_visibly_when_a_provider_closes_a_stream_with_no_events` | same name |
| `rejects_stream_bytes_beyond_the_requested_output_limit` | same name |
| `rejects_an_oversized_stream_event` | same name |
| `a_closed_consumer_cancels_the_request` | same name |
| `stops_before_reading_when_the_request_is_already_cancelled` | `stops_before_sending_when_the_request_is_already_cancelled` |
| `refuses_a_request_addressed_to_another_provider` | same name |
| `verification_reports_acceptance_and_rejection_without_echoing_the_key` | same name |
| `resolves_provider_identity_from_a_bounded_identifier` | same name |
| `every_endpoint_stays_on_the_disclosed_destination` | same name |
| `lists_gemini_completion_models_from_the_provider` | same name |
| `lists_groq_models_and_skips_inactive_entries` | `lists_openai_style_models_and_skips_inactive_or_non_chat_entries` |
| `model_listing_never_opens_a_socket_without_consent_and_maps_rejections` | `model_listing_refuses_before_the_network_and_maps_rejections` |
| `a_model_authority_can_widen_the_supported_set_beyond_the_catalog` | `refuses_a_path_traversing_model_id_even_when_the_authority_permits_it` |
| `accepts_only_loopback_http_endpoints` | same name |
| `streams_ollama_completion_through_the_provider_seam` | `streams_ndjson_deltas_and_reports_usage` |
| `verifies_a_real_gemini_key`, `streams_a_real_groq_completion` | `streams_a_real_groq_completion`, `streams_a_real_local_completion` |

### Deliberately not ported

`ships_a_valid_repository_catalogue_covering_every_provider` and every catalog,
archive, download, digest, install, pull, and process test: they belong to data
and lifecycle this crate does not own.

### New here

Cancellation before send on both adapters and after credential resolution;
redirect refusal; userinfo refusal; path-traversal refusal; unpermitted model
refused before the network on completion and on verification; unsupported
listing refused without a request; truncated body on both adapters; partial
usage dropped rather than zero-filled; usage arriving after the text finished;
cancellation during the read loop; the timeout terminal on both adapters;
Ollama's missing-model status separated from a rejected request; an unreachable
Ollama being an error rather than a reason to start one; the full credential
mapping table; listing validation bounds; and the six listing wire fixtures.

## Dependencies

`ai-core`, `reqwest` (0.13, `default-features = false`, `blocking`/`json`/
`rustls`), `schemars`, `serde`, `serde_json`, `thiserror`. No OS credential
store, no framework, no async runtime facade, no Skriuw dependency.

The `remote` and `ollama` features gate code, not dependencies: both speak HTTP
through the same client, so disabling one does not remove `reqwest`. All three
feature combinations build clean under `clippy -D warnings`.

`specs/schemas/model-listing.json` is generated by
`cargo run -p ai-providers --features schema-tool --bin ai-provider-schema -- generate`.
