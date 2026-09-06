# zenpng justfile

# Default recipe
default: check

# Full check: format, clippy, test
check: fmt clippy test

# Format code + regenerate the public-API surface snapshots (docs/public-api/).
# The snapshot runner lives in the standalone apidoc/ package, so it is never
# built or run by plain `cargo test` or any CI job.
fmt:
    cargo fmt
    cargo test --manifest-path apidoc/Cargo.toml

# Regenerate the public-API surface snapshots only
api-doc:
    cargo test --manifest-path apidoc/Cargo.toml

# Verify the committed snapshots are current
api-doc-check:
    ZEN_API_DOC=check cargo test --manifest-path apidoc/Cargo.toml

# Run clippy with all targets and features
clippy:
    cargo clippy --all-targets --all-features -- -D warnings

# Run tests
test:
    cargo test --all-features

# Build release
build:
    cargo build --release --all-features

# Generate documentation
doc:
    cargo doc --no-deps --all-features

# Run all CI checks locally
ci: fmt
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-features
    cargo doc --no-deps --all-features

# Run WASM tests (requires wasm32-wasip1 target and wasmtime)
wasm:
    RUSTFLAGS="-C target-feature=+simd128" CARGO_TARGET_WASM32_WASIP1_RUNNER="wasmtime --dir ." cargo test --lib --target wasm32-wasip1
    CARGO_TARGET_WASM32_WASIP1_RUNNER="wasmtime --dir ." cargo test --lib --target wasm32-wasip1

# Feature permutation checks (includes path-dep features that CI skips)
feature-check:
    cargo test
    cargo test --features zopfli
    cargo test --features unchecked
    cargo test --features quantette
    cargo test --features imagequant
    cargo test --features joint
    cargo test --features zencodec
    cargo test --all-features

# Native ARM unfilter audit; retain complete output for comparison.
arm-unfilters-macos group="":
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "$HOME/tmp"
    CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=4 OMP_NUM_THREADS=4 TMPDIR="$HOME/tmp" nice -n 19 /usr/bin/time -l cargo bench --locked --features _dev --bench unfilter_tiers -- --group="{{group}}" --format=llm 2>&1 | tee "$HOME/tmp/zenpng-unfilters-$(date -u +%Y%m%dT%H%M%SZ).log"
