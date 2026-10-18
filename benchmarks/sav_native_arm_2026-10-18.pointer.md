# Native ARM filter-score correctness check

Performance is **not measured or accepted**. The tested SIMD candidate is not
part of this commit's production source.

The frozen candidate `e94a61489784c3772d7c89159b664d6599080e9c` passed
`cargo test --locked --lib sav_score -- --nocapture --test-threads=1` on a
native Ampere Altra Neoverse-N1, separately with default features, `avx512`,
`_dev`, and `_dev,avx512`. Rust was 1.99.0. These checks exercise NEON/scalar
dispatch and the sum boundary above 2^32; they do not execute x86 kernels.

Retained artifacts, checked on **dev**:

`/mnt/v/output/zenpng/native-arm-sav-e94a6148-2026-10-18/`

- `ROOT_SHA256SUMS.json`: SHA-256
  `2251b291c08153a8be9cc5275e2b3952b292609f27c9e4f32508f1ec97a694d0`.
- Source archive: SHA-256
  `2cae00764077ef61c1ec2e010e83e16ffe7cac8f017956098c5a92575a3beb5f`.
- `src/encoder/filter.rs`: SHA-256
  `119fb3d08359a2d34ea2e434f1af72ea560a1b2d094ed34c49c1f1148aee0bd5`.

The archive, locked dependencies, source manifest, gate script, complete logs,
environment/configuration receipts, and four test executables are retained.
The source manifest was verified before and after testing. Cargo used a shared
target directory; the logs show compilation from this frozen source, and the
actual executables were copied and hashed. This was not a clean-target build.

Resource wrapper: `done rc=0 261s | peak-RSS 0.93GiB | min-avail 26185MiB |
peak-load 9.49`, with an 8 GiB cap and four jobs. No R2 or tower mirror has
been verified.
