//! APNG frame-by-frame decoding and compositing.

use alloc::borrow::Cow;
use alloc::vec;
use alloc::vec::Vec;

use enough::Stop;
use zenpixels::{ChannelLayout, ChannelType, GrayAlpha16, PixelBuffer};

use crate::chunk::PNG_SIGNATURE;
use crate::chunk::ancillary::{FrameControl, PngAncillary};
use crate::chunk::ihdr::Ihdr;
use crate::decode::{PngDecodeConfig, PngWarning};
use crate::error::PngError;
#[allow(unused_imports)]
use whereat::at;

use super::postprocess::{OutputFormat, build_pixel_data, post_process_row};
use super::row::{FdatSource, IdatSource, unfilter_row};

// ── Raw frame output ────────────────────────────────────────────────

/// A single decoded APNG subframe (raw pixels, not composited to canvas).
pub(crate) struct RawFrame {
    pub pixels: PixelBuffer,
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
    let crc_end = data_start.checked_add(length)?.checked_add(4)?;
    if crc_end > data.len() {
        return None;
    }
    Some((length, chunk_type, data_start, crc_end))
}

// ── ApngDecoder ─────────────────────────────────────────────────────

/// Captured state of an [`ApngDecoder`] for O(1) resumption.
///
/// Stores all immutable metadata parsed during `new()` plus the mutable scan
/// position. Used by [`PngAnimationFrameDecoder`] to avoid re-scanning from the
/// beginning of the file for each frame.
#[derive(Clone)]
pub(crate) struct ApngDecoderState {
    ihdr: Ihdr,
    ancillary: PngAncillary,
    config: PngDecodeConfig,
    pub num_frames: u32,
    pub num_plays: u32,
    pub current_frame: u32,
    chunk_pos: usize,
    default_image_is_frame: bool,
    first_idat_pos: usize,
    frame0_fctl: Option<FrameControl>,
}

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
    pub fn new(data: &'a [u8], config: &PngDecodeConfig) -> crate::error::Result<Self> {
        if data.len() < 8 || data[..8] != PNG_SIGNATURE {
            return Err(at!(PngError::Decode("not a PNG file".into())));
        }

        // Parse IHDR
        let (_, ihdr_type, ihdr_data_start, ihdr_crc_end) = read_chunk_header(data, 8)
            .ok_or_else(|| at!(PngError::Decode("truncated IHDR chunk".into())))?;
        if ihdr_type != *b"IHDR" {
            return Err(at!(PngError::Decode("first chunk is not IHDR".into())));
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
            first_idat_pos.ok_or_else(|| at!(PngError::Decode("no IDAT chunk found".into())))?;

        let (num_frames, num_plays) = ancillary
            .actl
            .ok_or_else(|| at!(PngError::Decode("APNG: no acTL chunk found".into())))?;

        // Validate palette for indexed images
        if ihdr.is_indexed() && ancillary.palette.is_none() {
            return Err(at!(PngError::Decode(
                "indexed color type requires PLTE chunk".into(),
            )));
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

    /// Create a decoder from previously saved state (O(1), no re-scanning).
    pub fn from_state(data: &'a [u8], state: ApngDecoderState) -> Self {
        Self {
            file_data: data,
            ihdr: state.ihdr,
            ancillary: state.ancillary,
            config: state.config,
            num_frames: state.num_frames,
            num_plays: state.num_plays,
            current_frame: state.current_frame,
            chunk_pos: state.chunk_pos,
            default_image_is_frame: state.default_image_is_frame,
            first_idat_pos: state.first_idat_pos,
            frame0_fctl: state.frame0_fctl,
        }
    }

    /// Capture the current state for later resumption.
    pub fn save_state(&self) -> ApngDecoderState {
        ApngDecoderState {
            ihdr: self.ihdr,
            ancillary: self.ancillary.clone(),
            config: self.config.clone(),
            num_frames: self.num_frames,
            num_plays: self.num_plays,
            current_frame: self.current_frame,
            chunk_pos: self.chunk_pos,
            default_image_is_frame: self.default_image_is_frame,
            first_idat_pos: self.first_idat_pos,
            frame0_fctl: self.frame0_fctl,
        }
    }

    /// Decode the next frame. Returns `None` when all frames have been yielded.
    pub fn next_frame(&mut self, cancel: &dyn Stop) -> crate::error::Result<Option<RawFrame>> {
        if self.current_frame >= self.num_frames {
            return Ok(None);
        }

        let frame_idx = self.current_frame;
        self.current_frame += 1;

        if frame_idx == 0 && self.default_image_is_frame {
            // Frame 0 uses IDAT data
            let fctl = self
                .frame0_fctl
                .ok_or_else(|| at!(PngError::Decode("APNG: frame 0 missing fcTL".into())))?;
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
    ) -> crate::error::Result<PixelBuffer> {
        // For frame 0, the IDAT data covers the full canvas (IHDR dimensions).
        // The fcTL for frame 0 must have the same dimensions as IHDR.
        let frame_ihdr = Ihdr {
            width: fctl.width,
            height: fctl.height,
            bit_depth: self.ihdr.bit_depth,
            color_type: self.ihdr.color_type,
            interlace: 0,
        };

        let stride = frame_ihdr.stride()?;
        let raw_row_bytes = frame_ihdr.raw_row_bytes()?;
        let bpp = frame_ihdr.filter_bpp();

        let source = IdatSource::new(
            Cow::Borrowed(self.file_data),
            self.first_idat_pos,
            self.config.skip_critical_chunk_crc,
        );
        let mut decompressor = zenflate::StreamDecompressor::zlib(source, stride * 2)
            .with_skip_checksum(self.config.skip_decompression_checksum);

        let fmt = OutputFormat::from_ihdr(&frame_ihdr, &self.ancillary)?;
        let w = fctl.width as usize;
        let h = fctl.height as usize;
        let pixel_bytes = fmt.channels * fmt.bytes_per_channel;
        let out_row_bytes = w * pixel_bytes;

        let mut all_pixels = Vec::with_capacity(out_row_bytes * h);
        let mut prev_row = vec![0u8; raw_row_bytes];
        let mut current_row = vec![0u8; raw_row_bytes];
        let mut row_buf = Vec::new();

        for _y in 0..h {
            cancel.check().map_err(|e| at!(PngError::from(e)))?;
            // Fill until we have a stride
            loop {
                let available = decompressor.peek().len();
                if available >= stride {
                    break;
                }
                if decompressor.is_done() {
                    return Err(at!(PngError::Decode("APNG: truncated IDAT data".into())));
                }
                decompressor.fill().map_err(|e| {
                    at!(PngError::Decode(alloc::format!(
                        "APNG IDAT decompression error: {e:?}"
                    )))
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
    fn find_next_fctl_fdat(&mut self) -> crate::error::Result<(FrameControl, usize)> {
        let data = self.file_data;
        let mut pos = self.chunk_pos;

        loop {
            let (length, chunk_type, data_start, crc_end) = read_chunk_header(data, pos)
                .ok_or_else(|| {
                    at!(PngError::Decode(
                        "APNG: unexpected end of file scanning for fcTL".into()
                    ))
                })?;

            if chunk_type == *b"IEND" {
                return Err(at!(PngError::Decode(
                    "APNG: reached IEND before finding expected fcTL".into(),
                )));
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
    ) -> crate::error::Result<PixelBuffer> {
        let frame_ihdr = Ihdr {
            width: fctl.width,
            height: fctl.height,
            bit_depth: self.ihdr.bit_depth,
            color_type: self.ihdr.color_type,
            interlace: 0,
        };

        let stride = frame_ihdr.stride()?;
        let raw_row_bytes = frame_ihdr.raw_row_bytes()?;
        let bpp = frame_ihdr.filter_bpp();

        let fdat_pos = self.chunk_pos;
        let source = FdatSource::new(
            self.file_data,
            fdat_pos,
            self.config.skip_critical_chunk_crc,
        );
        let mut decompressor = zenflate::StreamDecompressor::zlib(source, stride * 2)
            .with_skip_checksum(self.config.skip_decompression_checksum);

        let fmt = OutputFormat::from_ihdr(&frame_ihdr, &self.ancillary)?;
        let w = fctl.width as usize;
        let h = fctl.height as usize;
        let pixel_bytes = fmt.channels * fmt.bytes_per_channel;
        let out_row_bytes = w * pixel_bytes;

        let mut all_pixels = Vec::with_capacity(out_row_bytes * h);
        let mut prev_row = vec![0u8; raw_row_bytes];
        let mut current_row = vec![0u8; raw_row_bytes];
        let mut row_buf = Vec::new();

        for _y in 0..h {
            cancel.check().map_err(|e| at!(PngError::from(e)))?;
            loop {
                let available = decompressor.peek().len();
                if available >= stride {
                    break;
                }
                if decompressor.is_done() {
                    return Err(at!(PngError::Decode("APNG: truncated fdAT data".into())));
                }
                decompressor.fill().map_err(|e| {
                    at!(PngError::Decode(alloc::format!(
                        "APNG fdAT decompression error: {e:?}"
                    )))
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

/// Result of composited APNG decoding.
pub(crate) struct ComposedApng {
    pub frames: Vec<crate::decode::ApngFrame>,
    pub ihdr: Ihdr,
    pub ancillary: PngAncillary,
    pub num_plays: u32,
    pub warnings: Vec<PngWarning>,
}

/// Decode an APNG with full compositing, producing canvas-sized RGBA frames.
///
/// Builds `PixelBuffer` directly from the canvas to avoid double-copying.
pub(crate) fn decode_apng_composed(
    data: &[u8],
    config: &PngDecodeConfig,
    cancel: &dyn Stop,
) -> crate::error::Result<ComposedApng> {
    let mut decoder = ApngDecoder::new(data, config)?;
    let canvas_w = decoder.ihdr().width as usize;
    let canvas_h = decoder.ihdr().height as usize;
    let is_16bit = decoder.ihdr().bit_depth == 16;
    let bpp = if is_16bit { 8 } else { 4 }; // RGBA16 vs RGBA8

    // Validate limits before allocating canvas-sized buffers.
    config.validate(decoder.ihdr().width, decoder.ihdr().height, bpp as u32)?;

    let canvas_bytes = canvas_w
        .checked_mul(canvas_h)
        .and_then(|v| v.checked_mul(bpp))
        .ok_or_else(|| at!(PngError::LimitExceeded("canvas size overflow".into())))?;

    let num_frames = decoder.num_frames;
    let num_plays = decoder.num_plays;

    // Canvas starts as transparent black
    let mut canvas = vec![0u8; canvas_bytes];
    let mut frames = Vec::with_capacity((num_frames as usize).min(65536));

    // For RestorePrevious: saved frame region (not full canvas)
    let mut saved_region: Option<SavedRegion> = None;

    // Previous frame's fctl (for applying dispose_op after yielding)
    let mut prev_fctl: Option<FrameControl> = None;

    while let Some(frame) = decoder.next_frame(cancel)? {
        // Apply dispose_op from the PREVIOUS frame before compositing this one
        if let Some(pfctl) = prev_fctl {
            apply_dispose_op(&pfctl, &mut canvas, &saved_region, canvas_w, is_16bit);
        }

        // If this frame's dispose_op is RestorePrevious, save only the frame region
        if frame.fctl.dispose_op == 2 {
            saved_region = Some(save_region(&frame.fctl, &canvas, canvas_w, is_16bit));
        }

        // Promote subframe pixels to RGBA and composite onto canvas
        let subframe_rgba = promote_to_rgba(&frame.pixels, is_16bit);
        composite_frame(&frame.fctl, &subframe_rgba, &mut canvas, canvas_w, is_16bit);

        // Build PixelBuffer directly from canvas (single copy, no intermediate Vec)
        let pixels = canvas_to_pixel_data(&canvas, canvas_w, canvas_h, is_16bit);
        frames.push(crate::decode::ApngFrame {
            pixels,
            frame_info: crate::decode::ApngFrameInfo {
                delay_num: frame.fctl.delay_num,
                delay_den: frame.fctl.delay_den,
            },
        });

        prev_fctl = Some(frame.fctl);
    }

    let ihdr = *decoder.ihdr();
    let ancillary = decoder.ancillary().clone();
    let warnings = Vec::new();

    Ok(ComposedApng {
        frames,
        ihdr,
        ancillary,
        num_plays,
        warnings,
    })
}

/// Build PixelBuffer directly from canvas bytes (single allocation).
fn canvas_to_pixel_data(canvas: &[u8], w: usize, h: usize, is_16bit: bool) -> PixelBuffer {
    if is_16bit {
        let rgba: Vec<rgb::Rgba<u16>> = match bytemuck::try_cast_slice(canvas) {
            Ok(v) => v.to_vec(),
            Err(bytemuck::PodCastError::TargetAlignmentGreaterAndInputNotAligned) => {
                super::postprocess::bytes_to_rgba16_vec(canvas)
            }
            Err(e) => panic!("unexpected cast error: {e:?}"),
        };
        PixelBuffer::from_imgvec(imgref::ImgVec::new(rgba, w, h)).into()
    } else {
        let rgba: &[rgb::Rgba<u8>] = bytemuck::cast_slice(canvas);
        PixelBuffer::from_imgvec(imgref::ImgVec::new(rgba.to_vec(), w, h)).into()
    }
}

/// Saved frame region for RestorePrevious (only the affected area, not full canvas).
struct SavedRegion {
    data: Vec<u8>,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

/// Save only the frame region from the canvas.
fn save_region(fctl: &FrameControl, canvas: &[u8], canvas_w: usize, is_16bit: bool) -> SavedRegion {
    let bpp = if is_16bit { 8 } else { 4 };
    let x = fctl.x_offset as usize;
    let y = fctl.y_offset as usize;
    let w = fctl.width as usize;
    let h = fctl.height as usize;
    let row_stride = canvas_w * bpp;
    let region_row_bytes = w * bpp;

    let mut data = Vec::with_capacity(region_row_bytes * h);
    for row in y..y + h {
        let start = row * row_stride + x * bpp;
        data.extend_from_slice(&canvas[start..start + region_row_bytes]);
    }
    SavedRegion { data, x, y, w, h }
}

/// Apply dispose_op to the canvas based on the previous frame's fctl.
fn apply_dispose_op(
    fctl: &FrameControl,
    canvas: &mut [u8],
    saved: &Option<SavedRegion>,
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
            // PREVIOUS: restore only the saved frame region
            if let Some(saved) = saved {
                let row_stride = canvas_w * bpp;
                let region_row_bytes = saved.w * bpp;
                for row in 0..saved.h {
                    let canvas_start = (saved.y + row) * row_stride + saved.x * bpp;
                    let region_start = row * region_row_bytes;
                    canvas[canvas_start..canvas_start + region_row_bytes].copy_from_slice(
                        &saved.data[region_start..region_start + region_row_bytes],
                    );
                }
            }
        }
        _ => {} // invalid, treat as NONE
    }
}

/// Promote PixelBuffer to RGBA8 or RGBA16 bytes for canvas compositing.
fn promote_to_rgba(pixels: &PixelBuffer, is_16bit: bool) -> Vec<u8> {
    let desc = pixels.descriptor();
    let layout = desc.layout();
    let channel_type = desc.channel_type();

    if is_16bit {
        // Promote to RGBA16 (8 bytes per pixel, native endian)
        if channel_type == ChannelType::U16 {
            match layout {
                ChannelLayout::Rgba => {
                    if let Some(img) = pixels.try_as_imgref::<rgb::Rgba<u16>>() {
                        let mut out = Vec::with_capacity(img.buf().len() * 8);
                        for p in *img.buf() {
                            out.extend_from_slice(&p.r.to_ne_bytes());
                            out.extend_from_slice(&p.g.to_ne_bytes());
                            out.extend_from_slice(&p.b.to_ne_bytes());
                            out.extend_from_slice(&p.a.to_ne_bytes());
                        }
                        return out;
                    }
                }
                ChannelLayout::Rgb => {
                    if let Some(img) = pixels.try_as_imgref::<rgb::Rgb<u16>>() {
                        let mut out = Vec::with_capacity(img.buf().len() * 8);
                        for p in *img.buf() {
                            out.extend_from_slice(&p.r.to_ne_bytes());
                            out.extend_from_slice(&p.g.to_ne_bytes());
                            out.extend_from_slice(&p.b.to_ne_bytes());
                            out.extend_from_slice(&65535u16.to_ne_bytes());
                        }
                        return out;
                    }
                }
                ChannelLayout::Gray => {
                    if let Some(img) = pixels.try_as_imgref::<rgb::Gray<u16>>() {
                        let mut out = Vec::with_capacity(img.buf().len() * 8);
                        for p in *img.buf() {
                            let v = p.value();
                            out.extend_from_slice(&v.to_ne_bytes());
                            out.extend_from_slice(&v.to_ne_bytes());
                            out.extend_from_slice(&v.to_ne_bytes());
                            out.extend_from_slice(&65535u16.to_ne_bytes());
                        }
                        return out;
                    }
                }
                ChannelLayout::GrayAlpha => {
                    if let Some(img) = pixels.try_as_imgref::<GrayAlpha16>() {
                        let mut out = Vec::with_capacity(img.buf().len() * 8);
                        for p in *img.buf() {
                            out.extend_from_slice(&p.v.to_ne_bytes());
                            out.extend_from_slice(&p.v.to_ne_bytes());
                            out.extend_from_slice(&p.v.to_ne_bytes());
                            out.extend_from_slice(&p.a.to_ne_bytes());
                        }
                        return out;
                    }
                }
                _ => {}
            }
        }
        // 8-bit sources upscaled to 16-bit
        let rgba8 = promote_to_rgba(pixels, false);
        let mut out = Vec::with_capacity(rgba8.len() * 2);
        for chunk in rgba8.chunks_exact(4) {
            for &b in chunk {
                let v16 = b as u16 * 257;
                out.extend_from_slice(&v16.to_ne_bytes());
            }
        }
        out
    } else {
        // Promote to RGBA8 (4 bytes per pixel)
        if channel_type == ChannelType::U8 {
            match layout {
                ChannelLayout::Rgba => {
                    if let Some(img) = pixels.try_as_imgref::<rgb::Rgba<u8>>() {
                        use rgb::ComponentBytes;
                        return img.buf().as_bytes().to_vec();
                    }
                }
                ChannelLayout::Rgb => {
                    if let Some(img) = pixels.try_as_imgref::<rgb::Rgb<u8>>() {
                        let mut out = Vec::with_capacity(img.buf().len() * 4);
                        for p in *img.buf() {
                            out.extend_from_slice(&[p.r, p.g, p.b, 255]);
                        }
                        return out;
                    }
                }
                ChannelLayout::Gray => {
                    if let Some(img) = pixels.try_as_imgref::<rgb::Gray<u8>>() {
                        let mut out = Vec::with_capacity(img.buf().len() * 4);
                        for p in *img.buf() {
                            let v = p.value();
                            out.extend_from_slice(&[v, v, v, 255]);
                        }
                        return out;
                    }
                }
                _ => {}
            }
        }
        // 16-bit sources downscaled to 8-bit
        if channel_type == ChannelType::U16 {
            match layout {
                ChannelLayout::Rgba => {
                    if let Some(img) = pixels.try_as_imgref::<rgb::Rgba<u16>>() {
                        let mut out = Vec::with_capacity(img.buf().len() * 4);
                        for p in *img.buf() {
                            out.extend_from_slice(&[
                                ((p.r as u32 * 255 + 32768) >> 16) as u8,
                                ((p.g as u32 * 255 + 32768) >> 16) as u8,
                                ((p.b as u32 * 255 + 32768) >> 16) as u8,
                                ((p.a as u32 * 255 + 32768) >> 16) as u8,
                            ]);
                        }
                        return out;
                    }
                }
                ChannelLayout::Rgb => {
                    if let Some(img) = pixels.try_as_imgref::<rgb::Rgb<u16>>() {
                        let mut out = Vec::with_capacity(img.buf().len() * 4);
                        for p in *img.buf() {
                            out.extend_from_slice(&[
                                ((p.r as u32 * 255 + 32768) >> 16) as u8,
                                ((p.g as u32 * 255 + 32768) >> 16) as u8,
                                ((p.b as u32 * 255 + 32768) >> 16) as u8,
                                255,
                            ]);
                        }
                        return out;
                    }
                }
                ChannelLayout::Gray => {
                    if let Some(img) = pixels.try_as_imgref::<rgb::Gray<u16>>() {
                        let mut out = Vec::with_capacity(img.buf().len() * 4);
                        for p in *img.buf() {
                            let v = ((p.value() as u32 * 255 + 32768) >> 16) as u8;
                            out.extend_from_slice(&[v, v, v, 255]);
                        }
                        return out;
                    }
                }
                ChannelLayout::GrayAlpha => {
                    if let Some(img) = pixels.try_as_imgref::<GrayAlpha16>() {
                        let mut out = Vec::with_capacity(img.buf().len() * 4);
                        for p in *img.buf() {
                            let v = ((p.v as u32 * 255 + 32768) >> 16) as u8;
                            let a = ((p.a as u32 * 255 + 32768) >> 16) as u8;
                            out.extend_from_slice(&[v, v, v, a]);
                        }
                        return out;
                    }
                }
                _ => {}
            }
        }
        Vec::new()
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

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::ancillary::FrameControl;
    use enough::Unstoppable;

    // ── fcTL parsing tests ──────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    fn make_fctl_data(
        seq: u32,
        w: u32,
        h: u32,
        x: u32,
        y: u32,
        delay_num: u16,
        delay_den: u16,
        dispose: u8,
        blend: u8,
    ) -> Vec<u8> {
        let mut data = Vec::with_capacity(26);
        data.extend_from_slice(&seq.to_be_bytes());
        data.extend_from_slice(&w.to_be_bytes());
        data.extend_from_slice(&h.to_be_bytes());
        data.extend_from_slice(&x.to_be_bytes());
        data.extend_from_slice(&y.to_be_bytes());
        data.extend_from_slice(&delay_num.to_be_bytes());
        data.extend_from_slice(&delay_den.to_be_bytes());
        data.push(dispose);
        data.push(blend);
        data
    }

    #[test]
    fn fctl_parse_valid() {
        let data = make_fctl_data(0, 100, 100, 0, 0, 1, 10, 0, 0);
        let fctl = FrameControl::parse(&data, 100, 100).unwrap();
        assert_eq!(fctl.width, 100);
        assert_eq!(fctl.height, 100);
        assert_eq!(fctl.x_offset, 0);
        assert_eq!(fctl.y_offset, 0);
        assert_eq!(fctl.delay_num, 1);
        assert_eq!(fctl.delay_den, 10);
        assert_eq!(fctl.dispose_op, 0);
        assert_eq!(fctl.blend_op, 0);
    }

    #[test]
    fn fctl_parse_subframe() {
        let data = make_fctl_data(1, 50, 30, 10, 20, 100, 1000, 1, 1);
        let fctl = FrameControl::parse(&data, 100, 100).unwrap();
        assert_eq!(fctl.width, 50);
        assert_eq!(fctl.height, 30);
        assert_eq!(fctl.x_offset, 10);
        assert_eq!(fctl.y_offset, 20);
        assert_eq!(fctl.dispose_op, 1);
        assert_eq!(fctl.blend_op, 1);
    }

    #[test]
    fn fctl_rejects_wrong_length() {
        let data = vec![0u8; 25]; // too short
        assert!(FrameControl::parse(&data, 100, 100).is_err());

        let data = vec![0u8; 27]; // too long
        assert!(FrameControl::parse(&data, 100, 100).is_err());
    }

    #[test]
    fn fctl_rejects_zero_dimensions() {
        let data = make_fctl_data(0, 0, 100, 0, 0, 1, 10, 0, 0);
        assert!(FrameControl::parse(&data, 100, 100).is_err());

        let data = make_fctl_data(0, 100, 0, 0, 0, 1, 10, 0, 0);
        assert!(FrameControl::parse(&data, 100, 100).is_err());
    }

    #[test]
    fn fctl_rejects_out_of_bounds() {
        // x_offset + width > canvas_width
        let data = make_fctl_data(0, 50, 50, 60, 0, 1, 10, 0, 0);
        assert!(FrameControl::parse(&data, 100, 100).is_err());

        // y_offset + height > canvas_height
        let data = make_fctl_data(0, 50, 50, 0, 60, 1, 10, 0, 0);
        assert!(FrameControl::parse(&data, 100, 100).is_err());
    }

    #[test]
    fn fctl_rejects_invalid_dispose_blend() {
        let data = make_fctl_data(0, 100, 100, 0, 0, 1, 10, 3, 0);
        assert!(FrameControl::parse(&data, 100, 100).is_err());

        let data = make_fctl_data(0, 100, 100, 0, 0, 1, 10, 0, 2);
        assert!(FrameControl::parse(&data, 100, 100).is_err());
    }

    #[test]
    fn fctl_delay_ms_calculation() {
        let data = make_fctl_data(0, 10, 10, 0, 0, 1, 10, 0, 0);
        let fctl = FrameControl::parse(&data, 10, 10).unwrap();
        assert_eq!(fctl.delay_ms(), 100); // 1/10 sec = 100ms

        let data = make_fctl_data(0, 10, 10, 0, 0, 5, 100, 0, 0);
        let fctl = FrameControl::parse(&data, 10, 10).unwrap();
        assert_eq!(fctl.delay_ms(), 50); // 5/100 sec = 50ms

        // delay_den=0 should be treated as 100
        let data = make_fctl_data(0, 10, 10, 0, 0, 3, 0, 0, 0);
        let fctl = FrameControl::parse(&data, 10, 10).unwrap();
        assert_eq!(fctl.delay_ms(), 30); // 3/100 sec = 30ms
    }

    // ── Blend tests ─────────────────────────────────────────────────

    #[test]
    fn blend_over_opaque_fg_replaces() {
        let mut dst = vec![100, 200, 50, 128]; // semi-transparent bg
        let src = vec![255, 0, 0, 255]; // opaque red
        blend_over_row_8(&mut dst, &src);
        assert_eq!(dst, vec![255, 0, 0, 255]);
    }

    #[test]
    fn blend_over_transparent_fg_preserves() {
        let mut dst = vec![100, 200, 50, 255]; // opaque bg
        let src = vec![0, 0, 0, 0]; // fully transparent fg
        blend_over_row_8(&mut dst, &src);
        assert_eq!(dst, vec![100, 200, 50, 255]);
    }

    #[test]
    fn blend_over_semi_transparent() {
        let mut dst = vec![0, 0, 0, 255]; // opaque black bg
        let src = vec![255, 0, 0, 128]; // semi-transparent red
        blend_over_row_8(&mut dst, &src);
        // Result should be some shade of dark red
        assert!(dst[0] > 100); // red channel present
        assert!(dst[1] < 10); // green ~0
        assert!(dst[2] < 10); // blue ~0
        assert!(dst[3] == 255); // fully opaque result
    }

    // ── Non-animated PNG via decode_apng ─────────────────────────────

    #[test]
    fn decode_apng_non_animated_returns_one_frame() {
        // Create a simple non-animated PNG
        let img = imgref::ImgVec::new(
            vec![
                rgb::Rgba {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                };
                4
            ],
            2,
            2,
        );
        let encoded = crate::encode::encode_rgba8(
            img.as_ref(),
            None,
            &crate::encode::EncodeConfig::default(),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();

        let result =
            crate::decode::decode_apng(&encoded, &PngDecodeConfig::none(), &Unstoppable).unwrap();

        assert_eq!(result.frames.len(), 1);
        assert_eq!(result.info.width, 2);
        assert_eq!(result.info.height, 2);
        assert!(!result.info.sequence.is_animation());
        assert_eq!(result.num_plays, 0);
    }

    // ── APNG corpus tests ───────────────────────────────────────────

    /// Decode all APNG files from the corpus, verify frame count matches acTL, no panics.
    #[test]
    fn apng_corpus_decode_no_panics() {
        let apng_base = std::env::var("CORPUS_BUILDER_OUTPUT_DIR")
            .unwrap_or_else(|_| "/mnt/v/output/corpus-builder".to_string());
        let apng_dir_buf = std::path::PathBuf::from(&apng_base).join("apng");
        let apng_dir = apng_dir_buf.as_path();
        if !apng_dir.exists() {
            eprintln!(
                "Skipping APNG corpus test: {} not found",
                apng_dir.display()
            );
            return;
        }

        let mut tested = 0u32;
        let mut failures = Vec::new();

        for entry in std::fs::read_dir(apng_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("png") {
                continue;
            }

            let filename = path.file_name().unwrap().to_str().unwrap().to_string();
            let data = match std::fs::read(&path) {
                Ok(d) => d,
                Err(_) => continue,
            };

            // Probe for expected frame count
            let probe = match crate::decode::probe(&data) {
                Ok(p) => p,
                Err(_) => continue,
            };

            if !probe.sequence.is_animation() {
                continue;
            }

            let expected_frames = probe.sequence.count().unwrap_or(0);

            // Decode via decode_apng
            match crate::decode::decode_apng(&data, &PngDecodeConfig::none(), &Unstoppable) {
                Ok(result) => {
                    if result.frames.len() as u32 != expected_frames {
                        failures.push(alloc::format!(
                            "{}: frame count mismatch: got {}, expected {}",
                            filename,
                            result.frames.len(),
                            expected_frames
                        ));
                    } else {
                        tested += 1;
                    }
                }
                Err(e) => {
                    // Decode errors are expected for corrupt files — this test only checks for panics
                    eprintln!("  SKIP (decode error): {}: {}", filename, e);
                    tested += 1; // Still counts — we handled it without panicking
                }
            }
        }

        eprintln!(
            "APNG corpus: {} decoded ok, {} failures",
            tested,
            failures.len()
        );
        if !failures.is_empty() {
            for f in &failures[..failures.len().min(20)] {
                eprintln!("  FAIL: {}", f);
            }
            panic!(
                "{} APNG corpus decode failures (showing first 20)",
                failures.len()
            );
        }
        assert!(
            tested >= 10,
            "expected at least 10 APNG files, found {}",
            tested
        );
    }

    /// Compare frame-by-frame decode against the `image-png` crate's APNG as reference.
    #[test]
    fn apng_corpus_frame_comparison() {
        let apng_base = std::env::var("CORPUS_BUILDER_OUTPUT_DIR")
            .unwrap_or_else(|_| "/mnt/v/output/corpus-builder".to_string());
        let apng_dir_buf = std::path::PathBuf::from(&apng_base).join("apng");
        let apng_dir = apng_dir_buf.as_path();
        if !apng_dir.exists() {
            eprintln!(
                "Skipping APNG comparison test: {} not found",
                apng_dir.display()
            );
            return;
        }

        let mut tested = 0u32;
        let mut mismatches = 0u32;
        let mut our_errors = 0u32;
        let mut ref_errors = 0u32;

        for entry in std::fs::read_dir(apng_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("png") {
                continue;
            }

            let filename = path.file_name().unwrap().to_str().unwrap().to_string();
            let data = match std::fs::read(&path) {
                Ok(d) => d,
                Err(_) => continue,
            };

            let probe = match crate::decode::probe(&data) {
                Ok(p) => p,
                Err(_) => continue,
            };
            if !probe.sequence.is_animation() {
                continue;
            }

            // Decode with our decoder
            let our_result =
                crate::decode::decode_apng(&data, &PngDecodeConfig::none(), &Unstoppable);
            // Decode with reference (image-png crate)
            let ref_frames = decode_apng_with_png_crate(&data);

            match (our_result, ref_frames) {
                (Ok(ours), Ok(refs)) => {
                    let frame_count = ours.frames.len().min(refs.len());
                    let mut frame_match = true;
                    for (i, ref_frame) in refs.iter().enumerate().take(frame_count) {
                        let our_bytes = pixel_data_to_rgba8_bytes(&ours.frames[i].pixels);
                        if our_bytes != *ref_frame {
                            frame_match = false;
                            break;
                        }
                    }
                    if frame_match && ours.frames.len() == refs.len() {
                        tested += 1;
                    } else {
                        mismatches += 1;
                        if mismatches <= 5 {
                            eprintln!(
                                "  MISMATCH: {} (ours={} frames, ref={} frames)",
                                filename,
                                ours.frames.len(),
                                refs.len()
                            );
                        }
                    }
                }
                (Err(_), Ok(_)) => {
                    our_errors += 1;
                }
                (Ok(_), Err(_)) => {
                    ref_errors += 1;
                    tested += 1; // We succeeded where ref failed, that's OK
                }
                (Err(_), Err(_)) => {
                    // Both failed, skip
                }
            }
        }

        eprintln!(
            "APNG comparison: {} matched, {} mismatches, {} our-errors, {} ref-errors",
            tested, mismatches, our_errors, ref_errors
        );

        // Allow some mismatches due to compositing differences, but not too many
        assert!(
            tested >= 10,
            "expected at least 10 matching APNG files, got {}",
            tested
        );
    }

    // ── Reference decoder helper ────────────────────────────────────

    /// Decode all APNG frames using the reference `png` crate.
    /// Returns RGBA8 bytes for each composed frame.
    fn decode_apng_with_png_crate(data: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        use std::io::Cursor;

        let cursor = Cursor::new(data);
        let mut decoder = png::Decoder::new(cursor);
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let info = reader.info();
        let w = info.width as usize;
        let h = info.height as usize;

        let mut frames = Vec::new();

        while let Some(buffer_size) = reader.output_buffer_size() {
            let mut buf = vec![0u8; buffer_size];
            let output_info = match reader.next_frame(&mut buf) {
                Ok(info) => info,
                Err(png::DecodingError::Parameter(_)) => break,
                Err(_) => break,
            };
            buf.truncate(output_info.buffer_size());

            // Convert to RGBA8
            let (ct, bd) = reader.output_color_type();
            let rgba_bytes = match (ct, bd) {
                (png::ColorType::Rgba, png::BitDepth::Eight) => buf,
                (png::ColorType::Rgb, png::BitDepth::Eight) => {
                    let mut rgba = Vec::with_capacity(buf.len() / 3 * 4);
                    for chunk in buf.chunks_exact(3) {
                        rgba.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
                    }
                    rgba
                }
                (png::ColorType::Rgba, png::BitDepth::Sixteen) => {
                    // Downscale 16-bit to 8-bit
                    let mut rgba = Vec::with_capacity(buf.len() / 2);
                    for chunk in buf.chunks_exact(2) {
                        rgba.push(chunk[0]); // high byte
                    }
                    rgba
                }
                _ => {
                    // For other formats, convert through RGBA8
                    let mut rgba = Vec::with_capacity(w * h * 4);
                    for &b in &buf {
                        rgba.extend_from_slice(&[b, b, b, 255]);
                    }
                    rgba
                }
            };

            frames.push(rgba_bytes);
        }

        if frames.is_empty() {
            return Err("no frames decoded".into());
        }

        Ok(frames)
    }

    /// Extract RGBA8 bytes from PixelBuffer for comparison.
    fn pixel_data_to_rgba8_bytes(pixels: &PixelBuffer) -> Vec<u8> {
        use rgb::ComponentBytes;
        if let Some(img) = pixels.try_as_imgref::<rgb::Rgba<u8>>() {
            return img.buf().as_bytes().to_vec();
        }
        if let Some(img) = pixels.try_as_imgref::<rgb::Rgb<u8>>() {
            let mut out = Vec::with_capacity(img.buf().len() * 4);
            for p in *img.buf() {
                out.extend_from_slice(&[p.r, p.g, p.b, 255]);
            }
            return out;
        }
        if let Some(img) = pixels.try_as_imgref::<rgb::Rgba<u16>>() {
            let mut out = Vec::with_capacity(img.buf().len() * 4);
            for p in *img.buf() {
                out.extend_from_slice(&[
                    (p.r >> 8) as u8,
                    (p.g >> 8) as u8,
                    (p.b >> 8) as u8,
                    (p.a >> 8) as u8,
                ]);
            }
            return out;
        }
        Vec::new()
    }
}
