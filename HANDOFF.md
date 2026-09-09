# Session handoff: transcription is in the adapters

Updated: 2026-09-09.
Workspace: `/home/remcostoeten/dev/remcostoeten-aIo`.

## State

**Phases 0 through 5 are COMPLETE. Phase 6 has not begun and is not
authorized.** Speech-to-text was added on 2026-09-09 through the contract
evolution gate rather than as a phase; ADR 0006 records it.

There are now two version numbers, and which one answers a question matters.
The spec version is 0.2.0 — the shared contract, and both npm packages. The
Cargo `workspace.package.version` is 0.3.0, tagged `ai-v0.3.0`, because
transcription is Rust-only and serializes nothing. ADR 0006 amends ADR 0005's
one-number rule for exactly that case.

Four units:

- `crates/ai-core` — the Rust completion seam.
- `crates/ai-providers` — seven remote descriptors, the Gemini dialect, Ollama
  generation, credential and model-authority ports, and speech-to-text on the
  two descriptors that offer it (Groq Whisper, Gemini).
- `packages/core`, published as **`@remcostoeten/ai-core`** — the same
  contracts, run lifecycle, event consumer, NDJSON helpers and deterministic
  fake in TypeScript. Zero dependencies.
- `packages/ai-sdk`, published as **`@remcostoeten/ai-sdk`** — the Vercel AI SDK
  adapter and typed provider factories.

Both real consumers now sit on 0.2.0: Skriuw in Rust, Betalingen in TypeScript.

Read before touching anything:

- `docs/decisions/0005-distribution.md` — the channel decision, and the
  addendum recording that both publishing acts were performed at 0.2.0.
- `docs/integration-betalingen.md` — what Phase 5 changed and what it left.
- `docs/typescript-core.md` — the symbol map, the six shape differences between
  the languages, the two vendor quirks, and what is not implemented.
- `docs/decisions/0004-history-and-token-limit.md` — the first contract delta.
- `docs/decisions/0006-transcription.md` — the second, and why the provider
  boundary was not reopened to get it.
- `docs/transcription.md` — what moved, the five behavior changes, the five
  preserved holes, and the test mapping.
- The earlier inventories: `docs/extraction-inventory.md`,
  `docs/extraction-inventory-providers.md`, `docs/integration-skriuw.md`.

## Verification at closure

`./scripts/check.sh` is green end to end: `cargo fmt --all --check` and
`cargo clippy --all-targets --all-features -- -D warnings` clean (and clean
again with `--no-default-features`); `cargo test --workspace --all-features`
162 passed, 0 failed, 2 ignored (live); both schema tools pass `--check`;
`tsc --build --force` clean under strict TypeScript with
`noUncheckedIndexedAccess` and `exactOptionalPropertyTypes`; `bun test packages`
122 passed, 0 failed across 8 files. No step needs a network, a provider key, or
a sibling checkout.

In the consumers, on 2026-09-07:

- **Skriuw** (`ai-sdk-phase-3-extraction`): builds against 0.2.0 again. 418
  workspace tests and 74 desktop tests pass, `cargo fmt --check` and workspace
  `clippy -D warnings` clean, contract drift clean after regenerating
  `contracts/generated/ai-completion-request.schema.json`.
- **Betalingen**: 99 tests pass, `tsc --noEmit` and `oxlint` clean, the esbuild
  Vercel bundle builds, no OpenAPI drift from the migration, and the browser
  island carries no vendor package, key, model id or prompt.

## What the 2026-09-09 session did

**Transcription moved into `ai-providers`.** Skriuw had shipped voice dictation
on the same adapters, and its provider request syntax — Groq's multipart upload,
Gemini's base64 `inlineData` part, the endpoint joins, the transcript parsing —
belonged here. `RemoteAiProvider::transcribe` is inherent alongside
`list_models`. Nothing added is serialized, so `specs/` is untouched and
`packages/ai-sdk` gained nothing. The refused alternative was re-exposing
`base_url()`, `client()` and `credentials()`, which ADR 0003 §4 hid.

**`ai-v0.3.0` was tagged and pushed**, pointing at `4873a60` on `master`. No npm
publish accompanied it, and none is authorized.

**Skriuw's `ai-sdk-phase-3-extraction` was unblocked.** It is rebased onto
`daddy`, consumes `tag = "ai-v0.3.0"`, and is open as
[skriuw#357](https://github.com/remcostoeten/skriuw/pull/357). Its
`skriuw-ai-remote` is a projection layer again and no longer depends on
`reqwest` or `base64`. Its `check.sh` passes 12/12 with the sibling checkout
absent from disk: 422 workspace, 76 desktop bridge, 1520 renderer tests. The
unreviewed modal-vim commit was split onto `wip/vim-codemirror` off `daddy`, its
ADR renumbered 0039 to 0040 to clear the voice-transcription seam's number.

## What the 2026-09-07 session did

**Skriuw was fixed to 0.2.0.** ADR 0004's source break needed
`prior_messages: Vec::new()` at six request literals —
`crates/skriuw-domain/src/ai.rs`, `crates/skriuw-ai/src/lib.rs`,
`crates/skriuw-ai-ollama/src/lib.rs` (three), `app/src-tauri/src/ai_history.rs`,
`app/src-tauri/src/ai.rs` — plus the regenerated request schema, whose only
change is the two additive defaulted fields and the doc text around them.
Nothing else in Skriuw was touched. **The changes are not committed**, and they
sit in a working tree that already held unrelated in-progress work on the vim
editor.

**Distribution was decided.** ADR 0005: Rust ships as a tagged git dependency
(`ai-v0.2.0` on `github.com/remcostoeten/remcostoeten-aIo`), TypeScript as
`@remcostoeten/ai-core` and `@remcostoeten/ai-sdk` on npm. crates.io was
rejected for a stated reason — `ai-providers` is taken by an unrelated crate, so
publishing forces a public rename for no current benefit; `ai-core` is free.
Both npm names are free and the scope is already the owner's. The placeholder
`@ai-sdk-local/*` scope is gone from the packages, the docs and the tests.

**Betalingen was migrated.** See `docs/integration-betalingen.md`.

## What is still open

**Both publishing acts are done, on 2026-09-07.**
`@remcostoeten/ai-core@0.2.0` and `@remcostoeten/ai-sdk@0.2.0` are on npm under
`latest`, and `ai-v0.2.0` is pushed, pointing at `ef73940` on `master`. A
`npm install` of both from a clean directory streams the documented output from
the deterministic fake. See the addendum in `docs/decisions/0005-distribution.md`,
including the second tag (`ai-sdk-v0.2.0`) that one-tag-per-run produced.

**Skriuw is migrated; Betalingen is not.** Skriuw now consumes the tagged
crates, in an open PR. Betalingen still resolves the packages through `file:`
paths plus an `overrides` pin, which Vercel cannot install from, so it is still
not deployable.

**Betalingen is unblocked but not yet fixed.** It still resolves the
packages through `file:` paths plus an `overrides` pin, which Vercel cannot
install from, so it is still not deployable. The fix is now available and small:
replace those with `^0.2.0`. Its import specifiers are already final, so no code
changes. That edit is inside a reference repository and needs its own
instruction. Skriuw likewise still holds path dependencies and can move to
`tag = "ai-v0.2.0"` whenever that is authorized.

**Nothing has touched a live provider, in either language.** Every adapter test
in this repository, in Skriuw and in Betalingen runs against a fixture `fetch`.
Transcription has no live check at all, not even an `#[ignore]`d one: it would
upload a real recording to a paid endpoint. Nobody has confirmed that Groq's
Whisper endpoint or Gemini's `inlineData` transcription behave as the fixtures
claim.
Two Rust tests are `#[ignore]`d as live checks; there is no TypeScript
equivalent. No one has confirmed that Groq, Gemini or Anthropic behave as the
fixtures claim.

**Betalingen's work is still uncommitted.** Its changes sit in a working tree
where the entire AI feature was untracked before the 2026-09-07 session began.
Skriuw's is now committed and pushed.

Also still unresolved from earlier phases: no retries or key rotation, no
structured output, no provider fallback or routing, no run recording in
TypeScript, `sse_payload` is a line helper rather than an SSE decoder, and H3 (a
Rust stream reaching the 4 MiB cap is indistinguishable from one that finished)
still has no fixture. The Rust adapters still refuse `priorMessages` and
`maxOutputTokens` with a typed `rejected_request` rather than carrying them;
wiring them is provider work for its own phase.

Unrelated to this SDK, noticed and not fixed: `app/src-tauri` in Skriuw has two
pre-existing `clippy::cloned_ref_to_slice_refs` warnings in
`src/maintenance.rs`. That crate is not in Skriuw's clippy gate, so its own
`check.sh` does not see them.

## The next authorized boundary

**Phase 6 requires an explicit instruction to begin.** Its scope is
`docs/roadmap.md` "Phase 6": extract the Ollama lifecycle into
`ai-ollama-runtime`, only after its platform and behavior scope is approved, and
with Skriuw's migration onto it needing separate authorization again.

The publish that used to block Betalingen is done. What now blocks its
deployment is a one-line manifest change inside Betalingen itself, which is a
reference repository and needs its own instruction.
