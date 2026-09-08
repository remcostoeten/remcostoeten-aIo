# Roadmap

Status: Phases 0 through 5 complete; Phase 6 not begun and not authorized. Each phase requires explicit approval and ends at its exit criteria. This roadmap does not authorize migrations, production code, package creation, or deployment now.

The phase order follows AGENTS.md. The audit's larger package/feature list is not a required deliverable. Every phase defines a single purpose, preserves dependency direction, and records any behavior change separately.

## Phase 0: architecture and contract review

Purpose: find errors before extraction and define its smallest compatible boundary.

Deliverables: README, architecture, contracts, roadmap, ADRs 0001–0003, and the source-backed findings/decision list in architecture §§4–5. No code, manifests, dependency installation, or reference changes.

Validation: read the complete audit and reference code where needed; review type states, serialization, boundaries, migration impact and lifecycle races; correct docs and repeat the contradiction review.

Exit: documentation is consistent and blocking extraction decisions D1/D2 are resolved. **Met.** D1 selects the narrow lifecycle hardening in contracts §3.3; D2 selects the metadata summary plus borrowed request in contracts §2.8. Verdict: **Phase 0 COMPLETE, APPROVED FOR PHASE 1**. Approval is readiness, not authorization to execute.

## Phase 1: extract the smallest Rust completion core

Purpose: make Skriuw's provider-neutral completion seam independently usable with minimal application changes.

Prerequisite: an explicit instruction to begin Phase 1. D1 and D2 are already recorded — narrow lifecycle hardening (contracts §3.3) and the metadata summary with borrowed request (contracts §2.8) — so no further architecture decision is outstanding. No automatic progression follows from that.

### Sources and smallest allowed deliverable

Create one production crate, crates/ai-core, and the minimum workspace metadata. Extract:

- skriuw-domain/src/ai.rs: existing completion parameters/request/delta/usage/error/event/terminal, validation, bounds, cancellation, sink and completion trait.
- skriuw-ai/src/lib.rs: service/channel/start errors and deterministic fake/script/outcomes.
- Only the ai_history.rs accounting arithmetic and the token vocabulary that the D2 summary needs. The legacy `AiRunRecord`, its prompt struct, retention settings, filters, aggregates and storage all stay out. The RemoteAiCatalog implementation of the pricing port stays with provider/application code; a neutral fake pricing source suffices for tests.
- Schema generation/check tooling and request/event/error fixtures for the extracted shapes. Prefer a development target in ai-core; a separate xtask crate requires an actual tooling/dependency reason.

Keep source public Rust names where useful and preserve request/event JSON exactly. No production dependency points back to Skriuw. Record the source revision and a symbol/test inventory; do not import application prompts or catalogs merely to copy tests.

### Explicitly excluded

No messages/ModelRef migration, response format, structured validation/repair, stop/suffix/token controls, finish reason, new error vocabulary, usage source on Done, retry implementation, router flags/attempts, SSE/NDJSON codecs, real provider HTTP code, credential source/store extraction, environment resolver, tokio facade, Tauri/Specta, Ollama lifecycle, production TypeScript, or extra provider/packages scaffolding.

No edits to Skriuw, Dora, or Betalingen. Credential ports can wait for Phase 2 because the completion service/fake does not consume them.

### Verification

Port applicable domain/service/fake tests and replace product data with neutral fixtures. Retain source app-specific tests in Skriuw. Add characterization cases for existing null behavior, strict fields, accepted identifiers/empty inputs, sampling bounds, inactive retryCount, error values, reported/estimated accounting and fake outcomes.

Exercise ordered deltas, duplicate ids, cancel/Done races, close-induced cancellation, timeout outcome, bounds, spawn failure and recording. D1-approved fixes need barrier-controlled tests, including unknown-provider duplicate admission, provider panic cleanup, stale terminal/id reuse, and malformed provider output. Do not claim source tests already cover these cases.

For D2, prove both retained and redacted history can be reconstructed without copying Skriuw storage into the SDK. Use a local compatibility test module modeling the existing consumer's DTO/port calls. Do not modify or migrate the reference checkout to perform this proof. Account for synchronous unknown-provider completion and failed start.

Generate/check structural schemas and run semantic validation tests. Compare serialized fixtures with the source contract, not just renamed Rust types. Audit dependencies and source diff; no default HTTP, OS credential, framework, or async-runtime dependency.

### Exit

**Met on 2026-09-06.** `docs/extraction-inventory.md` records what moved, the two approved behavior changes, and the test mapping. 74 tests and the schema check pass; no runtime dependency on HTTP, credentials, a framework, an async runtime, or Skriuw. Phase 1 is finished when every item below is demonstrably true, and not before:

1. `crates/ai-core` exists with the extracted contracts, ports, service and fake; the source revision and a symbol/test inventory are recorded.
2. Request, event and error fixtures round-trip byte-compatibly in meaning with Skriuw's current wire shapes, compared semantically per ADR 0002.
3. The applicable extracted domain/service/fake tests pass with neutral fixtures, and the characterization cases above are present.
4. The deterministic fake reproduces its scripted segmentation and every outcome.
5. The D1 hardening is a separate reviewable step on top of characterization, with barrier-controlled tests for duplicate admission on both paths, cancel/commit races, id reuse against a stale terminal, provider panic cleanup, and malformed provider output.
6. The D2 harness reconstructs retained and redacted history through an application-side adapter, covering the §2.8 status mapping, unknown-provider synchronous recording, and start failures that record nothing.
7. Schema generation and the drift check run clean, and `specs/VERSION` is set.
8. A dependency audit shows no HTTP client, OS credential, framework, async-runtime, or Skriuw dependency in the production graph.

Excluded from Phase 1 regardless of convenience: any HTTP provider, credential resolver or store, messages/ModelRef redesign, retry engine, structured output, async facade, and any framework or TypeScript package.

This proves extraction compatibility locally, not that Skriuw has migrated. Actual Skriuw integration is Phase 3. Stop after the exit review.

## Contract evolution gate (not an implementation phase)

Before a later phase requires messages, capabilities, richer errors, model metadata, structured output, or different serialization, approve a bounded contract delta with a real consumer, exact types/validators, positive/negative fixtures and migration mapping.

The former conceptual v1 API is not mandatory Phase 1 output. contracts §4 records design constraints, not an implementation checklist. Defer unrelated features instead of bundling them into the next phase.

**Run once, on 2026-09-07, for Phase 4's history/token-limit prerequisite.** ADR 0004 records the approved delta: `priorMessages` on the request and `maxOutputTokens` on the parameters, both defaulted, with `systemPrompt`/`userPrompt` unchanged. Consumer, exact types, validators, positive and negative fixtures and the migration mapping are in that ADR. `specs/VERSION` moved to 0.2.0. Every other item this gate governs — messages-only requests, capabilities, richer errors, model metadata, structured output — remains ungated.

**Run a second time, on 2026-09-09, for Skriuw's shipped voice dictation.** ADR 0006 records the approved delta: speech-to-text on the existing remote descriptors, as an inherent `RemoteAiProvider::transcribe` alongside `list_models`. Consumer, exact types, validators, positive and negative fixtures and the migration mapping are in that ADR. `specs/VERSION` did **not** move, and neither did either npm package: nothing added is serialized, so no shared contract changed and the gate's serialization clause was not engaged. `workspace.package.version` moved to 0.3.0 for the `ai-v0.3.0` tag alone. The capability clause is now spent for transcription only — every other capability, and every remaining item this gate governs, stays ungated.

## Phase 2: extract Rust providers

Purpose: create ai-providers from the provider execution already used by Skriuw, retaining the Phase 1 core seam.

Allowed: HTTP dependencies and protocol features needed by extracted adapters, common internal bounded readers/error mapping, descriptor data for actual providers, provider-boundary credential resolution and model authority, explicit verification/listing, local HTTP fixtures. Dora's additional provider coverage is added incrementally when scoped; no obligation to produce all adapters at once.

Preserve Skriuw's model permission gate, credential-before-network ordering, request bodies, error vocabulary and Ollama /api/generate for its first integration. Keep consent-specific errors in Skriuw with a typed generic mapping at the credential port. Exact credential/admin signatures are designed before this phase begins.

Source EOF handling, read timeout classification, cancellation-before-send and response cap behavior are characterized first. Any corrections require explicit behavior deltas and fixtures. No structured-output ladder, retries/key rotation, lifecycle process management, application prompts, or reference modifications.

Exit: every introduced descriptor/adapter passes its local fixtures and conformance checks, including credentials/destination handling, malformed/truncated streams, usage, cancellation and timeout. The dependency graph matches actual manifests/features. Stop.

**Met on 2026-09-06.** `crates/ai-providers` holds seven remote descriptors, the Gemini dialect, the Ollama generation adapter, the credential and model-authority ports, and the model listing contract. `docs/extraction-inventory-providers.md` records the symbol map, the seven behavior changes (P1–P7), the preserved source defects (H1–H7), and the test mapping. 60 tests pass against local fixture servers with no network and no key; `clippy -D warnings` is clean under all three feature combinations; the listing schema and its fixtures are committed. No reference repository was modified. Skriuw has not migrated — that is Phase 3.

## Phase 3: prove Skriuw consumes the extraction

Purpose: integrate the extracted core and approved providers into Skriuw with preserved product behavior.

Only explicit migration authorization permits Skriuw changes. Re-export unchanged core types where sufficient; use the D2 adapter for recording if selected. Change the minimum Rust imports/wiring. Keep renderer request/event schemas, prompts, editor behavior, consent, model authority, retention, opt-in/lazy startup and command surfaces compatible.

No simultaneous messages/error-taxonomy migration. Keep Ollama lifecycle in Skriuw; replace only the completion implementation proven equivalent. Do not delete crates/modules until every exported responsibility has a replacement.

Verification: Skriuw's current check script, contract drift checks, applicable AI tests and native fake-provider integration; real-device/provider checks remain explicitly opt-in. Test retained/redacted history and credential refusal. Preserve source test inventory rather than relying on an old hard-coded count.

Exit: Skriuw builds and passes the relevant checks using extracted code, with only recorded D1/D2 changes and no SDK product logic. Stop; no Dora/Betalingen changes.

**Met on 2026-09-07.** `docs/integration-skriuw.md` records the crate graph, what stayed in Skriuw, the two seams that changed shape (D2 recording, P3 credentials), the contract-drift delta, and the retained test inventory. Skriuw's own `./scripts/check.sh` is 12/12 green on branch `ai-sdk-phase-3-extraction`, commit `0b18371e`: 418 workspace tests, 74 desktop, 1530 renderer, contract drift clean, clippy clean. The adapters, credential port and completion service are gone from Skriuw; the catalogue, consent vocabulary, prompt library, run history and Ollama lifecycle stayed. Two caveats stated rather than hidden: the crates are consumed as paths into a sibling checkout, which needs a real release channel before it is more than a local proof, and the generated AI schemas gained doc-comment descriptions with two string enums rendering as `oneOf` consts — same accepted values, and the renderer's types are hand-written, so nothing downstream moved.

## Phase 4: TypeScript core and Vercel adapter

Purpose: implement shared semantics natively for the first TypeScript consumer.

Prerequisite: a separately approved minimal history/token-limit contract delta needed by Betalingen. Do not flatten its conversation into one string just to avoid specifying that delta. Keep any legacy Rust extraction wire version explicit.

Create packages/core and packages/ai-sdk with the boundaries in ADRs 0001/0002. Use own typed provider factories, internal Vercel instances, portable event consumer/NDJSON helpers, explicit abort/timeout normalization and deterministic fake. Environment lookup stays in server application code. No public fromLanguageModel or third-party type leak.

Verification: positive/negative structural and semantic fixtures in both languages, normalized provider text/terminal/error/usage parity, fixed-script delta parity, iterator close/abort cleanup, bounded stream buffering, and strict TypeScript. Underlying AI SDK retries must remain disabled unless the SDK has an approved retry policy. Check the browser consumer bundle excludes ai/provider packages.

Exit: offline conformance and browser/Node/Bun portability checks pass for the units claiming those runtimes. No React, Tauri, Hono or routing package. Stop.

**Met on 2026-09-07.** The prerequisite gate ran first and is recorded as ADR 0004; `specs/VERSION` is 0.2.0. `packages/core` holds the contracts, decoders, validators, run lifecycle, event consumer, NDJSON helpers and deterministic fake with zero dependencies; `packages/ai-sdk` holds the Vercel adapter, the typed provider factories and the credential and model-authority ports. `docs/typescript-core.md` records the symbol map, the six shape differences (S1–S6), the two vendor quirks (V1–V2), and what was not implemented. 148 Rust tests and 122 TypeScript tests pass with no network and no key; `scripts/check.sh` runs both plus clippy, fmt and the schema drift check. Conformance is three layers: shared wire fixtures read by both languages, eight shared fake scripts compared on the ADR 0002 list, and schema assertions from the TypeScript side. Portability is checked by bundling `core` for browser/Node/Bun and by executing the emitted JavaScript under real Node. `createAdapter` and `ModelFactory` are unexported, so no third-party type is reachable from either entry point and there is no `fromLanguageModel`; `maxRetries` is 0 and a 429 is attempted exactly once. Three caveats stated rather than hidden: distribution is still unresolved for both languages, the ADR 0004 Rust source break means Skriuw's Phase 3 branch needs seven one-line edits before it builds against 0.2.0, and every provider test runs against a fixture `fetch` — nothing has confirmed a live endpoint matches those fixtures.

## Phase 5: migrate Betalingen

Purpose: replace provider execution without changing financial context, authorization, masking, prompts, or application HTTP policy.

With explicit authorization, adapt its route to the runtime while preserving its existing text/done/error event shape by default. A later explicit HTTP contract change can expose richer SDK events; typed categories are not required on an unchanged three-event wire. Keep server credentials and Vercel dependencies out of the browser island.

Exit: existing application behaviors/tests, OpenAPI drift, build and relevant bundle checks pass. Deployment is a separate authorized action, not an implicit SDK phase requirement. Stop.

**Met on 2026-09-07.** `docs/integration-betalingen.md` records the two changed files, what was preserved, what the run lifecycle adds, and what is not finished. `src/lib/ai/groq.ts` is now `createGroqProvider`/`buildRequest`/`createRuntime` instead of `createGroq`/`streamText`; `src/routes/ai.ts` changed only inside its `try` block; the three-event `text`/`done`/`error` NDJSON wire, the prompt, the source allowlist, the IBAN masking, the authorization gates and the Dutch failure message are untouched. Betalingen's 99 tests pass with its six AI tests unmodified, `tsc` and `oxlint` are clean, the OpenAPI shows no drift from this change, the esbuild bundle builds, and the browser island still carries no vendor package, key, model id or prompt. Two caveats stated rather than hidden: the packages are consumed through `file:` paths plus an `overrides` pin, so Betalingen is not deployable until ADR 0005's npm publish happens, and nothing here has touched a live Groq endpoint.

## Phase 6: extract Ollama lifecycle

Purpose: share process/filesystem/model-management behavior independently of completion.

Create ai-ollama-runtime only after its platform and behavior scope is approved. Port required detect/install/verify/spawn/stop/status/pull/remove/progress/shutdown logic with local fixtures; keep LocalRuntime/progress/error types here. Select platform fixes from Dora only with evidence and tests.

Applications compose lifecycle and generation; no provider-to-lifecycle dependency. Skriuw migration requires explicit authorization. Remote endpoint privacy behavior is an application decision; a loopback default does not prove inference locality.

Exit: lifecycle fixtures and authorized application integration pass, with device downloads/tests opt-in. Stop.

## Phase 7: conditional Tauri helpers

Purpose: decide whether surviving duplication justifies a Tauri dependency boundary.

Review Skriuw after migration and Dora's migration plan (Phase 8 need not already be executing). A new ADR records go/no-go. If needed, add only proven channel/blocking/operation helpers; avoid duplicating the completion registry. Consider application wrapper types versus optional Specta derives for Dora binding compatibility.

Exit: decision recorded; if go, helper tests and authorized integration pass. No fixed commands, product logic, or mandatory renderer package. Stop.

## Phase 8: migrate Dora

Purpose: use the SDK while keeping SQL and application policies in Dora.

Before implementation, approve a concrete migration mapping for prompts/context placement, model ids in settings and key records, event compatibility, usage, editable endpoints, and key rotation. Initial Skriuw-compatible adapters do not automatically replace all Dora behavior.

Preserve Dora's lenient SQL parser unless a separate structured-output change is approved. Removing 401/403/5xx rotation, changing remote endpoint rules, or enabling typed structured output are product changes, not extraction chores. No CredentialSource-only rotation decorator is assumed.

Exit: relevant Dora features and tests pass, encrypted keys/settings remain usable, and old adapters/lifecycle code are removed only after replacement coverage. Stop.

## Phase 9: optional routing

Purpose: serve a demonstrated multi-route consumer after a new routing ADR.

No precommitted TaskRequirements, HealthStore, attempts fields, or unchanged-core guarantee. Design the smallest policy with explicit locality authorization, no switching after visible output, bounded execution/accounting, and deterministic tests.

Exit: one approved real consumer and its offline fallback/privacy tests pass. Stop.

## Not scheduled

Agents, tools, embeddings, autocomplete-specific controls, prompt/task packages, React hooks, Hono/Next.js integrations, browser BYOK, telemetry storage, async Rust facade, and structured-output repair remain unscheduled until a concrete approved requirement exists.

Transcription left this list on 2026-09-09. Skriuw had shipped voice dictation and its provider request syntax belonged in the adapters; ADR 0006 records the requirement, the delta and its bounds. It arrived through the contract evolution gate above rather than as a phase, because it extends one adapter rather than moving a boundary.
