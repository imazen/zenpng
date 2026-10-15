//! APNG encoding: delta region computation, chunk writing, per-frame compression.

use alloc::vec::Vec;

use archmage::autoversion;
use enough::Stop;

use crate::chunk::PNG_SIGNATURE;
use crate::chunk::write::write_chunk;
use crate::encode::ApngFrameInput;
use crate::encoder::metadata::{
    PngWriteMetadata, metadata_size_estimate, write_all_metadata, write_tail_ancillary,
};
use crate::error::PngError;
#[allow(unused_imports)]
use whereat::at;

use super::CompressOptions;
use super::compress::{RowFormat, compress_filtered, compress_filtered_size, compress_filtered_t};

// APNG dispose/blend operation constants
const DISPOSE_NONE: u8 = 0;
const DISPOSE_BG: u8 = 1;
const DISPOSE_PREV: u8 = 2;
const BLEND_SOURCE: u8 = 0;
const BLEND_OVER: u8 = 1;

// ── Delta region computation ────────────────────────────────────────

/// Bounding box of pixels that differ between two frames.
#[derive(Clone, Copy, PartialEq, Eq)]
struct DeltaRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// Find the bounding box of differing pixels between two canvas-sized frames.
///
/// Returns `None` if the frames are identical.
fn compute_delta_region(
    prev: &[u8],
    curr: &[u8],
    w: u32,
    h: u32,
    bpp: usize,
) -> Option<DeltaRegion> {
    let w = w as usize;
    let h = h as usize;
    let row_bytes = w * bpp;
    let mut min_x = w;
    let mut max_x = 0usize;
    let mut min_y = h;
    let mut max_y = 0usize;

    for y in 0..h {
        let a = &prev[y * row_bytes..(y + 1) * row_bytes];
        let b = &curr[y * row_bytes..(y + 1) * row_bytes];
        if a == b {
            continue;
        }
        min_y = min_y.min(y);
        max_y = y;
        // The pixel holding a row's first/last differing byte is the row's
        // first/last differing pixel.
        let first = first_differing_byte(a, b) / bpp;
        let last = last_differing_byte(a, b) / bpp;
        min_x = min_x.min(first);
        max_x = max_x.max(last);
    }

    if min_x > max_x || min_y > max_y {
        return None; // identical
    }

    Some(DeltaRegion {
        x: min_x as u32,
        y: min_y as u32,
        width: (max_x - min_x + 1) as u32,
        height: (max_y - min_y + 1) as u32,
    })
}

/// Index of the first byte where equal-length `a` and `b` differ.
/// Callers guarantee at least one differs.
fn first_differing_byte(a: &[u8], b: &[u8]) -> usize {
    debug_assert!(a != b);
    let mut i = 0;
    while i + 8 <= a.len() {
        if a[i..i + 8] != b[i..i + 8] {
            break;
        }
        i += 8;
    }
    while a[i] == b[i] {
        i += 1;
    }
    i
}

/// Index of the last byte where equal-length `a` and `b` differ.
/// Callers guarantee at least one differs.
fn last_differing_byte(a: &[u8], b: &[u8]) -> usize {
    debug_assert!(a != b);
    let mut i = a.len();
    while i >= 8 {
        if a[i - 8..i] != b[i - 8..i] {
            break;
        }
        i -= 8;
    }
    while a[i - 1] == b[i - 1] {
        i -= 1;
    }
    i - 1
}

/// Extract a rectangular subregion from a canvas-sized buffer.
fn extract_subframe(pixels: &[u8], canvas_w: u32, region: &DeltaRegion, bpp: usize) -> Vec<u8> {
    let mut out = Vec::new();
    extract_subframe_into(&mut out, pixels, canvas_w, region, bpp);
    out
}

fn extract_subframe_into(
    out: &mut Vec<u8>,
    pixels: &[u8],
    canvas_w: u32,
    region: &DeltaRegion,
    bpp: usize,
) {
    let canvas_w = canvas_w as usize;
    let rw = region.width as usize;
    let rh = region.height as usize;
    let rx = region.x as usize;
    let ry = region.y as usize;

    out.clear();
    for y in ry..ry + rh {
        let row_start = (y * canvas_w + rx) * bpp;
        out.extend_from_slice(&pixels[row_start..row_start + rw * bpp]);
    }
}

// ── 6-way dispose/blend optimization ─────────────────────────────────

/// Result of per-frame dispose/blend optimization.
struct OptimizedFrame {
    dispose_op: u8,
    blend_op: u8,
    region: DeltaRegion,
    /// Pre-filtered subframe data (raw pixels, not yet PNG-compressed).
    subframe: Vec<u8>,
    /// Caller-known answer to "does `subframe` hold alpha-0 pixels whose RGB
    /// still needs normalizing": OVER subframes are built already normalized
    /// and opaque SOURCE subframes hold none at all, so both pass
    /// `Some(false)`. `None` = let the compressor scan and decide.
    has_transparent: Option<bool>,
}

/// Trial-compress a subframe at effort 2, return compressed size.
///
/// Used for comparing dispose/blend candidates cheaply. The compressed data is
/// discarded — only the size matters for the comparison.
fn trial_compress_size(
    subframe: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
    transparent_hint: Option<bool>,
    cancel: &dyn Stop,
) -> crate::error::Result<usize> {
    let opts = CompressOptions {
        preserve_transparent_rgb: false,
        parallel: false,
        cancel,
        deadline: &enough::Unstoppable,
        remaining_ns: None,
        max_threads: 0,
        decode_segments: 0,
    };
    compress_filtered_size(
        subframe,
        row_bytes,
        height,
        RowFormat::truecolor8(bpp),
        2,
        opts,
        transparent_hint,
    )
}

/// OVER blend eligibility, as [`region_pixel_scan_into`] computes it: for a changed
/// pixel, writing the target value via OVER only produces the correct result
/// when target alpha == 255 (OVER with opaque source replaces entirely) or
/// canvas alpha == 0 (OVER onto transparent canvas = direct placement).
/// For RGB (bpp=3): never eligible — OVER requires alpha to be useful.
///
/// Build an OVER subframe for truecolor RGBA8.
///
/// For BLEND_OP_OVER, unchanged pixels MUST have alpha=0 (transparent) so that
/// alpha compositing produces a no-op. Changed pixels use their actual values.
///
/// Only called from [`optimize_apng_truecolor`], which runs exclusively when
/// `preserve_transparent_rgb` is false, so every consumer of the result
/// normalizes alpha-0 RGB to zero before compression — unchanged pixels are
/// emitted as `[0,0,0,0]` directly.
///
/// Writes into `out` (cleared and re-zeroed first).
///
/// Caller must verify OVER eligibility via [`region_pixel_scan_into`] first.
#[autoversion]
fn build_over_subframe_into(
    out: &mut Vec<u8>,
    target: &[u8],
    canvas: &[u8],
    canvas_w: usize,
    region: &DeltaRegion,
    bpp: usize,
) {
    let rw = region.width as usize;
    let rh = region.height as usize;
    let rx = region.x as usize;
    let ry = region.y as usize;

    let row_stride = rw * bpp;
    // Unchanged pixels are `[0,0,0,0]`, which the zeroed buffer already holds;
    // only changed runs get written.
    out.clear();
    out.resize(rh * row_stride, 0);
    let result = out.as_mut_slice();

    for dy in 0..rh {
        let row = ((ry + dy) * canvas_w + rx) * bpp;
        let trow = &target[row..row + row_stride];
        let crow = &canvas[row..row + row_stride];
        let orow = &mut result[dy * row_stride..(dy + 1) * row_stride];

        // Walk changed/unchanged runs so changed pixels copy in bulk and
        // unchanged runs skip with no writes at all.
        let mut dx = 0usize;
        while dx < rw {
            let i = dx * bpp;
            let changed = trow[i..i + bpp] != crow[i..i + bpp];
            let start = i;
            loop {
                dx += 1;
                let j = dx * bpp;
                if dx >= rw || (trow[j..j + bpp] != crow[j..j + bpp]) != changed {
                    break;
                }
            }
            if changed {
                orow[start..dx * bpp].copy_from_slice(&trow[start..dx * bpp]);
            }
        }
    }
}

/// Check if BLEND_OP_OVER can correctly represent all changed pixels for indexed color.
///
/// For indexed OVER, writing target_idx composites palette[target_idx] onto the canvas.
/// This is correct only when palette[target_idx].alpha == 255 (opaque, replaces entirely)
/// OR when the canvas pixel's palette entry has alpha == 0 (transparent canvas).
///
/// Since indexed canvas tracking is by index, we check palette alpha of the target index
/// and whether the canvas index maps to a transparent palette entry.
#[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
fn can_use_over_indexed(
    target_indices: &[u8],
    canvas_indices: &[u8],
    canvas_w: usize,
    region: &DeltaRegion,
    palette_rgba: &[[u8; 4]],
) -> bool {
    let rx = region.x as usize;
    let ry = region.y as usize;
    let rw = region.width as usize;
    let rh = region.height as usize;

    for dy in 0..rh {
        let y = ry + dy;
        for dx in 0..rw {
            let x = rx + dx;
            let off = y * canvas_w + x;
            if target_indices[off] != canvas_indices[off] {
                let target_alpha = palette_rgba[target_indices[off] as usize][3];
                let canvas_alpha = palette_rgba[canvas_indices[off] as usize][3];
                if target_alpha < 255 && canvas_alpha > 0 {
                    return false;
                }
            }
        }
    }
    true
}

/// Build an OVER subframe for indexed color.
///
/// Unchanged pixels use `transparent_idx` so OVER compositing is a no-op.
/// Changed pixels use their actual target index.
///
/// Caller must verify `can_use_over_indexed()` first.
#[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
fn build_over_subframe_indexed(
    target_indices: &[u8],
    canvas_indices: &[u8],
    canvas_w: usize,
    region: &DeltaRegion,
    transparent_idx: u8,
) -> Vec<u8> {
    let rw = region.width as usize;
    let rh = region.height as usize;
    let rx = region.x as usize;
    let ry = region.y as usize;

    let mut result = Vec::with_capacity(rw * rh);

    for dy in 0..rh {
        let y = ry + dy;
        for dx in 0..rw {
            let x = rx + dx;
            let off = y * canvas_w + x;

            if target_indices[off] == canvas_indices[off] {
                result.push(transparent_idx);
            } else {
                result.push(target_indices[off]);
            }
        }
    }

    result
}

/// Apply a dispose operation to the canvas in place.
///
/// - DISPOSE_NONE: no-op (canvas already has the composited result)
/// - DISPOSE_BG: fill the region with transparent black `[0,0,0,0]`
/// - DISPOSE_PREV: copy the saved pre-composite region back
fn apply_dispose_in_place(
    canvas: &mut [u8],
    canvas_w: usize,
    region: &DeltaRegion,
    dispose_op: u8,
    bpp: usize,
    pre_composite: Option<&[u8]>,
) {
    match dispose_op {
        DISPOSE_NONE => {} // no-op
        DISPOSE_BG => {
            let rx = region.x as usize;
            let ry = region.y as usize;
            let rw = region.width as usize;
            let rh = region.height as usize;
            for dy in 0..rh {
                let y = ry + dy;
                let start = (y * canvas_w + rx) * bpp;
                let end = start + rw * bpp;
                canvas[start..end].fill(0);
            }
        }
        DISPOSE_PREV => {
            if let Some(saved) = pre_composite {
                let rx = region.x as usize;
                let ry = region.y as usize;
                let rw = region.width as usize;
                let rh = region.height as usize;
                for dy in 0..rh {
                    let y = ry + dy;
                    let canvas_start = (y * canvas_w + rx) * bpp;
                    let saved_start = dy * rw * bpp;
                    canvas[canvas_start..canvas_start + rw * bpp]
                        .copy_from_slice(&saved[saved_start..saved_start + rw * bpp]);
                }
            }
        }
        _ => {} // unknown dispose, treat as NONE
    }
}

/// Save a rectangular region from the canvas (for DISPOSE_PREV).
#[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
fn save_region(canvas: &[u8], canvas_w: usize, region: &DeltaRegion, bpp: usize) -> Vec<u8> {
    let mut saved = Vec::new();
    save_region_into(&mut saved, canvas, canvas_w, region, bpp);
    saved
}

fn save_region_into(
    out: &mut Vec<u8>,
    canvas: &[u8],
    canvas_w: usize,
    region: &DeltaRegion,
    bpp: usize,
) {
    let rx = region.x as usize;
    let ry = region.y as usize;
    let rw = region.width as usize;
    let rh = region.height as usize;

    out.clear();
    for dy in 0..rh {
        let y = ry + dy;
        let start = (y * canvas_w + rx) * bpp;
        out.extend_from_slice(&canvas[start..start + rw * bpp]);
    }
}

/// Copy a canvas and apply a dispose operation to the copy.
/// Returns the modified copy (lookahead canvas).
#[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
fn apply_dispose_copy(
    canvas: &[u8],
    canvas_w: usize,
    region: &DeltaRegion,
    dispose_op: u8,
    bpp: usize,
    pre_composite: Option<&[u8]>,
) -> Vec<u8> {
    let mut copy = canvas.to_vec();
    apply_dispose_in_place(&mut copy, canvas_w, region, dispose_op, bpp, pre_composite);
    copy
}

/// Blit (overwrite) the target frame's pixels into the canvas at the given region.
fn blit_region(
    canvas: &mut [u8],
    target: &[u8],
    canvas_w: usize,
    region: &DeltaRegion,
    bpp: usize,
) {
    let rx = region.x as usize;
    let ry = region.y as usize;
    let rw = region.width as usize;
    let rh = region.height as usize;
    for dy in 0..rh {
        let y = ry + dy;
        let start = (y * canvas_w + rx) * bpp;
        canvas[start..start + rw * bpp].copy_from_slice(&target[start..start + rw * bpp]);
    }
}

/// Zero RGB channels of fully-transparent (alpha=0) pixels in a canvas region.
///
/// This matches `compress_filtered()`'s transparent pixel zeroing behavior,
/// keeping the optimizer's canvas consistent with the decoder's actual canvas
/// state after decompressing and compositing encoded frame data.
fn zero_transparent_rgb_region(canvas: &mut [u8], canvas_w: usize, region: &DeltaRegion) {
    let rx = region.x as usize;
    let ry = region.y as usize;
    let rw = region.width as usize;
    let rh = region.height as usize;

    for dy in 0..rh {
        let y = ry + dy;
        let row_start = (y * canvas_w + rx) * 4;
        for px in canvas[row_start..row_start + rw * 4]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
        {
            if px[3] == 0 {
                px[0] = 0;
                px[1] = 0;
                px[2] = 0;
            }
        }
    }
}

/// Build a minimal 1×1 subframe at (0,0) for identical frames.
fn minimal_subframe(target: &[u8], bpp: usize) -> (DeltaRegion, Vec<u8>) {
    let region = DeltaRegion {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };
    (region, target[..bpp].to_vec())
}

// ── Truecolor APNG optimization ──────────────────────────────────────

/// One frame's evaluation against a particular canvas: the delta region, the
/// pixel-scan flags used by blend selection, and any trial sizes already
/// computed. Sizes stay `usize::MAX` until a trial compression runs, so an
/// evaluation carried over from a lookahead can skip every trial its owner
/// never needed.
struct FrameEval {
    /// Differing-pixel bounding box; `None` when target equals the canvas.
    region: Option<DeltaRegion>,
    /// Every pixel inside `region` differs between target and canvas. When
    /// true, the OVER subframe is byte-for-byte the SOURCE subframe (no
    /// unchanged pixels to rewrite), so OVER can at best tie SOURCE and the
    /// strict `<` comparison always keeps SOURCE: neither trial matters.
    all_changed: bool,
    /// OVER can reproduce every changed pixel (see `can_use_over_truecolor`).
    /// Always `false` for bpp != 4.
    can_use_over: bool,
    /// Every target pixel inside `region` has alpha 255 — a blit leaves no
    /// transparent pixel to zero. Only set for bpp == 4 scans.
    all_target_opaque: bool,
    /// Trial-compressed size of the SOURCE subframe; `usize::MAX` if untried.
    source_size: usize,
    /// Trial-compressed size of the OVER subframe; `usize::MAX` if untried.
    over_size: usize,
    /// The SOURCE subframe itself when a lookahead trial produced it —
    /// carried so the winning frame skips re-extracting the same bytes.
    source_sub: Option<Vec<u8>>,
    /// The OVER-layout subframe the pixel scan emitted (changed pixels get
    /// target bytes, unchanged get zero) — a valid OVER candidate iff
    /// `!all_changed && can_use_over`, and byte-equal to the SOURCE subframe
    /// when `all_changed`. Always `None` for bpp != 4 or identical frames.
    over_sub: Option<Vec<u8>>,
}

/// Alpha byte's position inside a pixel word (RGBA8, native-endian read).
#[cfg(target_endian = "little")]
const ALPHA_MASK: u32 = 0xFF00_0000;
/// Alpha byte's position inside a pixel word (RGBA8, native-endian read).
#[cfg(target_endian = "big")]
const ALPHA_MASK: u32 = 0x0000_00FF;

// A few image-sized scratch buffers per thread. The APNG optimizer copies the
// canvas for each dispose evaluation and builds an OVER subframe per trial;
// pooling those saves a fresh ~canvas-sized allocation (and its page faults)
// per reuse. Contents are never trusted — every take is overwritten before
// reading.
std::thread_local! {
    static APNG_BUFS: core::cell::RefCell<Vec<Vec<u8>>> =
        const { core::cell::RefCell::new(Vec::new()) };
}

fn apng_take(size: usize) -> Vec<u8> {
    APNG_BUFS.with(|s| {
        s.borrow_mut()
            .pop()
            .filter(|b| b.capacity() >= size)
            .unwrap_or_else(|| Vec::with_capacity(size))
    })
}

fn apng_give(buf: Vec<u8>) {
    if buf.capacity() == 0 {
        return;
    }
    APNG_BUFS.with(|s| {
        let mut s = s.borrow_mut();
        if s.len() < 6 {
            s.push(buf);
        }
    });
}

/// Scan the pixels inside `region` for RGBA8 (`bpp` must be 4) while writing
/// the OVER-layout subframe into `out`: changed pixels get their target
/// bytes, unchanged pixels get `[0,0,0,0]` — exactly what
/// [`build_over_subframe_into`] produces, emitted in the same pass that
/// computes the flags so the region is walked once instead of twice.
///
/// When `all_changed` the output is byte-equal to the SOURCE subframe
/// ([`extract_subframe`] of `target` over `region`): every pixel copied, no
/// zeros. When `!can_use_over` the bytes are not a usable OVER candidate —
/// but still source-equal when `all_changed`.
///
/// Returns `(all_changed, can_use_over, all_target_opaque)`:
/// - `all_changed`: every pixel inside the region differs between `target`
///   and `canvas`. When true, the OVER subframe is byte-for-byte the SOURCE
///   subframe (no unchanged pixels to rewrite), so OVER can at best tie.
/// - `can_use_over`: the OVER eligibility rule — false once a changed pixel
///   has target alpha < 255 over a nonzero canvas alpha.
/// - `all_target_opaque`: every changed-region target pixel has alpha 255,
///   meaning a blit of this region leaves nothing for
///   `zero_transparent_rgb_region` to do.
/// Written so wide-vector tiers see whole-word compares; `bpp != 4` callers
/// skip the scan entirely.
#[autoversion]
fn region_pixel_scan_into(
    out: &mut Vec<u8>,
    target: &[u8],
    canvas: &[u8],
    canvas_w: usize,
    region: &DeltaRegion,
) -> (bool, bool, bool) {
    let rx = region.x as usize;
    let ry = region.y as usize;
    let rw = region.width as usize;
    let rh = region.height as usize;
    let mut all_changed = true;
    let mut can_use_over = true;
    let mut all_target_opaque = true;
    let total = rh * rw * 4;
    if out.len() != total {
        out.clear();
        out.resize(total, 0);
    }
    let result = out.as_mut_slice();
    for dy in 0..rh {
        let row = ((ry + dy) * canvas_w + rx) * 4;
        let t = &target[row..row + rw * 4];
        let c = &canvas[row..row + rw * 4];
        let o = &mut result[dy * rw * 4..(dy + 1) * rw * 4];
        let (t4, _) = t.as_chunks::<4>();
        let (c4, _) = c.as_chunks::<4>();
        let (o4, _) = o.as_chunks_mut::<4>();
        for ((tp, cp), op) in t4.iter().zip(c4).zip(o4) {
            let t32 = u32::from_ne_bytes(*tp);
            let c32 = u32::from_ne_bytes(*cp);
            let ta = t32 & ALPHA_MASK;
            if ta != ALPHA_MASK {
                all_target_opaque = false;
            }
            if t32 == c32 {
                all_changed = false;
            } else if ta != ALPHA_MASK && c32 & ALPHA_MASK != 0 {
                can_use_over = false;
            }
            *op = if t32 == c32 { [0; 4] } else { *tp };
        }
    }
    (all_changed, can_use_over, all_target_opaque)
}

/// Evaluate `target` against `canvas`: delta region plus the pixel-scan flags
/// for RGBA input, and the OVER-layout subframe the scan writes as a
/// byproduct. No trial compressions run here.
fn eval_frame(canvas: &[u8], target: &[u8], canvas_w: u32, canvas_h: u32, bpp: usize) -> FrameEval {
    let region = compute_delta_region(canvas, target, canvas_w, canvas_h, bpp);
    let mut over_sub = None;
    let (all_changed, can_use_over, all_target_opaque) = match (&region, bpp) {
        (Some(r), 4) => {
            let mut buf = apng_take(r.width as usize * r.height as usize * 4);
            let flags = region_pixel_scan_into(&mut buf, target, canvas, canvas_w as usize, r);
            over_sub = Some(buf);
            flags
        }
        _ => (false, false, false),
    };
    FrameEval {
        region,
        all_changed,
        can_use_over,
        all_target_opaque,
        source_size: usize::MAX,
        over_size: usize::MAX,
        source_sub: None,
        over_sub,
    }
}

/// Evaluate the next frame under each post-dispose canvas and return the
/// winning dispose op. The winner's eval is stored in `carried`: it was
/// computed against the canvas that applying this dispose produces, so the
/// next iteration's own evaluation is already done.
///
/// Trial compressions only run where the decision can depend on their exact
/// sizes. `dispose_ops` is evaluated in order and the first candidate wins
/// ties, matching the previous `total < best_total` scan.
#[allow(clippy::too_many_arguments)]
fn eval_lookaheads(
    canvas: &[u8],
    pre_composite: &[u8],
    region: &DeltaRegion,
    next_target: &[u8],
    canvas_w: u32,
    canvas_h: u32,
    bpp: usize,
    dispose_ops: &[u8],
    max_threads: usize,
    carried: &mut Option<FrameEval>,
    cancel: &dyn Stop,
) -> crate::error::Result<u8> {
    let w = canvas_w as usize;
    let npx = w * canvas_h as usize;

    // Phase 1 — one eval per dispose op. DISPOSE_NONE leaves the canvas alone:
    // evaluate against it directly instead of copying. Other ops copy the
    // canvas and apply their dispose; each eval's scan also emits the
    // OVER-layout subframe, so the canvas copies are only needed until the
    // evals finish. Evals are pure functions of the inputs, so `par_map` may
    // run them on the caller's thread budget; order is preserved either way.
    let parts: Vec<(Vec<u8>, FrameEval)> =
        super::compress::par_map(dispose_ops, max_threads, |&d| {
            let mut buf = Vec::new();
            let e = if d == DISPOSE_NONE {
                eval_frame(canvas, next_target, canvas_w, canvas_h, bpp)
            } else {
                buf = apng_take(npx * bpp);
                buf.clear();
                buf.extend_from_slice(canvas);
                apply_dispose_in_place(&mut buf, w, region, d, bpp, Some(pre_composite));
                eval_frame(&buf, next_target, canvas_w, canvas_h, bpp)
            };
            (buf, e)
        });
    let (canvases, mut evals): (Vec<Vec<u8>>, Vec<FrameEval>) = parts.into_iter().unzip();
    debug_assert_eq!(evals.len(), dispose_ops.len());
    // The dispose canvases only fed the phase-1 evals — the OVER subframes
    // the scans produced carry all the state the trials need.
    for c in canvases {
        apng_give(c);
    }

    // Fast path: every dispose leaves the next frame with the same region and
    // every eval is either identical to the canvas (size 20) or decided by
    // its SOURCE size alone — the all-changed case, where OVER can only tie,
    // or no usable OVER at all. All next_sizes are then equal, so the first
    // op wins and no trial compression is needed at all.
    let same_region = evals.iter().all(|e| e.region == evals[0].region);
    if same_region
        && evals
            .iter()
            .all(|e| e.region.is_none() || e.all_changed || !e.can_use_over)
    {
        let win = evals.swap_remove(0);
        for e in &mut evals {
            if let Some(b) = e.source_sub.take() {
                apng_give(b);
            }
            if let Some(b) = e.over_sub.take() {
                apng_give(b);
            }
        }
        *carried = Some(win);
        return Ok(dispose_ops[0]);
    }

    // Phase 2 — trials. SOURCE subframes depend only on (next_target, region),
    // so equal regions share one trial; OVER trials run per eligible eval,
    // on the OVER-layout subframe the eval's scan already wrote. A SOURCE
    // trial can borrow any `all_changed` eval's scan buffer for the same
    // region — with every pixel changed it holds exactly `extract_subframe`'s
    // output — so extraction only runs when no eval can donate.
    // One flat job list so `par_map` spawns its workers once.
    let mut uniq: Vec<(DeltaRegion, Option<bool>, Option<usize>)> = Vec::new();
    for (idx, e) in evals.iter().enumerate() {
        if let Some(r) = e.region {
            match uniq.iter_mut().find(|(k, _, _)| *k == r) {
                Some((_, _, donor)) => {
                    if e.all_changed && donor.is_none() {
                        *donor = Some(idx);
                    }
                }
                None => {
                    // `all_target_opaque` depends only on (next_target,
                    // region), so every eval sharing the region has the
                    // same flag.
                    let hint = if e.all_target_opaque {
                        Some(false)
                    } else {
                        None
                    };
                    let donor = e.all_changed.then_some(idx);
                    uniq.push((r, hint, donor));
                }
            }
        }
    }
    let mut jobs: Vec<Job> = Vec::new();
    for (u, _) in uniq.iter().enumerate() {
        jobs.push(Job::Source(u));
    }
    for (idx, e) in evals.iter().enumerate() {
        if e.region.is_some() && !e.all_changed && e.can_use_over {
            jobs.push(Job::Over(idx));
        }
    }
    let mut job_sizes: Vec<crate::error::Result<(usize, Option<Vec<u8>>)>> =
        super::compress::par_map(&jobs, max_threads, |job| match *job {
            Job::Source(u) => {
                let (r, hint, donor) = uniq[u];
                if let Some(d) = donor {
                    // The donor's scan buffer is byte-identical to the
                    // SOURCE extract (every pixel differed). It stays on the
                    // eval — the merge moves it off if the donor wins.
                    let sub = evals[d].over_sub.as_deref().expect("rgba4 scans write");
                    trial_compress_size(
                        sub,
                        r.width as usize * bpp,
                        r.height as usize,
                        bpp,
                        hint,
                        cancel,
                    )
                    .map(|s| (s, None))
                } else {
                    let mut sub = apng_take(r.width as usize * r.height as usize * bpp);
                    extract_subframe_into(&mut sub, next_target, canvas_w, &r, bpp);
                    trial_compress_size(
                        &sub,
                        r.width as usize * bpp,
                        r.height as usize,
                        bpp,
                        hint,
                        cancel,
                    )
                    .map(|s| (s, Some(sub)))
                }
            }
            Job::Over(idx) => {
                // OVER subframes are built already normalized, so the
                // compressor has no transparency work to do.
                let r = evals[idx].region.expect("over jobs have a region");
                let sub = evals[idx].over_sub.as_deref().expect("rgba4 scans write");
                trial_compress_size(
                    sub,
                    r.width as usize * bpp,
                    r.height as usize,
                    bpp,
                    Some(false),
                    cancel,
                )
                .map(|s| (s, None))
            }
        });

    // Merge in eval order: strict first-wins argmin over next_size. Subframes
    // land on their eval so a carried winner skips re-extract/rebuild.
    let mut resolved_src: Vec<Option<usize>> = (0..uniq.len()).map(|_| None).collect();
    let mut uniq_subs: Vec<Option<Vec<u8>>> = (0..uniq.len()).map(|_| None).collect();
    let mut best = 0usize;
    let mut best_size = usize::MAX;
    let job_pos = |j: &Job| jobs.iter().position(|k| k == j).expect("job exists");
    for (idx, e) in evals.iter_mut().enumerate() {
        let size = match e.region {
            None => 20, // identical next frame: minimal subframe estimate
            Some(r) => {
                let u = uniq
                    .iter()
                    .position(|&(k, ..)| k == r)
                    .expect("region registered");
                let source_size = match resolved_src[u] {
                    Some(s) => s,
                    None => {
                        let si = job_pos(&Job::Source(u));
                        let (s, sub) =
                            core::mem::replace(&mut job_sizes[si], Ok((usize::MAX, None)))?;
                        resolved_src[u] = Some(s);
                        uniq_subs[u] = sub;
                        s
                    }
                };
                e.source_size = source_size;
                if e.all_changed || !e.can_use_over {
                    source_size
                } else {
                    let oi = job_pos(&Job::Over(idx));
                    let (over_size, _) =
                        core::mem::replace(&mut job_sizes[oi], Ok((usize::MAX, None)))?;
                    e.over_size = over_size;
                    source_size.min(over_size)
                }
            }
        };
        if size < best_size {
            best_size = size;
            best = idx;
        }
    }
    if let Some(r) = evals[best].region {
        let u = uniq
            .iter()
            .position(|&(k, ..)| k == r)
            .expect("region registered");
        // The winner's SOURCE subframe: the extracted buffer when a job ran
        // one, else the winner's own scan buffer when it is source-equal
        // (`all_changed`), else the donor eval's — same bytes either way.
        evals[best].source_sub = uniq_subs[u]
            .take()
            .or_else(|| {
                evals[best]
                    .all_changed
                    .then(|| evals[best].over_sub.take())
                    .flatten()
            })
            .or_else(|| {
                uniq[u]
                    .2
                    .and_then(|d| (d != best).then(|| evals[d].over_sub.take()).flatten())
            });
    }
    let win = evals.swap_remove(best);
    // Recycle every buffer the decision is done with.
    for s in uniq_subs.into_iter().flatten() {
        apng_give(s);
    }
    for r in job_sizes {
        if let Ok((_, Some(sub))) = r {
            apng_give(sub);
        }
    }
    for e in &mut evals {
        if let Some(b) = e.source_sub.take() {
            apng_give(b);
        }
        if let Some(b) = e.over_sub.take() {
            apng_give(b);
        }
    }
    *carried = Some(win);
    Ok(dispose_ops[best])
}

/// One parallelizable lookahead trial job.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Job {
    /// Trial-compress the SOURCE subframe for `uniq[index]`.
    Source(usize),
    /// Build + trial-compress the OVER subframe for eval `index`.
    Over(usize),
}

/// Optimize APNG truecolor frames by evaluating all 6 dispose/blend combinations.
///
/// Uses greedy 1-step lookahead: for each frame, picks the (dispose, blend) combo
/// that minimizes current_size + next_frame_best_size.
///
/// `frame_data` is the pixel data for each frame (may be RGBA or RGB, depending on bpp).
/// `bpp` is 4 for RGBA or 3 for RGB.
fn optimize_apng_truecolor(
    frames: &[ApngFrameInput<'_>],
    frame_data: &[&[u8]],
    canvas_w: u32,
    canvas_h: u32,
    bpp: usize,
    max_threads: usize,
    cancel: &dyn Stop,
) -> crate::error::Result<Vec<OptimizedFrame>> {
    let w = canvas_w as usize;
    let h = canvas_h as usize;
    let npx = w * h;

    let mut optimized = Vec::with_capacity(frames.len());
    let mut canvas = vec![0u8; npx * bpp]; // transparent initial canvas
    // Lookahead evals that survive to become the next frame's own evaluation.
    let mut carried: Option<FrameEval> = None;

    for i in 0..frames.len() {
        cancel.check().map_err(|e| at!(PngError::from(e)))?;

        let target = frame_data[i];

        // Frame 0 always uses full canvas as its region (IDAT constraint).
        // For frames 1+, compute delta region normally.
        if i == 0 {
            let full_region = DeltaRegion {
                x: 0,
                y: 0,
                width: canvas_w,
                height: canvas_h,
            };

            // Save pre-composite (entire canvas = all zeros for frame 0)
            let pre_composite = canvas.clone();

            // Composite frame 0 onto canvas
            canvas.copy_from_slice(&target[..npx * bpp]);
            // Zero RGB of alpha=0 pixels to match compress_filtered behavior (RGBA only)
            if bpp == 4 {
                zero_transparent_rgb_region(&mut canvas, w, &full_region);
            }

            // Choose dispose via lookahead (if not last frame)
            let best_dispose = if frames.len() == 1 {
                DISPOSE_NONE
            } else {
                // DISPOSE_BG clears to fully transparent black per APNG spec.
                // DISPOSE_PREV on frame 0 restores the initial transparent canvas.
                // For RGB (bpp=3) there's no alpha channel, so either operation
                // creates transparent regions the decoder can't represent correctly.
                // Frame 0 can only safely use DISPOSE_NONE for non-RGBA color types.
                let dispose_ops: &[u8] = if bpp == 4 {
                    &[DISPOSE_NONE, DISPOSE_BG, DISPOSE_PREV]
                } else {
                    &[DISPOSE_NONE]
                };
                eval_lookaheads(
                    &canvas,
                    &pre_composite,
                    &full_region,
                    frame_data[1],
                    canvas_w,
                    canvas_h,
                    bpp,
                    dispose_ops,
                    max_threads,
                    &mut carried,
                    cancel,
                )?
            };

            // Apply chosen dispose
            apply_dispose_in_place(
                &mut canvas,
                w,
                &full_region,
                best_dispose,
                bpp,
                Some(&pre_composite),
            );
            apng_give(pre_composite);

            optimized.push(OptimizedFrame {
                dispose_op: best_dispose,
                blend_op: BLEND_SOURCE,
                region: full_region,
                subframe: Vec::new(), // frame 0 uses frame_data directly
                has_transparent: None,
            });
            continue;
        }

        // Frames 1+: the winning lookahead already evaluated this frame
        // against the canvas that dispose choice produced.
        let eval = match carried.take() {
            Some(e) => e,
            None => eval_frame(&canvas, target, canvas_w, canvas_h, bpp),
        };

        let Some(source_region) = eval.region else {
            if let Some(b) = eval.source_sub {
                apng_give(b);
            }
            if let Some(b) = eval.over_sub {
                apng_give(b);
            }
            // Identical frame: minimal 1×1, no optimization needed
            let (region, sub) = minimal_subframe(target, bpp);
            optimized.push(OptimizedFrame {
                dispose_op: DISPOSE_NONE,
                blend_op: BLEND_SOURCE,
                region,
                subframe: sub,
                has_transparent: None,
            });
            // Canvas unchanged
            continue;
        };

        // Build SOURCE subframe (needed as output data either way) — reuse
        // the lookahead's trial subframe when it carried one: same region and
        // same target produce identical bytes. An `all_changed` scan buffer
        // holds exactly those bytes too (no unchanged pixels were zeroed).
        let source_row_bytes = source_region.width as usize * bpp;
        let source_height = source_region.height as usize;
        let mut eval = eval;
        let source_sub = match eval.source_sub.take() {
            Some(s) => s,
            None => match eval.over_sub.take() {
                Some(s) if eval.all_changed => s,
                s => {
                    eval.over_sub = s;
                    let mut b = apng_take(source_row_bytes * source_height);
                    extract_subframe_into(&mut b, target, canvas_w, &source_region, bpp);
                    b
                }
            },
        };

        // Blend choice. `all_changed` makes the OVER subframe identical to
        // the SOURCE one, so OVER can only tie and always loses; a changed
        // pixel OVER can't write rules it out directly. Either way SOURCE
        // wins without a trial — and the old source trial only fed a
        // loop-invariant sum, so skipping it changes no decision.
        let (best_blend, best_sub) = if eval.all_changed || !eval.can_use_over {
            if let Some(b) = eval.over_sub.take() {
                apng_give(b);
            }
            (BLEND_SOURCE, source_sub)
        } else {
            let source_size = if eval.source_size == usize::MAX {
                trial_compress_size(
                    &source_sub,
                    source_row_bytes,
                    source_height,
                    bpp,
                    if eval.all_target_opaque {
                        Some(false)
                    } else {
                        None
                    },
                    cancel,
                )?
            } else {
                eval.source_size
            };
            let over_sub = match eval.over_sub {
                Some(s) => s,
                None => {
                    let mut b = apng_take(source_row_bytes * source_height);
                    build_over_subframe_into(&mut b, target, &canvas, w, &source_region, bpp);
                    b
                }
            };
            let over_size = if eval.over_size == usize::MAX {
                trial_compress_size(
                    &over_sub,
                    source_row_bytes,
                    source_height,
                    bpp,
                    Some(false),
                    cancel,
                )?
            } else {
                eval.over_size
            };
            if over_size < source_size {
                apng_give(source_sub);
                (BLEND_OVER, over_sub)
            } else {
                apng_give(over_sub);
                (BLEND_SOURCE, source_sub)
            }
        };

        // Save pre-composite region (for DISPOSE_PREV)
        let mut pre_composite = apng_take(source_row_bytes * source_height);
        save_region_into(&mut pre_composite, &canvas, w, &source_region, bpp);

        // Composite frame onto canvas (blit target pixels into canvas)
        blit_region(&mut canvas, target, w, &source_region, bpp);
        // Zero RGB of alpha=0 pixels to match compress_filtered behavior (RGBA only)
        if bpp == 4 && !eval.all_target_opaque {
            zero_transparent_rgb_region(&mut canvas, w, &source_region);
        }

        // Choose dispose via lookahead (except last frame)
        let best_dispose = if i == frames.len() - 1 {
            DISPOSE_NONE
        } else {
            let dispose_ops: &[u8] = if bpp == 4 {
                &[DISPOSE_NONE, DISPOSE_BG, DISPOSE_PREV]
            } else {
                &[DISPOSE_NONE, DISPOSE_PREV]
            };
            eval_lookaheads(
                &canvas,
                &pre_composite,
                &source_region,
                frame_data[i + 1],
                canvas_w,
                canvas_h,
                bpp,
                dispose_ops,
                max_threads,
                &mut carried,
                cancel,
            )?
        };

        // Apply chosen dispose to canvas for next iteration
        apply_dispose_in_place(
            &mut canvas,
            w,
            &source_region,
            best_dispose,
            bpp,
            Some(&pre_composite),
        );
        apng_give(pre_composite);

        optimized.push(OptimizedFrame {
            dispose_op: best_dispose,
            blend_op: best_blend,
            region: source_region,
            subframe: best_sub,
            // OVER subs are built already normalized; an opaque SOURCE sub
            // holds no alpha-0 pixels. Either way the compressor's
            // transparent-RGB pass has nothing to do.
            has_transparent: if best_blend == BLEND_OVER || eval.all_target_opaque {
                Some(false)
            } else {
                None
            },
        });
    }

    Ok(optimized)
}

// ── Indexed APNG optimization ────────────────────────────────────────

/// Optimize APNG indexed frames by evaluating dispose/blend combinations.
///
/// If a transparent palette entry exists, evaluates all 6 combos (SOURCE + OVER).
/// Otherwise, evaluates 3 dispose options with SOURCE only.
#[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
fn optimize_apng_indexed(
    frame_indices: &[Vec<u8>],
    palette_rgba: &[[u8; 4]],
    canvas_w: u32,
    canvas_h: u32,
    cancel: &dyn Stop,
) -> crate::error::Result<Vec<OptimizedFrame>> {
    let w = canvas_w as usize;
    let h = canvas_h as usize;
    let bpp = 1usize;
    let npx = w * h;

    // Find transparent palette entry (alpha == 0)
    let transparent_idx = palette_rgba.iter().position(|e| e[3] == 0).map(|i| i as u8);

    let mut optimized = Vec::with_capacity(frame_indices.len());
    let mut canvas = vec![0u8; npx]; // initial canvas (index 0)

    for i in 0..frame_indices.len() {
        cancel.check().map_err(|e| at!(PngError::from(e)))?;

        let target = &frame_indices[i];

        // Frame 0: full canvas region, SOURCE only, just choose dispose
        if i == 0 {
            let full_region = DeltaRegion {
                x: 0,
                y: 0,
                width: canvas_w,
                height: canvas_h,
            };

            let row_bytes = w;
            let best_blend_size = trial_compress_size(target, row_bytes, h, bpp, None, cancel)?;

            let pre_composite = canvas.clone();
            canvas.copy_from_slice(&target[..npx]);

            let best_dispose = if frame_indices.len() == 1 {
                DISPOSE_NONE
            } else {
                let next_target = &frame_indices[1];
                let mut best_dispose = DISPOSE_NONE;
                let mut best_total = usize::MAX;

                for d in [DISPOSE_NONE, DISPOSE_BG, DISPOSE_PREV] {
                    let lookahead =
                        apply_dispose_copy(&canvas, w, &full_region, d, bpp, Some(&pre_composite));

                    let next_size = lookahead_next_frame_size_indexed(
                        &lookahead,
                        next_target,
                        canvas_w,
                        canvas_h,
                        w,
                        transparent_idx,
                        palette_rgba,
                        cancel,
                    )?;

                    let total = best_blend_size + next_size;
                    if total < best_total {
                        best_total = total;
                        best_dispose = d;
                    }
                }
                best_dispose
            };

            apply_dispose_in_place(
                &mut canvas,
                w,
                &full_region,
                best_dispose,
                bpp,
                Some(&pre_composite),
            );

            optimized.push(OptimizedFrame {
                dispose_op: best_dispose,
                blend_op: BLEND_SOURCE,
                region: full_region,
                subframe: Vec::new(),
                has_transparent: None,
            });
            continue;
        }

        // Frames 1+
        let source_region = compute_delta_region_indexed(&canvas, target, canvas_w, canvas_h);

        if source_region.is_none() {
            let region = DeltaRegion {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            };
            optimized.push(OptimizedFrame {
                dispose_op: DISPOSE_NONE,
                blend_op: BLEND_SOURCE,
                region,
                subframe: vec![target[0]],
                has_transparent: None,
            });
            continue;
        }

        let source_region = source_region.unwrap();
        let source_sub = extract_subframe_indexed(target, canvas_w, &source_region);
        let row_bytes = source_region.width as usize;
        let height = source_region.height as usize;

        let source_size = trial_compress_size(&source_sub, row_bytes, height, bpp, None, cancel)?;

        // OVER only possible with a transparent palette entry AND safe pixels
        let (best_blend, best_blend_size, best_sub) = if let Some(tidx) = transparent_idx {
            if can_use_over_indexed(target, &canvas, w, &source_region, palette_rgba) {
                let over_sub =
                    build_over_subframe_indexed(target, &canvas, w, &source_region, tidx);
                let over_size =
                    trial_compress_size(&over_sub, row_bytes, height, bpp, None, cancel)?;
                if over_size < source_size {
                    (BLEND_OVER, over_size, over_sub)
                } else {
                    (BLEND_SOURCE, source_size, source_sub)
                }
            } else {
                (BLEND_SOURCE, source_size, source_sub)
            }
        } else {
            (BLEND_SOURCE, source_size, source_sub)
        };

        // Save pre-composite (for DISPOSE_PREV)
        let pre_composite = save_region(&canvas, w, &source_region, bpp);

        // Blit target indices onto canvas
        for dy in 0..source_region.height as usize {
            let y = source_region.y as usize + dy;
            for dx in 0..source_region.width as usize {
                let x = source_region.x as usize + dx;
                canvas[y * w + x] = target[y * w + x];
            }
        }

        // Choose dispose via lookahead (except last frame)
        let best_dispose = if i == frame_indices.len() - 1 {
            DISPOSE_NONE
        } else {
            let next_target = &frame_indices[i + 1];
            let mut best_dispose = DISPOSE_NONE;
            let mut best_total = usize::MAX;

            for d in [DISPOSE_NONE, DISPOSE_BG, DISPOSE_PREV] {
                let lookahead =
                    apply_dispose_copy(&canvas, w, &source_region, d, bpp, Some(&pre_composite));

                let next_size = lookahead_next_frame_size_indexed(
                    &lookahead,
                    next_target,
                    canvas_w,
                    canvas_h,
                    w,
                    transparent_idx,
                    palette_rgba,
                    cancel,
                )?;

                let total = best_blend_size + next_size;
                if total < best_total {
                    best_total = total;
                    best_dispose = d;
                }
            }
            best_dispose
        };

        apply_dispose_in_place(
            &mut canvas,
            w,
            &source_region,
            best_dispose,
            bpp,
            Some(&pre_composite),
        );

        optimized.push(OptimizedFrame {
            dispose_op: best_dispose,
            blend_op: best_blend,
            region: source_region,
            subframe: best_sub,
            has_transparent: None,
        });
    }

    Ok(optimized)
}

/// Compute the best trial-compressed size for the next indexed frame.
#[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
#[allow(clippy::too_many_arguments)]
fn lookahead_next_frame_size_indexed(
    canvas: &[u8],
    next_target: &[u8],
    canvas_w: u32,
    canvas_h: u32,
    w: usize,
    transparent_idx: Option<u8>,
    palette_rgba: &[[u8; 4]],
    cancel: &dyn Stop,
) -> crate::error::Result<usize> {
    let bpp = 1usize;
    let next_region = compute_delta_region_indexed(canvas, next_target, canvas_w, canvas_h);

    if next_region.is_none() {
        return Ok(20);
    }

    let next_region = next_region.unwrap();
    let row_bytes = next_region.width as usize;
    let height = next_region.height as usize;

    let next_source = extract_subframe_indexed(next_target, canvas_w, &next_region);
    let source_size = trial_compress_size(&next_source, row_bytes, height, bpp, None, cancel)?;

    if let Some(tidx) = transparent_idx {
        if can_use_over_indexed(next_target, canvas, w, &next_region, palette_rgba) {
            let next_over = build_over_subframe_indexed(next_target, canvas, w, &next_region, tidx);
            let over_size = trial_compress_size(&next_over, row_bytes, height, bpp, None, cancel)?;
            Ok(source_size.min(over_size))
        } else {
            Ok(source_size)
        }
    } else {
        Ok(source_size)
    }
}

// ── Chunk writing helpers ───────────────────────────────────────────

/// Write an acTL (animation control) chunk.
fn write_actl(out: &mut Vec<u8>, num_frames: u32, num_plays: u32) {
    let mut data = [0u8; 8];
    data[0..4].copy_from_slice(&num_frames.to_be_bytes());
    data[4..8].copy_from_slice(&num_plays.to_be_bytes());
    write_chunk(out, b"acTL", &data);
}

/// Write an fcTL (frame control) chunk.
#[allow(clippy::too_many_arguments)]
fn write_fctl(
    out: &mut Vec<u8>,
    seq: u32,
    w: u32,
    h: u32,
    x: u32,
    y: u32,
    delay_num: u16,
    delay_den: u16,
    dispose_op: u8,
    blend_op: u8,
) {
    let mut data = [0u8; 26];
    data[0..4].copy_from_slice(&seq.to_be_bytes());
    data[4..8].copy_from_slice(&w.to_be_bytes());
    data[8..12].copy_from_slice(&h.to_be_bytes());
    data[12..16].copy_from_slice(&x.to_be_bytes());
    data[16..20].copy_from_slice(&y.to_be_bytes());
    data[20..22].copy_from_slice(&delay_num.to_be_bytes());
    data[22..24].copy_from_slice(&delay_den.to_be_bytes());
    data[24] = dispose_op;
    data[25] = blend_op;
    write_chunk(out, b"fcTL", &data);
}

/// Write an fdAT (frame data) chunk. Prepends the 4-byte sequence number.
fn write_fdat(out: &mut Vec<u8>, seq: u32, compressed: &[u8]) {
    let mut data = Vec::with_capacity(4 + compressed.len());
    data.extend_from_slice(&seq.to_be_bytes());
    data.extend_from_slice(compressed);
    write_chunk(out, b"fdAT", &data);
}

// ── Sequence number counter ─────────────────────────────────────────

/// Monotonic sequence number counter for fcTL and fdAT chunks.
struct SeqCounter {
    next: u32,
}

impl SeqCounter {
    fn new() -> Self {
        Self { next: 0 }
    }

    fn next(&mut self) -> u32 {
        let val = self.next;
        self.next += 1;
        val
    }
}

// ── Truecolor APNG encode ───────────────────────────────────────────

/// Check if all RGBA8 frames are fully opaque (all alpha == 255).
///
/// When true, we can encode as RGB (color_type=2, bpp=3) for 25% raw savings.
fn all_frames_opaque(frames: &[ApngFrameInput<'_>], expected_len: usize) -> bool {
    for frame in frames {
        for chunk in frame.pixels[..expected_len].as_chunks::<4>().0.iter() {
            if chunk[3] != 255 {
                return false;
            }
        }
    }
    true
}

/// Encode canvas-sized RGBA8 frames into a truecolor APNG file.
///
/// Automatically detects fully opaque animations and encodes as RGB (25% savings).
/// When effort > 2 and there are multiple frames, runs 6-way dispose/blend
/// optimization to find the best per-frame combination, then compresses each
/// optimized subframe at the target effort level.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn encode_apng_truecolor(
    frames: &[ApngFrameInput<'_>],
    canvas_width: u32,
    canvas_height: u32,
    write_meta: &PngWriteMetadata<'_>,
    num_plays: u32,
    effort: u32,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<Vec<u8>> {
    encode_apng_truecolor_depth(
        frames,
        canvas_width,
        canvas_height,
        8,
        write_meta,
        num_plays,
        effort,
        false,
        true,
        1,
        cancel,
        deadline,
    )
}

/// RGBA16 input is in PNG's big-endian order. The 8-bit optimizer is bypassed
/// at 16 bits; byte-exact delta regions still avoid encoding unchanged areas.
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_apng_truecolor_depth(
    frames: &[ApngFrameInput<'_>],
    canvas_width: u32,
    canvas_height: u32,
    bit_depth: u8,
    write_meta: &PngWriteMetadata<'_>,
    num_plays: u32,
    effort: u32,
    preserve_transparent_rgb: bool,
    allow_rgb: bool,
    max_threads: usize,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<Vec<u8>> {
    let expected_rgba = canvas_width as usize * canvas_height as usize * usize::from(bit_depth) / 2;

    // Detect all-opaque → use RGB (color_type=2, bpp=3, 25% raw savings)
    let is_opaque = bit_depth == 8 && allow_rgb && all_frames_opaque(frames, expected_rgba);

    // Frames already in the written layout; opaque RGBA input is reduced once.
    let rgb_frames;
    let packed: Vec<ApngFrameInput<'_>> = if is_opaque {
        rgb_frames = frames
            .iter()
            .map(|f| crate::optimize::rgba8_to_rgb8(&f.pixels[..expected_rgba]))
            .collect::<Vec<_>>();
        frames
            .iter()
            .zip(&rgb_frames)
            .map(|(f, rgb)| ApngFrameInput {
                pixels: rgb,
                delay_num: f.delay_num,
                delay_den: f.delay_den,
            })
            .collect()
    } else {
        frames.to_vec()
    };
    encode_apng_packed_depth(
        &packed,
        canvas_width,
        canvas_height,
        bit_depth,
        is_opaque,
        write_meta,
        num_plays,
        effort,
        preserve_transparent_rgb,
        max_threads,
        cancel,
        deadline,
    )
}

/// Encode canvas-sized frames already in the written sample layout: RGB8
/// when `opaque` is set for 8-bit input, RGBA8 otherwise, big-endian RGBA16
/// at 16 bits. `packed`'s `pixels` are the written-layout frame buffers.
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_apng_packed_depth(
    frames: &[ApngFrameInput<'_>],
    canvas_width: u32,
    canvas_height: u32,
    bit_depth: u8,
    opaque: bool,
    write_meta: &PngWriteMetadata<'_>,
    num_plays: u32,
    effort: u32,
    preserve_transparent_rgb: bool,
    max_threads: usize,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<Vec<u8>> {
    let num_frames = u32::try_from(frames.len())
        .map_err(|_| at!(PngError::InvalidInput("too many APNG frames".into())))?;
    let (bpp, color_type): (usize, u8) = if bit_depth == 16 {
        (8, 6)
    } else if opaque {
        (3, 2)
    } else {
        (4, 6)
    };
    let expected = canvas_width as usize * canvas_height as usize * bpp;

    // Run optimizer when effort > 2 and >1 frame (otherwise trial = final, no benefit)
    let use_optimizer =
        apng_uses_optimizer(bit_depth, effort, preserve_transparent_rgb, frames.len());
    if !use_optimizer {
        // Frame by frame, each against the one before: the same writer
        // streaming output uses, so both give the same bytes. Frames are
        // already in the written layout, so nothing converts or copies.
        let mut out = Vec::new();
        let mut writer = ApngStreamWriter::start(
            &mut out,
            canvas_width,
            canvas_height,
            bit_depth,
            opaque,
            write_meta,
            num_frames,
            num_plays,
            effort,
            preserve_transparent_rgb,
        )?;
        let mut prev: Option<&[u8]> = None;
        for frame in frames {
            cancel.check().map_err(|e| at!(PngError::from(e)))?;
            let curr = &frame.pixels[..expected];
            writer.write_frame(
                &mut out,
                curr,
                prev,
                frame.delay_num,
                frame.delay_den,
                cancel,
                deadline,
            )?;
            prev = Some(curr);
        }
        writer.finish(&mut out, write_meta);
        return Ok(out);
    }

    let frame_data: Vec<&[u8]> = frames.iter().map(|f| &f.pixels[..expected]).collect();
    let opt_frames = optimize_apng_truecolor(
        frames,
        &frame_data,
        canvas_width,
        canvas_height,
        bpp,
        max_threads,
        cancel,
    )?;

    // Estimate output size
    let frame_size_est = canvas_width as usize * canvas_height as usize * bpp;
    let est = 8 + 25 + 20 + 38 + frame_size_est + metadata_size_estimate(write_meta);
    let mut out = Vec::with_capacity(est);
    let mut seq = SeqCounter::new();
    write_apng_head(
        &mut out,
        canvas_width,
        canvas_height,
        bit_depth,
        color_type,
        write_meta,
        num_frames,
        num_plays,
    )?;

    {
        // ── Optimized path: use per-frame dispose/blend from optimizer ──
        for (i, opt) in opt_frames.iter().enumerate() {
            cancel.check().map_err(|e| at!(PngError::from(e)))?;

            let frame = &frames[i];
            let fctl_seq = seq.next();

            // Frame 0 uses full canvas as region (optimizer knows this)
            let (region_w, region_h, region_x, region_y) = if i == 0 {
                (canvas_width, canvas_height, 0u32, 0u32)
            } else {
                (
                    opt.region.width,
                    opt.region.height,
                    opt.region.x,
                    opt.region.y,
                )
            };

            write_fctl(
                &mut out,
                fctl_seq,
                region_w,
                region_h,
                region_x,
                region_y,
                frame.delay_num,
                frame.delay_den,
                opt.dispose_op,
                opt.blend_op,
            );

            let sub_row_bytes = region_w as usize * bpp;
            let sub_height = region_h as usize;
            let sub_data = if i == 0 {
                // Frame 0 always uses full canvas pixels
                frame_data[0]
            } else {
                &opt.subframe
            };

            let opts = CompressOptions {
                preserve_transparent_rgb,
                parallel: false,
                cancel,
                deadline,
                remaining_ns: None,
                max_threads: 0,
                decode_segments: 0,
            };
            let compressed = compress_filtered_t(
                sub_data,
                sub_row_bytes,
                sub_height,
                RowFormat::from_png(color_type, bit_depth),
                effort,
                opts,
                None,
                if i == 0 { None } else { opt.has_transparent },
            )?;

            if i == 0 {
                write_chunk(&mut out, b"IDAT", &compressed);
            } else {
                let fdat_seq = seq.next();
                write_fdat(&mut out, fdat_seq, &compressed);
            }
        }
    }

    // IEND
    write_tail_ancillary(&mut out, write_meta);
    write_chunk(&mut out, b"IEND", &[]);

    Ok(out)
}

/// Whether the truecolor APNG encoder runs its cross-frame optimizer, which
/// needs every frame before it writes one.
pub(crate) fn apng_uses_optimizer(
    bit_depth: u8,
    effort: u32,
    preserve_transparent_rgb: bool,
    frames: usize,
) -> bool {
    !preserve_transparent_rgb && bit_depth == 8 && effort > 2 && frames > 1
}

/// Signature, IHDR, metadata and acTL: everything before the first frame.
#[allow(clippy::too_many_arguments)]
fn write_apng_head(
    out: &mut Vec<u8>,
    canvas_width: u32,
    canvas_height: u32,
    bit_depth: u8,
    color_type: u8,
    write_meta: &PngWriteMetadata<'_>,
    num_frames: u32,
    num_plays: u32,
) -> crate::error::Result<()> {
    out.extend_from_slice(&PNG_SIGNATURE);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&canvas_width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&canvas_height.to_be_bytes());
    ihdr[8] = bit_depth;
    ihdr[9] = color_type;
    write_chunk(out, b"IHDR", &ihdr);
    write_all_metadata(out, write_meta)?;
    write_actl(out, num_frames, num_plays);
    Ok(())
}

/// Writes a truecolor APNG one frame at a time: frame 0 whole, each later
/// frame as the region that differs from the one before (dispose NONE,
/// blend SOURCE). Every frame's bytes are final when it is pushed, so
/// streaming output hands them out at once; the one-shot encoder uses the
/// same writer whenever the optimizer is off.
pub(crate) struct ApngStreamWriter {
    canvas_width: u32,
    canvas_height: u32,
    bit_depth: u8,
    color_type: u8,
    /// Bytes per pixel as written: 3 for RGB8, 4 for RGBA8, 8 for RGBA16.
    bpp: usize,
    effort: u32,
    preserve_transparent_rgb: bool,
    seq: SeqCounter,
    /// The previous frame as written (RGB8 when `color_type` is 2).
    prev: Option<Vec<u8>>,
}

impl ApngStreamWriter {
    /// Write everything before the first frame. `opaque` selects RGB8 for
    /// 8-bit input: every frame must then be opaque RGBA8.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start(
        out: &mut Vec<u8>,
        canvas_width: u32,
        canvas_height: u32,
        bit_depth: u8,
        opaque: bool,
        write_meta: &PngWriteMetadata<'_>,
        num_frames: u32,
        num_plays: u32,
        effort: u32,
        preserve_transparent_rgb: bool,
    ) -> crate::error::Result<Self> {
        let (bpp, color_type) = if bit_depth == 16 {
            (8, 6)
        } else if opaque {
            (3, 2)
        } else {
            (4, 6)
        };
        write_apng_head(
            out,
            canvas_width,
            canvas_height,
            bit_depth,
            color_type,
            write_meta,
            num_frames,
            num_plays,
        )?;
        Ok(Self {
            canvas_width,
            canvas_height,
            bit_depth,
            color_type,
            bpp,
            effort,
            preserve_transparent_rgb,
            seq: SeqCounter::new(),
            prev: None,
        })
    }

    /// Write one canvas-sized frame already in the written sample layout
    /// (RGB8 when the colour type is 2), keeping it as the next frame's
    /// reference.
    pub(crate) fn push_frame(
        &mut self,
        out: &mut Vec<u8>,
        curr: Vec<u8>,
        delay_num: u16,
        delay_den: u16,
        cancel: &dyn Stop,
        deadline: &dyn Stop,
    ) -> crate::error::Result<()> {
        let prev = self.prev.take();
        self.write_frame(
            out,
            &curr,
            prev.as_deref(),
            delay_num,
            delay_den,
            cancel,
            deadline,
        )?;
        self.prev = Some(curr);
        Ok(())
    }

    /// Write one frame already in the written sample layout (RGB8 when the
    /// colour type is 2), as the region that differs from `prev`, or whole
    /// when `prev` is `None` (the first frame).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_frame(
        &mut self,
        out: &mut Vec<u8>,
        curr: &[u8],
        prev: Option<&[u8]>,
        delay_num: u16,
        delay_den: u16,
        cancel: &dyn Stop,
        deadline: &dyn Stop,
    ) -> crate::error::Result<()> {
        let (region, subframe) = match prev {
            None => (
                DeltaRegion {
                    x: 0,
                    y: 0,
                    width: self.canvas_width,
                    height: self.canvas_height,
                },
                None,
            ),
            Some(prev) => match compute_delta_region(
                prev,
                curr,
                self.canvas_width,
                self.canvas_height,
                self.bpp,
            ) {
                Some(region) => {
                    let sub = extract_subframe(curr, self.canvas_width, &region, self.bpp);
                    (region, Some(sub))
                }
                None => {
                    let (region, sub) = minimal_subframe(curr, self.bpp);
                    (region, Some(sub))
                }
            },
        };
        write_fctl(
            out,
            self.seq.next(),
            region.width,
            region.height,
            region.x,
            region.y,
            delay_num,
            delay_den,
            DISPOSE_NONE,
            BLEND_SOURCE,
        );
        let opts = CompressOptions {
            preserve_transparent_rgb: self.preserve_transparent_rgb,
            parallel: false,
            cancel,
            deadline,
            remaining_ns: None,
            max_threads: 0,
            decode_segments: 0,
        };
        let compressed = compress_filtered(
            subframe.as_deref().unwrap_or(curr),
            region.width as usize * self.bpp,
            region.height as usize,
            RowFormat::from_png(self.color_type, self.bit_depth),
            self.effort,
            opts,
            None,
        )?;
        if prev.is_none() {
            write_chunk(out, b"IDAT", &compressed);
        } else {
            write_fdat(out, self.seq.next(), &compressed);
        }
        Ok(())
    }

    /// Write what follows the last frame.
    pub(crate) fn finish(self, out: &mut Vec<u8>, write_meta: &PngWriteMetadata<'_>) {
        write_tail_ancillary(out, write_meta);
        write_chunk(out, b"IEND", &[]);
    }
}

// ── Indexed delta region helpers ─────────────────────────────────────

/// Find the bounding box of differing indices between two canvas-sized index buffers.
///
/// Returns `None` if the buffers are identical.
#[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
fn compute_delta_region_indexed(prev: &[u8], curr: &[u8], w: u32, h: u32) -> Option<DeltaRegion> {
    let w = w as usize;
    let h = h as usize;
    let mut min_x = w;
    let mut max_x = 0usize;
    let mut min_y = h;
    let mut max_y = 0usize;

    for y in 0..h {
        let row_start = y * w;
        for x in 0..w {
            let off = row_start + x;
            if prev[off] != curr[off] {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }

    if min_x > max_x || min_y > max_y {
        return None; // identical
    }

    Some(DeltaRegion {
        x: min_x as u32,
        y: min_y as u32,
        width: (max_x - min_x + 1) as u32,
        height: (max_y - min_y + 1) as u32,
    })
}

/// Extract a rectangular subregion from a canvas-sized index buffer (1 byte/pixel).
#[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
fn extract_subframe_indexed(indices: &[u8], canvas_w: u32, region: &DeltaRegion) -> Vec<u8> {
    let canvas_w = canvas_w as usize;
    let rw = region.width as usize;
    let rh = region.height as usize;
    let rx = region.x as usize;
    let ry = region.y as usize;

    let mut out = Vec::with_capacity(rw * rh);
    for y in ry..ry + rh {
        let row_start = y * canvas_w + rx;
        out.extend_from_slice(&indices[row_start..row_start + rw]);
    }
    out
}

// ── Indexed APNG from pre-remapped indices ──────────────────────────

/// Encode canvas-sized frames into an indexed APNG from pre-remapped index buffers.
///
/// Takes pre-built palette and per-frame index buffers (from zenquant remap).
/// Delta regions are computed on index buffers directly (more correct with
/// temporal clamping since identical indices mean identical visual output).
#[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_apng_indexed_from_indices(
    frames: &[ApngFrameInput<'_>],
    palette_rgba: &[[u8; 4]],
    frame_indices: &[Vec<u8>],
    canvas_width: u32,
    canvas_height: u32,
    write_meta: &PngWriteMetadata<'_>,
    num_plays: u32,
    effort: u32,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<Vec<u8>> {
    let num_frames = frames.len() as u32;
    let n_colors = palette_rgba.len();

    // Every frame's indices must reference existing palette entries — the
    // packer would otherwise silently remap or emit spec-invalid values
    // (sweep issue #20; see validate_palette_indices).
    for indices in frame_indices {
        super::validate_palette_indices(indices, n_colors)?;
    }

    // Build separate RGB and alpha palette arrays
    let mut palette_rgb = Vec::with_capacity(n_colors * 3);
    let mut palette_alpha = Vec::with_capacity(n_colors);
    let mut has_transparency = false;

    for entry in palette_rgba {
        palette_rgb.push(entry[0]);
        palette_rgb.push(entry[1]);
        palette_rgb.push(entry[2]);
        palette_alpha.push(entry[3]);
        if entry[3] < 255 {
            has_transparency = true;
        }
    }

    let bit_depth = super::select_bit_depth(n_colors);
    let trns_data = super::truncate_trns(if has_transparency {
        Some(palette_alpha.as_slice())
    } else {
        None
    });

    let est = 8
        + 25
        + 20
        + (12 + n_colors * 3)
        + 38 * num_frames as usize
        + metadata_size_estimate(write_meta);
    let mut out = Vec::with_capacity(est);
    let mut seq = SeqCounter::new();

    // PNG signature
    out.extend_from_slice(&PNG_SIGNATURE);

    // IHDR: canvas dimensions, indexed color
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&canvas_width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&canvas_height.to_be_bytes());
    ihdr[8] = bit_depth;
    ihdr[9] = 3; // color type: indexed
    write_chunk(&mut out, b"IHDR", &ihdr);

    // Metadata
    write_all_metadata(&mut out, write_meta)?;

    // acTL
    write_actl(&mut out, num_frames, num_plays);

    // PLTE
    write_chunk(&mut out, b"PLTE", &palette_rgb[..n_colors * 3]);

    // tRNS
    if let Some(trns) = &trns_data {
        write_chunk(&mut out, b"tRNS", trns);
    }

    // Run optimizer when effort > 2 and >1 frame
    let use_optimizer = effort > 2 && frames.len() > 1;
    let optimized = if use_optimizer {
        Some(optimize_apng_indexed(
            frame_indices,
            palette_rgba,
            canvas_width,
            canvas_height,
            cancel,
        )?)
    } else {
        None
    };

    if let Some(ref opt_frames) = optimized {
        // ── Optimized path ──
        for (i, opt) in opt_frames.iter().enumerate() {
            cancel.check().map_err(|e| at!(PngError::from(e)))?;

            let frame = &frames[i];
            let (region_w, region_h, region_x, region_y) = if i == 0 {
                (canvas_width, canvas_height, 0u32, 0u32)
            } else {
                (
                    opt.region.width,
                    opt.region.height,
                    opt.region.x,
                    opt.region.y,
                )
            };

            let sub_indices = if i == 0 {
                frame_indices[0].clone()
            } else {
                opt.subframe.clone()
            };

            let packed = super::pack_all_rows(
                &sub_indices,
                region_w as usize,
                region_h as usize,
                bit_depth,
            );
            let row_bytes = super::packed_row_bytes(region_w as usize, bit_depth);

            let fctl_seq = seq.next();
            write_fctl(
                &mut out,
                fctl_seq,
                region_w,
                region_h,
                region_x,
                region_y,
                frame.delay_num,
                frame.delay_den,
                opt.dispose_op,
                opt.blend_op,
            );

            let opts = CompressOptions {
                preserve_transparent_rgb: false,
                parallel: false,
                cancel,
                deadline,
                remaining_ns: None,
                max_threads: 0,
                decode_segments: 0,
            };
            let compressed = compress_filtered(
                &packed,
                row_bytes,
                region_h as usize,
                RowFormat::INDEXED,
                effort,
                opts,
                None,
            )?;

            if i == 0 {
                write_chunk(&mut out, b"IDAT", &compressed);
            } else {
                let fdat_seq = seq.next();
                write_fdat(&mut out, fdat_seq, &compressed);
            }
        }
    } else {
        // ── Unoptimized path ──
        for (i, frame) in frames.iter().enumerate() {
            cancel.check().map_err(|e| at!(PngError::from(e)))?;

            let curr_indices = &frame_indices[i];

            let (region_x, region_y, region_w, region_h, sub_indices) = if i == 0 {
                (
                    0u32,
                    0u32,
                    canvas_width,
                    canvas_height,
                    curr_indices.clone(),
                )
            } else {
                let prev_indices = &frame_indices[i - 1];
                match compute_delta_region_indexed(
                    prev_indices,
                    curr_indices,
                    canvas_width,
                    canvas_height,
                ) {
                    Some(region) => {
                        let sub = extract_subframe_indexed(curr_indices, canvas_width, &region);
                        (region.x, region.y, region.width, region.height, sub)
                    }
                    None => (0, 0, 1, 1, vec![curr_indices[0]]),
                }
            };

            let packed = super::pack_all_rows(
                &sub_indices,
                region_w as usize,
                region_h as usize,
                bit_depth,
            );
            let row_bytes = super::packed_row_bytes(region_w as usize, bit_depth);

            let fctl_seq = seq.next();
            write_fctl(
                &mut out,
                fctl_seq,
                region_w,
                region_h,
                region_x,
                region_y,
                frame.delay_num,
                frame.delay_den,
                DISPOSE_NONE,
                BLEND_SOURCE,
            );

            let opts = CompressOptions {
                preserve_transparent_rgb: false,
                parallel: false,
                cancel,
                deadline,
                remaining_ns: None,
                max_threads: 0,
                decode_segments: 0,
            };
            let compressed = compress_filtered(
                &packed,
                row_bytes,
                region_h as usize,
                RowFormat::INDEXED,
                effort,
                opts,
                None,
            )?;

            if i == 0 {
                write_chunk(&mut out, b"IDAT", &compressed);
            } else {
                let fdat_seq = seq.next();
                write_fdat(&mut out, fdat_seq, &compressed);
            }
        }
    }

    // IEND
    write_tail_ancillary(&mut out, write_meta);
    write_chunk(&mut out, b"IEND", &[]);

    Ok(out)
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use zenpixels_convert::PixelBufferConvertTypedExt;

    // ── Delta region tests ──────────────────────────────────────────

    #[test]
    fn delta_region_known_diff() {
        // 4×4 RGBA8 frames, differing at (1,1) and (2,2)
        let w = 4u32;
        let h = 4u32;
        let prev = vec![0u8; (w * h * 4) as usize];
        let mut curr = prev.clone();

        // Change pixel (1,1)
        let off1 = ((w + 1) * 4) as usize;
        curr[off1] = 255;

        // Change pixel (2,2)
        let off2 = ((2 * w + 2) * 4) as usize;
        curr[off2] = 128;

        let region = compute_delta_region(&prev, &curr, w, h, 4).unwrap();
        assert_eq!(region.x, 1);
        assert_eq!(region.y, 1);
        assert_eq!(region.width, 2);
        assert_eq!(region.height, 2);
    }

    #[test]
    fn delta_region_identical_frames() {
        let w = 4u32;
        let h = 4u32;
        let frame = vec![42u8; (w * h * 4) as usize];
        assert!(compute_delta_region(&frame, &frame, w, h, 4).is_none());
    }

    #[test]
    fn subframe_extraction() {
        // 4×4 RGBA, extract 2×2 region at (1,1)
        let w = 4u32;
        let mut pixels = vec![0u8; (w * 4 * 4) as usize];
        // Mark pixel (1,1) = [1,2,3,4], (2,1) = [5,6,7,8]
        // (1,2) = [9,10,11,12], (2,2) = [13,14,15,16]
        let set = |px: &mut [u8], x: usize, y: usize, vals: [u8; 4]| {
            let off = (y * w as usize + x) * 4;
            px[off..off + 4].copy_from_slice(&vals);
        };
        set(&mut pixels, 1, 1, [1, 2, 3, 4]);
        set(&mut pixels, 2, 1, [5, 6, 7, 8]);
        set(&mut pixels, 1, 2, [9, 10, 11, 12]);
        set(&mut pixels, 2, 2, [13, 14, 15, 16]);

        let region = DeltaRegion {
            x: 1,
            y: 1,
            width: 2,
            height: 2,
        };
        let sub = extract_subframe(&pixels, w, &region, 4);
        assert_eq!(sub.len(), 2 * 2 * 4);
        assert_eq!(&sub[0..4], &[1, 2, 3, 4]);
        assert_eq!(&sub[4..8], &[5, 6, 7, 8]);
        assert_eq!(&sub[8..12], &[9, 10, 11, 12]);
        assert_eq!(&sub[12..16], &[13, 14, 15, 16]);
    }

    #[test]
    fn sequence_counter_monotonic() {
        let mut seq = SeqCounter::new();
        assert_eq!(seq.next(), 0);
        assert_eq!(seq.next(), 1);
        assert_eq!(seq.next(), 2);
        assert_eq!(seq.next(), 3);
    }

    // ── Chunk writing tests ─────────────────────────────────────────

    #[test]
    fn actl_chunk_format() {
        let mut out = Vec::new();
        write_actl(&mut out, 5, 0);
        // length(4) + type(4) + data(8) + crc(4) = 20
        assert_eq!(out.len(), 20);
        // Check chunk type
        assert_eq!(&out[4..8], b"acTL");
        // Check num_frames = 5
        assert_eq!(u32::from_be_bytes(out[8..12].try_into().unwrap()), 5);
        // Check num_plays = 0
        assert_eq!(u32::from_be_bytes(out[12..16].try_into().unwrap()), 0);
    }

    #[test]
    fn fctl_chunk_format() {
        let mut out = Vec::new();
        write_fctl(&mut out, 0, 100, 200, 10, 20, 1, 30, 0, 0);
        // length(4) + type(4) + data(26) + crc(4) = 38
        assert_eq!(out.len(), 38);
        assert_eq!(&out[4..8], b"fcTL");
        // Check sequence number
        assert_eq!(u32::from_be_bytes(out[8..12].try_into().unwrap()), 0);
        // Check width
        assert_eq!(u32::from_be_bytes(out[12..16].try_into().unwrap()), 100);
        // Check height
        assert_eq!(u32::from_be_bytes(out[16..20].try_into().unwrap()), 200);
    }

    #[test]
    fn fdat_chunk_format() {
        let mut out = Vec::new();
        let data = [1, 2, 3, 4, 5];
        write_fdat(&mut out, 42, &data);
        // length(4) + type(4) + seq(4) + data(5) + crc(4) = 21
        assert_eq!(out.len(), 21);
        assert_eq!(&out[4..8], b"fdAT");
        // Check sequence number in data
        assert_eq!(u32::from_be_bytes(out[8..12].try_into().unwrap()), 42);
        // Check payload
        assert_eq!(&out[12..17], &[1, 2, 3, 4, 5]);
    }

    // ── Integration: roundtrip tests ────────────────────────────────

    #[test]
    fn two_frame_roundtrip() {
        let w = 4u32;
        let h = 4u32;
        let npx = (w * h) as usize;

        // Frame 0: all red
        let mut f0 = vec![0u8; npx * 4];
        for px in f0.as_chunks_mut::<4>().0.iter_mut() {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }

        // Frame 1: all green
        let mut f1 = vec![0u8; npx * 4];
        for px in f1.as_chunks_mut::<4>().0.iter_mut() {
            px.copy_from_slice(&[0, 255, 0, 255]);
        }

        let frames = [
            ApngFrameInput {
                pixels: &f0,
                delay_num: 100,
                delay_den: 1000,
            },
            ApngFrameInput {
                pixels: &f1,
                delay_num: 200,
                delay_den: 1000,
            },
        ];

        let write_meta = PngWriteMetadata::from_metadata(None);
        let encoded = encode_apng_truecolor(
            &frames,
            w,
            h,
            &write_meta,
            0,
            6,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        // Decode and verify
        let decoded = crate::decode::decode_apng(
            &encoded,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();

        assert_eq!(decoded.frames.len(), 2);
        assert_eq!(decoded.info.width, w);
        assert_eq!(decoded.info.height, h);

        // Verify frame 0 pixels (RGBA8)
        let f0_pixels = decoded.frames[0].pixels.to_rgba8();
        let f0_img = f0_pixels.as_imgref();
        assert_eq!(f0_img.buf()[0].r, 255);
        assert_eq!(f0_img.buf()[0].g, 0);

        // Verify frame 1 pixels
        let f1_pixels = decoded.frames[1].pixels.to_rgba8();
        let f1_img = f1_pixels.as_imgref();
        assert_eq!(f1_img.buf()[0].r, 0);
        assert_eq!(f1_img.buf()[0].g, 255);
    }

    #[test]
    fn three_frame_delta_roundtrip() {
        let w = 8u32;
        let h = 8u32;
        let npx = (w * h) as usize;

        // Frame 0: all black
        let f0 = vec![0u8; npx * 4];

        // Frame 1: pixel (3,3) changed to white
        let mut f1 = f0.clone();
        let off = ((3 * w + 3) * 4) as usize;
        f1[off..off + 4].copy_from_slice(&[255, 255, 255, 255]);

        // Frame 2: pixel (5,5) also changed to red
        let mut f2 = f1.clone();
        let off2 = ((5 * w + 5) * 4) as usize;
        f2[off2..off2 + 4].copy_from_slice(&[255, 0, 0, 255]);

        let frames = [
            ApngFrameInput {
                pixels: &f0,
                delay_num: 1,
                delay_den: 10,
            },
            ApngFrameInput {
                pixels: &f1,
                delay_num: 1,
                delay_den: 10,
            },
            ApngFrameInput {
                pixels: &f2,
                delay_num: 1,
                delay_den: 10,
            },
        ];

        let write_meta = PngWriteMetadata::from_metadata(None);
        let encoded = encode_apng_truecolor(
            &frames,
            w,
            h,
            &write_meta,
            0,
            6,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let decoded = crate::decode::decode_apng(
            &encoded,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();

        assert_eq!(decoded.frames.len(), 3);

        // Frame 2: pixel (3,3) should be white, (5,5) should be red
        let f2_pixels = decoded.frames[2].pixels.to_rgba8();
        let f2_img = f2_pixels.as_imgref();
        let px33 = &f2_img.buf()[3 * w as usize + 3];
        assert_eq!((px33.r, px33.g, px33.b, px33.a), (255, 255, 255, 255));
        let px55 = &f2_img.buf()[5 * w as usize + 5];
        assert_eq!((px55.r, px55.g, px55.b, px55.a), (255, 0, 0, 255));
    }

    #[test]
    fn single_frame_apng() {
        let w = 2u32;
        let h = 2u32;
        let f0 = vec![128u8; (w * h * 4) as usize];

        let frames = [ApngFrameInput {
            pixels: &f0,
            delay_num: 0,
            delay_den: 0,
        }];

        let write_meta = PngWriteMetadata::from_metadata(None);
        let encoded = encode_apng_truecolor(
            &frames,
            w,
            h,
            &write_meta,
            0,
            1,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let decoded = crate::decode::decode_apng(
            &encoded,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();

        assert_eq!(decoded.frames.len(), 1);
    }

    #[test]
    fn timing_preservation() {
        let w = 2u32;
        let h = 2u32;
        let f0 = vec![0u8; (w * h * 4) as usize];
        let f1 = vec![255u8; (w * h * 4) as usize];

        let frames = [
            ApngFrameInput {
                pixels: &f0,
                delay_num: 1,
                delay_den: 30,
            },
            ApngFrameInput {
                pixels: &f1,
                delay_num: 5,
                delay_den: 100,
            },
        ];

        let write_meta = PngWriteMetadata::from_metadata(None);
        let encoded = encode_apng_truecolor(
            &frames,
            w,
            h,
            &write_meta,
            3,
            1,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let decoded = crate::decode::decode_apng(
            &encoded,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();

        assert_eq!(decoded.num_plays, 3);
        assert_eq!(decoded.frames[0].frame_info.delay_num, 1);
        assert_eq!(decoded.frames[0].frame_info.delay_den, 30);
        assert_eq!(decoded.frames[1].frame_info.delay_num, 5);
        assert_eq!(decoded.frames[1].frame_info.delay_den, 100);
    }

    #[test]
    fn all_compression_levels_valid() {
        let w = 4u32;
        let h = 4u32;
        let f0 = vec![100u8; (w * h * 4) as usize];
        let mut f1 = f0.clone();
        f1[0] = 200;

        for effort in [0u32, 2, 6, 10, 13, 16, 20] {
            let frames = [
                ApngFrameInput {
                    pixels: &f0,
                    delay_num: 1,
                    delay_den: 10,
                },
                ApngFrameInput {
                    pixels: &f1,
                    delay_num: 1,
                    delay_den: 10,
                },
            ];

            let write_meta = PngWriteMetadata::from_metadata(None);
            let encoded = encode_apng_truecolor(
                &frames,
                w,
                h,
                &write_meta,
                0,
                effort,
                &enough::Unstoppable,
                &enough::Unstoppable,
            )
            .unwrap();

            let decoded = crate::decode::decode_apng(
                &encoded,
                &crate::decode::PngDecodeConfig::none(),
                &enough::Unstoppable,
            )
            .unwrap();
            assert_eq!(decoded.frames.len(), 2, "effort {effort} failed");
        }
    }

    // ── Indexed APNG tests ──────────────────────────────────────────

    #[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
    #[test]
    fn indexed_apng_roundtrip() {
        let w = 4u32;
        let h = 4u32;
        let npx = (w * h) as usize;

        // Frame 0: all red
        let mut f0 = vec![0u8; npx * 4];
        for px in f0.as_chunks_mut::<4>().0.iter_mut() {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }

        // Frame 1: all blue
        let mut f1 = vec![0u8; npx * 4];
        for px in f1.as_chunks_mut::<4>().0.iter_mut() {
            px.copy_from_slice(&[0, 0, 255, 255]);
        }

        let frames = [
            ApngFrameInput {
                pixels: &f0,
                delay_num: 1,
                delay_den: 10,
            },
            ApngFrameInput {
                pixels: &f1,
                delay_num: 1,
                delay_den: 10,
            },
        ];

        let quantizer = crate::quantize::default_quantizer();
        let apng_config = crate::encode::ApngEncodeConfig::default();
        let apng_params = crate::indexed::ApngEncodeParams {
            frames: &frames,
            canvas_width: w,
            canvas_height: h,
            config: &apng_config,
            quantizer: &*quantizer,
            metadata: None,
            cancel: &enough::Unstoppable,
            deadline: &enough::Unstoppable,
        };
        let encoded = crate::indexed::encode_apng_indexed(&apng_params).unwrap();

        // Verify it decodes (will be indexed, decoded as palette)
        let decoded = crate::decode::decode_apng(
            &encoded,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.frames.len(), 2);
    }

    // ── Optimizer-specific tests ────────────────────────────────────

    #[test]
    fn optimizer_transparent_pixel_roundtrip() {
        // Tests that frames with alpha=0 pixels round-trip correctly.
        // This exercises the canvas divergence fix: compress_filtered() zeroes
        // RGB of alpha=0 pixels, which the optimizer must account for.
        let w = 8u32;
        let h = 8u32;
        let npx = (w * h) as usize;

        // Frame 0: gradient with some transparent pixels
        let mut f0 = vec![0u8; npx * 4];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let off = (y * w as usize + x) * 4;
                f0[off] = (x * 32) as u8; // R varies
                f0[off + 1] = (y * 32) as u8; // G varies
                f0[off + 2] = 128;
                // Make some pixels transparent
                f0[off + 3] = if x < 2 && y < 2 { 0 } else { 255 };
            }
        }

        // Frame 1: same but shift the transparent region
        let mut f1 = f0.clone();
        for y in 0..h as usize {
            for x in 0..w as usize {
                let off = (y * w as usize + x) * 4;
                // Move transparent region to (6,6)-(7,7)
                f1[off + 3] = if x >= 6 && y >= 6 { 0 } else { 255 };
            }
        }

        // Frame 2: everything opaque
        let mut f2 = f0.clone();
        for px in f2.as_chunks_mut::<4>().0.iter_mut() {
            px[3] = 255;
        }

        let frames = [
            ApngFrameInput {
                pixels: &f0,
                delay_num: 1,
                delay_den: 10,
            },
            ApngFrameInput {
                pixels: &f1,
                delay_num: 1,
                delay_den: 10,
            },
            ApngFrameInput {
                pixels: &f2,
                delay_num: 1,
                delay_den: 10,
            },
        ];

        let write_meta = PngWriteMetadata::from_metadata(None);
        let encoded = encode_apng_truecolor(
            &frames,
            w,
            h,
            &write_meta,
            0,
            6,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let decoded = crate::decode::decode_apng(
            &encoded,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();

        assert_eq!(decoded.frames.len(), 3);
        for (i, (orig_input, dec_frame)) in frames.iter().zip(decoded.frames.iter()).enumerate() {
            let dec_buf: Vec<rgb::Rgba<u8>> =
                dec_frame.pixels.to_rgba8().as_imgref().buf().to_vec();
            for (j, px) in dec_buf.iter().enumerate() {
                let off = j * 4;
                let orig = rgb::Rgba {
                    r: orig_input.pixels[off],
                    g: orig_input.pixels[off + 1],
                    b: orig_input.pixels[off + 2],
                    a: orig_input.pixels[off + 3],
                };
                if orig.a == 0 && px.a == 0 {
                    continue; // both transparent
                }
                assert_eq!(*px, orig, "pixel {j} mismatch frame {i}");
            }
        }
    }

    #[test]
    fn optimizer_moving_sprite_roundtrip() {
        // Static background with a moving opaque sprite — ideal scenario for
        // dispose/blend optimization (DISPOSE_PREV + OVER gives big savings).
        let w = 16u32;
        let h = 16u32;
        let npx = (w * h) as usize;

        // Blue background
        let make_frame = |sprite_x: usize, sprite_y: usize| -> Vec<u8> {
            let mut buf = vec![0u8; npx * 4];
            for px in buf.as_chunks_mut::<4>().0.iter_mut() {
                px.copy_from_slice(&[0, 0, 200, 255]); // blue bg
            }
            // 4×4 red sprite
            for sy in 0..4 {
                for sx in 0..4 {
                    let x = sprite_x + sx;
                    let y = sprite_y + sy;
                    if x < w as usize && y < h as usize {
                        let off = (y * w as usize + x) * 4;
                        buf[off..off + 4].copy_from_slice(&[255, 0, 0, 255]);
                    }
                }
            }
            buf
        };

        let f0 = make_frame(2, 2);
        let f1 = make_frame(6, 2);
        let f2 = make_frame(10, 2);
        let f3 = make_frame(10, 6);
        let f4 = make_frame(6, 6);

        let frames: Vec<ApngFrameInput<'_>> = [&f0, &f1, &f2, &f3, &f4]
            .iter()
            .map(|f| ApngFrameInput {
                pixels: f,
                delay_num: 1,
                delay_den: 10,
            })
            .collect();

        let write_meta = PngWriteMetadata::from_metadata(None);
        let encoded = encode_apng_truecolor(
            &frames,
            w,
            h,
            &write_meta,
            0,
            10,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let decoded = crate::decode::decode_apng(
            &encoded,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();

        assert_eq!(decoded.frames.len(), 5);
        for (i, (orig, dec)) in frames.iter().zip(decoded.frames.iter()).enumerate() {
            let dec_buf: Vec<rgb::Rgba<u8>> = dec.pixels.to_rgba8().as_imgref().buf().to_vec();
            for (j, px) in dec_buf.iter().enumerate() {
                let off = j * 4;
                let orig_px = rgb::Rgba {
                    r: orig.pixels[off],
                    g: orig.pixels[off + 1],
                    b: orig.pixels[off + 2],
                    a: orig.pixels[off + 3],
                };
                assert_eq!(*px, orig_px, "pixel {j} mismatch frame {i}");
            }
        }
    }

    #[test]
    fn optimizer_semi_transparent_roundtrip() {
        // Tests animation with semi-transparent pixels (0 < alpha < 255).
        // These cannot use BLEND_OP_OVER for changed pixels, testing
        // the can_use_over_truecolor safety check.
        let w = 8u32;
        let h = 8u32;
        let npx = (w * h) as usize;

        // Frame 0: semi-transparent red
        let mut f0 = vec![0u8; npx * 4];
        for px in f0.as_chunks_mut::<4>().0.iter_mut() {
            px.copy_from_slice(&[255, 0, 0, 128]);
        }

        // Frame 1: semi-transparent green (different everywhere)
        let mut f1 = vec![0u8; npx * 4];
        for px in f1.as_chunks_mut::<4>().0.iter_mut() {
            px.copy_from_slice(&[0, 255, 0, 200]);
        }

        // Frame 2: mix of semi-transparent values
        let mut f2 = vec![0u8; npx * 4];
        for (i, px) in f2.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let alpha = (50 + (i * 3) % 200) as u8;
            px.copy_from_slice(&[128, 128, 0, alpha]);
        }

        let frames = [
            ApngFrameInput {
                pixels: &f0,
                delay_num: 1,
                delay_den: 10,
            },
            ApngFrameInput {
                pixels: &f1,
                delay_num: 1,
                delay_den: 10,
            },
            ApngFrameInput {
                pixels: &f2,
                delay_num: 1,
                delay_den: 10,
            },
        ];

        let write_meta = PngWriteMetadata::from_metadata(None);
        let encoded = encode_apng_truecolor(
            &frames,
            w,
            h,
            &write_meta,
            0,
            10,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let decoded = crate::decode::decode_apng(
            &encoded,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();

        assert_eq!(decoded.frames.len(), 3);
        for (i, (orig, dec)) in frames.iter().zip(decoded.frames.iter()).enumerate() {
            let dec_buf: Vec<rgb::Rgba<u8>> = dec.pixels.to_rgba8().as_imgref().buf().to_vec();
            for (j, px) in dec_buf.iter().enumerate() {
                let off = j * 4;
                let orig_px = rgb::Rgba {
                    r: orig.pixels[off],
                    g: orig.pixels[off + 1],
                    b: orig.pixels[off + 2],
                    a: orig.pixels[off + 3],
                };
                assert_eq!(*px, orig_px, "pixel {j} mismatch frame {i}");
            }
        }
    }

    // ── APNG opaque RGB downconversion test ───────────────────────

    #[test]
    fn opaque_apng_rgb_roundtrip() {
        let w = 8u32;
        let h = 8u32;
        let npx = (w * h) as usize;

        // Frame 0: all red, fully opaque
        let mut f0 = vec![0u8; npx * 4];
        for px in f0.as_chunks_mut::<4>().0.iter_mut() {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }

        // Frame 1: all blue, fully opaque
        let mut f1 = vec![0u8; npx * 4];
        for px in f1.as_chunks_mut::<4>().0.iter_mut() {
            px.copy_from_slice(&[0, 0, 255, 255]);
        }

        // Frame 2: mixed colors with some black, fully opaque
        let mut f2 = vec![0u8; npx * 4];
        for (i, px) in f2.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let r = ((i * 7) % 256) as u8;
            let g = ((i * 13) % 256) as u8;
            let b = ((i * 31) % 256) as u8;
            px.copy_from_slice(&[r, g, b, 255]);
        }

        let frames = [
            ApngFrameInput {
                pixels: &f0,
                delay_num: 100,
                delay_den: 1000,
            },
            ApngFrameInput {
                pixels: &f1,
                delay_num: 100,
                delay_den: 1000,
            },
            ApngFrameInput {
                pixels: &f2,
                delay_num: 100,
                delay_den: 1000,
            },
        ];

        let write_meta = PngWriteMetadata::from_metadata(None);
        let encoded = encode_apng_truecolor(
            &frames,
            w,
            h,
            &write_meta,
            0,
            6,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        // Verify IHDR has color_type=2 (RGB) since all frames are opaque
        // IHDR is at offset 8 (signature) + 8 (chunk header), color_type at +9
        assert_eq!(encoded[8 + 8 + 9], 2, "expected RGB color_type in IHDR");

        // Decode and verify pixel-exact roundtrip
        let decoded = crate::decode::decode_apng(
            &encoded,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();

        assert_eq!(decoded.frames.len(), 3);

        let all_source = [&f0, &f1, &f2];
        for (i, (frame, &src)) in decoded.frames.iter().zip(all_source.iter()).enumerate() {
            let decoded_rgba = frame.pixels.to_rgba8();
            let img = decoded_rgba.as_imgref();
            for (j, px) in img.buf().iter().enumerate() {
                let off = j * 4;
                let orig = rgb::Rgba {
                    r: src[off],
                    g: src[off + 1],
                    b: src[off + 2],
                    a: src[off + 3],
                };
                assert_eq!(*px, orig, "pixel {j} mismatch frame {i}");
            }
        }
    }

    #[test]
    fn semi_transparent_apng_stays_rgba() {
        let w = 4u32;
        let h = 4u32;
        let npx = (w * h) as usize;

        // Frame 0: opaque red
        let mut f0 = vec![0u8; npx * 4];
        for px in f0.as_chunks_mut::<4>().0.iter_mut() {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }

        // Frame 1: semi-transparent green
        let mut f1 = vec![0u8; npx * 4];
        for px in f1.as_chunks_mut::<4>().0.iter_mut() {
            px.copy_from_slice(&[0, 255, 0, 128]); // alpha=128
        }

        let frames = [
            ApngFrameInput {
                pixels: &f0,
                delay_num: 100,
                delay_den: 1000,
            },
            ApngFrameInput {
                pixels: &f1,
                delay_num: 100,
                delay_den: 1000,
            },
        ];

        let write_meta = PngWriteMetadata::from_metadata(None);
        let encoded = encode_apng_truecolor(
            &frames,
            w,
            h,
            &write_meta,
            0,
            6,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        // Verify IHDR has color_type=6 (RGBA) since frame 1 has semi-transparency
        assert_eq!(encoded[8 + 8 + 9], 6, "expected RGBA color_type in IHDR");
    }

    // ── Corpus test ─────────────────────────────────────────────────

    #[test]
    fn corpus_apng_roundtrip() {
        let corpus_base = std::env::var("CORPUS_BUILDER_OUTPUT_DIR")
            .unwrap_or_else(|_| "/mnt/v/output/corpus-builder".to_string());
        let corpus_dir = std::path::PathBuf::from(&corpus_base).join("apng");
        let corpus_dir = corpus_dir.as_path();
        let entries: Vec<_> = match std::fs::read_dir(corpus_dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|ext| ext == "png" || ext == "apng")
                })
                .collect(),
            Err(_) => {
                eprintln!("Skipping APNG corpus roundtrip: {corpus_dir:?} not available");
                return;
            }
        };

        if entries.is_empty() {
            eprintln!("Skipping APNG corpus roundtrip: no files found");
            return;
        }

        for entry in entries {
            let path = entry.path();
            let data = std::fs::read(&path).unwrap();

            // Decode original
            let original = match crate::decode::decode_apng(
                &data,
                &crate::decode::PngDecodeConfig::none(),
                &enough::Unstoppable,
            ) {
                Ok(o) => o,
                Err(_) => continue, // skip files we can't decode
            };

            if original.frames.is_empty() {
                continue;
            }

            // Build ApngFrameInputs from decoded frames
            let frame_data: Vec<Vec<u8>> = original
                .frames
                .iter()
                .map(|f| {
                    let rgba = f.pixels.to_rgba8();
                    rgba.copy_to_contiguous_bytes()
                })
                .collect();

            let inputs: Vec<ApngFrameInput<'_>> = original
                .frames
                .iter()
                .zip(frame_data.iter())
                .map(|(f, data)| ApngFrameInput {
                    pixels: data,
                    delay_num: f.frame_info.delay_num,
                    delay_den: f.frame_info.delay_den,
                })
                .collect();

            let write_meta = PngWriteMetadata::from_metadata(None);
            let encoded = match encode_apng_truecolor(
                &inputs,
                original.info.width,
                original.info.height,
                &write_meta,
                original.num_plays,
                6,
                &enough::Unstoppable,
                &enough::Unstoppable,
            ) {
                Ok(data) => data,
                Err(e) => {
                    eprintln!("Skipping {:?}: encode error: {e}", path.file_name());
                    continue;
                }
            };

            // Re-decode
            let redecoded = crate::decode::decode_apng(
                &encoded,
                &crate::decode::PngDecodeConfig::none(),
                &enough::Unstoppable,
            )
            .unwrap();

            assert_eq!(
                redecoded.frames.len(),
                original.frames.len(),
                "frame count mismatch for {:?}",
                path.file_name()
            );

            // Compare pixel data (transparent pixels may have zeroed RGB)
            for (i, (orig, redo)) in original
                .frames
                .iter()
                .zip(redecoded.frames.iter())
                .enumerate()
            {
                let orig_buf: Vec<rgb::Rgba<u8>> =
                    orig.pixels.to_rgba8().as_imgref().buf().to_vec();
                let redo_buf: Vec<rgb::Rgba<u8>> =
                    redo.pixels.to_rgba8().as_imgref().buf().to_vec();
                for (j, (o, r)) in orig_buf.iter().zip(redo_buf.iter()).enumerate() {
                    if o.a == 0 && r.a == 0 {
                        continue; // both transparent — RGB may differ due to zeroing
                    }
                    assert_eq!(
                        o,
                        r,
                        "pixel {j} mismatch frame {i} for {:?}",
                        path.file_name()
                    );
                }
            }
        }
    }
}
