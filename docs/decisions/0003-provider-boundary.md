# ADR 0003: provider boundary

Status: accepted (Phase 0, 2026-09-06)

## Context

Provider integrations are the most duplicated code in the three repositories (audit §7, §28):

- OpenAI-compatible chat is implemented in Skriuw (`crates/skriuw-ai-remote/src/provider.rs`, `OpenAiCompatible` rows for Groq, DeepSeek, Moonshot, Z.ai, DashScope, AI/ML API) and Dora (`services/ai/compat.rs`, `CompatSpec` consts for OpenAI, Groq, DeepSeek, Kimi, GLM, Qwen, OpenRouter). Both are "a static row per provider interpreted by one client". Skriuw's rows carry `destination` and `stream_usage_option`; Dora's carry `chat_temperature` and `models_url`.
- Gemini is implemented twice; Dora puts the key in the query string (`gemini.rs` line 149), Skriuw in the `x-goog-api-key` header.
- Anthropic exists only in Dora (`anthropic.rs`).
- Ollama generation is implemented twice on different endpoints (`/api/chat` vs `/api/generate`).
- SSE parsing, status-code mapping, cancellation checks, deadline checks, output bounds, and usage extraction are re-implemented per repository, and in Dora per adapter.
- Skriuw's adapter tests (23 local-TCP-server tests) are the only provider tests in any repository.

The desired experience: adding an OpenAI-compatible provider is a data change plus fixtures; only providers whose protocols materially differ need code.

## Decision

### 1. Provider-neutral core

`ai-core` / `packages/core` define the `Provider` contract (`contracts.md` §2.16) and contain no provider name, URL, header, or SDK. The runtime dispatches on `ModelRef.providerId` to a `Provider` registered by the application. A provider's `id()` is its descriptor id or, for custom adapters, a constant.

### 2. Shared skeleton

`crates/ai-providers` has one internal skeleton that every remote adapter uses and must not reimplement:

```text
build request  →  authorize (header placement)  →  send (blocking reqwest, timeout = deadline)
   →  bounded line reader (SSE `data:` or NDJSON dialect)
   →  per line: check cancel · check deadline · parse one event via the dialect → { text?, usage?, finished?, finishReason? }
   →  enforce maxOutputBytes and MAX_RESPONSE_BYTES · validate delta · sink.sendDelta
   →  terminal: done{usage, finishReason} | provider_error(status_error / transport_error)
```

Owned by the skeleton: stream handling, bounds, cancellation, timeouts, error mapping (default status → category table), usage extraction, drain-and-discard of error bodies under a size cap, redaction for diagnostics. Source: `crates/skriuw-ai-remote/src/lib.rs::stream_completion`, `status_error`, `transport_error`, `sse_payload`.

In TypeScript, `packages/ai-sdk` plays the same role: one mapping from `fullStream` parts and `APICallError` to `CompletionEvent`/`ProviderError`, written once, used for every provider the Vercel AI SDK supports.

### 3. OpenAI-compatible providers are data

```text
type ProviderDescriptor = {
  id:                Id<Provider>              // canonical, one per vendor: vendor name (moonshot, zai, dashscope), not product name
  label:             string
  destination:       string                    // host shown in disclosures ("api.groq.com")
  baseUrl:           Url
  chatPath:          string                    // "v1/chat/completions"
  modelsPath?:       string                    // absent → no listing
  auth:              bearer | header { name }  // never query
  streamUsageOption: bool                      // whether stream_options.include_usage is accepted (Z.ai: false)
  jsonMode:          CapabilitySupport         // provider-level default for json_object
  jsonSchema:        CapabilitySupport         // provider-level default for json_schema
  envKeyPrefix?:     string                    // "GROQ" → GROQ_API_KEY for the env resolver
  extraHeaders:      Map<name, value>          // fixed, non-secret (OpenRouter HTTP-Referer / X-Title)
  listingFilter?:    openai_chat_only | aggregator_chat_type   // bounded enum of known listing quirks
}
```

`specs/data/providers.json` holds the rows; `OpenAiCompatibleProvider::new(descriptor, credentials)` interprets them. Initial rows: openai, groq, deepseek, moonshot, zai, dashscope, aimlapi, openrouter. Adding Cerebras, SambaNova, MiniMax, or a self-hosted vLLM is a row plus fixtures, no code. Known deviations become bounded row flags (as `streamUsageOption` already is), never free-form escape hatches. If a deviation cannot be expressed as a bounded flag, the provider gets a custom adapter instead.

Applications may register additional rows at runtime (Dora lets users type endpoints for Ollama; a self-hosted OpenAI-compatible server is the same case), validated by the same rules.

### 4. Providers requiring custom adapters

Anthropic (`/v1/messages`, `content_block_delta`, `x-api-key`, `anthropic-version`), Gemini (`streamGenerateContent?alt=sse`, `systemInstruction`, `x-goog-api-key`, `usageMetadata`), and Ollama (`/api/chat` NDJSON, `num_predict`, eval counts, local error mapping) implement a small **dialect** on the skeleton:

```text
dialect {
  endpoint(descriptor, model, streaming) -> Url
  authorize(request, credential)                       // header only
  completionBody(request, streaming) -> Json
  parseEvent(line) -> { text?, usage?, finished?, finishReason? } | Skip | Malformed
  verificationBody(model) -> Json                       // max_tokens: 1 class
  parseModelListing(body) -> ModelInfo[]
  mapStatus?(status, body) -> (ErrorCategory, RecoveryAction)   // only for documented quirks; default table otherwise
}
```

This is `RemoteProviderKind`'s method set in Skriuw (`provider.rs`: `endpoint`, `authorize`, `completion_body`, `parse_event`, `verification_body`, `parse_model_listing`); the Gemini arm is roughly 150 lines and is the size budget for a custom adapter. Anthropic's shapes come from Dora's `anthropic.rs`; Ollama's `/api/chat` mapping from Dora's `ollama.rs` on Skriuw's bounded reader.

Providers that need a different transport entirely (WebSocket, gRPC) are out of scope until a consumer needs one.

### 5. Credentials

Adapters receive a `CredentialSource` and resolve per request, after the runtime has validated the request and after the adapter has checked the model is permitted, so nothing is resolved and no socket opens for a request that will be refused (`RemoteAiProvider::complete` ordering). Credentials go in headers only. Adapters never store, log, or serialize credentials; `Credential` is zeroized on drop and prints redacted. Local providers (Ollama, fake) take no credential source.

### 6. Model discovery

`listModels()` is explicit, never automatic on startup (Skriuw fetches only on "Refresh models"; Dora falls back to curated on failure). Results are `ModelInfo` with `source: listed`. Merge rule: catalog beats listed beats unknown; listed entries widen the permitted set (`RemoteAiModelAuthority` in Skriuw). A model id typed by a user is permitted only after validation as `Id<Model>`, and is encoded before use in a URL segment.

### 7. Model capabilities

Adapters expose `modelInfo(model)` from three layers: descriptor defaults (`jsonMode`, `jsonSchema`, `streaming: yes` for every OpenAI-compatible endpoint), the shipped catalog, and listing results. Everything unknown is `unknown`. Adapters never derive capabilities from model-name substrings. Applications may override with `source: declared`.

### 8. Streaming

Every adapter streams by default and must honor the invariants in `contracts.md` §3 as a provider: check cancel and deadline on every read, enforce bounds before emitting, return exactly one terminal, emit nothing after it. Non-streaming requests (`complete`, `generateObject`) use the provider's non-stream endpoint where one exists and a single-terminal path otherwise; the runtime still presents the same event sequence internally.

### 9. Usage

Adapters request usage where the protocol allows (`stream_options.include_usage` when `streamUsageOption`; Gemini `usageMetadata`; Ollama `prompt_eval_count`/`eval_count`; Anthropic `message_start`/`message_delta` usage) and put it on `done.usage` with `source: reported`. Adapters never estimate; the runtime estimates when usage is absent.

### 10. Structured output support

Adapters implement the `native` strategy when the protocol has one (OpenAI `response_format: json_schema`, Gemini `responseSchema`, Ollama `format`, Anthropic single forced tool) and the `json_mode` hint when the descriptor says `jsonMode: yes | unknown`. The skeleton renders the schema into the system prompt for `json_mode` and `prompt_only`. Parsing, validation, and repair live in core, not in adapters. A 400 in response to a JSON hint on an `unknown` descriptor maps to `invalid_request`, and the application (or a future router) may retry with a lower strategy.

### 11. Errors

The default status → category table lives in the skeleton and is mirrored by `fixtures/errors/`:

```text
400/413/422 → invalid_request       401/403 → invalid_credential     402 → quota_exceeded
404 (model)  → model_unavailable     408 / deadline → timeout terminal 429 → rate_limited (+ Retry-After)
5xx          → provider_unavailable  connect refused (local) → local_runtime_unavailable
404 (local model) → local_model_missing   parse/bounds → malformed_response
```

An adapter overrides a mapping only for a documented quirk, with a comment stating the upstream behavior (per `AGENTS.md` comment policy). Provider bodies never reach `message`; a bounded redacted excerpt goes to `ErrorSource.bodyExcerpt` only. Dora's key rotation on 401/403/5xx is not carried over.

### 12. Fixtures

Every descriptor row and every custom adapter ships at least: one happy-path stream fixture (`.sse`/`.ndjson` + expected `events.json`), one usage fixture, one malformed-stream fixture, and an error table covering each status in §11. Fixtures are replayed by the Rust conformance suite against a local TCP fixture server (Skriuw's `serve_once`) and by the TypeScript suite through injected `fetch` (Betalingen's pattern). Adding a provider without fixtures fails CI.

### 13. Provider conformance testing

One suite, parameterized over every registered adapter and descriptor row:

1. streams ordered, gapless deltas and exactly one terminal
2. honors cancellation before the request is sent and mid-stream
3. honors the deadline mid-stream with a `timeout` terminal
4. enforces `maxOutputBytes` and `MAX_RESPONSE_BYTES` with `malformed_response`
5. maps every status in the error table to the expected category and recovery
6. opens no socket when the credential is missing or refused
7. never places the credential in a URL or in any error text
8. parses reported usage when present and omits it when absent
9. handles `[DONE]`, missing terminal, oversized event, malformed JSON line
10. `verify` sends a minimal body and maps its status
11. `listModels` parses the provider's listing shape into validated `ModelInfo`
12. applies `jsonMode`/`jsonSchema` hints only when the descriptor permits

Live-provider smoke tests (one minimal call per adapter) are opt-in behind `AI_LIVE_TESTS=1` and provider keys, never default CI.

## Consequences

- Phase 2 produces one skeleton, one data-driven adapter, three dialects, the descriptor and catalog data, and the conformance suite. Dora's ~1,700 lines of adapters are superseded rather than ported.
- Provider ids become canonical across applications; Dora's stored settings need a mapping (Phase 8).
- The TypeScript side inherits provider breadth from the Vercel AI SDK; the same descriptor rows configure `@ai-sdk/openai-compatible` through `openaiCompatible(descriptor, credentials)`, so a provider added as data is available in both languages.
- Security properties (header-only credentials, body isolation, bounded reads, no socket without credential) are enforced by tests rather than by review.
