//! APNG frame-by-frame decoding and compositing.

use alloc::vec;
use alloc::vec::Vec;

use enough::Stop;
use zencodec_types::PixelData;

use crate::chunk::PNG_SIGNATURE;
use crate::chunk::ancillary::{FrameControl, PngAncillary};
use crate::chunk::ihdr::Ihdr;
use crate::decode::{PngDecodeConfig, PngWarning};
use crate::error::PngError;

use super::postprocess::{OutputFormat, build_pixel_data, post_process_row};
use super::row::{FdatSource, IdatSource, unfilter_row};

// ── Raw frame output ────────────────────────────────────────────────

/// A single decoded APNG subframe (raw pixels, not composited to canvas).
pub(crate) struct RawFrame {
    pub pixels: PixelData,
    pub fctl: FrameControl,
}

// ── Chunk scanning helpers ──────────────────────────────────────────

/// Read a chunk header at `pos`, returning (length, chunk_type, data_start, crc_end).
/// Returns None if there's not enough data.
fn read_chunk_header(data: &[u8], pos: usize) -> Option<(usize, [u8; 4], usize, usize)> {
    if pos + 12 > data.len() {
        return None;
    }
    let length = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
    let chunk_type: [u8; 4] = data[pos + 4..pos + 8].try_into().unwrap();
    let data_start = pos + 8;
    let crc_end = data_start + length + 4;
    if crc_end > data.len() {
        return None;
    }
    Some((length, chunk_type, data_start, crc_end))
}

// ── ApngDecoder ─────────────────────────────────────────────────────

/// Stateful APNG frame decoder. Yields one raw subframe per `next_frame()` call.
pub(crate) struct ApngDecoder<'a> {
    file_data: &'a [u8],
    ihdr: Ihdr,
    ancillary: PngAncillary,
    config: PngDecodeConfig,
    pub num_frames: u32,
    pub num_plays: u32,
    current_frame: u32,
    /// Scan position in file (byte offset of next chunk to examine).
    chunk_pos: usize,
    /// Whether the default image (IDAT) is frame 0 (fcTL before IDAT).
    default_image_is_frame: bool,
    /// Byte offset of the first IDAT chunk header.
    first_idat_pos: usize,
    /// fcTL for frame 0 (stored when found before IDAT).
    frame0_fctl: Option<FrameControl>,
}

impl<'a> ApngDecoder<'a> {
    /// Create a new APNG decoder from PNG file bytes.
    pub fn new(data: &'a [u8], config: &PngDecodeConfig) -> Result<Self, PngError> {
        if data.len() < 8 || data[..8] != PNG_SIGNATURE {
            return Err(PngError::Decode("not a PNG file".into()));
        }

        // Parse IHDR
        let (_, ihdr_type, ihdr_data_start, ihdr_crc_end) = read_chunk_header(data, 8)
            .ok_or_else(|| PngError::Decode("truncated IHDR chunk".into()))?;
        if ihdr_type != *b"IHDR" {
            return Err(PngError::Decode("first chunk is not IHDR".into()));
        }
        let ihdr_length = u32::from_be_bytes(data[8..12].try_into().unwrap()) as usize;
        let ihdr = Ihdr::parse(&data[ihdr_data_start..ihdr_data_start + ihdr_length])?;

        // Scan pre-IDAT chunks: collect ancillary, find acTL, look for fcTL before IDAT
        let mut ancillary = PngAncillary::default();
        let mut pos = ihdr_crc_end;
        let mut first_idat_pos = None;
        let mut frame0_fctl = None;
        let mut default_image_is_frame = false;

        loop {
            let Some((length, chunk_type, data_start, crc_end)) = read_chunk_header(data, pos)
            else {
                break;
            };

            match &chunk_type {
                b"IDAT" => {
                    first_idat_pos = Some(pos);
                    // Resume scanning from after IDAT chunks
                    let mut scan = crc_end;
                    loop {
                        let Some((_, ct, _, ce)) = read_chunk_header(data, scan) else {
                            break;
                        };
                        if ct != *b"IDAT" {
                            break;
                        }
                        scan = ce;
                    }
                    pos = scan;
                    break;
                }
                b"fcTL" => {
                    // fcTL before IDAT means the default image is frame 0
                    let fctl = FrameControl::parse(
                        &data[data_start..data_start + length],
                        ihdr.width,
                        ihdr.height,
                    )?;
                    frame0_fctl = Some(fctl);
                    default_image_is_frame = true;
                    pos = crc_end;
                }
                b"acTL" => {
                    // Already handled by ancillary.collect, but we need to call it
                    let chunk = crate::chunk::ChunkRef {
                        chunk_type,
                        data: &data[data_start..data_start + length],
                    };
                    ancillary.collect(&chunk)?;
                    pos = crc_end;
                }
                _ => {
                    let chunk = crate::chunk::ChunkRef {
                        chunk_type,
                        data: &data[data_start..data_start + length],
                    };
                    ancillary.collect(&chunk)?;
                    pos = crc_end;
                }
            }
        }

        let first_idat_pos =
            first_idat_pos.ok_or_else(|| PngError::Decode("no IDAT chunk found".into()))?;

        let (num_frames, num_plays) = ancillary
            .actl
            .ok_or_else(|| PngError::Decode("APNG: no acTL chunk found".into()))?;

        // Validate palette for indexed images
        if ihdr.is_indexed() && ancillary.palette.is_none() {
            return Err(PngError::Decode(
                "indexed color type requires PLTE chunk".into(),
            ));
        }

        Ok(Self {
            file_data: data,
            ihdr,
            ancillary,
            config: config.clone(),
            num_frames,
            num_plays,
            current_frame: 0,
            chunk_pos: pos, // positioned right after IDAT chunks
            default_image_is_frame,
            first_idat_pos,
            frame0_fctl,
        })
    }

    /// Decode the next frame. Returns `None` when all frames have been yielded.
    pub fn next_frame(&mut self, cancel: &dyn Stop) -> Result<Option<RawFrame>, PngError> {
        if self.current_frame >= self.num_frames {
            return Ok(None);
        }

        let frame_idx = self.current_frame;
        self.current_frame += 1;

        if frame_idx == 0 && self.default_image_is_frame {
            // Frame 0 uses IDAT data
            let fctl = self
                .frame0_fctl
                .ok_or_else(|| PngError::Decode("APNG: frame 0 missing fcTL".into()))?;
            let pixels = self.decode_idat_frame(&fctl, cancel)?;
            return Ok(Some(RawFrame { pixels, fctl }));
        }

        if frame_idx == 0 && !self.default_image_is_frame {
            // Default image is NOT part of the animation.
            // We still need to skip it and find the first fcTL+fdAT.
            // (This is rare but spec-valid)
        }

        // Frames 1+ (or frame 0 when default image is not a frame):
        // Scan for the next fcTL + fdAT sequence
        let (fctl, fdat_pos) = self.find_next_fctl_fdat()?;
        self.chunk_pos = fdat_pos; // will be advanced by decode_fdat_frame
        let pixels = self.decode_fdat_frame(&fctl, cancel)?;
        Ok(Some(RawFrame { pixels, fctl }))
    }

    /// Decode frame 0 from IDAT chunks.
    fn decode_idat_frame(
        &self,
        fctl: &FrameControl,
        cancel: &dyn Stop,
    ) -> Result<PixelData, PngError> {
        // For frame 0, the IDAT data covers the full canvas (IHDR dimensions).
        // The fcTL for frame 0 must have the same dimensions as IHDR.
        let frame_ihdr = Ihdr {
            width: fctl.width,
            height: fctl.height,
            bit_depth: self.ihdr.bit_depth,
            color_type: self.ihdr.color_type,
            interlace: 0,
        };

        let stride = frame_ihdr.stride();
        let raw_row_bytes = frame_ihdr.raw_row_bytes();
        let bpp = frame_ihdr.filter_bpp();

        let source = IdatSource::new(
            self.file_data,
            self.first_idat_pos,
            self.config.skip_critical_chunk_crc,
        );
        let mut decompressor = zenflate::StreamDecompressor::zlib(source, stride * 2)
            .with_skip_checksum(self.config.skip_decompression_checksum);

        let fmt = OutputFormat::from_ihdr(&frame_ihdr, &self.ancillary);
        let w = fctl.width as usize;
        let h = fctl.height as usize;
        let pixel_bytes = fmt.channels * fmt.bytes_per_channel;
        let out_row_bytes = w * pixel_bytes;

        let mut all_pixels = Vec::with_capacity(out_row_bytes * h);
        let mut prev_row = vec![0u8; raw_row_bytes];
        let mut current_row = vec![0u8; raw_row_bytes];
        let mut row_buf = Vec::new();

        for _y in 0..h {
            cancel.check()?;
            // Fill until we have a stride
            loop {
                let available = decompressor.peek().len();
                if available >= stride {
                    break;
                }
                if decompressor.is_done() {
                    return Err(PngError::Decode("APNG: truncated IDAT data".into()));
                }
                decompressor.fill().map_err(|e| {
                    PngError::Decode(alloc::format!("APNG IDAT decompression error: {e:?}"))
                })?;
            }

            let peeked = decompressor.peek();
            let filter_byte = peeked[0];
            current_row[..raw_row_bytes].copy_from_slice(&peeked[1..stride]);
            decompressor.advance(stride);

            unfilter_row(
                filter_byte,
                &mut current_row[..raw_row_bytes],
                &prev_row,
                bpp,
            )?;
            post_process_row(
                &current_row[..raw_row_bytes],
                &frame_ihdr,
                &self.ancillary,
                &mut row_buf,
            );
            all_pixels.extend_from_slice(&row_buf);

            core::mem::swap(&mut current_row, &mut prev_row);
        }

        build_pixel_data(&frame_ihdr, &self.ancillary, all_pixels, w, h)
    }

    /// Scan from `self.chunk_pos` to find the next fcTL followed by fdAT.
    /// Returns the FrameControl and the byte offset of the first fdAT chunk.
    fn find_next_fctl_fdat(&mut self) -> Result<(FrameControl, usize), PngError> {
        let data = self.file_data;
        let mut pos = self.chunk_pos;

        loop {
            let (length, chunk_type, data_start, crc_end) = read_chunk_header(data, pos)
                .ok_or_else(|| {
                    PngError::Decode("APNG: unexpected end of file scanning for fcTL".into())
                })?;

            if chunk_type == *b"IEND" {
                return Err(PngError::Decode(
                    "APNG: reached IEND before finding expected fcTL".into(),
                ));
            }

            if chunk_type == *b"fcTL" {
                let fctl = FrameControl::parse(
                    &data[data_start..data_start + length],
                    self.ihdr.width,
                    self.ihdr.height,
                )?;
                // The next chunk(s) should be fdAT
                let fdat_pos = crc_end;
                self.chunk_pos = crc_end;
                return Ok((fctl, fdat_pos));
            }

            pos = crc_end;
        }
    }

    /// Decode a frame from fdAT chunks starting at `self.chunk_pos`.
    fn decode_fdat_frame(
        &mut self,
        fctl: &FrameControl,
        cancel: &dyn Stop,
    ) -> Result<PixelData, PngError> {
        let frame_ihdr = Ihdr {
            width: fctl.width,
            height: fctl.height,
            bit_depth: self.ihdr.bit_depth,
            color_type: self.ihdr.color_type,
            interlace: 0,
        };

        let stride = frame_ihdr.stride();
        let raw_row_bytes = frame_ihdr.raw_row_bytes();
        let bpp = frame_ihdr.filter_bpp();

        let fdat_pos = self.chunk_pos;
        let source = FdatSource::new(
            self.file_data,
            fdat_pos,
            self.config.skip_critical_chunk_crc,
        );
        let mut decompressor = zenflate::StreamDecompressor::zlib(source, stride * 2)
            .with_skip_checksum(self.config.skip_decompression_checksum);

        let fmt = OutputFormat::from_ihdr(&frame_ihdr, &self.ancillary);
        let w = fctl.width as usize;
        let h = fctl.height as usize;
        let pixel_bytes = fmt.channels * fmt.bytes_per_channel;
        let out_row_bytes = w * pixel_bytes;

        let mut all_pixels = Vec::with_capacity(out_row_bytes * h);
        let mut prev_row = vec![0u8; raw_row_bytes];
        let mut current_row = vec![0u8; raw_row_bytes];
        let mut row_buf = Vec::new();

        for _y in 0..h {
            cancel.check()?;
            loop {
                let available = decompressor.peek().len();
                if available >= stride {
                    break;
                }
                if decompressor.is_done() {
                    return Err(PngError::Decode("APNG: truncated fdAT data".into()));
                }
                decompressor.fill().map_err(|e| {
                    PngError::Decode(alloc::format!("APNG fdAT decompression error: {e:?}"))
                })?;
            }

            let peeked = decompressor.peek();
            let filter_byte = peeked[0];
            current_row[..raw_row_bytes].copy_from_slice(&peeked[1..stride]);
            decompressor.advance(stride);

            unfilter_row(
                filter_byte,
                &mut current_row[..raw_row_bytes],
                &prev_row,
                bpp,
            )?;
            post_process_row(
                &current_row[..raw_row_bytes],
                &frame_ihdr,
                &self.ancillary,
                &mut row_buf,
            );
            all_pixels.extend_from_slice(&row_buf);

            core::mem::swap(&mut current_row, &mut prev_row);
        }

        // Advance chunk_pos past the fdAT chunks we consumed
        self.chunk_pos = decompressor.source_ref().post_fdat_pos;

        build_pixel_data(&frame_ihdr, &self.ancillary, all_pixels, w, h)
    }

    /// Get the IHDR info.
    pub fn ihdr(&self) -> &Ihdr {
        &self.ihdr
    }

    /// Get the ancillary metadata.
    pub fn ancillary(&self) -> &PngAncillary {
        &self.ancillary
    }
}

// ── Compositing ─────────────────────────────────────────────────────

/// A single composed APNG frame (canvas-sized, RGBA8 pixels).
pub(crate) struct ComposedFrame {
    /// RGBA8 canvas pixels.
    pub pixels: Vec<u8>,
    /// Frame control metadata.
    pub fctl: FrameControl,
}

/// Result of composited APNG decoding.
pub(crate) struct ComposedApng {
    pub frames: Vec<ComposedFrame>,
    pub ihdr: Ihdr,
    pub ancillary: PngAncillary,
    pub num_plays: u32,
    pub warnings: Vec<PngWarning>,
}

/// Decode an APNG with full compositing, producing canvas-sized RGBA8 frames.
///
/// For non-animated PNGs, falls through to regular decode and returns a single frame.
pub(crate) fn decode_apng_composed(
    data: &[u8],
    config: &PngDecodeConfig,
    cancel: &dyn Stop,
) -> Result<ComposedApng, PngError> {
    let mut decoder = ApngDecoder::new(data, config)?;
    let canvas_w = decoder.ihdr().width as usize;
    let canvas_h = decoder.ihdr().height as usize;
    // Determine if this is a 16-bit source
    let is_16bit = decoder.ihdr().bit_depth == 16;
    let bytes_per_canvas_pixel = if is_16bit { 8 } else { 4 }; // RGBA16 vs RGBA8
    let canvas_bytes = canvas_w * canvas_h * bytes_per_canvas_pixel;

    let num_frames = decoder.num_frames;
    let num_plays = decoder.num_plays;

    // Canvas starts as transparent black
    let mut canvas = vec![0u8; canvas_bytes];
    let mut composed_frames = Vec::with_capacity(num_frames as usize);

    // For RestorePrevious: saved canvas state
    let mut saved_canvas: Option<Vec<u8>> = None;

    // Previous frame's fctl (for applying dispose_op after yielding)
    let mut prev_fctl: Option<FrameControl> = None;

    while let Some(frame) = decoder.next_frame(cancel)? {
        // Apply dispose_op from the PREVIOUS frame before compositing this one
        if let Some(ref pfctl) = prev_fctl {
            apply_dispose_op(pfctl, &mut canvas, &saved_canvas, canvas_w, is_16bit);
        }

        // If this frame's dispose_op is RestorePrevious, save the canvas BEFORE compositing
        if frame.fctl.dispose_op == 2 {
            saved_canvas = Some(canvas.clone());
        }

        // Promote subframe pixels to RGBA8 (or RGBA16) and composite onto canvas
        let subframe_rgba = promote_to_rgba(&frame.pixels, is_16bit);
        composite_frame(&frame.fctl, &subframe_rgba, &mut canvas, canvas_w, is_16bit);

        // Clone the composited canvas as this frame's output
        composed_frames.push(ComposedFrame {
            pixels: canvas.clone(),
            fctl: frame.fctl,
        });

        prev_fctl = Some(frame.fctl);
    }

    let ihdr = *decoder.ihdr();
    let ancillary = decoder.ancillary().clone();
    let warnings = Vec::new(); // TODO: collect warnings from decoder

    Ok(ComposedApng {
        frames: composed_frames,
        ihdr,
        ancillary,
        num_plays,
        warnings,
    })
}

/// Apply dispose_op to the canvas based on the previous frame's fctl.
fn apply_dispose_op(
    fctl: &FrameControl,
    canvas: &mut [u8],
    saved: &Option<Vec<u8>>,
    canvas_w: usize,
    is_16bit: bool,
) {
    let bpp = if is_16bit { 8 } else { 4 };

    match fctl.dispose_op {
        0 => {} // NONE: leave canvas as-is
        1 => {
            // BACKGROUND: fill the frame region with transparent black
            let x = fctl.x_offset as usize;
            let y = fctl.y_offset as usize;
            let w = fctl.width as usize;
            let h = fctl.height as usize;
            let row_stride = canvas_w * bpp;

            for row in y..y + h {
                let start = row * row_stride + x * bpp;
                let end = start + w * bpp;
                canvas[start..end].fill(0);
            }
        }
        2 => {
            // PREVIOUS: restore saved canvas
            if let Some(saved) = saved {
                canvas.copy_from_slice(saved);
            }
        }
        _ => {} // invalid, treat as NONE
    }
}

/// Promote PixelData to RGBA8 or RGBA16 bytes.
fn promote_to_rgba(pixels: &PixelData, is_16bit: bool) -> Vec<u8> {
    if is_16bit {
        // Promote to RGBA16 (8 bytes per pixel, native endian)
        match pixels {
            PixelData::Rgba16(img) => {
                let mut out = Vec::with_capacity(img.buf().len() * 8);
                for p in img.buf() {
                    out.extend_from_slice(&p.r.to_ne_bytes());
                    out.extend_from_slice(&p.g.to_ne_bytes());
                    out.extend_from_slice(&p.b.to_ne_bytes());
                    out.extend_from_slice(&p.a.to_ne_bytes());
                }
                out
            }
            PixelData::Rgb16(img) => {
                let mut out = Vec::with_capacity(img.buf().len() * 8);
                for p in img.buf() {
                    out.extend_from_slice(&p.r.to_ne_bytes());
                    out.extend_from_slice(&p.g.to_ne_bytes());
                    out.extend_from_slice(&p.b.to_ne_bytes());
                    out.extend_from_slice(&65535u16.to_ne_bytes());
                }
                out
            }
            PixelData::Gray16(img) => {
                let mut out = Vec::with_capacity(img.buf().len() * 8);
                for p in img.buf() {
                    let v = p.value();
                    out.extend_from_slice(&v.to_ne_bytes());
                    out.extend_from_slice(&v.to_ne_bytes());
                    out.extend_from_slice(&v.to_ne_bytes());
                    out.extend_from_slice(&65535u16.to_ne_bytes());
                }
                out
            }
            PixelData::GrayAlpha16(img) => {
                let mut out = Vec::with_capacity(img.buf().len() * 8);
                for p in img.buf() {
                    out.extend_from_slice(&p.v.to_ne_bytes());
                    out.extend_from_slice(&p.v.to_ne_bytes());
                    out.extend_from_slice(&p.v.to_ne_bytes());
                    out.extend_from_slice(&p.a.to_ne_bytes());
                }
                out
            }
            // 8-bit sources upscaled to 16-bit
            other => {
                let rgba8 = promote_to_rgba(other, false);
                let mut out = Vec::with_capacity(rgba8.len() * 2);
                for chunk in rgba8.chunks_exact(4) {
                    for &b in chunk {
                        let v16 = b as u16 * 257;
                        out.extend_from_slice(&v16.to_ne_bytes());
                    }
                }
                out
            }
        }
    } else {
        // Promote to RGBA8 (4 bytes per pixel)
        match pixels {
            PixelData::Rgba8(img) => {
                use rgb::ComponentBytes;
                img.buf().as_bytes().to_vec()
            }
            PixelData::Rgb8(img) => {
                let mut out = Vec::with_capacity(img.buf().len() * 4);
                for p in img.buf() {
                    out.extend_from_slice(&[p.r, p.g, p.b, 255]);
                }
                out
            }
            PixelData::Gray8(img) => {
                let mut out = Vec::with_capacity(img.buf().len() * 4);
                for p in img.buf() {
                    let v = p.value();
                    out.extend_from_slice(&[v, v, v, 255]);
                }
                out
            }
            // 16-bit sources downscaled to 8-bit
            PixelData::Rgba16(img) => {
                let mut out = Vec::with_capacity(img.buf().len() * 4);
                for p in img.buf() {
                    out.extend_from_slice(&[
                        (p.r >> 8) as u8,
                        (p.g >> 8) as u8,
                        (p.b >> 8) as u8,
                        (p.a >> 8) as u8,
                    ]);
                }
                out
            }
            PixelData::Rgb16(img) => {
                let mut out = Vec::with_capacity(img.buf().len() * 4);
                for p in img.buf() {
                    out.extend_from_slice(&[
                        (p.r >> 8) as u8,
                        (p.g >> 8) as u8,
                        (p.b >> 8) as u8,
                        255,
                    ]);
                }
                out
            }
            PixelData::Gray16(img) => {
                let mut out = Vec::with_capacity(img.buf().len() * 4);
                for p in img.buf() {
                    let v = (p.value() >> 8) as u8;
                    out.extend_from_slice(&[v, v, v, 255]);
                }
                out
            }
            PixelData::GrayAlpha16(img) => {
                let mut out = Vec::with_capacity(img.buf().len() * 4);
                for p in img.buf() {
                    let v = (p.v >> 8) as u8;
                    let a = (p.a >> 8) as u8;
                    out.extend_from_slice(&[v, v, v, a]);
                }
                out
            }
            _ => Vec::new(),
        }
    }
}

/// Composite subframe onto canvas at the given offset with the given blend mode.
fn composite_frame(
    fctl: &FrameControl,
    subframe_rgba: &[u8],
    canvas: &mut [u8],
    canvas_w: usize,
    is_16bit: bool,
) {
    let bpp = if is_16bit { 8 } else { 4 };
    let x = fctl.x_offset as usize;
    let y = fctl.y_offset as usize;
    let w = fctl.width as usize;
    let h = fctl.height as usize;
    let canvas_row_stride = canvas_w * bpp;
    let sub_row_stride = w * bpp;

    for row in 0..h {
        let canvas_row_start = (y + row) * canvas_row_stride + x * bpp;
        let sub_row_start = row * sub_row_stride;

        if fctl.blend_op == 0 {
            // SOURCE: overwrite directly
            canvas[canvas_row_start..canvas_row_start + sub_row_stride]
                .copy_from_slice(&subframe_rgba[sub_row_start..sub_row_start + sub_row_stride]);
        } else {
            // OVER: alpha composite
            if is_16bit {
                blend_over_row_16(
                    &mut canvas[canvas_row_start..canvas_row_start + sub_row_stride],
                    &subframe_rgba[sub_row_start..sub_row_start + sub_row_stride],
                );
            } else {
                blend_over_row_8(
                    &mut canvas[canvas_row_start..canvas_row_start + sub_row_stride],
                    &subframe_rgba[sub_row_start..sub_row_start + sub_row_stride],
                );
            }
        }
    }
}

/// Per-pixel alpha composite for RGBA8.
/// out_c = fg_c * fg_a / 255 + bg_c * bg_a * (255 - fg_a) / (255*255)
/// Simplified APNG spec formula: out = fg + bg * (255 - fg_a) / 255
fn blend_over_row_8(dst: &mut [u8], src: &[u8]) {
    for (dst_px, src_px) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let fg_a = src_px[3] as u32;
        if fg_a == 255 {
            dst_px.copy_from_slice(src_px);
        } else if fg_a == 0 {
            // Fully transparent foreground — leave background unchanged
        } else {
            let bg_a = dst_px[3] as u32;
            let inv_fg_a = 255 - fg_a;

            // APNG spec over operation:
            // out_a = fg_a + bg_a * (255 - fg_a) / 255
            let out_a = fg_a + bg_a * inv_fg_a / 255;
            if out_a == 0 {
                dst_px.fill(0);
            } else {
                // out_c = (fg_c * fg_a + bg_c * bg_a * (255 - fg_a) / 255) / out_a
                for i in 0..3 {
                    let fg_c = src_px[i] as u32;
                    let bg_c = dst_px[i] as u32;
                    let num = fg_c * fg_a + bg_c * bg_a * inv_fg_a / 255;
                    dst_px[i] = (num / out_a).min(255) as u8;
                }
                dst_px[3] = out_a.min(255) as u8;
            }
        }
    }
}

/// Per-pixel alpha composite for RGBA16 (native endian).
fn blend_over_row_16(dst: &mut [u8], src: &[u8]) {
    for (dst_px, src_px) in dst.chunks_exact_mut(8).zip(src.chunks_exact(8)) {
        let fg_a = u16::from_ne_bytes([src_px[6], src_px[7]]) as u64;
        if fg_a == 65535 {
            dst_px.copy_from_slice(src_px);
        } else if fg_a == 0 {
            // Fully transparent — leave background unchanged
        } else {
            let bg_a = u16::from_ne_bytes([dst_px[6], dst_px[7]]) as u64;
            let inv_fg_a = 65535 - fg_a;
            let out_a = fg_a + bg_a * inv_fg_a / 65535;
            if out_a == 0 {
                dst_px.fill(0);
            } else {
                for i in 0..3 {
                    let off = i * 2;
                    let fg_c = u16::from_ne_bytes([src_px[off], src_px[off + 1]]) as u64;
                    let bg_c = u16::from_ne_bytes([dst_px[off], dst_px[off + 1]]) as u64;
                    let num = fg_c * fg_a + bg_c * bg_a * inv_fg_a / 65535;
                    let val = (num / out_a).min(65535) as u16;
                    dst_px[off..off + 2].copy_from_slice(&val.to_ne_bytes());
                }
                let a_val = out_a.min(65535) as u16;
                dst_px[6..8].copy_from_slice(&a_val.to_ne_bytes());
            }
        }
    }
}
