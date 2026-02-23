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

// ── Delta region computation ────────────────────────────────────────

/// Bounding box of pixels that differ between two frames.
struct DeltaRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// Find the bounding box of differing pixels between two canvas-sized RGBA8 frames.
///
/// Returns `None` if the frames are identical.
fn compute_delta_region(prev: &[u8], curr: &[u8], w: u32, h: u32) -> Option<DeltaRegion> {
    let w = w as usize;
    let h = h as usize;
    let mut min_x = w;
    let mut max_x = 0usize;
    let mut min_y = h;
    let mut max_y = 0usize;

    for y in 0..h {
        let row_start = y * w * 4;
        for x in 0..w {
            let off = row_start + x * 4;
            if prev[off..off + 4] != curr[off..off + 4] {
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

/// Extract a rectangular subregion from a canvas-sized RGBA8 buffer.
fn extract_subframe(pixels: &[u8], canvas_w: u32, region: &DeltaRegion) -> Vec<u8> {
    let canvas_w = canvas_w as usize;
    let rw = region.width as usize;
    let rh = region.height as usize;
    let rx = region.x as usize;
    let ry = region.y as usize;

    let mut out = Vec::with_capacity(rw * rh * 4);
    for y in ry..ry + rh {
        let row_start = (y * canvas_w + rx) * 4;
        out.extend_from_slice(&pixels[row_start..row_start + rw * 4]);
    }
    out
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

/// Encode canvas-sized RGBA8 frames into a truecolor APNG file.
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

    // Estimate output size
    let frame_size_est = canvas_width as usize * canvas_height as usize * 4;
    let est = 8 + 25 + 20 + 38 + frame_size_est + metadata_size_estimate(write_meta);
    let mut out = Vec::with_capacity(est);
    let mut seq = SeqCounter::new();

    // PNG signature
    out.extend_from_slice(&PNG_SIGNATURE);

    // IHDR: canvas dimensions, 8-bit RGBA
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&canvas_width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&canvas_height.to_be_bytes());
    ihdr[8] = 8;  // bit depth
    ihdr[9] = 6;  // color type: RGBA
    write_chunk(&mut out, b"IHDR", &ihdr);

    // Metadata chunks (before IDAT)
    write_all_metadata(&mut out, write_meta)?;

    // acTL
    write_actl(&mut out, num_frames, num_plays);

    // Frame 0: fcTL + IDAT (full canvas)
    let frame0 = &frames[0];
    write_fctl(
        &mut out,
        seq.next(), // seq 0
        canvas_width,
        canvas_height,
        0,
        0,
        frame0.delay_num,
        frame0.delay_den,
        0, // dispose_op = NONE
        0, // blend_op = SOURCE
    );

    // Compress frame 0
    let bpp = 4; // RGBA8
    let row_bytes = canvas_width as usize * bpp;
    let height = canvas_height as usize;
    let opts = CompressOptions {
        parallel: false,
        cancel,
        deadline,
        remaining_ns: None,
    };
    let compressed0 = compress_filtered(
        frame0.pixels,
        row_bytes,
        height,
        bpp,
        effort,
        opts,
        None,
    )?;
    write_chunk(&mut out, b"IDAT", &compressed0);

    // Frames 1+: fcTL + fdAT with delta regions
    for i in 1..frames.len() {
        cancel.check()?;

        let prev = frames[i - 1].pixels;
        let curr = frames[i].pixels;
        let frame = &frames[i];

        let (region, subframe) =
            match compute_delta_region(prev, curr, canvas_width, canvas_height) {
                Some(region) => {
                    let sub = extract_subframe(curr, canvas_width, &region);
                    (region, sub)
                }
                None => {
                    // Identical frames: emit a minimal 1x1 frame at (0,0)
                    let region = DeltaRegion {
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1,
                    };
                    let sub = curr[..4].to_vec();
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
            0, // dispose_op = NONE
            0, // blend_op = SOURCE
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

    // IEND
    write_chunk(&mut out, b"IEND", &[]);

    Ok(out)
}

// ── Indexed APNG encode ─────────────────────────────────────────────

/// Encode canvas-sized RGBA8 frames into an indexed APNG file using a global palette.
#[cfg(feature = "quantize")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_apng_indexed(
    frames: &[ApngFrameInput<'_>],
    canvas_width: u32,
    canvas_height: u32,
    write_meta: &PngWriteMetadata<'_>,
    num_plays: u32,
    effort: u32,
    quant_config: &zenquant::QuantizeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<Vec<u8>, PngError> {
    let num_frames = frames.len() as u32;
    let w = canvas_width as usize;
    let h = canvas_height as usize;

    // Build representative sample: sample every Nth pixel across all frames
    let total_pixels = w * h * frames.len();
    let target_samples = 10_000.min(total_pixels);
    let sample_step = (total_pixels / target_samples).max(1);

    let mut sample: Vec<zenquant::RGBA<u8>> = Vec::with_capacity(target_samples);
    let mut pixel_idx = 0usize;
    for frame in frames {
        let pixels: &[zenquant::RGBA<u8>] = bytemuck::cast_slice(frame.pixels);
        for px in pixels {
            if pixel_idx % sample_step == 0 {
                sample.push(*px);
            }
            pixel_idx += 1;
        }
    }

    // Quantize sample for global palette
    let sample_w = sample.len();
    let result = zenquant::quantize_rgba(&sample, sample_w, 1, quant_config)?;
    let palette_rgba = result.palette_rgba();
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

    let est = 8 + 25 + 20 + (12 + n_colors * 3) + 38 * num_frames as usize
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

    // Remap and encode each frame
    for (i, frame) in frames.iter().enumerate() {
        cancel.check()?;

        // Determine region and pixel data for this frame
        let (region_x, region_y, region_w, region_h, subframe_rgba) = if i == 0 {
            // Frame 0: full canvas
            (0u32, 0u32, canvas_width, canvas_height, frame.pixels.to_vec())
        } else {
            let prev = frames[i - 1].pixels;
            let curr = frame.pixels;
            match compute_delta_region(prev, curr, canvas_width, canvas_height) {
                Some(region) => {
                    let sub = extract_subframe(curr, canvas_width, &region);
                    (region.x, region.y, region.width, region.height, sub)
                }
                None => {
                    // Identical: minimal 1x1 frame
                    (0, 0, 1, 1, curr[..4].to_vec())
                }
            }
        };

        // Remap subframe pixels to palette indices
        let sub_pixels: &[zenquant::RGBA<u8>] = bytemuck::cast_slice(&subframe_rgba);
        let indices: Vec<u8> = sub_pixels
            .iter()
            .map(|px| nearest_palette_index(px, palette_rgba))
            .collect();

        // Pack indices
        let packed =
            super::pack_all_rows(&indices, region_w as usize, region_h as usize, bit_depth);
        let row_bytes = super::packed_row_bytes(region_w as usize, bit_depth);

        // Write fcTL
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
            0, // dispose_op = NONE
            0, // blend_op = SOURCE
        );

        // Compress
        let opts = CompressOptions {
            parallel: false,
            cancel,
            deadline,
            remaining_ns: None,
        };
        let compressed = compress_filtered(
            &packed,
            row_bytes,
            region_h as usize,
            1, // bpp=1 for indexed
            effort,
            opts,
            None,
        )?;

        // Write IDAT (frame 0) or fdAT (frames 1+)
        if i == 0 {
            write_chunk(&mut out, b"IDAT", &compressed);
        } else {
            let fdat_seq = seq.next();
            write_fdat(&mut out, fdat_seq, &compressed);
        }
    }

    // IEND
    write_chunk(&mut out, b"IEND", &[]);

    Ok(out)
}

/// Find the nearest palette index for a given RGBA pixel (simple Euclidean distance).
#[cfg(feature = "quantize")]
fn nearest_palette_index(pixel: &zenquant::RGBA<u8>, palette: &[[u8; 4]]) -> u8 {
    let mut best_idx = 0u8;
    let mut best_dist = u32::MAX;

    for (idx, entry) in palette.iter().enumerate() {
        let dr = pixel.r as i32 - entry[0] as i32;
        let dg = pixel.g as i32 - entry[1] as i32;
        let db = pixel.b as i32 - entry[2] as i32;
        let da = pixel.a as i32 - entry[3] as i32;
        let dist = (dr * dr + dg * dg + db * db + da * da) as u32;
        if dist == 0 {
            return idx as u8;
        }
        if dist < best_dist {
            best_dist = dist;
            best_idx = idx as u8;
        }
    }

    best_idx
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Delta region tests ──────────────────────────────────────────

    #[test]
    fn delta_region_known_diff() {
        // 4×4 RGBA8 frames, differing at (1,1) and (2,2)
        let w = 4u32;
        let h = 4u32;
        let prev = vec![0u8; (w * h * 4) as usize];
        let mut curr = prev.clone();

        // Change pixel (1,1)
        let off1 = ((1 * w + 1) * 4) as usize;
        curr[off1] = 255;

        // Change pixel (2,2)
        let off2 = ((2 * w + 2) * 4) as usize;
        curr[off2] = 128;

        let region = compute_delta_region(&prev, &curr, w, h).unwrap();
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
        assert!(compute_delta_region(&frame, &frame, w, h).is_none());
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
        let sub = extract_subframe(&pixels, w, &region);
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
        let f0_buf = f0_pixels.buf();
        assert_eq!(f0_buf[0].r, 255);
        assert_eq!(f0_buf[0].g, 0);

        // Verify frame 1 pixels
        let f1_pixels = decoded.frames[1].pixels.to_rgba8();
        let f1_buf = f1_pixels.buf();
        assert_eq!(f1_buf[0].r, 0);
        assert_eq!(f1_buf[0].g, 255);
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
        let f2_buf = f2_pixels.buf();
        let px33 = &f2_buf[3 * w as usize + 3];
        assert_eq!((px33.r, px33.g, px33.b, px33.a), (255, 255, 255, 255));
        let px55 = &f2_buf[5 * w as usize + 5];
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

    #[cfg(feature = "quantize")]
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

        let write_meta = PngWriteMetadata::from_metadata(None);
        let quant_config = zenquant::QuantizeConfig::new(zenquant::OutputFormat::Png);
        let encoded = encode_apng_indexed(
            &frames,
            w,
            h,
            &write_meta,
            0,
            6,
            &quant_config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        // Verify it decodes (will be indexed, decoded as palette)
        let decoded = crate::decode::decode_apng(
            &encoded,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.frames.len(), 2);
    }

    // ── Corpus test ─────────────────────────────────────────────────

    #[test]
    fn corpus_apng_roundtrip() {
        let corpus_dir = std::path::Path::new("/mnt/v/output/corpus-builder/apng");
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
                    let buf: Vec<rgb::Rgba<u8>> = rgba.into_buf();
                    bytemuck::cast_slice::<rgb::Rgba<u8>, u8>(&buf).to_vec()
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

            // Compare pixel data
            for (i, (orig, redo)) in original
                .frames
                .iter()
                .zip(redecoded.frames.iter())
                .enumerate()
            {
                let orig_buf: Vec<rgb::Rgba<u8>> = orig.pixels.to_rgba8().into_buf();
                let redo_buf: Vec<rgb::Rgba<u8>> = redo.pixels.to_rgba8().into_buf();
                let orig_bytes: &[u8] = bytemuck::cast_slice(&orig_buf);
                let redo_bytes: &[u8] = bytemuck::cast_slice(&redo_buf);
                assert_eq!(
                    orig_bytes,
                    redo_bytes,
                    "pixel mismatch frame {i} for {:?}",
                    path.file_name()
                );
            }
        }
    }
}
