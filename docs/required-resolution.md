# Required native PNG density

The `zencodec::encode::EncodeJob::try_set_resolution` implementation writes
the native pHYs carrier. The builder form and checked `EncodeRequest` use that
same primitive. No capability query is needed: unsupported representation is
a typed error before encoding, and rejection leaves the previous job intact.

Known physical units convert exactly to pixels/metre. Unknown units retain an
equivalent reduced pixel-aspect ratio. Values must be positive, finite and
representable within PNG's 31-bit integer range. This follows
[PNG pHYs and its integer definition](https://www.w3.org/TR/png-3/#11pHYs).
There is no rounding: 254×127 DPI becomes 10000×5000 pixels/metre; 300×150 DPI
refuses because its exact values require fractions.

The requirement refers to stored pixel axes. The setter neither rotates nor
resizes pixels, modifies video sample aspect, nor reconciles separate EXIF/XMP
copies. A coupled metadata-preservation adapter must resolve those obligations
before claiming consistency across carriers. This is a native-carrier contract,
not a full metadata-normalization operation.

`tests/required_resolution.rs` exercises generic, erased, registered and
transferable jobs; pending row encoding at three effort settings; and buffered
and declared-frame-count APNG. It checks original strided samples, asymmetric
density, unit conversion, aspect reduction, the carrier maximum and rejected
replacement after a valid requirement. The probe and decoded pixels in these
tests use zenpng; independent reader qualification belongs in the separate
zencodec native harness.

The pinned shared implementation is zencodec `008b93c3`. No dependency was
published. Validation from the isolated normal-resolution export:

```text
# just resolution-check: default/no-default tests and strict Clippy
run-heavy: done rc=0 19s | peak-RSS 0.58GiB | min-avail 15786MiB | peak-load 1.04
# just controlled-integration: controlled/metadata tests, strict Clippy,
# no-default check, separately locked fuzz builds and public API snapshot
run-heavy: done rc=0 19s | peak-RSS 0.61GiB | min-avail 15854MiB | peak-load 1.03
```

These are complete test-process receipts, not codec-memory bounds.
