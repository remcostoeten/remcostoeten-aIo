# Phase 3 integration record: Skriuw

Consumer: Skriuw at revision `64827f5e81d097321715e789c9fcd795303c1595`, the
revision Phases 1 and 2 extracted from, on branch `ai-sdk-phase-3-extraction`.
Integration commit: `0b18371e`.

Phases 1 and 2 proved the extraction compiled and passed its own tests. This
phase proves a real consumer runs on it. The two inventories record what moved;
this file records what the move cost the consumer.

## What Skriuw now consumes

| Skriuw crate | Source | Consumes |
| --- | --- | --- |
| `skriuw-domain` | `ai.rs`, `ai_history.rs`, `remote_ai.rs` | `ai-core` only |
| `skriuw-ai` | `lib.rs` | `ai-core` |
| `skriuw-ai-remote` | `lib.rs` | `ai-providers` |
| `skriuw-ai-ollama` | `lib.rs` | `ai-providers` (dev only) |
| `skriuw-app` | `ai.rs`, `ai_credentials.rs`, `ai_history.rs`, `commands/ai.rs` | both |

`skriuw-domain` deliberately does not depend on `ai-providers`: that crate
carries an HTTP client, and the domain graph has none. The credential and
listing types are adapted at the provider and application layers, which already
depend on `reqwest`.

Dependencies are paths to a sibling checkout, not a registry or git release.
The SDK has no published version, and both checkouts must sit under the same
parent directory. That is a Phase 3 artefact, not a shipping arrangement.

## What stayed in Skriuw

`models.json` and the catalogue types, `CatalogModelAuthority`, the six-variant
consent and vault vocabulary, `RemoteAiModelListing` and its directory merge,
the prompt library, `AiRunRecord` with its retention and redaction, filters,
aggregates and SQLite storage, the whole Ollama lifecycle including
`with_endpoint_override`'s status degradation, and every Tauri command.

`skriuw-ai` and `skriuw-ai-remote` survive as thin crates rather than being
deleted: they re-export the SDK under the names the command layer already
imports and own the product data the SDK will not. No crate was removed while
it still exported a responsibility without a replacement.

## The two seams that changed shape

### D2 — recording

`AiRunRecorder::record` now takes `(AiRunSummary, &AiCompletionRequest)`.
`skriuw_domain::ai_run_record` maps the discriminated status onto Skriuw's
independent `state`/`error_category` pair and copies the prompts out of the
borrowed request inside the callback, before anything is queued. Redaction
still happens at the storage boundary, so `retain_prompts` behaviour is
unchanged.

Proven by `ai_history::tests::history_retains_or_redacts_prompts_and_maps_every_status`,
which drives the SDK port and asserts both retention states and the failed-run
category.

### P3 — credentials

`AiCredentialSource::resolve` now returns the SDK's three refusals plus an
application message. `AiCredentialStore::resolve_credential` keeps Skriuw's six
variants and `refusal` narrows them: `Invalid` stays `Invalid`, the two vault
states become `Unavailable`, and absent-or-unconsented becomes `Missing`. Each
carries its existing copy.

`every_skriuw_refusal_reaches_the_sdk_port_with_its_own_copy` asserts the
mapped `AiProviderError` is *identical* to the one the variant produced before
the migration — same category, recovery action and message — for all six.

## Behaviour deltas visible to Skriuw

Everything else is unchanged. Requests, endpoints, headers, framing, error
vocabulary, event serialization and bounds are as extracted.

- **P2 user agent.** `skriuw_ai_remote::USER_AGENT` sends `Skriuw`; the local
  adapter is constructed with `Skriuw local AI`. Both preserve the previous
  requests exactly.
- **P1 redirects refused, P4 cancellation rechecked before send, P5 validated
  endpoints.** These are hardening the SDK applies to every consumer. No
  shipped provider redirects, so no observable change.
- **P6 non-loopback Ollama.** The lifecycle runtime still resolves the endpoint
  and still degrades to the default with a status notice; the completion
  adapter is bound to whatever the runtime resolved, so the fallback decision
  stays where it was. `the_completion_adapter_reaches_the_runtime_resolved_endpoint`
  is the composition test.
- **P7 no default authority.** `remote_provider` requires one at every call
  site. `refresh_remote_ai_models` now passes the fetched-model store instead
  of the catalogue authority; `list_models` never consults the authority, so
  nothing changes and no fetch becomes self-authorising.

## Contract drift

`contracts/generated/ai-completion-{request,event}.schema.json` and
`ai-history-view.schema.json` regenerate with two differences:

1. `description` fields appear throughout, from the SDK's doc comments.
2. `AiProviderErrorCategory` and `AiTokenSource` render as `oneOf` of `const`
   strings rather than one `enum` list, because schemars emits per-variant
   descriptions.

Accepted values, field names, tags and null behaviour are identical. The
renderer's `app/src/contracts/ai.ts` is hand-written, not generated, so no
renderer type moved. `xtask generate --check` is green against the regenerated
files.

## Verification

`./scripts/check.sh`, Skriuw's own gate, 12/12 steps green:

- generated contracts clean, `cargo fmt --all --check` clean,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean
- 418 workspace tests, 11 ignored (device/live)
- 74 desktop-bridge tests
- 1530 renderer tests, plus the UI-architecture, renderer-store and cloud
  suites and the renderer type gate

The desktop crate is not covered by the workspace clippy step. Running clippy
against it separately surfaces two `cloned_ref_to_slice_refs` findings in
`src/maintenance.rs` — pre-existing, untouched by this migration, and outside
Phase 3's scope.

## Test inventory

Nothing was dropped without a replacement. Ported source tests live in the SDK
per the two extraction inventories. Retained in Skriuw:

| Test | Where | Why it stayed |
| --- | --- | --- |
| `the_fake_provider_runs_every_built_in_prompt` | `skriuw-ai` | the prompt library is Skriuw's |
| `ships_a_valid_repository_catalogue_covering_every_provider` | `skriuw-ai-remote` | catalogue is product data |
| `validates_bounded_provider_neutral_request` and the two wire-tag cases | `skriuw-domain::ai` | Skriuw's own assertions about the bounds it depends on, now run against the extracted definition |
| every lifecycle, archive, pull and process test | `skriuw-ai-ollama` | Phase 6 |
| the credential store, consent and model-store suites | `skriuw-app` | consent and vault policy |

New in Skriuw for this phase: the refusal mapping table, the SDK-port consent
gate, the retained/redacted history proof, the runtime-endpoint composition
test, and the catalogue-authority and listing-projection cases.

## Not done here

No messages or error-taxonomy migration. No Ollama lifecycle extraction — that
is Phase 6. No Dora or Betalingen change. The path dependencies need a real
release channel before this can be anything but a local proof.
