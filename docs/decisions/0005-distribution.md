# ADR 0005: how the units reach a consumer

Status: accepted 2026-09-07. Resolves the open question carried from Phase 3 and
restated at the close of Phase 4. Spec version stays 0.2.0; this changes no
contract.

## Context

Both languages shipped without a distribution answer.

- `crates/ai-core` and `crates/ai-providers` are consumed by Skriuw as path
  dependencies into a sibling checkout: `ai-core = { path =
  "../remcostoeten-aIo/crates/ai-core" }` in `Cargo.toml` and again in
  `app/src-tauri/Cargo.toml`. That only works because both repositories happen
  to sit next to each other under `~/dev`.
- `packages/core` and `packages/ai-sdk` are workspace-only under the
  placeholder scope `@ai-sdk-local/*`, unpublished, reachable through
  `workspaces: ["packages/*"]` and nothing else.

Phase 3 called the path dependency "a local proof" and said it needed a real
release channel. Phase 4 did not answer it and added a second language to the
same problem.

The forcing consumer is Betalingen. It is a separate repository that builds on
Vercel: `bun run build` runs esbuild against `node_modules`, and `bun run
deploy` runs `vercel --prod`. No sibling checkout exists in that build. A path
dependency cannot survive it.

Two registry facts constrain the answer, checked on 2026-09-07:

- `ai-core` is free on crates.io. `ai-providers` is **taken** — an unrelated
  crate, last published 2025-05-24.
- `@remcostoeten/ai-core` and `@remcostoeten/ai-sdk` are both free on npm, and
  the scope is already in use by `@remcostoeten/auth-drawer`, which Betalingen
  depends on today. The publishing path is proven.

## Decision

Distribution follows the consumer's build environment, so the two languages get
different channels. This is asymmetric on purpose, not by neglect.

### Rust: git tags, not crates.io

Consumers become:

```toml
ai-core = { git = "https://github.com/remcostoeten/remcostoeten-aIo", tag = "ai-v0.2.0" }
ai-providers = { git = "https://github.com/remcostoeten/remcostoeten-aIo", tag = "ai-v0.2.0" }
```

Every Rust consumer — Skriuw today, Dora at Phase 8 — is a local checkout the
owner builds himself. None of them installs from a registry, and Cargo resolves
a tagged git dependency with a `Cargo.lock` entry that pins the commit, so the
build is reproducible without publishing anything.

crates.io is rejected *now* for a concrete reason rather than a vague one:
`ai-providers` is taken, so publishing forces a rename of at least one public
crate, which renames the `ai_providers::` import path in Skriuw's four
manifests and every `use` that follows it. That is a public-surface break paid
for no current benefit. Revisit it when a Rust consumer exists that is not the
owner's own checkout; at that point the rename is worth its cost and this ADR
is superseded rather than amended.

### TypeScript: npm under `@remcostoeten`

The placeholder scope is replaced by the real one:

| was                     | becomes                    |
| ----------------------- | -------------------------- |
| `@ai-sdk-local/core`    | `@remcostoeten/ai-core`    |
| `@ai-sdk-local/ai-sdk`  | `@remcostoeten/ai-sdk`     |

npm rather than git here because the consumer's build is the difference. Vercel
installs from a registry into `node_modules` and esbuild bundles from there; a
git dependency on a TypeScript package would have to carry either a build step
Vercel runs or a committed `dist/`, and neither is better than a publish. The
scope is already the owner's and already consumed by this exact application.

`ai-core` matches the crate name so one concept keeps one name across both
languages. `core` alone was rejected: it says nothing in an unscoped import.

### Versioning

The npm `version` and the Cargo `workspace.package.version` both track
`specs/VERSION`, which is the contract version ADR 0002 governs and ADR 0004
last moved. One number, so "0.2.0" means the same seam in both languages and a
consumer cannot pair a 0.2.0 package with a 0.1.0 crate.

The git tag is `ai-v0.2.0` rather than `v0.2.0`, leaving the bare `v*` namespace
free in a repository that may later hold something other than this SDK.

Note the current mismatch this creates work for: `Cargo.toml` still declares
`workspace.package.version = "0.1.0"` while `specs/VERSION` is `0.2.0`. The
crate version was never moved when ADR 0004 moved the spec. Aligning it is part
of implementing this ADR.

## What this ADR does and does not authorize

Authorized as a consequence of accepting it:

- renaming the two packages to `@remcostoeten/*` and updating every internal
  import, manifest and test that names the old scope;
- aligning `workspace.package.version` to `specs/VERSION`;
- adding whatever `publishConfig`, `repository` and Cargo metadata a publish
  would need.

**Not** authorized here, and each still needs its own explicit instruction:

- running `npm publish` or `bun publish`. Publishing is outward-facing and
  irreversible at a version number; it is a separate deliberate act.
- pushing the `ai-v0.2.0` tag.
- editing Skriuw's manifests from path to git dependencies. Skriuw is a
  reference repository and AGENTS.md forbids touching it without instruction.
  Until that happens Skriuw keeps its path dependencies, which continue to work.

So the state after this ADR is: the names, channels and versioning rule are
settled and the code carries them, and the two publishing acts remain pending
and deliberate.

## Consequence for Phase 5

Betalingen can be migrated against the packages before they are published, by
resolving `@remcostoeten/ai-core` and `@remcostoeten/ai-sdk` to this workspace
locally. Its imports are then already the final specifiers and do not change
when the publish happens. Its `bun test` and `tsc` pass on that arrangement;
`vercel --prod` does not, and cannot until the packages are published.

The roadmap already separates these: Phase 5's exit is "existing application
behaviors/tests, OpenAPI drift, build and relevant bundle checks pass", and
"deployment is a separate authorized action, not an implicit SDK phase
requirement". A deployable Betalingen therefore depends on the publish, and the
publish is a decision the owner makes, not a step Phase 5 performs.
