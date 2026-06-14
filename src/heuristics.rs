// Copyright (c) Imazen LLC and the JPEG XL Project Authors.
// Licensed under AGPL-3.0-or-later. Commercial licenses at https://www.imazen.io/pricing

//! PNG encode/decode resource estimation (peak memory + time).
//!
//! Mirrors the zen per-codec estimation pattern (cf. `zenwebp::heuristics`)
//! with separate [`EncodeEstimate`] (min / typical / max peak memory, time,
//! output) and [`DecodeEstimate`] (peak memory, output, time).
//!
//! PNG's dominant cost is the **compression level (effort)** — it governs
//! BOTH time and the encoder's working set, over an enormous range. Decode
//! is a cheap DEFLATE inflate, nearly free.
//!
//! ## Model
//!
//! ```text
//! encode_peak = input + ENCODE_FIXED + encode_bpp(effort, alpha, depth) · pixels
//! encode_time = encode_us_per_px(effort) · pixels   (× alpha factor)
//! decode_peak = DECODE_FIXED + DECODE_BPP · pixels
//! decode_time = DECODE_US_PER_PX · pixels
//! ```
//!
//! Both `encode_bpp` and `encode_us_per_px` rise with effort and are
//! interpolated from measured anchors.
//!
//! ## Calibration (2026-06-14)
//!
//! Measured marginal working set (`png_probe` `VmHWM` delta around the codec
//! call) + wall + user/sys CPU (`/proc/self/stat`, `with_parallel(false)`),
//! one process per op, over 5 PNG content classes × 256–1024 px × effort
//! {1,6,13,19,24} (+ {27,30} anchors) × rgb/rgba × 8/16-bit. Provenance:
//! `benchmarks/png_resource_{main,higheffort,alphadepth}_2026-06-14.tsv`;
//! harness `scripts/png_resource_calibrate.py`.
//!
//! Measured (8-bit RGB, single-thread; time is linear in pixels):
//!
//! | effort        | 1    | 6    | 13   | 19   | 24   | 27   | 30   |
//! |---------------|------|------|------|------|------|------|------|
//! | mem B/px      | 18   | 46   | 60   | 91   | 95   | ~95  | ~120 |
//! | time µs/px    | 0.03 | 0.17 | 0.54 | 2.47 | 6.46 | ~10  | ~125 |
//!
//! Decode: ~5 B/px, ~0.006 µs/px. Alpha (4th channel): +23 B/px, +35 %
//! encode time. 16-bit: +16 B/px (time ≈ unchanged). Efforts 31–200
//! (`Brag`/`Minutes`) are much slower still and unmeasured — the e30 value
//! is a lower bound there.

/// Resource estimate for a PNG encode. `#[non_exhaustive]`.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct EncodeEstimate {
    /// Best-case peak memory in bytes (simple / low-entropy content).
    pub peak_memory_bytes_min: u64,
    /// Typical (≈ p50) peak memory in bytes for natural content.
    pub peak_memory_bytes: u64,
    /// Conservative upper-bound peak memory in bytes (worst content + margin).
    pub peak_memory_bytes_max: u64,
    /// Rough single-thread encode time in ms (effort-dominated). Divide by
    /// thread count for an approximate wall-latency estimate.
    pub time_ms: f32,
    /// Rough estimated output size in bytes (lossless — very content-
    /// dependent; this is a coarse ≈ 0.5× input default).
    pub output_bytes: u64,
}

/// Resource estimate for a PNG decode. `#[non_exhaustive]`.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct DecodeEstimate {
    /// Typical peak memory in bytes.
    pub peak_memory_bytes: u64,
    /// Decoded pixel-buffer size in bytes.
    pub output_bytes: u64,
    /// Rough decode time in ms (a cheap DEFLATE inflate).
    pub time_ms: f32,
}

// ── Calibrated constants (2026-06-14, png_probe marginal working set) ──

const ENCODE_FIXED_OVERHEAD: u64 = 6 << 20;
/// `(effort, B/px)` — encoder working set rises with compression level
/// (more filter strategies + brute-force / Zopfli search buffers).
const ENCODE_BPP_ANCHORS: [(f64, f64); 6] = [
    (1.0, 18.0),
    (6.0, 46.0),
    (13.0, 60.0),
    (19.0, 91.0),
    (24.0, 95.0),
    (30.0, 120.0),
];
/// `(effort, µs/px)` — single-thread encode time, linear in pixels.
const ENCODE_TIME_ANCHORS: [(f64, f64); 7] = [
    (1.0, 0.03),
    (6.0, 0.17),
    (13.0, 0.54),
    (19.0, 2.47),
    (24.0, 6.46),
    (27.0, 10.5),
    (30.0, 125.0),
];
/// Alpha extra-channel: +23 B/px and ~+35 % encode time.
const ENCODE_ALPHA_BPP: f64 = 23.0;
const ENCODE_ALPHA_TIME_FACTOR: f64 = 1.35;
/// 16-bit: +16 B/px (encode time per pixel ≈ unchanged).
const ENCODE_DEPTH16_BPP: f64 = 16.0;
/// Content-spread multipliers on the working set (zenwebp parity).
const MULT_MIN: f64 = 0.8;
const MULT_MAX: f64 = 1.8;
/// Coarse lossless output fraction of raw input bytes (very content-variable).
const ENCODE_OUTPUT_RATIO: f64 = 0.5;

const DECODE_FIXED_OVERHEAD: u64 = 4 << 20;
const DECODE_BPP: f64 = 5.0;
const DECODE_US_PER_PX: f64 = 0.01;

/// Linear interpolation over `(x, y)` anchors, clamped to the endpoints.
fn interp(anchors: &[(f64, f64)], x: f64) -> f64 {
    let x = x.clamp(anchors[0].0, anchors[anchors.len() - 1].0);
    for w in anchors.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if x <= x1 {
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0);
        }
    }
    anchors[anchors.len() - 1].1
}

/// Estimate peak memory / time / output for a PNG encode.
///
/// * `width`, `height` — image dimensions in pixels.
/// * `input_bpp` — input bytes per pixel; also selects the stratum:
///   3 = RGB8, 4 = RGBA8, 6 = RGB16, 8 = RGBA16. Alpha (bpp 4/8) and 16-bit
///   (bpp 6/8) cost extra working set.
/// * `effort` — compression level (`Compression::effort()`, 0–200). The
///   dominant cost knob for both time and memory.
///
/// Returns `None` only on dimension overflow.
#[must_use]
pub fn estimate_encode(
    width: u32,
    height: u32,
    input_bpp: u8,
    effort: u32,
) -> Option<EncodeEstimate> {
    let pixels = (width as u64).checked_mul(height as u64)?;
    let input = pixels.checked_mul(input_bpp as u64)?;
    let has_alpha = input_bpp == 4 || input_bpp == 8;
    let high_depth = input_bpp >= 6;
    let e = effort as f64;

    let mut bpp = interp(&ENCODE_BPP_ANCHORS, e);
    if has_alpha {
        bpp += ENCODE_ALPHA_BPP;
    }
    if high_depth {
        bpp += ENCODE_DEPTH16_BPP;
    }
    let working = (pixels as f64 * bpp) as u64;
    let base = ENCODE_FIXED_OVERHEAD.checked_add(input)?;
    let typical = base.checked_add(working)?;
    let min = base + (working as f64 * MULT_MIN) as u64;
    let max = base + (working as f64 * MULT_MAX) as u64;

    let mut us_px = interp(&ENCODE_TIME_ANCHORS, e);
    if has_alpha {
        us_px *= ENCODE_ALPHA_TIME_FACTOR;
    }
    let time_ms = (pixels as f64 * us_px / 1000.0) as f32;
    let output_bytes = (input as f64 * ENCODE_OUTPUT_RATIO) as u64;

    Some(EncodeEstimate {
        peak_memory_bytes_min: min,
        peak_memory_bytes: typical,
        peak_memory_bytes_max: max,
        time_ms,
        output_bytes,
    })
}

/// Estimate peak memory / time for a PNG decode.
///
/// * `width`, `height` — image dimensions in pixels.
/// * `output_bpp` — bytes per pixel of the decoded buffer.
///
/// Returns `None` only on dimension overflow.
#[must_use]
pub fn estimate_decode(width: u32, height: u32, output_bpp: u8) -> Option<DecodeEstimate> {
    let pixels = (width as u64).checked_mul(height as u64)?;
    let output_bytes = pixels.checked_mul(output_bpp as u64)?;
    let peak = DECODE_FIXED_OVERHEAD + (pixels as f64 * DECODE_BPP) as u64;
    let time_ms = (pixels as f64 * DECODE_US_PER_PX / 1000.0) as f32;
    Some(DecodeEstimate {
        peak_memory_bytes: peak,
        output_bytes,
        time_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compression level dominates: higher effort → strictly more time and
    /// memory, with the measured ~4000× time spread (e1 → e30).
    #[test]
    fn effort_dominates_time_and_mem() {
        let (w, h) = (2048, 2048);
        let t = |e| estimate_encode(w, h, 3, e).unwrap().time_ms;
        let m = |e| estimate_encode(w, h, 3, e).unwrap().peak_memory_bytes;
        assert!(
            t(30) > t(24) && t(24) > t(13) && t(13) > t(6) && t(6) > t(1),
            "time ↑ with effort"
        );
        assert!(m(24) > m(13) && m(13) > m(1), "mem ↑ with effort");
        assert!(t(30) > t(1) * 1000.0, "e30 ≫ e1 time, got {}", t(30) / t(1));
        // effort outside the measured range clamps (no panic).
        assert_eq!(t(0), t(1));
        assert_eq!(t(200), t(30));
    }

    /// Decode is far cheaper than encode (PNG decode is a DEFLATE inflate).
    #[test]
    fn decode_far_cheaper_than_encode() {
        let (w, h) = (2048, 2048);
        let enc = estimate_encode(w, h, 3, 13).unwrap();
        let dec = estimate_decode(w, h, 3).unwrap();
        assert!(dec.time_ms < enc.time_ms / 20.0, "decode ≪ encode time");
        assert!(
            dec.peak_memory_bytes < enc.peak_memory_bytes,
            "decode < encode mem"
        );
    }

    /// Alpha and 16-bit each add encode working set; alpha also adds time.
    #[test]
    fn alpha_and_depth_add_cost() {
        let (w, h) = (2048, 2048);
        let rgb = estimate_encode(w, h, 3, 13).unwrap();
        let rgba = estimate_encode(w, h, 4, 13).unwrap();
        let rgb16 = estimate_encode(w, h, 6, 13).unwrap();
        assert!(rgba.peak_memory_bytes > rgb.peak_memory_bytes && rgba.time_ms > rgb.time_ms);
        assert!(rgb16.peak_memory_bytes > rgb.peak_memory_bytes);
    }

    #[test]
    fn overflow_returns_none() {
        assert!(estimate_encode(u32::MAX, u32::MAX, 8, 13).is_none());
        assert!(estimate_decode(u32::MAX, u32::MAX, 8).is_none());
    }
}
