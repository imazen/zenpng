# zenpng justfile

# Default recipe
default: check

# Full check: format, clippy, test
check: fmt clippy test

# Format code
fmt:
    cargo fmt

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
