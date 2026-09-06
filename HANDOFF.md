# Session handoff: Phase 3 is closed

Updated: 2026-09-07.
Workspace: `/home/remcostoeten/dev/remcostoeten-aIo`.

## State

**Phases 0 through 3 are COMPLETE. Phase 4 has not begun and is not authorized.**

`crates/ai-core` holds the completion seam; `crates/ai-providers` holds seven
remote descriptors, the Gemini dialect, the Ollama generation adapter, and the
credential, authority and listing ports. Phase 3 put a real consumer on top of
both: Skriuw now runs on the extracted crates.

Read before Phase 4:

- `docs/extraction-inventory.md` — what moved into `ai-core`, and the two
  approved behaviour changes (D1, D2).
- `docs/extraction-inventory-providers.md` — what moved into `ai-providers`,
  the seven behaviour changes (P1–P7), and the seven preserved source defects
  (H1–H7).
- `docs/integration-skriuw.md` — what consuming the SDK actually cost Skriuw.

## Verification at closure

SDK, in this repository:

- `cargo test --all-features`: 135 passed, 0 failed, 2 ignored (live).
- `cargo clippy --all-targets --all-features -- -D warnings`: clean, and also
  clean for `--no-default-features` and each protocol feature alone.
- Both schema tools pass `--check`: 6 core schemas, 1 provider schema.

Consumer, in `/home/remcostoeten/dev/skriuw` on branch
`ai-sdk-phase-3-extraction`, commit `0b18371e`:

- `./scripts/check.sh`: 12/12 steps green. 418 workspace tests, 74 desktop
  bridge, 1530 renderer, plus the UI-architecture, renderer-store and cloud
  suites and the renderer type gate. Contract drift clean, `cargo fmt` clean,
  workspace clippy clean.
- Skriuw's `master` is untouched; the migration lives only on that branch, and
  its unrelated in-flight editor work was left out of the commit.

`/home/remcostoeten/dev/{dora,betalingen}` are untouched.

## What Phase 3 proved, and what it did not

Proved: the extracted contracts, service, fake, remote adapters and Ollama
generation carry a real application with its product behaviour preserved —
consent, model authority, retention and redaction, prompts, the Ollama
lifecycle, and every command surface unchanged. Two seams changed shape by
design (D2 recording, P3 credentials); both are covered by tests asserting the
resulting provider error and history record are identical to before.

Did not prove: anything about distribution. Skriuw consumes the crates as path
dependencies into a sibling checkout, so the integration is reproducible only
where both repositories sit side by side. A release channel — registry, git
tag, or vendoring — is unresolved and blocks calling this shippable.

Also unresolved from earlier phases: no retries or key rotation, no structured
output, no provider fallback, `sse_payload` is a line helper rather than an SSE
decoder, and H3 (a stream reaching the 4 MiB cap is indistinguishable from one
that finished) still has no fixture.

## The next authorized boundary

**Phase 4 requires an explicit instruction to begin.** Its scope is
`docs/roadmap.md` "Phase 4": a TypeScript core and a Vercel AI SDK adapter for
the first TypeScript consumer, with the shared semantics implemented natively
rather than bridged.

Before it starts, two things deserve a decision rather than a default: how the
Rust crates are distributed, and whether the contract evolution gate in
`docs/roadmap.md` needs to run first for anything Phase 4 requires.
