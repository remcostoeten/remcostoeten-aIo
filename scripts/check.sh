#!/usr/bin/env bash
# Everything CI runs, in the order that fails fastest.
#
# No step needs a network, a provider key, or a sibling checkout. A step that
# did would not belong here.
set -euo pipefail

cd "$(dirname "$0")/.."

step() { printf '\n\033[1m== %s\033[0m\n' "$1"; }

step 'rust: format'
cargo fmt --all --check

step 'rust: clippy (all features)'
cargo clippy --all-targets --all-features -- -D warnings

step 'rust: clippy (no default features)'
cargo clippy --all-targets --no-default-features -- -D warnings

step 'rust: tests'
cargo test --workspace --all-features

step 'contracts: schema drift'
cargo run -q -p ai-core --features schema-tool --bin ai-schema -- generate --check
cargo run -q -p ai-providers --features schema-tool --bin ai-provider-schema -- generate --check

step 'typescript: build and typecheck'
bunx tsc --build --force

step 'typescript: tests'
bun test packages

printf '\n\033[1;32mall checks passed\033[0m\n'
