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
    group_args=()
    if [[ -n '{{group}}' ]]; then group_args+=("--group={{group}}"); fi
    CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=4 OMP_NUM_THREADS=4 TMPDIR="$HOME/tmp" nice -n 19 /usr/bin/time -l cargo bench --features _dev --bench unfilter_tiers -- "${group_args[@]}" --cpu-policy=unrestricted --format=llm 2>&1 | tee "$HOME/tmp/zenpng-unfilters-$(date -u +%Y%m%dT%H%M%SZ).log"

# Native ARM predicate scan comparisons, with complete output retained.
arm-scan-tiers-macos:
    mkdir -p "$HOME/tmp"
    CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=4 OMP_NUM_THREADS=4 TMPDIR="$HOME/tmp" nice -n 19 /usr/bin/time -l cargo bench -p zenpng --bench scalar_vs_simd --features _dev -- --cpu-policy=unrestricted --format=llm > "$HOME/tmp/png-arm-scan-tiers.log" 2>&1

# Exact PNG cLLI, including independent PNG reader and APNG paths.
test-content-light-raw:
    cargo test --no-default-features --test integration content_light_raw::
    cargo test --no-default-features --lib raw_hdr_mastering_payload_is_exact

# Exact animation, native color and strip-config API integration.
test-api-integration:
    cargo test --release --lib --test integration --test animation_color_negotiation
    cargo test --release --no-default-features --test integration --test animation_color_negotiation
    cargo test --doc
    cargo clippy --all-targets --all-features -- -D warnings
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
    just api-doc

# Review gate for threading and APNG integration, including expert knobs.
pr-review-check:
    cargo fmt -p zenpng -- --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-features
    cargo test --no-default-features
    just api-doc-check

# Compile every fuzz target with the coordinated API dependency pins.
check-fuzz-api:
    cargo check --manifest-path fuzz/Cargo.toml --bins

# Exact HDR payload output, retained API routes, strict lint/docs and snapshots.
test-mastering-raw: test-api-integration

# Hidden RGB, ICC authority and retained PNG/APNG API behavior.
test-exact-transcode: test-api-integration

# Reused inventory suites plus the always-callable generic/dynamic contract.
metadata-inventory-check:
    cargo test --no-default-features --features inventory --test inventory --test inventory_review --test inventory_review2 --test inventory_review3 --test inventory_review4 --test inventory_cost

metadata-inventory-oracle:
    INVENTORY_ORACLE_EXIFTOOL=exiftool cargo test --no-default-features --features inventory --test inventory oracle_exiftool_chunk_offsets -- --nocapture

metadata-feature-check:
    cargo test --no-default-features --test inventory_feature
    cargo test --no-default-features --features inventory --test inventory_feature --test fuzz_regression
    cargo clippy --all-targets --no-default-features --features inventory -- -D warnings
    cargo check --manifest-path fuzz/Cargo.toml --bin inventory

metadata-api-check:
    #!/usr/bin/env bash
    set -euo pipefail
    baseline=$(cargo read zenpng==0.1.4 --path-only)
    RUSTC_BOOTSTRAP=1 cargo rustdoc --lib --all-features -- -Zunstable-options --output-format json
    RUSTC_BOOTSTRAP=1 cargo rustdoc --locked --manifest-path "$baseline/Cargo.toml" --target-dir "$PWD/target/published-api/0.1.4" --lib --all-features -- -Zunstable-options --output-format json
    cargo semver-checks check-release -p zenpng --current-rustdoc target/doc/zenpng.json --baseline-rustdoc target/published-api/0.1.4/doc/zenpng.json --release-type patch

metadata-i686-check:
    CARGO_TARGET_I686_UNKNOWN_LINUX_GNU_RUNNER=env cargo test --no-default-features --features inventory --target i686-unknown-linux-gnu --test inventory --test inventory_review --test inventory_review2 --test inventory_review3 --test inventory_review4 --test inventory_feature --test fuzz_regression

# Existing decode/encode behavior with inventory compiled in, plus public snapshots.
metadata-existing-check:
    cargo test --no-default-features --features inventory --lib --test integration --test animation_color_negotiation
    cargo test --doc --features inventory
    just api-doc

metadata-rewrite-check:
    cargo test -p zenpng --no-default-features --features metadata-rewrite --test metadata_rewrite
    cargo clippy -p zenpng --no-default-features --features metadata-rewrite --lib --test metadata_rewrite -- -D warnings
    CARGO_TARGET_I686_UNKNOWN_LINUX_GNU_RUNNER=env cargo test -p zenpng --no-default-features --features metadata-rewrite --test metadata_rewrite --target i686-unknown-linux-gnu

# Coordinated shared-contract migration, including the existing timing oracle.
shared-contract-check:
    cargo clippy -p zenpng --no-default-features --test shared_contracts -- -D warnings
    cargo test -p zenpng --no-default-features --features inventory,metadata-rewrite --test shared_contracts
    cargo test -p zenpng --no-default-features --features metadata-rewrite --test shared_contracts
    cargo clippy -p zenpng --no-default-features --features inventory,metadata-rewrite --test shared_contracts -- -D warnings
    just metadata-rewrite-check
    cargo test -p zenpng --no-default-features --test integration animation_timing

# Use the resolved Cargo.lock for profiling and input-generation binaries.
coz-build:
    CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release --locked --example coz_harness --example gen_bench_inputs

[positional-arguments]
coz-run *args:
    target/release/examples/coz_harness "$@"

[positional-arguments]
coz-generate-inputs *args:
    target/release/examples/gen_bench_inputs "$@"

# Shared policy/inventory, rewrite behavior and standalone inventory compilation.
contract-pin-check:
    cargo fmt -p zenpng
    just shared-contract-check
    cargo check --manifest-path fuzz/Cargo.toml --bin inventory
