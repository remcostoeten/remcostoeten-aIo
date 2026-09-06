# AI SDK

This repository will become a reusable, provider-agnostic AI SDK for Rust and TypeScript applications.

The project is currently in the architecture and extraction-planning stage.

Do not treat this as a greenfield AI framework.

Much of the correct architecture already exists in the reference applications, especially Skriuw. The goal is to extract, generalize, and unify proven concepts while keeping product-specific behavior inside the applications that own it.

Because this is an SDK, public contract quality, type safety, backwards compatibility, predictable behavior, and invalid-state prevention take priority over implementation convenience.

## Reference repositories

These repositories are read-only references for this SDK:

* Dora: `/home/remcostoeten/dev/dora`
* Skriuw: `/home/remcostoeten/dev/skriuw`
* Betalingen: `/home/remcostoeten/dev/betalingen`

Never modify these repositories unless a future task explicitly requests a migration inside that repository.

The architecture audit is available at:
`./dev-docs/knowledge/architecture-research-across-my-ai-apps.md`

Read it before making architectural decisions.

If documentation and implementation disagree in a reference repository, implementation wins.

## Primary goal

Create a small reusable AI foundation that applications can consume without duplicating:

* completion request contracts
* streaming contracts
* cancellation
* provider adapters
* provider errors
* provider verification
* model identity
* model capabilities
* model discovery
* structured output
* credential resolution ports
* usage metadata
* deterministic fake providers
* shared contract validation
* eventually provider/model routing
* eventually native Ollama runtime management
* eventually optional Tauri and React helpers

The SDK must support both:

* Rust
* TypeScript/JavaScript

Potential consumers include:

* Tauri
* Bun
* Node.js
* Hono
* React
* Next.js server environments
* serverless runtimes

Framework integrations must remain optional.

## Non-goals

Do not turn this project into:

* an agent framework
* a LangChain replacement
* a workflow engine
* a chains framework
* a prompt marketplace
* a vector database abstraction
* an embeddings platform
* a React UI library
* a chat UI kit
* a database abstraction
* an application state library
* a Tauri plugin with fixed commands
* a shared telemetry database
* a Hono framework
* a Next.js framework
* a replacement for Vercel AI SDK

Do not add capabilities because they may be useful someday.

Add abstractions only when supported by an existing consumer or a clearly approved upcoming requirement.

## Core architectural boundary

The SDK owns AI execution.

Applications own product meaning.

The SDK may understand concepts such as:

```text
ModelRef
ModelInfo
Capability
Message
CompletionRequest
CompletionEvent
Provider
ProviderError
CredentialSource
Runtime
ResponseFormat
Usage
```

The SDK must not understand concepts such as:

```text
Dora database connections
Dora database schemas
Dora SQL editor state

Skriuw notes
Skriuw workspaces
ProseMirror selections
Skriuw task mutations

Betalingen financial records
Betalingen screen context
Betalingen authorization rules

application-specific prompts
application-specific consent copy
application-specific UI state
```

The intended flow is:

```text
application
    ↓
build product-specific context
    ↓
build messages/request
    ↓
AI runtime
    ↓
provider
    ↓
model
    ↓
generic events/result
    ↓
application interprets result
```

## Core operations

Keep the core model-oriented.

Likely generic operations are:

```text
complete
stream
generateObject
```

Do not add application-shaped APIs such as:

```text
generateSql()
fixDoraQuery()
rewriteSkriuwNote()
extractBetalingenData()
```

Task helpers may exist later as an optional layer, but the core must not know task names.

## Architecture principles

Prefer extraction over rewrite.

Prefer proven contracts over speculative abstractions.

Prefer one narrow completion seam over many convenience APIs.

Prefer stable contracts over implementation cleverness.

Prefer typed errors over string parsing.

Prefer explicit capability modeling over provider-name checks.

Prefer application-owned context over SDK-owned product logic.

Prefer deterministic fake providers over network-dependent tests.

Prefer small migrations over big-bang migrations.

Preserve behavior during extraction unless a behavior change has been explicitly approved.

Do not solve provider differences by weakening types.

## Type safety

Type safety is a first-class SDK requirement.

This SDK defines contracts consumed across applications, languages, providers, frameworks, IPC boundaries, HTTP boundaries, and runtimes.

Invalid states should be difficult or impossible to represent.

### TypeScript

Use strict TypeScript.

Requirements:

* use `type`, never `interface`
* never use `any`
* avoid letting `unknown` escape a trust boundary
* narrow external `unknown` immediately
* avoid broad casts used merely to satisfy the compiler
* avoid `as` assertions unless crossing a genuinely untyped external boundary
* use discriminated unions for states and events
* use exhaustive handling for bounded variants
* do not represent known states with arbitrary strings
* do not expose `Record<string, unknown>` configuration bags in public contracts
* do not expose `extra`, `options`, or `metadata` bags as an escape hatch for provider behavior
* provider-specific functionality must use typed capability extensions
* validate HTTP, IPC, provider, environment, filesystem, and serialized input at trust boundaries
* use Zod or generated schemas where appropriate
* public contracts should remain serializable unless explicitly documented otherwise
* do not leak Vercel AI SDK types through the public API
* do not leak React types through core
* do not leak Hono types through core
* do not leak Tauri types through core
* do not leak Bun or Node-specific types through core
* do not leak provider SDK types through core

Prefer:

```text
CompletionEvent =
    Delta
    | Done
    | Cancelled
    | Timeout
    | ProviderError
```

over:

```text
{
    type: string
    data?: unknown
    error?: string
}
```

### Rust

Prefer strongly typed domain contracts.

Requirements:

* use enums for bounded states
* use newtypes when raw strings would allow invalid identities to propagate
* use typed errors rather than classifying error strings
* use `Result` for recoverable failures
* avoid `unwrap()` and `expect()` in library/runtime code
* `unwrap()` and `expect()` are acceptable in tests or extremely local statically guaranteed invariants
* avoid `serde_json::Value` in public contracts when structure is known
* untyped JSON is acceptable at truly dynamic provider/schema boundaries
* validate identifiers at trust boundaries
* validate sizes and bounds
* validate externally supplied values
* preserve explicit cancellation states
* preserve explicit timeout states
* preserve explicit terminal states
* do not collapse provider failures into strings
* keep provider-specific types out of `ai-core`
* use `#[non_exhaustive]` where public enums are intentionally extensible

### Cross-language contracts

Rust and TypeScript must represent the same semantic states for shared contracts.

Shared contracts should eventually be protected through:

* JSON Schema
* Rust serialization tests
* TypeScript schema validation
* golden fixtures
* schema drift checks
* shared error fixtures
* shared streaming fixtures

A change that compiles independently in both languages but changes the serialized meaning of a shared contract is still a contract change.

Do not weaken contracts merely to make the Rust and TypeScript implementations easier to reconcile.

## TypeScript conventions

When TypeScript implementation begins:

* use `type`, not `interface`
* use function declarations
* avoid classes
* keep filenames in kebab-case
* keep names concise and descriptive
* prefer platform Web APIs where practical
* prioritize performance
* keep framework dependencies outside core
* keep public contracts serializable
* avoid unnecessary barrel files
* use barrel files only when a folder has multiple meaningful exports

Core must not require React.

Core must not require Hono.

Core must not require Next.js.

Core must not require Tauri.

Core must not require Vercel AI SDK.

## Code comments

Prefer self-explanatory code over comments.

Do not add comments that simply describe what the next line or function already expresses.

Avoid comments such as:

```text
// Get provider

// Check cancellation

// Loop through models

// Return result

// Parse response
```

Use naming, types, modules, and function boundaries to make normal behavior understandable.

Comments are appropriate when they explain information that cannot reasonably be expressed by the code itself, particularly:

* monkey patches
* compatibility hacks
* temporary workarounds
* provider API quirks
* undocumented upstream behavior
* unusual platform-specific behavior
* protocol requirements that look unnecessary but are required
* security-critical invariants
* `unsafe` Rust invariants
* behavior intentionally preserved for backwards compatibility

For a monkey patch or workaround, briefly explain:

1. why it exists
2. what external behavior requires it
3. when it can be removed, if known

Keep comments short.

Do not write narrative commentary throughout implementation files.

### Public API documentation

This is an SDK, so concise public API documentation is allowed where consumers genuinely need it.

Document:

* important constraints
* invariants
* failure behavior
* serialization behavior
* lifecycle requirements
* security implications
* non-obvious usage requirements

Do not document obvious getters, fields, arguments, or constructors merely to increase documentation coverage.

Prefer one useful contract-level explanation over repetitive per-line comments.

## Rust foundation

Prefer extracting and generalizing Skriuw's existing provider-neutral AI seam rather than inventing a replacement.

Important reference concepts include:

```text
AiComplete
AiCompletionRequest
AiCompletionEvent
AiCompletionTerminal
AiCancellation
AiProviderError
AiCredentialSource
AiRunRecorder
FakeAiProvider
AiCompletionService
```

Dora is an additional reference for:

* Anthropic
* OpenAI-compatible providers
* OpenRouter
* multi-key behavior
* model discovery
* provider configuration
* Ollama platform handling

Do not move Dora's database schema or SQL domain logic into the SDK.

## TypeScript architecture

The TypeScript implementation may use Vercel AI SDK internally.

Do not expose Vercel AI SDK contracts publicly.

The intended boundary is:

```text
application
    ↓
our SDK contracts
    ↓
our Vercel AI SDK adapter
    ↓
Vercel AI SDK
    ↓
provider
```

Consumers should interact with our concepts such as:

```text
CompletionRequest
CompletionEvent
ProviderError
ModelRef
Provider
Runtime
```

not:

```text
LanguageModel
UIMessage
TextStreamPart
APICallError
```

This prevents consumers from being coupled to Vercel AI SDK major versions.

## Cross-language strategy

Rust and TypeScript share semantic contracts, not implementation.

Likely shared artifacts:

```text
JSON Schema
error categories
recovery actions
capabilities
provider descriptors
model metadata
golden stream fixtures
golden error fixtures
fake-provider fixtures
```

Do not create a Rust-to-TypeScript runtime bridge where none is required.

## Providers

Prefer descriptor-driven support for OpenAI-compatible providers.

Adding an OpenAI-compatible provider should ideally require:

```text
provider descriptor
model metadata
fixtures
```

rather than:

```text
new HTTP implementation
new streaming implementation
new cancellation implementation
new error implementation
```

Use custom provider implementations only when protocols materially differ.

Likely custom adapters include:

```text
Anthropic
Gemini
Ollama
```

Shared provider infrastructure should own common concerns such as:

```text
stream handling
bounds
cancellation
timeouts
error mapping
usage extraction
```

Provider implementations should not repeatedly implement these mechanisms.

## Model capabilities

Models should eventually expose explicit capabilities rather than being only opaque IDs.

Potential capabilities include:

```text
streaming
json
jsonSchema
tools
vision
audio
reasoning
embeddings
```

Capability knowledge may need three states:

```text
yes
no
unknown
```

Do not infer critical capability support from model-name substrings.

## Credentials

Core may define a credential resolution contract.

Core must not own credential persistence.

Potential implementations include:

```text
environment
session memory
OS keyring
encrypted application database
server secrets
```

Application-specific consent and disclosure rules remain outside the SDK.

Secrets must never appear in serializable configuration structures.

## Ollama

Treat Ollama generation and Ollama lifecycle management as different responsibilities.

Generation:

```text
messages
    ↓
Ollama HTTP API
    ↓
completion events
```

Runtime lifecycle:

```text
detect
install
verify
spawn
stop
status
list models
pull models
remove models
progress
shutdown
```

Do not combine these merely because both use Ollama.

## Tauri

Do not create fixed SDK Tauri commands.

Dora and Skriuw own their command surfaces.

A future optional Tauri layer may provide generic primitives such as:

```text
ChannelSink
OperationRegistry
runBlocking
```

It must not know application-specific command names or product context.

## Routing

Routing is not part of the initial core.

Core should expose enough information for routing to be implemented later:

```text
ProviderError
ErrorCategory
ModelInfo
Capability
Usage
```

A future router may use:

```text
required capabilities
latency
locality
provider health
rate limits
cost
task requirements
user policy
```

Never silently fall back from local inference to a remote/cloud provider.

For example:

```text
Ollama unavailable
```

must not automatically become:

```text
send private document to Gemini
```

unless the consuming application explicitly permits remote fallback.

Never switch provider/model after output has already been emitted to the consumer.

## Error behavior

Errors should be semantic and provider-independent.

Likely categories include:

```text
missing_credential
invalid_credential
invalid_request
model_unavailable
unsupported_capability
rate_limited
quota_exceeded
timeout
network
provider_unavailable
cancelled
malformed_response
structured_output_invalid
local_runtime_unavailable
local_model_missing
internal
```

Do not expose provider HTTP status codes as the primary application contract.

Provider-specific details may be retained internally for diagnostics.

## Streaming invariants

Streaming must be deterministic.

Likely invariants:

* events belong to one request id
* deltas have monotonically increasing sequence numbers
* a request emits at most one terminal event
* successful completion emits a terminal event
* cancellation emits an explicit cancellation terminal
* timeout emits an explicit timeout terminal
* provider failure emits a typed provider-error terminal
* consumers may reject foreign request ids
* consumers may reject out-of-order events
* no provider fallback occurs after the first user-visible delta

These invariants should eventually be enforced by tests and shared fixtures.

## Structured output

Structured output should be provider-neutral.

The SDK may eventually support modes resembling:

```text
text
json
jsonSchema
```

Do not assume every provider/model supports native JSON Schema.

Provider/model capabilities determine which execution strategy is available.

A controlled fallback may use prompt-based JSON plus validation where explicitly allowed.

Invalid structured output must produce a typed failure rather than silently becoming arbitrary text.

## Application-specific logic

### Dora owns

* database schema context
* database engine/dialect selection
* SQL prompts
* SQL safety behavior
* SQL result application
* query execution
* SQL UI
* Dora-specific credential persistence
* Dora-specific usage persistence
* recommended SQL models

### Skriuw owns

* note/editor extraction
* ProseMirror behavior
* CodeMirror behavior
* applying AI results
* inline review UI
* task/tag plan application
* workspace prompts
* opt-in policy
* consent copy
* run-history persistence
* settings UI
* recommended writing models

### Betalingen owns

* financial context
* authorization
* source allowlists
* masking
* Dutch financial prompts
* screen context
* application HTTP routes
* UI

## Testing philosophy

Normal tests must not require paid provider access.

The SDK should eventually include:

```text
deterministic fake provider
provider fixture servers
stream fixtures
error fixtures
cancellation tests
timeout tests
terminal ordering tests
structured-output tests
schema drift tests
Rust/TypeScript contract conformance tests
```

Live-provider tests must be explicit and opt-in.

Adding a provider without provider fixtures should eventually fail CI.

## Current phase

The repository starts in:

# Phase 0: Architecture contract

No production SDK implementation should be written during Phase 0.

Do not create production Rust or TypeScript implementation files unless explicitly instructed to leave Phase 0.

Do not scaffold every package/crate from the proposed final architecture.

Allowed work during Phase 0:

* architecture documentation
* ADRs
* contract design
* pseudocode
* package boundary design
* dependency design
* schema planning
* fixture planning
* migration planning
* naming decisions
* testing strategy

Pseudocode is allowed.

Production implementation is not.

## Phase gates

Do not automatically continue from one phase into another.

Each phase must:

1. have one clearly defined purpose
2. have explicit deliverables
3. make the smallest reasonable change
4. preserve dependency direction
5. include appropriate tests once code exists
6. define exit criteria
7. stop after the exit criteria are met

The next phase begins only after explicit approval.

Tentative roadmap:

```text
Phase 0
Architecture and contract design

Phase 1
Rust ai-core extraction from Skriuw

Phase 2
Rust provider layer

Phase 3
Prove Skriuw works against extracted crates

Phase 4
TypeScript core and Vercel AI SDK adapter

Phase 5
Migrate Betalingen

Phase 6
Extract Ollama runtime management

Phase 7
Extract Tauri helpers only if duplication still justifies them

Phase 8
Migrate Dora

Phase 9
Optional provider/model routing

Phase 10+
Only features justified by real consumers
```

This roadmap is not permission to execute every phase.

## Initial repository philosophy

Prefer fewer packages initially.

Do not create ten empty packages because they appear in a future architecture diagram.

Initial repository:

```text
ai-sdk/
├── AGENTS.md
├── CLAUDE.md
├── dev-docs/knowledge/architecture-research-across-my-ai-apps.md
├── README.md
├── docs/
├── specs/
├── crates/
└── packages/
```

Likely first implementation units later:

```text
crates/ai-core
crates/ai-providers

packages/core
packages/ai-sdk
```

Possible future units:

```text
ai-credentials
ai-ollama-runtime
ai-tauri
ai-router
react
tauri
tasks
```

Create those only when their phase begins and their boundary is justified.

## Working procedure

Before any task:

1. read this file completely
2. read `CLAUDE.md` if present
3. read `dev-docs/knowledge/architecture-research-across-my-ai-apps.md`
4. determine the current phase
5. inspect relevant reference implementation code
6. identify the smallest deliverable required
7. stay within that scope

After completing a task:

1. verify the phase exit criteria
2. report files created or modified
3. report architectural decisions
4. report unresolved questions
5. stop

Do not silently continue to another phase.

## Current instruction

Until explicitly told otherwise:

**Remain in Phase 0.**

Research, architecture documentation, ADRs, contract design, and pseudocode are allowed.

Production SDK implementation is not.
