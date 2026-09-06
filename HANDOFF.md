# Session handoff: Phase 2 is closed

Updated: 2026-09-06.
Workspace: `/home/remcostoeten/dev/remcostoeten-aIo`.

## State

**Phases 0, 1 and 2 are COMPLETE. Phase 3 has not begun and is not authorized.**

`crates/ai-core` holds the Phase 1 completion seam. `crates/ai-providers` holds
the Phase 2 deliverable: seven remote descriptors sharing one OpenAI-compatible
path plus the Gemini dialect, the Ollama generation adapter, the credential and
model-authority ports, the model listing contract, and its schema tool. All
tests run against local fixture servers in-crate; the two live tests are
`#[ignore]`d.

`docs/extraction-inventory-providers.md` is the symbol map, the behavior-change
record, and the test mapping. Read it before Phase 3.

## Verification at closure

- `cargo test --all-features`: 135 passed, 0 failed, 2 ignored (live).
- `cargo clippy --all-targets --all-features -- -D warnings`: clean, and also
  clean for `--no-default-features` and each protocol feature alone.
- Both schema tools pass `--check`: 6 core schemas, 1 provider schema.
- `cargo fmt --all`: applied.
- Runtime graph for `ai-providers`: `ai-core`, `reqwest`, `schemars`, `serde`,
  `serde_json`, `thiserror`. No OS credential store, framework, async runtime
  facade, or Skriuw dependency.

`/home/remcostoeten/dev/{skriuw,dora,betalingen}` are untouched — verified by
file mtime, not assumed.

## What changed against the source, and what did not

Seven behavior changes, all recorded as P1–P7 in the inventory: redirects
refused rather than followed, user agent as a construction parameter, a generic
credential vocabulary with application-owned copy, a cancellation recheck before
sending, validated endpoint construction, a refused non-loopback Ollama
endpoint, and no default model authority.

Seven source defects preserved deliberately as H1–H7, including the inconsistent
EOF handling between adapters, read timeouts surfacing as transport failures,
and the response cap that can disguise truncation as completion. Fixing any of
them is a separate approved change with its own fixtures.

## The next authorized boundary

**Phase 3 requires an explicit instruction to begin.** Its scope is
`docs/roadmap.md` "Phase 3": integrate the extracted crates into Skriuw with
preserved product behavior, under explicit migration authorization only.

Skriuw keeps: `models.json` and the catalog types, `CatalogModelAuthority`,
consent and vault policy (mapping its six credential errors onto the three
refusals with its own copy), the Ollama lifecycle including
`with_endpoint_override`'s status degradation, prompts, retention, and its
command surface. It supplies its own user agent to preserve its current
requests. Re-check the source revision for drift before starting; facts here
were verified against `64827f5e81d097321715e789c9fcd795303c1595`.

## Known limitations, stated rather than hidden

Beyond the Phase 1 list in `docs/contracts.md` §3.3 — one terminal but no
delivery guarantee, cooperative cancellation only, no watchdog — the provider
layer adds:

- No retries and no key rotation. A resolver called once cannot react to a later
  429; rotation needs an execution owner the SDK does not have.
- No structured output, no provider fallback, and never a switch from a local
  endpoint to a remote one.
- `sse_payload` is a line helper, not a general SSE decoder.
- A stream that reaches the 4 MiB response cap is indistinguishable from one
  that finished (H3), and no fixture covers it.
