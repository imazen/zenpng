# Exact PNG mastering-display output

Source `7e4020b63dddaa766e5198a2c1d063674862e861` adds a 24-byte raw mDCV
setter to native and common configurations. It shares existing still/APNG/row
metadata emission, bypasses float conversion, and keeps PNG's cICP requirement.
Raw values take precedence; the typed native setter clears the override.

`just test-mastering-raw` passed the full API integration recipe: default library,
default/minimal integration and native animation color tests, doctests, strict
all-target/all-feature Clippy, strict docs and regenerated API snapshots. The new
regression checks every integer field through the independent `png` reader on
still/APNG and effort 0/1/7 row routes, including u32::MAX and odd chromaticity
codes; it also checks raw precedence, fallback and suppression without cICP.
Existing ignores were unchanged. This is local x86 correctness, not native ARM
or encoder speed/memory qualification.

Whole-job resources: `run-heavy: done rc=0 70s | peak-RSS 0.76GiB |
min-avail 21334MiB | peak-load 6.22`.

Full log key: `output/zenpng/mastering_raw_2026-10-12/gate.log` on canonical
and dev mirror `/mnt/v/output/zenpng/mastering_raw_2026-10-12`.
SHA256: `4bc000bf37418539530750b914d07a51be6e709bf3f7bfab068bf32376f118aa`.
