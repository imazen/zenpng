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

# Native ARM predicate scan comparisons, with complete output retained.
arm-scan-tiers-macos:
    mkdir -p "$HOME/tmp"
    CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=4 OMP_NUM_THREADS=4 TMPDIR="$HOME/tmp" nice -n 19 /usr/bin/time -l cargo bench --locked -p zenpng --bench scalar_vs_simd --features _dev -- --format=llm > "$HOME/tmp/png-arm-scan-tiers.log" 2>&1

# Structural-inventory corpus runs: every PNG in codec-corpus pngsuite, png-conformance and
# apng-conformance (local checkout, read-only), plus exiftool -v3 as an independent chunk dumper.
inventory-oracle:
    INVENTORY_ORACLE_EXIFTOOL="$(command -v exiftool)" ZENPNG_CODEC_CORPUS="${ZENPNG_CODEC_CORPUS:-$HOME/work/codec-corpus}" cargo test --test inventory -- --nocapture corpus_conformance_sets oracle_exiftool_chunk_offsets

# Structural-inventory fuzz target (nightly + cargo-fuzz); seeds from the conformance sets.
inventory-fuzz seconds="660":
    cd fuzz && nice -n 19 cargo +nightly fuzz run inventory --target x86_64-unknown-linux-gnu -- -max_total_time={{seconds}} -dict=png.dict -max_len=65536

# Worst-case cost of the inventory's zlib-end placement; one process per case so peak RSS is per case.
inventory-cost:
    cargo test --release --test inventory_cost --no-run
    for c in excess_stored broken_after_rows wrong_adler_full_image bomb_excess bomb_huge_ihdr bomb_huge_ihdr_wrong_adler wrong_adler_deflate_128m; do \
      INVENTORY_COST_CASE=$c /usr/bin/time -v $(ls -t target/release/deps/inventory_cost-* | grep -v '\.d$' | head -1) --nocapture 2>&1 | grep -E "COST|Maximum resident|Elapsed|User time"; done

# Both-direction check: flip a byte in every consumed leaf of the conformance corpus and list
# the leaves where nothing a caller receives changes (opt-in; slow).
inventory-sweep:
    INVENTORY_MUTATION_SWEEP=1 ZENPNG_CODEC_CORPUS="${ZENPNG_CODEC_CORPUS:-$HOME/work/codec-corpus}" cargo test --release --test inventory_review4 r4_overwrite_consumed_leaves_sweep -- --nocapture
