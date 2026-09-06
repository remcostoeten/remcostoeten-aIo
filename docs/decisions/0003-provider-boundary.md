# ADR 0003: provider boundary

Status: accepted at Phase 0 closure, 2026-09-06. Phase 2 design direction, not permission to implement or a promise that every adapter can share one parser. Phase 2 still requires its own explicit approval.

## Context

Skriuw's `OpenAiCompatible` rows and Dora's `CompatSpec` demonstrate shared HTTP protocol behavior. Anthropic, Gemini, and Ollama have different response framing and request schemas. Current Skriuw uses Ollama /api/generate; Dora uses /api/chat. Changing endpoint/message construction can change output and is not a neutral extraction.

Skriuw's `RemoteAiModelAuthority` also restricts models before credential resolution. Syntax validation of an arbitrary model id does not preserve that authorization policy.

## Decision

### 1. Core seam

Keep Phase 1's completion-only trait and application registration map. No URLs, real provider names, headers, credential persistence, or provider SDK types in core. Administration (verification/discovery) is separately typed at the provider boundary when introduced; no compulsory dummy methods for fake/local providers.

### 2. Shared provider infrastructure

One Rust provider crate has meaningful HTTP dependencies, feature-gated by protocol. Extract common bounded reads, framing, cancellation/deadline checks, error normalization, and usage validation internally. Do not export a universal HTTP/dialect framework or force every protocol into a line parser.

The original `{text?, usage?, finished?, finishReason?}` parse result allowed empty/contradictory states. A later internal parser should produce typed events such as Text, Usage, Finished, Skip, and Failed, with a bounded sequence when one frame contains several signals. Aggregate start/end usage correctly. A finish reason is not necessarily transport completion; usage may arrive after text finishes.

SSE framing must distinguish an event from a line (multi-line data, blank delimiters, comments, CRLF, split UTF-8). Skriuw's `sse_payload` is a provider-specific line helper, not a complete generic SSE decoder. NDJSON has a different framing contract. Bound both per-frame allocation and total raw bytes, including ignored frames; a capped reader must not disguise truncation as successful EOF.

The TypeScript adapter reuses Vercel's parser where appropriate and maps its typed parts internally. It does not introduce a second competing provider SSE parser in TS core.

### 3. Descriptor scope

Start from actual Skriuw rows. Add Dora's OpenAI/OpenRouter rows with fixtures when needed. No prospective Cerebras/SambaNova/MiniMax rows, runtime endpoint registration system, or flags without a consumer.

A descriptor belongs to the provider package/spec data, not CompletionRequest. Its typed fields describe endpoint, authentication mode, usage support, and known listing behavior. Descriptor and runtime credential binding are distinct.

Do not expose `extraHeaders: Map<string,string>` as a public escape hatch. If OpenRouter attribution becomes needed, use bounded named attribution fields. Header-auth extensions must use validated header names, prohibit auth/host overrides and CRLF, and carry secret values only via the credential port. Self-hosted endpoints may need no authentication; model that explicitly when supported.

Vendor identity is not runtime registration identity. Multiple endpoint/account instances require distinct ids and credential bindings. A shipped vendor id can be a default registration, not a universal uniqueness rule.

A new row is sufficient only when protocol fixtures prove it fits the implemented behavior. Provider-specific SDK packages may have behavior not reproduced by a generic OpenAI-compatible adapter. Do not promise that arbitrary descriptors automatically work identically in Rust and Vercel's provider factories.

### 4. Endpoint and credential safety

Credential resolution follows request validation and application model authorization. Recheck cancellation before sending after a potentially blocking resolver. Credential resolution time and network time must have documented budgets; the core currently has no watchdog.

Cloud credentials go in headers, never URL parameters. Endpoint construction validates scheme, authority, userinfo, path joining, and disclosure destination. Preserve Skriuw's endpoint pinning and model authority. A custom endpoint, if later approved, must have an explicitly bound credential source and destination; arbitrary request input cannot redirect a saved vendor key. Redirect behavior must not forward credentials to another origin.

Provider-specific error bodies remain internal and discarded by default. Regex prefix redaction cannot guarantee that arbitrary bodies contain neither prompts nor secrets. Do not serialize an ErrorSource or enable body excerpts via a global runtime flag.

### 5. Custom adapters and compatibility

Preserve Skriuw's /api/generate request body and stream behavior for its first migration. /api/chat support for Dora is a separately tested adapter mode or later approved migration; it must not silently replace Skriuw's endpoint.

Preserve model authority separately from metadata/discovery. A listing can extend Skriuw's permitted set only through its application policy. An unknown model being syntactically valid is not equivalent to permission to invoke it.

Model listing, reachability checks, and credential verification are different operations. Verification must be explicit and bounded, may incur a minimal call, and cannot claim all models/capabilities are usable. Unsupported listing differs from an empty successful listing. Choose explicit typed outcomes when these operations are designed.

Ollama's generation adapter does not install, spawn, stop, or auto-start processes. Applications compose lifecycle and generation. Locality is based on the configured endpoint, never the model name; loopback alone is not a proof of where inference ultimately happens.

### 6. Completion and terminals

Use one completion seam. A later text-collecting convenience accumulates deltas from that seam. Do not secretly switch to a non-stream endpoint behind a convenience API; a native non-stream path needs its own equivalence fixtures.

Providers generate ordered deltas and return a terminal. Service guarantees, known holes, and the approved Phase 1 hardening are specified in `contracts.md` §3; adapters must satisfy the identity, sequence and bounds validation the service will enforce there rather than assume it forwards unchecked. Cooperative checks do not interrupt blocked reads. Preserve existing behaviors for initial fixtures, then approve any correction explicitly: remote Skriuw currently accepts EOF after a parsed event, whereas Ollama requires a terminal flag; read timeout errors can be reported as transport failures.

Later adapter conformance should require explicit protocol completion and distinguish truncated/oversized streams. These are deliberate improvements, not claims about every source adapter today.

### 7. Errors, usage, and retries

Preserve existing categories for extraction. A later semantic split is versioned with caller mappings. HTTP status alone is insufficient to identify all conditions (a 404 may be an endpoint error; 403 may be permission rather than a bad key). Use provider-specific typed body codes internally where fixtures justify them, with safe messages.

Do not fabricate missing usage as zero or turn partial provider usage into complete reported totals. Validate both counters and document estimation separately. Cost accounting remains advisory.

Automatic retries and key rotation are not Phase 1 or implicit Phase 2 behavior. A resolver called once cannot react to a later 429; rotation needs an execution owner and feedback, not merely a CredentialSource decorator. Dora's current rotation on 401/403/5xx and transport failure remains an application compatibility decision. No automatic local-to-remote fallback and no provider/model switch after visible output.

### 8. Structured output

Deferred pending the gate in `contracts.md` §4.3. Native schema, JSON mode, and prompt-based generation are distinct capabilities with explicit caller permission. No automatic repair, format downgrade, forced-tool machinery, or schema prompt injection is introduced by the provider extraction.

### 9. Fixtures and verification

Each implemented adapter/descriptor ships local fixtures for request construction, stream text, usage, malformed/truncated/oversized input, relevant errors, and credential/model refusal before network. Include cancellation before send and during reads, closed consumer, timeouts, final usage after text finish, and destination/redirect checks.

Phase 2 first characterizes Skriuw's behavior; approved hardening gets separate expected fixtures. TypeScript later compares semantic results, not arbitrary delta chunk boundaries. A provider without its required fixtures fails conformance. Live smoke tests remain explicit opt-in; no paid access in normal CI.

## Consequences

One HTTP crate is justified; separate crates per vendor, a generic dialect public API, and a core codec package are not. Provider breadth is added incrementally and never requires weakening the public request. Changing source error, endpoint, retry, or EOF behavior is recorded as a compatibility change before an application switches.
