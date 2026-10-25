# Controlled PNG interpretation

Enable `controlled-decode` to use `DecodeJob::dyn_controlled_decoder` through
`DecodeRequest::start_controlled`, `zencodec_io::PolicyDecoder`, or
`ControlledStepper`. Registration remains `CodecSet::with_decoder`.

The session reads the 33-byte signature/IHDR prefix and suspends with provisional
facts. Resuming with `ContinueReading` permits metadata discovery. It reads the
remaining encoded input, probes complete metadata and suspends again. Only
`AdoptInterpretation` permits native reconstruction and pixel sink effects.
Repeated stepping at either suspension yields the same boundary without reads.
A legacy driver cannot silently bypass these decisions.

The qualified scope is one PNG image in its native layout. Noninterlaced images
use the native row decoder; Adam7 reconstructs a canvas after adoption, then
publishes rows. Preferences permit the native fallback. A required descriptor
is checked by `DecodeRequest` before adoption. APNG and nonzero starting-frame
requests refuse; they need the sequence protocol rather than a still-image
fallback. This does not replace or remove the existing animation decoder.

All seven recognized color chunk families are inspected even when native lenient
parsing would ignore them. Late carriers, bad carrier CRCs, duplicate ICC/cICP/
sRGB, malformed carriers, suppressed required ICC, unsupported cICP code points,
and gamma/chromaticity-only interpretation refuse before sink effects. Native
P3 ICC and BT.2020 PQ samples retain their bytes and reported interpretation.
Late EXIF is visible, including residual orientation, at final adoption. PNG does
not rotate those stored samples.

This is **whole-source controlled decoding**, not byte-resumable inflation.
The supplied storage allowance accounts for retained compressed input and the
coroutine's owned buffers. Native metadata extraction, inflate storage, worker
storage and Adam7 canvas allocations are outside that allowance. Do not advertise
it as a bound on the complete operation's memory. Pixel sinks own their storage.
A network driver may need to learn the complete source length before admission.
The current chunk size is an implementation choice, not a measured optimum.

`tests/integration/controlled_decode.rs` compares 16 named existing fixtures with
an independent `png` decoder: packed 1/2/4-bit grayscale, gray/gray-alpha/RGB/RGBA
8/16-bit, 1/2/4/8-bit palette with transparency, and Adam7. It also checks direct
and registered dispatch, slice and `ReadAt` drivers, CRC/malformed/late carriers,
required versus preferred layouts, source admission before reads, repeated
suspension, cancellation and callback unwind, and native error-chain recovery.
Failure releases managed owners even when the failed adapter remains alive.
Corrupt image data may fail after adoption and partial rows; successful completion
must still be checked. This is not a transactional pixel sink.

Validation (2026-10-25): `just controlled-check` passed the complete integration
binary, strict no-default-feature Clippy and no-default-feature compilation.
Resource-wrapper receipt: rc=0, 2 s, peak RSS 0.33 GiB, minimum available
17340 MiB, peak load 0.32. These describe the test process, not native workspace.
The CI matrix repeats the controlled integration suite on its native platforms.

The separate fuzz graph and API snapshot gate also pass (`just
controlled-integration`, rc=0, 4 s, peak RSS 0.30 GiB, minimum available
17356 MiB, peak load 0.32). No binary fixtures or generated image outputs
were added; the tests author metadata mutations in memory.

## Gain-map discovery scope

Final discovery marks gain maps absent only after scanning an entire PNG
datastream with a recognized chunk vocabulary and no trailing bytes. Android's
`gmAP`/`gdAT` and proposed `gMAP`/`gDAT` carriers refuse this primary-only route;
the nested image is not silently discarded. Unknown chunks also refuse because
this route cannot establish whether an extension carries reconstruction data.
That limitation can be replaced by an explicit scoped discovery/disclosure
choice once such an executing adapter exists. Ordinary native decode behavior
is unchanged.

Source checked 2026-10-25: [Android HDR screenshots](https://source.android.com/docs/core/graphics/hdr-screenshots)
documents the nested PNG and ISO metadata layout; the [PNG gain-map proposal](https://github.com/w3c/png/issues/380)
uses the public chunk spellings. Tests cover each carrier before/after IDAT,
unknown ancillary/critical extensions and trailing data.

Discovery qualification: `just controlled-check` passes with gain-map/extension
controls and malformed or duplicate legacy/HDR carriers (2026-10-25, rc=0,
2 s; run-heavy peak RSS 0.35 GiB, minimum available 17285 MiB, peak load 0.57).

End-to-end integration exposed a GrayAlpha16 descriptor inconsistency: the
ordinary adapter used the element type's unknown transfer rather than the same
sRGB assumption as other untagged integer PNG layouts. Gray16+tRNS and gray-alpha16
now report GRAYA16_SRGB in probe-output, one-shot and row paths. Explicit cICP
still overrides that assumption. The regression test checks native descriptors
and independent sample equality; no pixels are transformed by this correction.

GrayAlpha16 qualification: `just controlled-check` passes (2026-10-25, rc=0,
3 s; run-heavy peak RSS 0.31 GiB, minimum available 17340 MiB, peak load 0.42).
