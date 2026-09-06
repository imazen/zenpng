# PNG predicate scan run

Production `3a72e318`; benchmark patch described in README.md. Apple M4 Pro, rustc 1.98 / LLVM 22; no target-cpu=native.

Command: `CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=4 OMP_NUM_THREADS=4 TMPDIR=/Users/lilith/tmp nice -n 19 /usr/bin/time -l cargo bench --locked -p zenpng --bench scalar_vs_simd --features _dev -- --format=llm`.

Full log: `/Users/lilith/tmp/arm-all-2026-09-06/png-scan-tiers.log`

SHA-256: `5f3ab70c360213ba6b30a5613bbdd3345639953c24e78a84d89ebc1d030e986d`

Elapsed 119.22 s; maximum resident set size 1059422208 bytes for the cargo bench invocation. No memory scaling claim. No cloud/NAS copy made.
