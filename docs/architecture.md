# Architecture

Status: Phase 0 closed 2026-09-06. Review complete, decisions taken, verdict in §6. No implementation is authorized until Phase 1 is explicitly begun.

`AGENTS.md` governs scope. The audit at `dev-docs/knowledge/architecture-research-across-my-ai-apps.md` is evidence and a set of proposals; reference source wins when describing existing behavior. The requested root file `ai-sdk-architecture-research.md` is absent. This review read the complete available audit and all three existing ADRs.

Companions: `contracts.md` distinguishes extraction from deferred design, `roadmap.md` defines phase gates, and ADRs 0001–0003 record corrected boundaries.

## 1. Central flow

```text
application builds product context and prompts
  -> existing CompletionRequest + separate origin
  -> CompletionService validates, selects registered provider, tracks cancellation
  -> AiComplete sends deltas and returns one terminal
  -> service attempts terminal delivery, then invokes app recorder
  -> application reviews/interprets results and owns persistence
```

This is Skriuw's current seam, with its actual limitations documented in `contracts.md` §3. It is not a claim that a thread can interrupt arbitrary blocking work or deliver into a closed channel.

Dora's SchemaContext and prompt building move to Dora's side of this boundary only in its approved migration. Betalingen already renders financial context before calling its provider. Neither application's prompts or context types enter the SDK.

## 2. Boundaries

### 2.1 Application

Applications own prompts, context extraction, consent, credentials at rest, history retention, result parsing/application, model selection and recommendations, startup policy, UI, and commands/routes. Skriuw's built-in prompts and optional prompt retention are real behavior to preserve, not unused domain modules to copy wholesale.

### 2.2 Phase 1 Rust core

One production crate contains the existing completion DTOs and validators, cancellation/sink/channel ports, completion-only trait, service, deterministic fake, and the metadata-only recording port selected in D2. Preserve existing public names initially where that avoids migration work.

The crate has no HTTP client, real provider dispatch, URLs, credential persistence, async runtime, Tauri/Specta, provider SDK, application prompt library, or product types. The `fake` provider name is an intentional exception for deterministic tests/playground behavior.

Schema generation is development tooling. A target within ai-core is sufficient initially; a separate xtask crate is conditional, never part of the public dependency graph. No empty crates/packages are created.

### 2.3 Provider layer, later

A separate ai-providers crate is justified by HTTP/protocol dependencies. It owns SSE/NDJSON provider parsing, transport bounds, endpoints, authorization, provider error decoding, catalog data, model listing and verification. Keep shared mechanics internal and extract them only as adapters demonstrate duplication.

Keep completion separate from administration. Syntax validation and model metadata are not substitutes for Skriuw's RemoteAiModelAuthority. See ADR 0003.

### 2.4 TypeScript and Vercel AI SDK

Two later packages have a real dependency boundary:

- `packages/core` (`@remcostoeten/ai-core`): our contracts, validators, ordered consumer, runtime/fake, and framework-free SDK-event NDJSON helpers as required by Betalingen.
- `packages/ai-sdk` (`@remcostoeten/ai-vercel`): provider factories using our typed configuration/credential ports, internally creating Vercel model instances and mapping its events/errors.

No public signature accepts LanguageModel, UIMessage, TextStreamPart, APICallError, or an opaque unknown substitute. A public fromLanguageModel factory would couple consumers to the very API being isolated; it has been removed from the plan. Custom integrations implement our Provider contract in application code.

Core uses portable Web APIs and no node:*, bun:*, React, Hono, Tauri, Next.js, or provider imports. Environment lookup occurs in application/server integration; browsers have no portable process environment. Framework-free SDK-event NDJSON helpers are distinct from provider SSE parsing.

The synchronous Rust service uses native threads. Lack of tokio does not establish browser/WASM execution support. A future Rust server or browser adapter needs an explicit executor/platform decision.

### 2.5 Recording and credentials

The existing service supplies AiRunPrompts to AiRunRecorder, and Skriuw's storage layer removes or retains them according to settings. D2 selects a metadata-only SDK summary with the request borrowed for the callback, so an application adapter — not the SDK — reconstructs that record and keeps retention where it already lives. This is an explicit compatibility approach proven by a Phase 1 harness, not a silent feature removal. Recording happens after a terminal send attempt; the callback is not a durable transaction or inherently nonblocking.

Credential resolution is not needed by the Phase 1 fake/service seam. Introduce its generic port with providers, keeping Skriuw's consent/vault types in Skriuw. Session and environment resolvers are not required extraction work. Credentials are runtime-only, never serialized configuration. Rust/TS secrecy lifecycle guarantees differ; see `contracts.md` §4.2.

### 2.6 Models and structured output, deferred

Keep flat providerId/modelId in Phase 1. Nested ModelRef, messages, a richer taxonomy, capabilities, model info, maxOutputTokens, and response formats can be designed against the next real consumer in an approved contract gate.

Structured-output strategy, validation dialect, typed decoding and repair remain unresolved for a future phase; no automatic ladder ships. Application text/list parsing remains supported. No unused tools/vision/audio/embeddings/reasoning variants, content-part nesting, finish reason, stop/suffix controls, or router attempt list are reserved.

### 2.7 Ollama and platform helpers

Generation and lifecycle remain separate responsibilities and dependency sets. Application code composes runtime startup with generation. No optional provider-to-lifecycle edge is allowed.

Skriuw's /api/generate must remain compatible for its initial migration. Dora's /api/chat is a later adapter capability; changing endpoint and prompt representation is behavior change.

LocalRuntime and progress types live in the lifecycle crate, not core. Tauri helpers are conditional on surviving duplication; no fixed commands or extra completion registry duplicating core ownership. Specta support is considered only when needed by Dora. React helpers are not scheduled.

### 2.8 Routing

No runtime routing, retry implementation, health store, fallback table, key-rotation decorator, or attempts field is required now. A later router can evolve versioned contracts; speculative fields are not needed to guarantee a future zero-change integration.

Never silently fall back from local to remote inference. Never switch provider or model after user-visible output. Caller policy, model authority, and destination consent remain binding even before the first delta.

## 3. Dependencies and verification

| Unit | Why separate | Dependencies/boundaries |
| --- | --- | --- |
| ai-core, Phase 1 | Completion contract and service | Existing serde/schemars/thiserror needs; serde_json for schemas/tests as needed; std threads; no application crate dependency |
| Schema tool, Phase 1 | Development generation/checking | Prefer a target in the existing crate; never shipped as a runtime dependency |
| ai-providers, Phase 2 | HTTP and provider protocols | ai-core, HTTP/JSON libraries required by implemented adapters; feature isolation verified |
| packages/core, Phase 4 | Portable contracts and consumption | Runtime validation dependency only as justified; no Vercel or framework dependency |
| packages/ai-sdk, Phase 4 | Isolate Vercel provider dependencies | packages/core and selected AI SDK/provider packages, with tested version compatibility |
| ai-ollama-runtime, Phase 6 | Process/filesystem/archive lifecycle | Core cancellation if useful; its own progress/error types; no completion logic |
| ai-tauri, conditional Phase 7 | Tauri channel/platform glue | ai-core + Tauri; applications assemble providers |

Do not state impossible dependency lists (the former roadmap required providers to use only core/reqwest while the architecture also required serde_json). Assess actual Cargo feature closure, not just manifest labels. Optional features do not excuse a forbidden responsibility.

Tests are phase-specific and offline. Phase 1 ports domain/service/fake tests, separates product prompt/catalog test fixtures, characterizes gaps, and compares existing wire fixtures. Phase 2 adds local provider fixture servers. Phase 4 adds TS structural/semantic validation and normalized provider-result parity. Rust schema generation alone does not prove cross-language validation parity.

## 4. Phase 0 critical review

Findings are ordered by severity. "Original sections" identifies the reviewed document locations before this revision; corrected destinations are given so the historical findings remain traceable after restructuring. The audit remains unchanged, including recommendations this review rejects.

### F1 — Critical: Phase 1 was a redesign masquerading as extraction

Original sections: roadmap Phase 1 Objective/Allowed/Tests/Exit and Phase 3 Allowed; contracts introductory status and §§2.1–2.7, 2.10–2.18; ADR 0001 Core; ADR 0002 Consequences.

The plan replaced systemPrompt/userPrompt, nested model identity, added origin to the provider request, renamed error states and recoveryAction, changed usage shape, doubled the temperature range, narrowed identifiers, and relaxed event decoding. It also added retries, structured output, codecs, administration, credentials, and tokio. Source `skriuw-domain/src/ai.rs` instead accepts empty prompts/deltas, uses temperature 0–1000, allows provider.local/v1, and rejects unknown event fields. Re-exporting changed types cannot preserve callers' struct literals or renderer JSON.

Correction: extraction baseline in contracts §§1–2 and narrowed roadmap Phase 1. All generalizations require a later contract gate. Do not promise unchanged Skriuw behavior merely because renamed types compile.

### F2 — Critical: exactly-one terminal and cancellation claims exceed the source

Original sections: architecture §§1, 2.4, 2.9; contracts §§2.17, 3 (invariants 3, 5, 9, 14); roadmap Phase 1 tests.

Source `skriuw-ai/src/lib.rs::start` looks up unknown providers before duplicate detection, removes the registry entry before sending the terminal, and has no panic guard. Concurrent reuse can receive an old terminal; a provider panic can leave an active entry and no terminal. Sink closure makes guaranteed delivery impossible. Start errors also omitted WorkerUnavailable and invented unknown_provider/shutting_down behavior.

Correction: contracts §3 distinguishes terminal commitment, delivery attempt, successful receipt, and durable recording. Actual start/shutdown behavior is explicit. D1 requires a decision on narrowly scoped hardening rather than silently preserving or rewriting these defects.

### F3 — High: prompt-retention compatibility was silently lost

Original sections: architecture §§2.1, 2.2, 2.4; contracts §2.18; roadmap Phases 1 and 3; ADR 0001 Core.

Source `skriuw-ai/src/lib.rs::run_record` constructs prompts: Some(AiRunPrompts). `skriuw-sqlite/src/ai_history.rs::append_run` applies retain_prompts. The proposed record removed prompts and claimed zero behavior change. A wholesale ai_history extraction would conversely bring retention/filter/storage-oriented contracts into core.

Correction: minimum accounting dependency inventory in contracts §2.8; D2 compares explicit compatibility approaches. Retention remains Skriuw-owned and must be demonstrated by an offline compatibility test before extraction exits.

### F4 — High: timeout ownership and deadline scope were undefined

Original sections: architecture §§2.4, 2.9–2.10; contracts §§2.16–2.17, 3.6, 4.1; ADR 0003 §§2, 8.

The planned runtime backstop had no execution context/deadline parameter or interruption mechanism. Skriuw remote starts timing after credentials and maps read errors to TransportFailure; Ollama has request timeout but no explicit per-read deadline check. An atomic flag cannot interrupt a blocked read. The error table also allowed retry of an exhausted deadline.

Correction: contracts §§3.2–3.3 documents cooperative baseline and excludes a watchdog from Phase 1. A future whole-request deadline must cover scheduling/resolution/retries explicitly and specify blocked-worker cleanup and delivery limits.

### F5 — High: shared wire claims were false

Original sections: contracts §§1, 2.3, 2.9, 2.18, 5; ADR 0002 Wire conventions/Versioning/Golden fixtures; architecture §2.13.

Unbounded u64 costs/timestamps are not exactly representable by TS numbers; intermediate price arithmetic may lose precision. Integers do not fix JSON ordering/escaping or delta segmentation. UTF-8 bounds are not JS string length. Closed enum additions are breaking even with non_exhaustive; mapping unknown values to internal discards meaning. Source TS types are hand-written, not Zod-inferred.

Correction: ADR 0002 specifies exact numeric arithmetic, byte validation, null behavior, semantic fixture comparisons, negative fixtures, and honest versioning. History numeric encoding is deferred with its shared DTO, not mislabeled portable.

### F6 — High: proposed contracts permit contradictory states

Original sections: contracts §§2.13–2.14, 2.17–2.18; ADR 0003 §§2, 4.

ProviderError allowed timeout/cancelled inside provider_error, arbitrary issues on unrelated failures, and a body diagnostics field. CompletionOutcome allowed success without value or failure with value, with usage duplicated at two levels. RunRecord allowed state:error without category and non-error with category. Parse results were optional-field bags; Pricing named both a record and a port.

Correction: preserve the existing completion error enum initially; contracts §4 requires discriminated outcomes and category-specific issues, removes speculative attempts/duplicate states, separates legacy record compatibility from the future summary, and ADR 0003 uses typed internal parser signals.

### F7 — High: structured output had no implementable common contract

Original sections: architecture §2.8; contracts §§2.6, 2.17; ADR 0003 §10; roadmap Phases 1, 2, 4, 8.

Automatic weaker strategies contradicted fail-fast capability rules and explicit fallback permission. schema appeared in both request and method arguments; strict was undefined; unconstrained T was not proven by a JSON schema. No dialect/ref/resource policy existed. Zod alone cannot validate arbitrary JSON Schema. Fence stripping and repair changed Dora behavior; all-wire-integers contradicted dynamic schema/result numbers.

Correction: defer execution; contracts §4.3 lists the single-schema, typed-decoder, bounded-validation, explicit-strategy and buffering requirements before approval. Existing consumers keep their parsers. Provider-native support needs fixtures at implementation time.

### F8 — High: public Vercel escape hatch violated the stated boundary

Original sections: architecture §§2.11–2.12; roadmap Phases 4–5; ADR 0002 framework discussion.

fromLanguageModel accepts a third-party instance even if called opaque. Either its signature leaks that type or broad unknown plus an assertion recreates the coupling. The audit's recommendation was internally inconsistent on this point.

Correction: typed SDK-owned provider factories create Vercel models internally. Application custom adapters implement our Provider contract. No public arbitrary model input.

### F9 — High: security and migration policies changed without evidence of equivalence

Original sections: architecture §§2.5–2.7, 2.14; contracts §2.15; ADR 0003 §§3, 5–7, 11; roadmap Phases 2, 3, 8.

Syntax-only model permission drops Skriuw's model authority. Arbitrary descriptor URLs plus a saved vendor key can redirect credentials. extraHeaders can carry secrets despite a comment. Regex-redacted body excerpts can expose prompts/keys. A resolver has no feedback to rotate on later HTTP errors. Dropping Dora's 401/403/5xx rotation and moving Skriuw from /api/generate to /api/chat are behavior changes.

Correction: ADR 0003 retains model authority, constrains endpoint/credential binding, removes body/header escape hatches and implicit rotation, and preserves Skriuw's endpoint for its first migration. Dora policy changes require its later explicit decision.

### F10 — Medium: core and packages acquired speculative responsibilities

Original sections: architecture §§2.2, 2.6, 2.10–2.11, 2.15–2.17, 3; ADR 0001 Core/Consequences; roadmap Phases 1, 6, 9.

Provider codecs, environment access, unused capabilities, a text-only ContentPart wrapper, future autocomplete controls, retries/router metadata and tokio had no Phase 1 consumer. The provider-to-lifecycle optional dependency contradicted "no process management." Verification/listing were bolted onto every completion implementation. A separate xtask was mandatory without testing whether a target sufficed.

Correction: one production core crate; protocol mechanics in providers; portable event helpers in TS core only when needed; lifecycle composed by applications; admin traits introduced separately. No placeholder crates, async facade, or reserved features.

### F11 — Medium: source audit was treated as authority when convenient

Original sections: architecture opening status; ADR 0002 schema tooling; roadmap exit criteria.

The opening paragraph explicitly overrode reference implementation, against AGENTS.md. Several audit recommendations were accepted as proven behavior: complete SSE parsing, exactly-once delivery, WASM support, a default retry mechanism, and an unchanged schema command. Even some audit statements were inaccurate (e.g. audit §15.1 says SDK result types do not cross a module boundary, but Betalingen's answerWithGroq returns streamText's inferred result).

Correction: source-first status, corrected evidence and limitations throughout. Commands, dependency sets and test counts must be checked at extraction, not copied as immutable facts.

### F12 — Medium: migration gates demanded unnecessary breaking changes

Original sections: roadmap Phases 3, 5, 7–9; architecture §2.16.

Phase 3 combined adopting crates with renderer wire changes and deleted old modules before proving all responsibilities moved. Phase 5 allowed preserving the application event shape but required new typed categories on that same wire. Phase 7 referenced Phase 8 as already underway; Phase 9 promised core never needs to change.

Correction: roadmap separates extraction compatibility from later generalization, preserves app contracts by default, conditions deletions on replacement coverage, and removes circular/future-proofing promises. Real migration tests remain in the owning application.

## 5. Decisions taken

D1 and D2 were the two blocking extraction decisions. Both are now selected. The user asked for concrete phase closure and stated that autonomous decisions are welcome; that authorizes the selection below within the documentation task. It does not authorize implementation, and it does not extend to the later gates listed at the end of this section.

### D1 — Phase 1 lifecycle scope: narrow hardening (selected)

Selected: approve the narrow hardening specified in contracts §3.3 inside Phase 1, as a reviewable step that follows source characterization. It addresses duplicate-id admission on every path, terminal commitment and id reuse, provider-output validation, and panic cleanup, without adding HTTP, retries, a hard timeout scheduler, permanent shutdown, or worker joining.

Rejected alternative: exact extraction with the defects merely documented. The defects are reachable from ordinary concurrent use — the same id can take both the unknown-provider and registered-provider paths, a removed registry entry suppresses both terminal and recording, and a provider panic strands an active entry — so an SDK whose selling point is explicit terminals should not ship them unchanged.

This changes observable edge behavior. Phase 1 therefore keeps characterization and hardening as separately reviewable steps, tests races with barriers rather than sleeps, and continues to state the limitations in contracts §3.3 that hardening does not remove.

### D2 — Recording compatibility: metadata summary with borrowed request (selected)

Selected: the metadata-only `RunSummary` plus a request borrowed for the synchronous callback, specified in contracts §2.8, with a Skriuw-owned adapter reconstructing the existing history record.

```text
RunRecorder.record(&self, summary: RunSummary, request: &CompletionRequest)
```

Rejected alternative: temporarily preserving the existing prompt-bearing record inside the SDK. It minimizes call-site churn, but it makes the extracted contract carry prompts and structurally contradictory state/category combinations, which is the defect F3 and F6 identified.

Prompt retention stays Skriuw's storage-time decision; the SDK summary has no prompt field, storage, or retention policy, and need not be a shared serializable DTO in Phase 1. Contracts §2.8 fixes the status/state mapping and the invocation rules. Phase 1 proves this with a local compatibility harness covering retained and redacted history, unknown-provider synchronous completion, and start failure. No global prompt cache, no new persistence package, no disabled retention, and no reference-repository edits before Phase 3 is authorized.

### Later decisions (not Phase 1 blockers)

Before their respective gates: message-history/wire migration; model-instance identity and catalog provenance; safe history numeric encoding; provider EOF/read-timeout corrections; structured schema dialect/decoder and explicit strategy policy; Dora key-rotation/privacy behavior; optional Specta/Tauri package need. These do not justify reserving fields now.

## 6. Verdict

**Phase 0 COMPLETE. APPROVED FOR PHASE 1.**

The twelve findings in §4 are corrected across README, architecture, contracts, roadmap, and ADRs 0001–0003. The two blocking extraction decisions are selected in §5: narrow lifecycle hardening, and a metadata-only recorder summary with a borrowed request. Contracts §§2.8 and 3.3 are now definitive Phase 1 requirements, and roadmap Phase 1 carries a finite acceptance checklist.

Review passes checked phase scope, shared wire rules, terminal ownership, prompt/credential ownership, provider/lifecycle edges, and future-feature gates for contradictions. Source facts were re-verified against `skriuw-ai/src/lib.rs` and `skriuw-domain/src/ai_history.rs` at closure. No production code, package manifests, or reference repository changes exist; no runtime tests were run, because this phase produced documentation only.

Readiness is not authorization. Phase 1 begins only on an explicit instruction to begin it, and stops at its own exit criteria.
