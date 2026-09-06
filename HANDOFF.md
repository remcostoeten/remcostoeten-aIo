# Session handoff: Phase 1 is closed

Updated: 2026-09-06.
Workspace: `/home/remcostoeten/dev/remcostoeten-aIo`.

## State

**Phases 0 and 1 are COMPLETE. Phase 2 has not begun and is not authorized.**

`crates/ai-core` exists and holds the whole Phase 1 deliverable: completion
contracts and validators, cancellation/sink/channel ports, the completion trait,
the service, the deterministic fake, and the D2 recorder port. Schema generation
lives behind the `schema-tool` feature; six schemas and fourteen wire fixtures
are committed under `specs/`.

Both approved behavior changes landed as separate commits, deliberately:

- `77b6146` extracts the source lifecycle verbatim, with the defects pinned by
  `defect_*` tests.
- the following commit applies the D1 hardening and renames those tests to
  `hardened_*`, so the diff between them *is* the record of what changed.

`docs/extraction-inventory.md` is the symbol-by-symbol and test-by-test map.

## Verification at closure

- `cargo test --all-features`: 74 passed, 0 failed.
- `cargo clippy --all-targets --all-features -- -D warnings`: clean.
- `cargo run -p ai-core --features schema-tool --bin ai-schema -- generate --check`: 6 schemas match.
- `cargo fmt --all`: applied.
- Runtime dependency graph: `schemars`, `serde`, `thiserror` direct; `serde_json`
  only transitively through `schemars`. No HTTP client, OS credential store,
  framework, async runtime, or Skriuw dependency.

No reference repository was modified. `/home/remcostoeten/dev/{skriuw,dora,betalingen}`
are untouched, which was a hard requirement of the phase, not an accident.

## The next authorized boundary

**Phase 2 requires an explicit instruction to begin.** Its scope is
`docs/roadmap.md` "Phase 2": extract `crates/ai-providers` from the provider
execution Skriuw already uses, with HTTP dependencies, local fixture servers,
and `ADR 0003` as its design constraint. It is not permission to migrate Skriuw
— that is Phase 3 — and not permission to change provider behavior: source EOF
handling, read-timeout classification, cancellation-before-send and response
caps must be characterized first, exactly as the lifecycle was here.

Before starting it, re-read the Skriuw provider crates for drift. Source facts
in this repo were verified against revision
`64827f5e81d097321715e789c9fcd795303c1595`.

## Known limitations, stated rather than hidden

These are deliberate and documented in `docs/contracts.md` §3.3, not open bugs:

- No delivery guarantee to a closed channel. Commitment guarantees one terminal
  and one send attempt, not receipt.
- Cancellation cannot interrupt a provider that blocks or never returns.
- Nothing survives process abort or an aborting panic.
- `timeout_ms` is observed by the provider; there is no service-wide watchdog.
- `shutdown` cancels current runs only; it does not close admission or join
  workers.
