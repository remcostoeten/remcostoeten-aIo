# Phase 1 extraction inventory

Source: Skriuw at revision `64827f5e81d097321715e789c9fcd795303c1595`, read-only.
Destination: `crates/ai-core`. This file records what moved, what changed, and
what deliberately stayed behind, so Phase 3 can diff intent against reality.

## Symbols

### From `crates/skriuw-domain/src/ai.rs`

| Source symbol | Destination | Change |
| --- | --- | --- |
| `MAX_AI_*` constants (8) | `contracts` | none |
| `AiValidationError` | `contracts` | none |
| `AiCompletionParameters` + `Default` + `validate` | `contracts` | none |
| `AiCompletionRequest` + `validate` | `contracts` | none |
| `AiCompletionDelta` + `validate` | `contracts` | none |
| `AiUsage` + `validate` | `contracts` | none |
| `AiProviderErrorCategory` | `contracts` | none |
| `AiRecoveryAction` | `contracts` | none |
| `AiProviderError` + `new` + `validate` | `contracts` | none |
| `AiCompletionEvent` + `validate` | `contracts` | none |
| `AiCompletionTerminal` + `into_event` | `contracts` | none |
| `AiCancellation` | `ports` | none |
| `AiSinkError` | `ports` | none |
| `AiEventSink` | `ports` | none |
| `AiComplete` | `ports` | none |
| private validators, `bounded_message` | `contracts` | none |

### From `crates/skriuw-ai/src/lib.rs`

| Source symbol | Destination | Change |
| --- | --- | --- |
| `AiStartError` + `Display` + `Error` | `service` | none |
| `AiCompletionChannel` | `ports` | none |
| `AiCompletionService::{new, recording, start, cancel, shutdown}` | `service` | lifecycle hardened per D1, below |
| `CompletionServiceSink` | `service::ValidatingSink` | now validates identity, sequence and bounds (D1) |
| `now_millis` | `service` | none |
| `run_record` | `service::run_summary` | produces the D2 summary instead of the legacy record |
| `FakeCompletionOutcome` | `fake` | none |
| `FakeCompletionScript` + `success` | `fake` | none |
| `FakeAiProvider` + `AiComplete` impl | `fake` | none |
| `with_fake_provider` | — | not extracted; it hard-codes a `fake` registration a consumer can make itself |

### From `crates/skriuw-domain/src/ai_history.rs`

Only what the completion seam needs. Everything else — retention settings,
filters, aggregates, pagination, SQLite, the remote catalog — stays in Skriuw.

| Source symbol | Destination | Change |
| --- | --- | --- |
| `AI_TOKEN_ESTIMATE_BYTES` | `recording` | none |
| `AiTokenSource` | `recording` | none |
| `AiRunTokens` + `reported` + `estimated` | `recording` | none |
| `estimate_ai_tokens` | `recording` | none |
| `AiModelPrice` | `recording` | none |
| `AiModelPricing` | `recording` | none |
| `ai_run_cost_micros` | `recording` | none |
| `AiRunRecorder` | `recording` | signature changed per D2 |
| `AiRunState` + `AiRunPrompts` + `AiRunRecord` | — | replaced by `AiRunStatus` / `AiRunSummary`; the legacy record stays application-owned |
| `AiHistoryRetention`, filters, aggregates, `AI_RUN_ORIGIN_PLAYGROUND` | — | application concerns |
| `impl AiModelPricing for RemoteAiCatalog` | — | catalog data is application-owned |

## Behavior changes

Only two, both approved in Phase 0. Everything else is byte-for-byte contract
compatible: serialized field names, tags, bounds, null behavior, accepted
identifier grammar, empty prompts and deltas, inert `retryCount`, sampling
range, error vocabulary, estimation and cost arithmetic.

### D1 — lifecycle hardening (`docs/contracts.md` §3.3)

| Edge | Source | Now |
| --- | --- | --- |
| Duplicate id on the unknown-provider path | admitted, second terminal sent | `DuplicateRequest` |
| Id reservation | released before the terminal is sent | held through the delivery attempt |
| `cancel` after commitment | could still flip `Done` to `Cancelled` | returns `false`; the terminal stands |
| Foreign request id in a delta | forwarded to the consumer | `malformed_response` terminal |
| Sequence gap in a delta | forwarded | `malformed_response` terminal |
| Oversized delta or output budget overrun | counted, forwarded | `malformed_response` terminal |
| Invalid reported usage or provider error | published | `malformed_response` terminal |
| Provider panic | run stranded, no terminal, no record | `internal_failure` terminal, cleaned up, recorded |
| `shutdown` | cancels all tracked runs | cancels uncommitted runs only |

Unchanged by choice: `shutdown` still does not close admission or join workers;
there is still no whole-request deadline; an unregistered provider still returns
`Ok(())` with a synchronous terminal.

### D2 — recording port (`docs/contracts.md` §2.8)

`AiRunRecorder::record(&self, record: AiRunRecord)` became
`record(&self, summary: AiRunSummary, request: &AiCompletionRequest)`. The
summary is metadata-only with a discriminated status; prompts are copied from
the borrowed request by an application adapter, which is where retention already
lives. Proven by `tests/consumer_compatibility.rs`.

## Tests

### Ported

| Source test | Here |
| --- | --- |
| `ai.rs::validates_bounded_provider_neutral_request` | split across ten focused cases in `tests/contracts.rs` |
| `ai.rs::bounds_and_normalizes_safe_provider_errors` | `normalizes_and_bounds_provider_error_messages` |
| `ai.rs::serializes_transport_events_with_stable_terminal_tags` | `serializes_events_with_the_source_tags_and_field_names` |
| `ai_history.rs::prices_a_known_token_count_from_the_catalog` | `tests/recording.rs::prices_a_run_rounding_half_up`, with a fake pricing source instead of the catalog |
| `ai_history.rs::marks_derived_counts_as_estimates` | `estimated_tokens_keep_estimated_provenance` |
| `skriuw-ai::streams_ordered_tokens_and_completes` | `tests/service.rs::streams_ordered_deltas_then_one_terminal` |
| `skriuw-ai::aborts_token_production_mid_stream` | `cancellation_before_completion_produces_a_cancelled_terminal`, barrier-controlled rather than timed |
| `skriuw-ai::times_out_before_delivering_a_late_token` | `cancellation_does_not_erase_a_timeout_outcome` |
| `skriuw-ai::classifies_malformed_provider_output` | `cancellation_does_not_erase_a_provider_error_outcome` |
| `skriuw-ai::rejects_response_bytes_beyond_the_request_limit` | `hardened_exceeding_the_output_budget_is_rejected_as_malformed_output` |
| `skriuw-ai::closed_surface_cancels_and_discards_late_results` | `a_closed_channel_cancels_the_run` |
| `skriuw-ai::service_streams_events_and_releases_the_request_id` | `hardened_a_reserved_id_is_released_once_delivery_has_been_attempted` |
| `skriuw-ai::service_rejects_duplicate_ids_and_cancels_the_active_provider` | `rejects_a_duplicate_request_id_on_the_registered_provider_path` |
| `skriuw-ai::records_a_successful_run_with_provider_reported_usage_and_catalogue_cost` | `records_provider_reported_usage_and_catalogue_cost` |
| `skriuw-ai::records_cancelled_and_failed_runs_with_flagged_estimates` | `estimates_usage_for_runs_that_report_none` plus the mapping cases in `tests/consumer_compatibility.rs` |
| `skriuw-ai::records_an_unavailable_provider_without_starting_a_worker` | `records_an_unknown_provider_run_without_starting_a_worker` |

### Deliberately not ported

- `skriuw-ai::the_fake_provider_runs_every_built_in_prompt` — exercises Skriuw's
  prompt library. Neutral strings replace it here; the prompt catalog stays an
  application concern and its test stays in Skriuw.
- `ai_history.rs::redaction_removes_every_prompt_byte` and
  `retention_clamps_and_windows` — retention is application-owned. The
  equivalent guarantee is proven from the outside in
  `tests/consumer_compatibility.rs`.

### New here

Characterization and hardening cases with no source equivalent: identifier
grammar including traversal sequences, empty prompts and deltas, omitted versus
null decoding, positive and negative wire fixtures, exact-arithmetic and
saturation cases, every `hardened_*` case, reentrant recorder and delivery
ordering, and schema drift.

## Dependencies

Direct: `schemars`, `serde`, `thiserror`. Dev: `serde_json`, also an optional
direct dependency behind `schema-tool`.

Full runtime graph: `schemars`, `serde`, `serde_core`, `serde_json`,
`thiserror`, `dyn-clone`, `ref-cast`, `itoa`, `memchr`, `zmij`, plus proc-macro
crates. `serde_json` is transitive through `schemars` and cannot be excluded;
no `serde_json` type appears in the library's public API.

No HTTP client, no OS credential store, no framework, no async runtime, and no
dependency on any Skriuw crate.
