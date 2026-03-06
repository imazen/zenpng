//! APNG encoding: delta region computation, chunk writing, per-frame compression.

use alloc::vec::Vec;

use enough::Stop;

use crate::chunk::PNG_SIGNATURE;
use crate::chunk::write::write_chunk;
use crate::encode::ApngFrameInput;
use crate::encoder::metadata::{PngWriteMetadata, metadata_size_estimate, write_all_metadata};
use crate::error::PngError;

use super::CompressOptions;
use super::compress::compress_filtered;

// APNG dispose/blend operation constants
const DISPOSE_NONE: u8 = 0;
const DISPOSE_BG: u8 = 1;
const DISPOSE_PREV: u8 = 2;
const BLEND_SOURCE: u8 = 0;
const BLEND_OVER: u8 = 1;

// ── Delta region computation ────────────────────────────────────────

/// Bounding box of pixels that differ between two frames.
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
    let mut min_x = w;
    let mut max_x = 0usize;
    let mut min_y = h;
    let mut max_y = 0usize;

    for y in 0..h {
        let row_start = y * w * bpp;
        for x in 0..w {
            let off = row_start + x * bpp;
            if prev[off..off + bpp] != curr[off..off + bpp] {
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

/// Extract a rectangular subregion from a canvas-sized buffer.
fn extract_subframe(pixels: &[u8], canvas_w: u32, region: &DeltaRegion, bpp: usize) -> Vec<u8> {
    let canvas_w = canvas_w as usize;
    let rw = region.width as usize;
    let rh = region.height as usize;
    let rx = region.x as usize;
    let ry = region.y as usize;

    let mut out = Vec::with_capacity(rw * rh * bpp);
    for y in ry..ry + rh {
        let row_start = (y * canvas_w + rx) * bpp;
        out.extend_from_slice(&pixels[row_start..row_start + rw * bpp]);
    }
    out
}

// ── 6-way dispose/blend optimization ─────────────────────────────────

/// Result of per-frame dispose/blend optimization.
struct OptimizedFrame {
    dispose_op: u8,
    blend_op: u8,
    region: DeltaRegion,
    /// Pre-filtered subframe data (raw pixels, not yet PNG-compressed).
    subframe: Vec<u8>,
}

/// Trial-compress a subframe at effort 2 (Paeth + Turbo), return compressed size.
///
/// Used for comparing dispose/blend candidates cheaply. The compressed data is
/// discarded — only the size matters for the comparison.
fn trial_compress_size(
    subframe: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
    cancel: &dyn Stop,
) -> Result<usize, PngError> {
    let opts = CompressOptions {
        parallel: false,
        cancel,
        deadline: &enough::Unstoppable,
        remaining_ns: None,
    };
    let compressed = compress_filtered(subframe, row_bytes, height, bpp, 2, opts, None)?;
    Ok(compressed.len())
}

/// Check if BLEND_OP_OVER can correctly represent all changed pixels in a region.
///
/// OVER alpha-composites the subframe onto the canvas. For a changed pixel, writing
/// the target value via OVER only produces the correct result when:
/// - target alpha == 255 (OVER with opaque source replaces entirely), OR
/// - canvas alpha == 0 (OVER onto transparent canvas = direct placement)
///
/// For RGB (bpp=3): always returns false — OVER requires alpha to be useful.
fn can_use_over_truecolor(
    target: &[u8],
    canvas: &[u8],
    canvas_w: usize,
    region: &DeltaRegion,
    bpp: usize,
) -> bool {
    // OVER blend requires alpha channel
    if bpp != 4 {
        return false;
    }

    let rx = region.x as usize;
    let ry = region.y as usize;
    let rw = region.width as usize;
    let rh = region.height as usize;

    for dy in 0..rh {
        let y = ry + dy;
        for dx in 0..rw {
            let x = rx + dx;
            let off = (y * canvas_w + x) * 4;
            let target_px = &target[off..off + 4];
            let canvas_px = &canvas[off..off + 4];

            if target_px != canvas_px {
                // Changed pixel: check if OVER can reproduce it
                if target_px[3] < 255 && canvas_px[3] > 0 {
                    return false;
                }
            }
        }
    }
    true
}

/// Build an OVER subframe for truecolor RGBA8 with dirty transparency.
///
/// For BLEND_OP_OVER, unchanged pixels MUST have alpha=0 (transparent) so that
/// alpha compositing produces a no-op. Changed pixels use their actual values.
///
/// **Dirty transparency optimization:** for unchanged pixels, instead of always
/// using `[0,0,0,0]`, we copy RGB from the pixel directly above (row dy-1).
/// This creates zero residuals for the Up filter (the most common winner for
/// animation frames with static backgrounds). First-row unchanged pixels use
/// `[0,0,0,0]` since there's no row above.
///
/// Caller must verify `can_use_over_truecolor()` first.
fn build_over_subframe(
    target: &[u8],
    canvas: &[u8],
    canvas_w: usize,
    region: &DeltaRegion,
    bpp: usize,
) -> Vec<u8> {
    let rw = region.width as usize;
    let rh = region.height as usize;
    let rx = region.x as usize;
    let ry = region.y as usize;

    let row_stride = rw * bpp;
    let mut result = Vec::with_capacity(rh * row_stride);

    for dy in 0..rh {
        let y = ry + dy;
        for dx in 0..rw {
            let x = rx + dx;
            let off = (y * canvas_w + x) * bpp;

            let target_px = &target[off..off + bpp];
            let canvas_px = &canvas[off..off + bpp];

            if target_px == canvas_px {
                // Unchanged pixel: alpha must be 0 for OVER no-op.
                // Use dirty transparency: copy RGB from row above to minimize
                // Up filter residuals (3 out of 4 bytes become zero residuals).
                if dy > 0 && bpp == 4 {
                    let above_off = result.len() - row_stride;
                    let ar = result[above_off];
                    let ag = result[above_off + 1];
                    let ab = result[above_off + 2];
                    result.push(ar);
                    result.push(ag);
                    result.push(ab);
                    result.push(0); // alpha = 0
                } else {
                    result.extend_from_slice(&[0u8; 4][..bpp]);
                }
            } else {
                result.extend_from_slice(target_px);
            }
        }
    }

    result
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
fn save_region(canvas: &[u8], canvas_w: usize, region: &DeltaRegion, bpp: usize) -> Vec<u8> {
    let rx = region.x as usize;
    let ry = region.y as usize;
    let rw = region.width as usize;
    let rh = region.height as usize;

    let mut saved = Vec::with_capacity(rw * rh * bpp);
    for dy in 0..rh {
        let y = ry + dy;
        let start = (y * canvas_w + rx) * bpp;
        saved.extend_from_slice(&canvas[start..start + rw * bpp]);
    }
    saved
}

/// Copy a canvas and apply a dispose operation to the copy.
/// Returns the modified copy (lookahead canvas).
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
        for px in canvas[row_start..row_start + rw * 4].chunks_exact_mut(4) {
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
    cancel: &dyn Stop,
) -> Result<Vec<OptimizedFrame>, PngError> {
    let w = canvas_w as usize;
    let h = canvas_h as usize;
    let npx = w * h;

    let mut optimized = Vec::with_capacity(frames.len());
    let mut canvas = vec![0u8; npx * bpp]; // transparent initial canvas

    for i in 0..frames.len() {
        cancel.check()?;

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

            // Frame 0: blend is always SOURCE (OVER on transparent canvas is equivalent
            // but adds no compression benefit since all pixels are "changed")
            // Trial-compress to get size for lookahead comparison
            let row_bytes = w * bpp;
            let best_blend_size = trial_compress_size(target, row_bytes, h, bpp, cancel)?;

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
                let next_target = frame_data[1];
                let mut best_dispose = DISPOSE_NONE;
                let mut best_total = usize::MAX;

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
                for &d in dispose_ops {
                    let lookahead_canvas =
                        apply_dispose_copy(&canvas, w, &full_region, d, bpp, Some(&pre_composite));

                    let next_size = lookahead_next_frame_size(
                        &lookahead_canvas,
                        next_target,
                        canvas_w,
                        canvas_h,
                        w,
                        bpp,
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

            // Apply chosen dispose
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
                subframe: Vec::new(), // frame 0 uses frame_data directly
            });
            continue;
        }

        // Frames 1+: compute delta region
        let source_region = compute_delta_region(&canvas, target, canvas_w, canvas_h, bpp);

        if source_region.is_none() {
            // Identical frame: minimal 1×1, no optimization needed
            let (region, sub) = minimal_subframe(target, bpp);
            optimized.push(OptimizedFrame {
                dispose_op: DISPOSE_NONE,
                blend_op: BLEND_SOURCE,
                region,
                subframe: sub,
            });
            // Canvas unchanged
            continue;
        }

        let source_region = source_region.unwrap();

        // Build SOURCE subframe
        let source_sub = extract_subframe(target, canvas_w, &source_region, bpp);

        // Trial compress SOURCE
        let source_row_bytes = source_region.width as usize * bpp;
        let source_height = source_region.height as usize;
        let source_size =
            trial_compress_size(&source_sub, source_row_bytes, source_height, bpp, cancel)?;

        // Try OVER only if all changed pixels can be correctly represented
        let (best_blend, best_blend_size, best_sub) =
            if can_use_over_truecolor(target, &canvas, w, &source_region, bpp) {
                let over_sub = build_over_subframe(target, &canvas, w, &source_region, bpp);
                let over_size =
                    trial_compress_size(&over_sub, source_row_bytes, source_height, bpp, cancel)?;
                if over_size < source_size {
                    (BLEND_OVER, over_size, over_sub)
                } else {
                    (BLEND_SOURCE, source_size, source_sub)
                }
            } else {
                (BLEND_SOURCE, source_size, source_sub)
            };

        // Save pre-composite region (for DISPOSE_PREV)
        let pre_composite = save_region(&canvas, w, &source_region, bpp);

        // Composite frame onto canvas (blit target pixels into canvas)
        blit_region(&mut canvas, target, w, &source_region, bpp);
        // Zero RGB of alpha=0 pixels to match compress_filtered behavior (RGBA only)
        if bpp == 4 {
            zero_transparent_rgb_region(&mut canvas, w, &source_region);
        }

        // Choose dispose via lookahead (except last frame)
        let best_dispose = if i == frames.len() - 1 {
            DISPOSE_NONE
        } else {
            let next_target = frame_data[i + 1];
            let mut best_dispose = DISPOSE_NONE;
            let mut best_total = usize::MAX;

            let dispose_ops: &[u8] = if bpp == 4 {
                &[DISPOSE_NONE, DISPOSE_BG, DISPOSE_PREV]
            } else {
                &[DISPOSE_NONE, DISPOSE_PREV]
            };
            for &d in dispose_ops {
                let lookahead_canvas =
                    apply_dispose_copy(&canvas, w, &source_region, d, bpp, Some(&pre_composite));

                let next_size = lookahead_next_frame_size(
                    &lookahead_canvas,
                    next_target,
                    canvas_w,
                    canvas_h,
                    w,
                    bpp,
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

        // Apply chosen dispose to canvas for next iteration
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
        });
    }

    Ok(optimized)
}

/// Compute the best trial-compressed size for the next frame against a given canvas.
///
/// Evaluates both SOURCE and OVER blend modes, returns the smaller size.
fn lookahead_next_frame_size(
    canvas: &[u8],
    next_target: &[u8],
    canvas_w: u32,
    canvas_h: u32,
    w: usize,
    bpp: usize,
    cancel: &dyn Stop,
) -> Result<usize, PngError> {
    let next_region = compute_delta_region(canvas, next_target, canvas_w, canvas_h, bpp);

    if next_region.is_none() {
        // Identical: minimal frame, ~20 bytes compressed
        return Ok(20);
    }

    let next_region = next_region.unwrap();
    let row_bytes = next_region.width as usize * bpp;
    let height = next_region.height as usize;

    // SOURCE
    let next_source = extract_subframe(next_target, canvas_w, &next_region, bpp);
    let source_size = trial_compress_size(&next_source, row_bytes, height, bpp, cancel)?;

    // OVER (only if safe)
    if can_use_over_truecolor(next_target, canvas, w, &next_region, bpp) {
        let next_over = build_over_subframe(next_target, canvas, w, &next_region, bpp);
        let over_size = trial_compress_size(&next_over, row_bytes, height, bpp, cancel)?;
        Ok(source_size.min(over_size))
    } else {
        Ok(source_size)
    }
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
) -> Result<Vec<OptimizedFrame>, PngError> {
    let w = canvas_w as usize;
    let h = canvas_h as usize;
    let bpp = 1usize;
    let npx = w * h;

    // Find transparent palette entry (alpha == 0)
    let transparent_idx = palette_rgba.iter().position(|e| e[3] == 0).map(|i| i as u8);

    let mut optimized = Vec::with_capacity(frame_indices.len());
    let mut canvas = vec![0u8; npx]; // initial canvas (index 0)

    for i in 0..frame_indices.len() {
        cancel.check()?;

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
            let best_blend_size = trial_compress_size(target, row_bytes, h, bpp, cancel)?;

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
            });
            continue;
        }

        let source_region = source_region.unwrap();
        let source_sub = extract_subframe_indexed(target, canvas_w, &source_region);
        let row_bytes = source_region.width as usize;
        let height = source_region.height as usize;

        let source_size = trial_compress_size(&source_sub, row_bytes, height, bpp, cancel)?;

        // OVER only possible with a transparent palette entry AND safe pixels
        let (best_blend, best_blend_size, best_sub) = if let Some(tidx) = transparent_idx {
            if can_use_over_indexed(target, &canvas, w, &source_region, palette_rgba) {
                let over_sub =
                    build_over_subframe_indexed(target, &canvas, w, &source_region, tidx);
                let over_size = trial_compress_size(&over_sub, row_bytes, height, bpp, cancel)?;
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
) -> Result<usize, PngError> {
    let bpp = 1usize;
    let next_region = compute_delta_region_indexed(canvas, next_target, canvas_w, canvas_h);

    if next_region.is_none() {
        return Ok(20);
    }

    let next_region = next_region.unwrap();
    let row_bytes = next_region.width as usize;
    let height = next_region.height as usize;

    let next_source = extract_subframe_indexed(next_target, canvas_w, &next_region);
    let source_size = trial_compress_size(&next_source, row_bytes, height, bpp, cancel)?;

    if let Some(tidx) = transparent_idx {
        if can_use_over_indexed(next_target, canvas, w, &next_region, palette_rgba) {
            let next_over = build_over_subframe_indexed(next_target, canvas, w, &next_region, tidx);
            let over_size = trial_compress_size(&next_over, row_bytes, height, bpp, cancel)?;
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
        for chunk in frame.pixels[..expected_len].chunks_exact(4) {
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
pub(crate) fn encode_apng_truecolor(
    frames: &[ApngFrameInput<'_>],
    canvas_width: u32,
    canvas_height: u32,
    write_meta: &PngWriteMetadata<'_>,
    num_plays: u32,
    effort: u32,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<Vec<u8>, PngError> {
    let num_frames = frames.len() as u32;
    let expected_rgba = canvas_width as usize * canvas_height as usize * 4;

    // Detect all-opaque → use RGB (color_type=2, bpp=3, 25% raw savings)
    let is_opaque = all_frames_opaque(frames, expected_rgba);
    let (bpp, color_type): (usize, u8) = if is_opaque { (3, 2) } else { (4, 6) };

    // Convert frames to RGB if opaque
    let rgb_frames: Vec<Vec<u8>>;
    let frame_data: Vec<&[u8]> = if is_opaque {
        rgb_frames = frames
            .iter()
            .map(|f| crate::optimize::rgba8_to_rgb8(&f.pixels[..expected_rgba]))
            .collect();
        rgb_frames.iter().map(|v| v.as_slice()).collect()
    } else {
        frames.iter().map(|f| &f.pixels[..expected_rgba]).collect()
    };

    // Run optimizer when effort > 2 and >1 frame (otherwise trial = final, no benefit)
    let use_optimizer = effort > 2 && frames.len() > 1;
    let optimized = if use_optimizer {
        Some(optimize_apng_truecolor(
            frames,
            &frame_data,
            canvas_width,
            canvas_height,
            bpp,
            cancel,
        )?)
    } else {
        None
    };

    // Estimate output size
    let frame_size_est = canvas_width as usize * canvas_height as usize * bpp;
    let est = 8 + 25 + 20 + 38 + frame_size_est + metadata_size_estimate(write_meta);
    let mut out = Vec::with_capacity(est);
    let mut seq = SeqCounter::new();

    // PNG signature
    out.extend_from_slice(&PNG_SIGNATURE);

    // IHDR: canvas dimensions, detected color type
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&canvas_width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&canvas_height.to_be_bytes());
    ihdr[8] = 8; // bit depth
    ihdr[9] = color_type;
    write_chunk(&mut out, b"IHDR", &ihdr);

    // Metadata chunks (before IDAT)
    write_all_metadata(&mut out, write_meta)?;

    // acTL
    write_actl(&mut out, num_frames, num_plays);

    if let Some(ref opt_frames) = optimized {
        // ── Optimized path: use per-frame dispose/blend from optimizer ──
        for (i, opt) in opt_frames.iter().enumerate() {
            cancel.check()?;

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
                parallel: false,
                cancel,
                deadline,
                remaining_ns: None,
            };
            let compressed =
                compress_filtered(sub_data, sub_row_bytes, sub_height, bpp, effort, opts, None)?;

            if i == 0 {
                write_chunk(&mut out, b"IDAT", &compressed);
            } else {
                let fdat_seq = seq.next();
                write_fdat(&mut out, fdat_seq, &compressed);
            }
        }
    } else {
        // ── Unoptimized path: hardcoded NONE/SOURCE (effort ≤ 2 or single frame) ──

        // Frame 0: fcTL + IDAT (full canvas)
        let frame0 = &frames[0];
        write_fctl(
            &mut out,
            seq.next(),
            canvas_width,
            canvas_height,
            0,
            0,
            frame0.delay_num,
            frame0.delay_den,
            DISPOSE_NONE,
            BLEND_SOURCE,
        );

        let row_bytes = canvas_width as usize * bpp;
        let height = canvas_height as usize;
        let opts = CompressOptions {
            parallel: false,
            cancel,
            deadline,
            remaining_ns: None,
        };
        let compressed0 =
            compress_filtered(frame_data[0], row_bytes, height, bpp, effort, opts, None)?;
        write_chunk(&mut out, b"IDAT", &compressed0);

        // Frames 1+: fcTL + fdAT with delta regions
        for i in 1..frames.len() {
            cancel.check()?;

            let prev = frame_data[i - 1];
            let curr = frame_data[i];
            let frame = &frames[i];

            let (region, subframe) =
                match compute_delta_region(prev, curr, canvas_width, canvas_height, bpp) {
                    Some(region) => {
                        let sub = extract_subframe(curr, canvas_width, &region, bpp);
                        (region, sub)
                    }
                    None => {
                        let (region, sub) = minimal_subframe(curr, bpp);
                        (region, sub)
                    }
                };

            let fctl_seq = seq.next();
            write_fctl(
                &mut out,
                fctl_seq,
                region.width,
                region.height,
                region.x,
                region.y,
                frame.delay_num,
                frame.delay_den,
                DISPOSE_NONE,
                BLEND_SOURCE,
            );

            let sub_row_bytes = region.width as usize * bpp;
            let sub_height = region.height as usize;
            let opts = CompressOptions {
                parallel: false,
                cancel,
                deadline,
                remaining_ns: None,
            };
            let compressed = compress_filtered(
                &subframe,
                sub_row_bytes,
                sub_height,
                bpp,
                effort,
                opts,
                None,
            )?;

            let fdat_seq = seq.next();
            write_fdat(&mut out, fdat_seq, &compressed);
        }
    }

    // IEND
    write_chunk(&mut out, b"IEND", &[]);

    Ok(out)
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
) -> Result<Vec<u8>, PngError> {
    let num_frames = frames.len() as u32;
    let n_colors = palette_rgba.len();

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
            cancel.check()?;

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
                parallel: false,
                cancel,
                deadline,
                remaining_ns: None,
            };
            let compressed =
                compress_filtered(&packed, row_bytes, region_h as usize, 1, effort, opts, None)?;

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
            cancel.check()?;

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
                parallel: false,
                cancel,
                deadline,
                remaining_ns: None,
            };
            let compressed =
                compress_filtered(&packed, row_bytes, region_h as usize, 1, effort, opts, None)?;

            if i == 0 {
                write_chunk(&mut out, b"IDAT", &compressed);
            } else {
                let fdat_seq = seq.next();
                write_fdat(&mut out, fdat_seq, &compressed);
            }
        }
    }

    // IEND
    write_chunk(&mut out, b"IEND", &[]);

    Ok(out)
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use zencodec_types::PixelBufferConvertExt;

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
        for px in f0.chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }

        // Frame 1: all green
        let mut f1 = vec![0u8; npx * 4];
        for px in f1.chunks_exact_mut(4) {
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
        for px in f0.chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }

        // Frame 1: all blue
        let mut f1 = vec![0u8; npx * 4];
        for px in f1.chunks_exact_mut(4) {
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
        for px in f2.chunks_exact_mut(4) {
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
            for px in buf.chunks_exact_mut(4) {
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
        for px in f0.chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 0, 0, 128]);
        }

        // Frame 1: semi-transparent green (different everywhere)
        let mut f1 = vec![0u8; npx * 4];
        for px in f1.chunks_exact_mut(4) {
            px.copy_from_slice(&[0, 255, 0, 200]);
        }

        // Frame 2: mix of semi-transparent values
        let mut f2 = vec![0u8; npx * 4];
        for (i, px) in f2.chunks_exact_mut(4).enumerate() {
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
        for px in f0.chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }

        // Frame 1: all blue, fully opaque
        let mut f1 = vec![0u8; npx * 4];
        for px in f1.chunks_exact_mut(4) {
            px.copy_from_slice(&[0, 0, 255, 255]);
        }

        // Frame 2: mixed colors with some black, fully opaque
        let mut f2 = vec![0u8; npx * 4];
        for (i, px) in f2.chunks_exact_mut(4).enumerate() {
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
        for px in f0.chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }

        // Frame 1: semi-transparent green
        let mut f1 = vec![0u8; npx * 4];
        for px in f1.chunks_exact_mut(4) {
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
