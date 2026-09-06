# ADR 0002: cross-language contracts

Status: accepted at Phase 0 closure, 2026-09-06. Supersedes the original numeric, versioning, and fixture-parity claims.

## Context

Skriuw generates schemas from Rust and hand-writes TypeScript types in `app/src/contracts/ai.ts`. That file is not a set of Zod schemas inferred into types. Betalingen does infer its application contracts from Zod. The SDK can adopt Zod later, but must describe that as new work.

Rust/TypeScript interoperation currently happens through Tauri JSON. Betalingen runs independently. No runtime bridge, embedded JS engine, FFI, sidecar, or WASM execution layer is required.

## Decision

Share semantic contracts and fixtures, with Rust generating structural JSON Schema. Preserve the Skriuw extraction wire contract first. TypeScript implementation starts only in Phase 4.

### Phase 1 artifacts

Generate schemas only for extracted serializable contracts; commit positive and negative request/event/error fixtures and the spec version. Preserve source serialization and null behavior. Fake scripts can remain Rust test inputs initially; do not invent a shared serialization for `Duration` or runtime objects just to populate a fixtures directory. The D2 `RunSummary` is an in-process port argument, not a shared wire contract: it gets no schema and no fixtures in Phase 1, and history numeric encoding stays deferred with the shared history DTO.

Generation and check commands must be defined by the implementation tooling. The actual Skriuw command is `cargo run -p xtask -- generate --check`, not `-- check`. A new `xtask` crate is not mandatory; see ADR 0001.

Credential errors, model catalogs, capabilities, structured results, and local-runtime schemas are introduced only with their consuming phase. No retry/fallback policy table belongs in the extraction spec.

### Wire rules

- Fields use camelCase; existing union tags and string enums use snake_case. Preserve `recoveryAction`, existing error categories, and strict unknown-field rejection from Skriuw in Phase 1.
- Preserve nullable fields: Rust emits `null` for absent sampling parameters and `done.usage`; decoders accept omission where source serde does. Canonical fixture output uses explicit `null` for these fields. Do not silently change to omission-only output.
- JSON objects are compared semantically; key order, whitespace, and escape spelling are not a contract. Integer fields do not make JSON byte-identical.
- JavaScript numbers cannot represent all Rust `u64` or `i64` values. Future shared integer fields must have explicit safe bounds (absolute value at most 9,007,199,254,740,991), or use a separately specified decimal-string encoding. Phase 1 completion token counts already have a 1,000,000,000 bound; do not claim the existing history timestamp/cost types have the same guarantee.
- Monetary computation can overflow JavaScript's exact-integer range in intermediate products even when inputs and final amounts are safe. Preserve Skriuw's round-half-up formula using exact intermediate arithmetic (`u128` in Rust; an equivalent exact method, such as internal BigInt, in TS). BigInt is not emitted directly as JSON.
- UTF-8 byte limits require UTF-8 measurement in TS, not `.length` or JSON Schema `maxLength`. Reject unpaired UTF-16 surrogates at a TS boundary intended to match Rust strings. JSON Schema alone cannot express every aggregate byte budget or stream-state invariant.
- Optional or nullable fields must be described per contract, not with a global rule accepting null everywhere. Serde `rename_all` on enums does not automatically rename fields within variants; inspect fixtures for tags and fields separately.

### Validation and drift

Phase 1 preserves existing explicit `validate()` calls. Derived schemas describe structure; successful `Deserialize` or schema parsing does not imply the request satisfies runtime validation.

Later TypeScript schemas may be hand-written Zod with inferred types. Compare structural acceptance and semantic validators separately. Test positive and negative cases: unknown fields/kinds/enums, omitted/null fields, overflow, UTF-8 boundaries, invalid identifiers, variant combinations, and stream sequences. Keep explicit numeric and string-boundary fixtures and generated property cases. A finite fixture set catches regressions; it does not prove validator equivalence for every possible value.

Provider fixtures in Phase 2 preserve adapter parsing behavior. Cross-language conformance in Phase 4 compares accumulated text, identity, valid sequence, terminal, error category, and usage. Transport/network chunk boundaries may differ between parsers; do not require identical delta segmentation. Fake providers with a fixed script must reproduce the same segmentation and outcome, but elapsed wall-clock timing is not compared byte-for-byte.

### Versioning

`specs/VERSION` identifies a semantic version; generated schema `$id` values include it. Do not add a version field to completion events during extraction.

- Patch: descriptions or fixtures clarifying unchanged behavior.
- Minor: compatible additions only on contracts explicitly designed to accept them, or catalog entries under an already supported descriptor format.
- Breaking: adding fields to a strict request/event, changing accepted bounds or null behavior incompatibly, adding a value to a closed enum, adding a union variant, removing/renaming fields, or changing their meaning. Record these explicitly even while the SDK is prerelease.

Rust `#[non_exhaustive]` changes source matching obligations; it does not teach serde or a TS validator to accept unknown wire values. A TS fallback to `internal` would erase a real state and does not make a closed enum forward-compatible. Reserve no unused enum variants to avoid future versioning.

### Framework boundary

Tauri carries application-selected JSON payloads. Command names, arguments, Specta integration, and IPC errors are application concerns. Optional future derives must not make Tauri a core dependency.

Vercel AI SDK models, errors, stream parts, and Zod transforms are not language-neutral wire types. The Vercel adapter creates its models internally from our typed configuration and credential port. There is no public `fromLanguageModel(unknown)` workaround.

## Consequences

Schema tooling is development-only. Phase 1 freezes a compatibility baseline; later changes are versioned rather than advertised as a re-export. Shared schemas cover shared DTOs, not every Rust in-process port or application history record.
