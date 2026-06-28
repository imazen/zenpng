<!-- GENERATED FROM README.md by zenutils gen-readme-crates.sh — DO NOT EDIT. -->

# zenpng

PNG encoder and decoder in pure, safe Rust (`#![forbid(unsafe_code)]`).
SIMD-accelerated unfiltering, a progressive 4-phase compression engine with 31
effort presets (plus a fine-grained 0–200 effort dial), APNG support, palette
auto-quantization, and full metadata roundtrip (ICC / EXIF / XMP / cICP / HDR).
Compression is backed by [zenflate](https://github.com/imazen/zenflate); palette
reduction by [zenquant](https://github.com/imazen/zenquant).

## Quick start

The one-shot path needs nothing but `zenpng`: encode tightly-packed RGBA8 bytes
to a PNG, and decode any PNG back to RGBA8 + dimensions, each in a single call.

```toml
[dependencies]
zenpng = "0.1"
```

```rust
use zenpng::{encode_rgba8_bytes, decode_rgba8};

// 2×2 RGBA, tightly packed (width * height * 4 bytes)
let (width, height) = (2u32, 2u32);
let rgba = vec![
    255, 0, 0, 255,    0, 255, 0, 255,
    0, 0, 255, 255,    255, 255, 255, 255,
];

let png = encode_rgba8_bytes(&rgba, width, height)?;
let (pixels, w, h) = decode_rgba8(&png)?;

assert_eq!((w, h), (width, height));
assert_eq!(pixels, rgba); // PNG is lossless — exact round-trip
```

`decode_rgba8` normalizes grayscale / indexed / RGB / 16-bit sources to 8-bit
RGBA (opaque sources get `A = 255`). For a specific filter/effort, embedded
ICC/EXIF/XMP, 16-bit / grayscale / indexed I/O, resource limits, or cooperative
cancellation, drop down to the typed builder API below.

### Power API — builder & typed buffers

```toml
[dependencies]
zenpng = "0.1"
zenpixels-convert = { version = "0.2", features = ["rgb", "imgref"] } # .to_rgba8(), .as_imgref()
imgref = "1"     # ImgRef for encode input
rgb = "0.8"      # rgb::Rgba<u8> + FromSlice::as_rgba
enough = "0.4"   # Unstoppable / cancellation tokens
zencodec = "0.1" # zencodec::Metadata, for the `metadata` encode arg (see "Metadata")
```

```rust
use zenpng::{decode, encode_rgba8, EncodeConfig, Compression, PngDecodeConfig};
use enough::Unstoppable;
use zenpixels_convert::PixelBufferConvertTypedExt; // brings `.to_rgba8()` onto PixelBuffer

// Decode
// --- Decode to packed RGBA8 ---
let png_bytes: &[u8] = &[/* ... */];
let output = decode(png_bytes, &PngDecodeConfig::default(), &Unstoppable)?;
println!("{}x{}, alpha={}", output.info.width, output.info.height, output.info.has_alpha);

// `output.pixels` is a `zenpixels::PixelBuffer` in the file's NATIVE color type
// (grayscale / indexed / RGB / 16-bit ...). Normalize to RGBA8 with the
// the `PixelBufferConvertTypedExt` trait imported above (from `zenpixels-convert`):
let rgba = output.pixels.to_rgba8();                        // PixelBuffer<rgb::Rgba<u8>>
let rgba_bytes: Vec<u8> = rgba.copy_to_contiguous_bytes();  // width*height*4, packed R,G,B,A

// --- Encode RGBA8 back to PNG ---
// 2nd arg is `metadata: Option<&zencodec::Metadata>`: pass `Some(&meta)` to embed
// ICC / EXIF / XMP / HDR chunks; `None` writes NO such metadata (see "Metadata" below).
// The two trailing args are the cancellation token and the deadline; pass
// `&Unstoppable` for both to opt out. `rgba.as_imgref()` reuses the buffer above;
// to encode your OWN flat RGBA bytes, use `imgref::ImgRef::new(rgb::FromSlice::as_rgba(&bytes), w, h)`.
let encoded = encode_rgba8(rgba.as_imgref(), None, &EncodeConfig::default(), &Unstoppable, &Unstoppable)?;
//                                            ^^^^ None drops all ICC/EXIF/XMP — pass Some(&meta) to keep them.

// Encode with a specific preset
let config = EncodeConfig::default().with_compression(Compression::High);
let smaller = encode_rgba8(rgba.as_imgref(), None, &config, &Unstoppable, &Unstoppable)?;
```

The `encode_*` family covers every native pixel type: `encode_rgba8`,
`encode_rgb8`, `encode_gray8`, plus the 16-bit `encode_rgba16` / `encode_rgb16` /
`encode_gray16`.

**Errors (for a server).** `decode`/`encode_*` return `Result<_, whereat::At<PngError>>`
where `PngError` is re-exported at the crate root (`use zenpng::PngError;`). The
`At<…>` adds a build-time source location for logs. `At<PngError>` implements
`std::error::Error`, so `?` bubbles it straight into a
`fn main() -> Result<(), Box<dyn std::error::Error>>` or any `anyhow`/`eyre` chain
— no manual conversion. To inspect it instead, unwrap with `err.error()` (borrow)
or `err.decompose().0` (owned), then match on the `PngError` enum (`LimitExceeded`
→ 413, `InvalidInput`/`Decode` → 400, etc.; it is `#[non_exhaustive]`, so keep a
wildcard arm):

```rust
match zenpng::decode(png_bytes, &PngDecodeConfig::default(), &enough::Unstoppable) {
    Ok(output) => { /* ... */ }
    Err(e) => {
        // The first trace frame is the whereat capture site (file:line) — log it for triage.
        if let Some(loc) = e.frames().next().and_then(|f| f.location()) {
            eprintln!("decode failed at {}:{}", loc.file(), loc.line());
        }
        match e.error() {
            zenpng::PngError::LimitExceeded(msg) => eprintln!("too large: {msg}"), // 413
            zenpng::PngError::InvalidInput(msg) | zenpng::PngError::Decode(msg) => eprintln!("bad PNG: {msg}"), // 400
            other => eprintln!("decode failed: {other:?}"),
        }
    }
}
```

**Cancellation.** Every `decode`/`encode_*` takes a `&dyn enough::Stop`; `&enough::Unstoppable`
opts out. For a real, thread-safe cancel/deadline token, use
[`almost_enough::Stopper`](https://crates.io/crates/almost-enough) (`cargo add almost-enough`):

```rust
use almost_enough::Stopper;
use std::sync::Arc;

let stop = Arc::new(Stopper::new());
let watcher = Arc::clone(&stop);
// flip it from a request-deadline or client-disconnect watcher:
std::thread::spawn(move || watcher.cancel());

let output = zenpng::decode(png_bytes, &PngDecodeConfig::default(), &*stop)?;
```

The encoder automatically optimizes color type and bit depth: RGBA→RGB when
fully opaque, RGB→Grayscale when R==G==B, 16-bit→8-bit when samples fit, and
truecolor→indexed when ≤256 unique colors. All lossless. (To force byte-for-byte
RGBA8 output, set the downcast policy off via `EncodeConfig` — see its rustdoc.)

## Compression presets

Presets are placed at Pareto-optimal points on the effort curve, approximately
log-spaced in encode time (each step roughly doubles wall time).

| Preset | Effort | What it does |
|---------|--------|-------------|
| `None` | 0 | Uncompressed (stored DEFLATE blocks) |
| `Fastest` | 1 | 1 strategy (Paeth), turbo DEFLATE |
| `Turbo` | 2 | 3 strategies, turbo DEFLATE |
| `Fast` | 7 | 5 strategies, FastHt screen-only |
| `Balanced` | 13 | 9 strategies, screen + lazy refine |
| `Thorough` | 17 | 9 strategies, lazy2 multi-tier + brute-force |
| `High` | 19 | Near-optimal multi-tier + brute-force |
| `Aggressive` | 22 | Near-optimal + extended brute-force |
| `Intense` | 24 | Full brute-force + near-optimal |
| `Crush` | 27 | Full brute-force + beam search + zenzop (requires `zopfli` feature) |
| `Maniac` | 30 | Maximum standard pipeline + zenzop (requires `zopfli` feature) |
| `Brag` | 31 | Full pipeline + 15 FullOptimal iterations — competitive with ECT-9 |
| `Minutes` | 200 | Full pipeline + 184 FullOptimal iterations |

`Crush`, `Maniac`, and `Brag` fall back to `Intense` if the `zopfli` feature isn't enabled.
`Minutes` runs the full Maniac pipeline plus FullOptimal recompression at
maximum iterations — expect minutes per megapixel.

## Fine-grained effort

For precise control, use `Compression::Effort(n)` with any value from 0 to 200:

```rust
let config = EncodeConfig::default()
    .with_compression(Compression::Effort(17));
```

Effort 0–30 uses zenflate's standard compression pipeline. Effort 31+ adds
FullOptimal recompression with iterative forward-DP parsing — the iteration
count is `effort - 16`, so effort 46 runs 30 iterations, and `Minutes` (effort
200) runs 184 iterations. Higher iterations find better DEFLATE representations
at the cost of time.

With the `zopfli` feature enabled, effort 31+ uses zenzop (an enhanced zopfli
fork with ECT-derived optimizations) instead of zenflate's FullOptimal. On a
13-image test corpus, effort 31 (15 iterations) compresses within 0.11% of
ECT at `-9` (60 zopfli iterations + 8 filter strategies). The corpus is small,
so take that number as a rough indicator rather than a guarantee.

## APNG

```rust
use zenpng::{encode_apng, ApngEncodeConfig, ApngFrameInput};
use enough::Unstoppable;

let frames = vec![
    ApngFrameInput::new(&frame0_rgba, 1, 30),
    ApngFrameInput::new(&frame1_rgba, 1, 30),
];

let config = ApngEncodeConfig::default();
// 5th arg is `metadata: Option<&zencodec::Metadata>` — `None` drops all ICC/EXIF/XMP;
// pass `Some(&meta)` to embed it (same as `encode_rgba8`, see "Metadata" below).
let apng = encode_apng(&frames, width, height, &config, None, &Unstoppable, &Unstoppable)?;
```

All frames are canvas-sized RGBA8. The encoder automatically reduces to RGB
when all frames are fully opaque (25% raw data savings). Delta regions between
consecutive frames are computed automatically, and all 6 dispose/blend
combinations are evaluated per frame (greedy 1-step lookahead) at effort > 2.
Transparent pixel RGB channels are zeroed before compression to improve
DEFLATE performance.

Decoding APNG returns fully composited canvas-sized frames via `decode_apng()`.

## Auto-indexed encoding

When any quantizer feature is enabled (`quantize`, `imagequant`, or `quantette`),
`encode_auto()` quantizes to 256 colors and checks a quality gate before committing
to indexed output:

```rust
use zenpng::{encode_auto, QualityGate, EncodeConfig, default_quantizer};
use enough::Unstoppable;

let quantizer = default_quantizer();
let result = encode_auto(
    img.as_ref(),
    &EncodeConfig::default(),
    &*quantizer,
    QualityGate::MaxDeltaE(0.02),
    None,
    &Unstoppable,
    &Unstoppable,
)?;

// result.indexed: whether palette encoding was used
// result.quality_loss: mean OKLab ΔE (0.0 for truecolor or exact palette)
// result.mpe_score: masked perceptual error (when MaxMpe or MinSsim2 gate used)
// result.ssim2_estimate: estimated SSIMULACRA2 score (when MaxMpe or MinSsim2 gate used)
// result.butteraugli_estimate: estimated butteraugli distance (when MaxMpe or MinSsim2 gate used)
```

If the image has ≤256 unique colors, an exact palette is used with zero quality
loss — no quantization, just a lookup table. Otherwise, zenquant quantizes to
256 colors and the quality gate decides whether the result is acceptable. If
the gate fails, the encoder falls back to lossless truecolor.

Three gate types:

| Gate | Scale | Good default | Meaning |
|------|-------|-------------|---------|
| `MaxDeltaE(f64)` | 0.0 – ∞ | 0.02 | Mean OKLab ΔE (lower = stricter) |
| `MaxMpe(f32)` | 0.0 – ∞ | 0.008 | Masked perceptual error (lower = stricter) |
| `MinSsim2(f32)` | 0 – 100 | 85.0 | Estimated SSIMULACRA2 (higher = stricter) |

`encode_apng_auto()` works the same way but checks the gate per frame and
falls back to truecolor if any frame fails.

## Decode options

```rust
use zenpng::{decode, probe, PngDecodeConfig};
use enough::Unstoppable;

// Probe metadata without decoding pixels
let info = probe(png_bytes)?;

// Default: 120 MP limit, 4 GiB memory limit, checksums skipped
let output = decode(png_bytes, &PngDecodeConfig::default(), &Unstoppable)?;

// No limits, no checksums
let output = decode(png_bytes, &PngDecodeConfig::none(), &Unstoppable)?;

// Verify Adler-32 and CRC-32
let output = decode(png_bytes, &PngDecodeConfig::strict(), &Unstoppable)?;

// Custom
let config = PngDecodeConfig::default()
    .with_max_pixels(1_000_000_000)
    .with_skip_decompression_checksum(false);
```

Checksums are skipped by default for speed. When CRC is skipped, computation
is elided entirely. The decoder handles 8-bit and 16-bit, truecolor and indexed,
interlaced and non-interlaced PNGs.

## Metadata

ICC profiles, EXIF, and XMP roundtrip through encode/decode — **but only if you
pass them.** They ride the `metadata: Option<&zencodec::Metadata>` argument (the
2nd positional arg of `encode_rgba8` / `encode_rgb8` / … and the 5th of
`encode_apng`). **`None` writes no ICC / EXIF / XMP at all** — there is no
`EncodeConfig` setter for those three, so an `encode_*(…, None, …)` call silently
drops them. To preserve metadata across a decode → encode roundtrip, build a
`Metadata` from the decode `output.info` and pass `Some(&meta)`:

```rust
use zenpng::{EncodeConfig, PngDecodeConfig};
use zencodec::Metadata;
use enough::Unstoppable;
use zenpixels_convert::PixelBufferConvertTypedExt; // .to_rgba8()

let output = zenpng::decode(png_bytes, &PngDecodeConfig::default(), &Unstoppable)?;

// Re-build a Metadata from the fields the decoder surfaced on `output.info`.
// `with_icc`/`with_exif`/`with_xmp` accept `Vec<u8>`, `&[u8]`, or `Arc<[u8]>`.
let mut meta = Metadata::none();
if let Some(icc)  = output.info.icc_profile { meta = meta.with_icc(icc); }
if let Some(exif) = output.info.exif        { meta = meta.with_exif(exif); }
if let Some(xmp)  = output.info.xmp         { meta = meta.with_xmp(xmp); }
if let Some(cicp) = output.info.cicp        { meta = meta.with_cicp(cicp); }

let rgba = output.pixels.to_rgba8();
let encoded = zenpng::encode_rgba8(
    rgba.as_imgref(),
    Some(&meta),                  // <-- carries ICC/EXIF/XMP through; `None` would drop them
    &EncodeConfig::default(),
    &Unstoppable,
    &Unstoppable,
)?;
```

The PNG color-space chunks gAMA, sRGB, and cHRM are set on `EncodeConfig` instead
(they have no `Metadata` carrier):

```rust
let config = EncodeConfig::default()
    .with_source_gamma(Some(45455))   // 1/2.2
    .with_srgb_intent(Some(0));       // perceptual
```

cICP and the HDR chunks (cLLI / mDCV) can be set on *either* side — via
`EncodeConfig::with_cicp` / `with_content_light_level`, or on the `Metadata` — and
when present on both, the `EncodeConfig` value wins. The decoder warns on
conflicting color metadata (e.g., both sRGB and cICP present) via `PngWarning`
variants.

## Resource estimation

Before allocating, ask the calibrated cost model how much an encode or decode
will need — handy for server admission control and time budgets:

```rust
use zenpng::heuristics::{estimate_encode, estimate_decode};

// width, height, input bytes-per-pixel (3=RGB8, 4=RGBA8, 6/8=16-bit), effort
let enc = estimate_encode(4000, 3000, 4, 19).unwrap();
println!("encode ~{} MiB peak, ~{:.0} ms", enc.peak_memory_bytes >> 20, enc.time_ms);

// width, height, output bytes-per-pixel
let dec = estimate_decode(4000, 3000, 4).unwrap();
println!("decode ~{} MiB peak", dec.peak_memory_bytes >> 20);
```

`EncodeEstimate` carries `peak_memory_bytes_min` / `peak_memory_bytes` (typical) /
`peak_memory_bytes_max` bounds plus `time_ms` and `output_bytes`. The typical
field is a *safe* admission-gating upper bound — it's regression-tested against
measured VmHWM peaks to never under-predict. When driven through the zencodec
traits, decode also honors `zencodec::AllocPreference` (fallible vs infallible
allocation) and encode honors a `zencodec::encode::Fidelity` target
(`PngEncoderConfig::with_fidelity`).

## Source analysis

`zenpng::detect::probe` parses a PNG's chunk structure (no pixel decode) to report
how it was compressed, identify the creating tool, and recommend whether
re-encoding is worthwhile:

```rust
let probe = zenpng::detect::probe(png_bytes)?;      // Result<PngProbe, ProbeError>
if probe.is_improvable() {
    let effort = probe.recommended_effort();        // suggested Compression::Effort(n)
    // ... re-encode at `effort`
}
```

## Feature flags

| Feature | Default | Description |
|---------|---------|-------------|
| `quantize` | yes | Auto-indexed encoding via zenquant (perceptual quality metrics, joint optimization) |
| `imagequant` | no | libimagequant quantizer backend (high-quality dithering) |
| `quantette` | no | quantette quantizer backend (fast k-means, RGB only) |
| `zopfli` | no | Zenzop recompression for Crush/Maniac and effort 31+ (enhanced zopfli fork) |
| `joint` | no | Joint quantization (requires `quantize`) |
| `cms` | no | ICC synthesis for the zencodec color-emit path (bundled profile blob + pure-Rust LZ4; covers the full H.273 CICP grid incl. PQ/HLG) |
| `unchecked` | no | Forward `zenflate/unchecked` (drops some decompression checksum verification for speed) |

The zencodec trait integration (`PngEncoder` / `PngDecoder` / `PngEncoderConfig` /
`PngDecoderConfig`) is always compiled in — no feature flag required.

## Performance

The decoder uses SIMD-accelerated PNG unfiltering via archmage dispatch:

- **Paeth filter**: 1.6x (RGB) to 2.1x (RGBA) speedup over scalar, branchless i16 predictor (SSE4.2)
- **Sub filter**: ~1.2x on RGBA (SSE2); marginal on RGB due to sequential dependency
- **Up/Average**: LLVM auto-vectorizes scalar to equivalent performance

Dispatch is per-row via `incant!` — no per-pixel overhead. The full decode path
uses ~7 heap allocations total and zenflate decompression accounts for only 0.5%
of instructions (the rest is unfiltering and pixel output).

The encoder's 4-phase pipeline (screen → refine → brute-force → recompress)
automatically adjusts to the effort level. See the
[benchmark charts](https://github.com/imazen/zenpng/tree/main/benchmarks) for the
compression-vs-time tradeoff across all 31 standard effort presets.


## MSRV

The minimum supported Rust version is **1.93**.

## AI-Generated Code Notice

Developed with Claude (Anthropic). Not all code manually reviewed. Review critical paths before production use.

## License

Dual-licensed: [AGPL-3.0](https://github.com/imazen/zenpng/blob/main/LICENSE-AGPL3) or [commercial](https://github.com/imazen/zenpng/blob/main/LICENSE-COMMERCIAL).

I've maintained and developed open-source image server software — and the 40+
library ecosystem it depends on — full-time since 2011. Fifteen years of
continual maintenance, backwards compatibility, support, and the (very rare)
security patch. That kind of stability requires sustainable funding, and
dual-licensing is how we make it work without venture capital or rug-pulls.
Support sustainable and secure software; swap patch tuesday for patch leap-year.

[Our open-source products](https://www.imazen.io/open-source)

**Your options:**

- **Startup license** — $1 if your company has under $1M revenue and fewer
  than 5 employees. [Get a key →](https://www.imazen.io/pricing)
- **Commercial subscription** — Governed by the Imazen Site-wide Subscription
  License v1.1 or later. Apache 2.0-like terms, no source-sharing requirement.
  Sliding scale by company size.
  [Pricing & 60-day free trial →](https://www.imazen.io/pricing)
- **AGPL v3** — Free and open. Share your source if you distribute.

See [LICENSE-COMMERCIAL](https://github.com/imazen/zenpng/blob/main/LICENSE-COMMERCIAL) for details.

## Image tech I maintain

| | |
|:--|:--|
| **Codecs** ¹ | [zenjpeg] · **zenpng** · [zenwebp] · [zengif] · [zenavif] · [zenjxl] · [zenbitmaps] · [heic] · [zentiff] · [zenpdf] · [zensvg] · [zenjp2] · [zenraw] · [ultrahdr] |
| Codec internals | [zenjxl-decoder] · [jxl-encoder] · [zenrav1e] · [rav1d-safe] · [zenavif-parse] · [zenavif-serialize] |
| Compression | [zenflate] · [zenzop] · [zenzstd] |
| Processing | [zenresize] · [zenquant] · [zenblend] · [zenfilters] · [zensally] · [zentone] |
| Pixels & color | [zenpixels] · [zenpixels-convert] · [linear-srgb] · [garb] |
| Pipeline & framework | [zenpipe] · [zencodec] · [zencodecs] · [zenlayout] · [zennode] · [zenwasm] · [zentract] |
| Metrics | [zensim] · [fast-ssim2] · [butteraugli] · [zenmetrics] · [resamplescope-rs] |
| Pickers & ML | [zenanalyze] · [zenpredict] · [zenpicker] |
| Products | [Imageflow] image engine ([.NET][imageflow-dotnet] · [Node][imageflow-node] · [Go][imageflow-go]) · [Imageflow Server] · [ImageResizer] (C#) |

<sub>¹ pure-Rust, `#![forbid(unsafe_code)]` codecs, as of 2026</sub>

### General Rust awesomeness

[zenbench] · [archmage] · [magetypes] · [enough] · [whereat] · [cargo-copter]

[Open source](https://www.imazen.io/open-source) · [@imazen](https://github.com/imazen) · [@lilith](https://github.com/lilith) · [lib.rs/~lilith](https://lib.rs/~lilith)

[zenjpeg]: https://github.com/imazen/zenjpeg
[zenwebp]: https://github.com/imazen/zenwebp
[zengif]: https://github.com/imazen/zengif
[zenavif]: https://github.com/imazen/zenavif
[zenjxl]: https://github.com/imazen/zenjxl
[zenbitmaps]: https://github.com/imazen/zenbitmaps
[heic]: https://github.com/imazen/heic
[zentiff]: https://github.com/imazen/zentiff
[zenpdf]: https://github.com/imazen/zenpdf
[zensvg]: https://github.com/imazen/zenextras
[zenjp2]: https://github.com/imazen/zenextras
[zenraw]: https://github.com/imazen/zenraw
[ultrahdr]: https://github.com/imazen/ultrahdr
[zenjxl-decoder]: https://github.com/imazen/zenjxl-decoder
[jxl-encoder]: https://github.com/imazen/jxl-encoder
[zenrav1e]: https://github.com/imazen/zenrav1e
[rav1d-safe]: https://github.com/imazen/rav1d-safe
[zenavif-parse]: https://github.com/imazen/zenavif-parse
[zenavif-serialize]: https://github.com/imazen/zenavif-serialize
[zenflate]: https://github.com/imazen/zenflate
[zenzop]: https://github.com/imazen/zenzop
[zenzstd]: https://github.com/imazen/zenzstd
[zenresize]: https://github.com/imazen/zenresize
[zenquant]: https://github.com/imazen/zenquant
[zenblend]: https://github.com/imazen/zenblend
[zenfilters]: https://github.com/imazen/zenfilters
[zensally]: https://github.com/imazen/zensally
[zentone]: https://github.com/imazen/zentone
[zenpixels]: https://github.com/imazen/zenpixels
[zenpixels-convert]: https://github.com/imazen/zenpixels
[linear-srgb]: https://github.com/imazen/linear-srgb
[garb]: https://github.com/imazen/garb
[zenpipe]: https://github.com/imazen/zenpipe
[zencodec]: https://github.com/imazen/zencodec
[zencodecs]: https://github.com/imazen/zencodecs
[zenlayout]: https://github.com/imazen/zenlayout
[zennode]: https://github.com/imazen/zennode
[zenwasm]: https://github.com/imazen/zenwasm
[zentract]: https://github.com/imazen/zentract
[zensim]: https://github.com/imazen/zensim
[fast-ssim2]: https://github.com/imazen/fast-ssim2
[butteraugli]: https://github.com/imazen/butteraugli
[zenmetrics]: https://github.com/imazen/zenmetrics
[resamplescope-rs]: https://github.com/imazen/resamplescope-rs
[zenanalyze]: https://github.com/imazen/zenanalyze
[zenpredict]: https://github.com/imazen/zenanalyze
[zenpicker]: https://github.com/imazen/zenanalyze
[zenbench]: https://github.com/imazen/zenbench
[archmage]: https://github.com/imazen/archmage
[magetypes]: https://github.com/imazen/archmage
[enough]: https://github.com/imazen/enough
[whereat]: https://github.com/lilith/whereat
[cargo-copter]: https://github.com/imazen/cargo-copter
[Imageflow]: https://github.com/imazen/imageflow
[Imageflow Server]: https://github.com/imazen/imageflow-dotnet-server
[ImageResizer]: https://github.com/imazen/resizer
[imageflow-dotnet]: https://github.com/imazen/imageflow-dotnet
[imageflow-node]: https://github.com/imazen/imageflow-node
[imageflow-go]: https://github.com/imazen/imageflow-go
