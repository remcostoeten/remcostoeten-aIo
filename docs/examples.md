# Example usage

Worked examples for the code that exists today: `crates/ai-core`, `crates/ai-providers`, `packages/core` and `packages/ai-sdk`. Everything below uses only exported API and the shapes in [contracts](contracts.md).

Nothing here is a feature announcement. Retries, key rotation, provider fallback, structured output and routing do not exist in either language, and no example works around their absence.

The two languages implement the same seam rather than sharing an implementation: a validated request goes in, ordered deltas and exactly one terminal come out. Read an example in the language you are not using and the other one will be recognisable.

## Contents

- [TypeScript](#typescript)
  - [Stream from the deterministic fake](#stream-from-the-deterministic-fake)
  - [Reach a real provider](#reach-a-real-provider)
  - [Serve a conversation over NDJSON](#serve-a-conversation-over-ndjson)
  - [Consume that stream](#consume-that-stream)
  - [Handle every terminal](#handle-every-terminal)
  - [Cancel a run](#cancel-a-run)
  - [Accept an untrusted request](#accept-an-untrusted-request)
- [Rust](#rust)
  - [Stream from the deterministic fake](#stream-from-the-deterministic-fake-1)
  - [Reach a remote provider](#reach-a-remote-provider)
  - [Generate against a local Ollama](#generate-against-a-local-ollama)
  - [Cancel a run](#cancel-a-run-1)
  - [Record what a run cost](#record-what-a-run-cost)
  - [List a provider's models](#list-a-providers-models)
- [What no example shows](#what-no-example-shows)

## TypeScript

`@ai-sdk-local/core` has no dependencies: a browser consumer that imports it pulls in nothing else. `@ai-sdk-local/ai-sdk` adds the Vercel AI SDK and the vendor package for whichever factory is called, so it belongs on a server.

Both are workspace packages. There is no published release channel yet, so a consumer in this repository depends on them through the Bun workspace:

```json
{
  "dependencies": {
    "@ai-sdk-local/core": "workspace:*",
    "@ai-sdk-local/ai-sdk": "workspace:*"
  }
}
```

### Stream from the deterministic fake

The fake ships in core, needs no key and no network, and produces the same segmentation every time — which is what makes an application's own tests deterministic.

```ts
import { buildRequest, consumeEvents, createFakeProvider, createRuntime, successScript } from '@ai-sdk-local/core'

const runtime = createRuntime({
  providers: [createFakeProvider(successScript(['Blue', ' and', ' green.']))],
})

const built = buildRequest({
  requestId: 'request-1',
  providerId: 'fake',
  modelId: 'model',
  messages: [{ role: 'user', content: 'Name two colours.' }],
})
if (!built.ok) throw new Error(built.error.reason)

const started = runtime.stream(built.request)
if (!started.ok) throw new Error(started.error.reason)

const outcome = await consumeEvents(started.events)
// outcome.status === 'done', outcome.text === 'Blue and green.'
```

`buildRequest` takes the `messages` array an application actually holds and splits it into the contract's shape: the last turn becomes `userPrompt`, everything before it becomes `priorMessages`. It refuses a conversation whose last turn is not the user's rather than promoting an assistant turn into the question.

Both results are discriminated unions, so a request that never ran cannot be mistaken for a run that produced no events.

### Reach a real provider

Two ports have no defaults, deliberately. `models` decides which model ids this application permits; `credentials` produces the key. Both are consulted before a socket is opened, so an unauthorized model or an unconfigured provider fails without any network access.

```ts
import { createRuntime } from '@ai-sdk-local/core'
import { createGroqProvider, staticCredential } from '@ai-sdk-local/ai-sdk'

const PERMITTED = new Set(['llama-3.3-70b-versatile'])

const provider = await createGroqProvider({
  credentials: staticCredential(process.env['GROQ_API_KEY'] ?? ''),
  models: { permits: (_providerId, modelId) => PERMITTED.has(modelId) },
})

const runtime = createRuntime({ providers: [provider] })
```

`createGeminiProvider` and `createAnthropicProvider` take the same options. `createOpenAiCompatibleProvider` additionally requires a `providerId` and a `baseURL`, because "compatible" has no default host and because two endpoints or two accounts are two providers:

```ts
import { createOpenAiCompatibleProvider } from '@ai-sdk-local/ai-sdk'

const provider = await createOpenAiCompatibleProvider({
  providerId: 'internal-gateway',
  baseURL: 'https://gateway.internal/v1',
  credentials: staticCredential(gatewayKey),
  models: { permits: () => true },
})
```

Reading `process.env` is the application's, above the SDK: nothing in either package touches it, which is what lets the same code run in Node, Bun, a Worker or a Tauri sidecar.

A `CredentialSource` may be async and may refuse. A refusal is a closed vocabulary — `missing` or `withheld` — and arrives at the consumer as a typed `missing_credential` or `invalid_credential` terminal, never as a thrown error:

```ts
import type { CredentialSource } from '@ai-sdk-local/ai-sdk'

function keyring(userId: string): CredentialSource {
  return {
    async resolve(providerId) {
      if (!(await hasConsented(userId, providerId))) {
        return { ok: false, refusal: 'withheld', message: 'this workspace has not enabled remote AI' }
      }
      const apiKey = await readKey(userId, providerId)
      return apiKey ? { ok: true, apiKey } : { ok: false, refusal: 'missing', message: 'no key is configured' }
    },
  }
}
```

### Serve a conversation over NDJSON

`toNdjsonStream` turns the event stream into a `ReadableStream<Uint8Array>`, so a route is a Web `Response` and nothing more. The same function works under Hono, Next.js, a bare Worker or `Bun.serve`.

```ts
import { buildRequest, createRuntime, defaultParameters, toNdjsonStream, type Message } from '@ai-sdk-local/core'

async function chatRoute(messages: readonly Message[]): Promise<Response> {
  const built = buildRequest({
    requestId: crypto.randomUUID(),
    providerId: 'groq',
    modelId: 'llama-3.3-70b-versatile',
    systemPrompt: 'You answer in one paragraph.',
    messages,
    parameters: { ...defaultParameters(), maxOutputTokens: 1800, temperatureMillis: 200 },
  })
  if (!built.ok) return Response.json({ error: built.error.reason }, { status: 400 })

  const started = runtime.stream(built.request)
  if (!started.ok) return Response.json({ error: started.error.reason }, { status: 400 })

  return new Response(toNdjsonStream(started.events), {
    status: 200,
    headers: {
      'content-type': 'application/x-ndjson; charset=utf-8',
      'cache-control': 'no-store',
      'x-content-type-options': 'nosniff',
    },
  })
}
```

Sampling values are thousandths — `temperatureMillis: 200` is a temperature of 0.2 — so the wire contract stays integers. `maxOutputTokens` is what the provider is asked for; `maxOutputBytes` is the local accumulation cap, enforced on this side of the boundary whether or not the provider respects the token ceiling.

Nothing reaches the provider until the response body is read. The stream is lazy, so a client that disconnects before reading spends nothing.

Each line is one event document, exactly the shape in `specs/fixtures/valid/`:

```
{"type":"delta","requestId":"request-1","sequence":0,"text":"Blue"}
{"type":"done","requestId":"request-1","usage":{"inputTokens":12,"outputTokens":34}}
```

### Consume that stream

`fromNdjsonStream` decodes bytes back into events, validating each line; `consumeEvents` folds them into one outcome while checking the streaming invariants — one request id, sequences from zero increasing by one, at most one terminal, nothing after it.

```ts
import { consumeEvents, fromNdjsonStream } from '@ai-sdk-local/core'

const response = await fetch('/api/chat', { method: 'POST', body: JSON.stringify({ messages }) })
if (!response.body) throw new Error('no body')

const outcome = await consumeEvents(fromNdjsonStream(response.body), {
  onText: (text) => appendToEditor(text),
})
```

Checking is not repair. A sequence gap is reported as a `violated` outcome, never filled; text is never reordered; an event carrying a foreign request id is refused rather than adopted. The accumulated text comes back in every case, including a violation — showing a partial answer is the application's call, not this function's.

### Handle every terminal

`CompletionOutcome` is a closed union, so a `switch` over it is exhaustive under `strict` TypeScript and a new state would break the build rather than fall through a default.

```ts
import type { CompletionOutcome } from '@ai-sdk-local/core'

function present(outcome: CompletionOutcome): string {
  switch (outcome.status) {
    case 'done':
      return outcome.usage ? `${outcome.text} (${outcome.usage.outputTokens} tokens)` : outcome.text
    case 'cancelled':
      return outcome.text
    case 'timeout':
      return `${outcome.text}\n\nThe model ran out of time.`
    case 'provider_error':
      return describe(outcome.error.category, outcome.error.recoveryAction)
    case 'violated':
      return `${outcome.text}\n\nThe stream broke: ${outcome.violation.kind}.`
  }
}
```

Branch on `category` and `recoveryAction`, not on a status code or a message. The categories are provider-independent (`rate_limited`, `missing_credential`, `quota_exhausted`, `transport_failure`, …) and the message is a bounded, normalized sentence — no provider response body crosses the adapter boundary, because bodies carry echoed prompts and, from some providers, a key prefix.

### Cancel a run

Two independent mechanisms, and the terminal tells them apart: a caller's `AbortSignal` and the runtime's own registry.

```ts
const controller = new AbortController()
const started = runtime.stream(request, { signal: controller.signal })

// From the UI:
controller.abort()

// Or, from anywhere holding the id:
runtime.cancel(request.requestId)

// Or, on shutdown:
runtime.shutdown()
```

Either produces a `cancelled` terminal. `parameters.timeoutMs` elapsing produces `timeout` instead, so "the person stopped it" stays distinguishable from "it ran out of time" — a distinction only the runtime can make, since it aborts the same signal to enforce the deadline.

Aborting cannot interrupt a provider wedged inside an un-abortable await. The run terminalizes as cancelled and the generator is returned; a provider that ignores its signal keeps its own work alive.

### Accept an untrusted request

At an HTTP or IPC boundary, `decodeRequest` proves shape and values in one step and keeps the two stages distinguishable in its rejection.

```ts
import { decodeRequest, describeValidationError } from '@ai-sdk-local/core'

const result = decodeRequest(await request.json())
if (!result.ok) {
  const { rejection } = result
  return Response.json(
    rejection.stage === 'decode'
      ? { error: 'malformed request', path: rejection.issue.path, code: rejection.issue.code }
      : { error: describeValidationError(rejection.error) },
    { status: 400 },
  )
}

const started = runtime.stream(result.request)
```

Do not hand a parsed body straight to `runtime.stream`. The runtime validates and would reject it, but the rejection reaches you as a `StartError` rather than as the decode issue that explains which field was wrong.

## Rust

`ai-core` is the contracts, the service, the ports and the fake: no HTTP client, no credential store, no framework, no async runtime. `ai-providers` adds the adapters, behind the `ollama` and `remote` features (both on by default).

```toml
[dependencies]
ai-core = { path = "../ai-sdk/crates/ai-core" }
ai-providers = { path = "../ai-sdk/crates/ai-providers", default-features = false, features = ["remote"] }
```

The crates are consumed as path dependencies today; a release channel is unresolved, and [the handoff](../HANDOFF.md) states what that blocks.

### Stream from the deterministic fake

The service is synchronous and thread-based: `start` admits a request, runs it on its own worker, and delivers events through a channel the caller supplies.

```rust
use std::sync::{Arc, mpsc};

use ai_core::{
    AiComplete, AiCompletionChannel, AiCompletionEvent, AiCompletionParameters,
    AiCompletionRequest, AiCompletionService, AiSinkError, FakeAiProvider, FakeCompletionScript,
};

struct Collector(mpsc::Sender<AiCompletionEvent>);

impl AiCompletionChannel for Collector {
    fn send(&self, event: AiCompletionEvent) -> Result<(), AiSinkError> {
        self.0.send(event).map_err(|_| AiSinkError::Closed)
    }
}

let provider: Arc<dyn AiComplete> =
    Arc::new(FakeAiProvider::new(FakeCompletionScript::success(["Blue", " and green."])));
let service = AiCompletionService::new([("fake".to_owned(), provider)]);

let (sender, events) = mpsc::channel();
service.start(
    "editor".to_owned(),
    AiCompletionRequest {
        request_id: "request-1".to_owned(),
        provider_id: "fake".to_owned(),
        model_id: "model".to_owned(),
        system_prompt: String::new(),
        user_prompt: "Name two colours.".to_owned(),
        prior_messages: Vec::new(),
        parameters: AiCompletionParameters::default(),
    },
    Collector(sender),
)?;

let mut text = String::new();
let mut failure = None;
for event in events {
    match event {
        AiCompletionEvent::Delta(delta) => text.push_str(&delta.text),
        AiCompletionEvent::Done { .. }
        | AiCompletionEvent::Cancelled { .. }
        | AiCompletionEvent::Timeout { .. } => break,
        AiCompletionEvent::ProviderError { error, .. } => {
            failure = Some(error);
            break;
        }
    }
}
```

The first argument to `start` is the run's origin — an application label that reaches accounting and never reaches a provider. Dropping the receiving end cancels the run: a closed channel is how a consumer says it stopped listening.

An unregistered provider id is not a start error. It produces a synchronous `unavailable_provider` terminal and returns `Ok(())`, so a caller never has two ways to learn the same thing.

### Reach a remote provider

Seven descriptors ship: Gemini, Groq, DeepSeek, Moonshot, Z.ai, DashScope and AI/ML API. The same two ports as in TypeScript, and the same ordering guarantee — the authority is consulted, then the credential resolves, and only then is a socket opened.

```rust
use std::sync::Arc;

use ai_core::{AiComplete, AiCompletionService};
use ai_providers::{
    AiCredential, AiCredentialError, AiCredentialSource, AiModelAuthority, RemoteAiProvider,
    RemoteProviderKind,
};

struct SettingsKeys {
    groq: String,
}

impl AiCredentialSource for SettingsKeys {
    fn resolve(&self, _provider_id: &str) -> Result<AiCredential, AiCredentialError> {
        AiCredential::new(self.groq.clone())
    }
}

struct CatalogAuthority;

impl AiModelAuthority for CatalogAuthority {
    fn permits(&self, provider_id: &str, model_id: &str) -> bool {
        provider_id == "groq" && model_id == "openai/gpt-oss-20b"
    }
}

let provider = RemoteAiProvider::new(
    RemoteProviderKind::Groq,
    Arc::new(SettingsKeys { groq: key }),
    Arc::new(CatalogAuthority),
    "my-app/1.0",
)?;

let service = AiCompletionService::new([(
    "groq".to_owned(),
    Arc::new(provider) as Arc<dyn AiComplete>,
)]);
```

`AiCredential::new` accepts only printable ASCII within its length bounds, and the credential never enters anything serializable — `expose()` is the single accessor, which keeps every read greppable.

Each descriptor uses its own published endpoint. Unlike the TypeScript factories, there is no public base-URL override: the constructor takes a kind, not a host, so a Rust consumer cannot redirect a vendor's conversation and key elsewhere.

### Generate against a local Ollama

Generation and lifecycle are separate responsibilities. This adapter only generates — detecting, installing, spawning and pulling stay with the application until a phase extracts them.

```rust
use ai_providers::{OLLAMA_PROVIDER_ID, OllamaProvider};

let provider = OllamaProvider::new(None, "my-app/1.0")?; // http://127.0.0.1:11434
let service = AiCompletionService::new([(
    OLLAMA_PROVIDER_ID.to_owned(),
    Arc::new(provider) as Arc<dyn AiComplete>,
)]);
```

A non-loopback endpoint is refused rather than accepted quietly: whether a remote Ollama is acceptable is a privacy decision belonging to the application, and no option for it exists yet. There is also no fallback from here to a remote provider — Ollama being unavailable must never become "send the document to a cloud model".

### Cancel a run

```rust
let cancelled = service.cancel("request-1"); // false when there is no such active run
service.shutdown(); // requests cancellation of every active run
```

Cancellation is cooperative and best-effort. A provider blocked in a read is not interrupted, and `timeout_ms` is observed by the provider rather than enforced by a service watchdog — [contracts §3](contracts.md) states the limits precisely.

### Record what a run cost

The recorder is called once per committed run, after the terminal is already published and with no service lock held, so accounting never delays delivery. It receives a metadata-only summary plus the request borrowed for the duration of the call.

```rust
use ai_core::{AiCompletionRequest, AiRunRecorder, AiRunStatus, AiRunSummary};

struct History;

impl AiRunRecorder for History {
    fn record(&self, summary: AiRunSummary, request: &AiCompletionRequest) {
        let prompt = retains_prompts().then(|| request.user_prompt.clone());
        let outcome = match summary.status {
            AiRunStatus::Done => "done".to_owned(),
            AiRunStatus::Cancelled => "cancelled".to_owned(),
            AiRunStatus::Timeout => "timeout".to_owned(),
            AiRunStatus::ProviderError { category } => format!("failed: {category:?}"),
        };
        queue_insert(summary.run_id, summary.duration_ms, summary.tokens, prompt, outcome);
    }
}

let service = AiCompletionService::new(providers).recording(Arc::new(History), pricing);
```

An application that retains prompts copies them out inside the callback, before queueing anything. That keeps retention and redaction in the application's storage layer instead of making the SDK cache prompts.

`summary.tokens.source` distinguishes counts a provider reported from counts derived from transferred bytes; an estimate must never be presented as exact. `summary.cost_micros` being `None` means unpriced, never free.

### List a provider's models

Listing is administration, not completion, so it is a method on the provider rather than an event on a run.

```rust
match provider.list_models() {
    Ok(listings) => for listing in listings {
        println!("{} — {:?}", listing.model_id, listing.source);
    },
    Err(error) => report(error.category, error.recovery_action),
}
```

A listing narrows what is offered; it never widens what `AiModelAuthority` permits. Some descriptors publish no listing at all, which fails as `unavailable_provider` and is not the same as a successful empty listing. `verify_credential` spends one key on the smallest metered request a provider supports and reports only whether it was accepted — evidence about that key and that model at that moment, not a claim about quota or capabilities.

## What no example shows

Because none of it exists:

- retries, backoff, or key rotation — a credential source is called once per run and cannot react to a later 429
- fallback between providers or models, and never from a local endpoint to a remote one
- structured output, JSON mode, or schema-constrained generation
- tools, agents, or embeddings
- environment variable lookup, credential storage, or consent policy
- routing, health tracking, or cost-based model selection
- React, Hono, Next.js or Tauri helpers

Each is either an application responsibility today or an unscheduled phase in [the roadmap](roadmap.md). An example that faked one would be a promise the code does not keep.
