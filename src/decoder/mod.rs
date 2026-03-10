//! PNG decode pipeline: chunk parsing, row decoding, color conversion, info assembly.

pub(crate) mod apng;
pub(crate) mod interlace;
pub(crate) mod postprocess;
pub(crate) mod row;

use alloc::vec;
use alloc::vec::Vec;

use enough::Stop;
use imgref::ImgVec;
use zencodec::{Cicp, ContentLightLevel, MasteringDisplay};
use zenpixels::PixelBuffer;

use crate::chunk::ancillary::PngAncillary;
use crate::chunk::ihdr::Ihdr;
use crate::chunk::{ChunkIter, PNG_SIGNATURE};
use crate::decode::{PngChromaticities, PngDecodeOutput, PngInfo};
use crate::error::PngError;

use self::interlace::decode_interlaced;
use self::postprocess::{OutputFormat, build_pixel_data, post_process_row};
use self::row::RowDecoder;

// ── PngInfo construction ────────────────────────────────────────────

/// Build `PngInfo` from parsed IHDR and ancillary metadata.
pub(crate) fn build_png_info(ihdr: &Ihdr, ancillary: &PngAncillary) -> PngInfo {
    let has_alpha = ihdr.has_alpha() || ancillary.trns.is_some();
    let sequence = if let Some((n, _)) = ancillary.actl {
        zencodec::ImageSequence::Animation {
            frame_count: Some(n),
            loop_count: None,
            random_access: false,
        }
    } else {
        zencodec::ImageSequence::Single
    };

    let source_gamma = ancillary.gamma;
    let srgb_intent = ancillary.srgb_intent;

    let chromaticities = ancillary.chrm.map(|c| PngChromaticities {
        white_x: c[0],
        white_y: c[1],
        red_x: c[2],
        red_y: c[3],
        green_x: c[4],
        green_y: c[5],
        blue_x: c[6],
        blue_y: c[7],
    });

    let cicp = ancillary
        .cicp
        .map(|c| Cicp::new(c[0], c[1], c[2], c[3] != 0));

    let content_light_level = ancillary.clli.map(|c| {
        ContentLightLevel::new(
            (c[0] / 10000).min(65535) as u16,
            (c[1] / 10000).min(65535) as u16,
        )
    });

    let mastering_display = ancillary.mdcv.as_ref().and_then(|m| {
        if m.len() < 24 {
            return None;
        }
        // mDCV: 6×u16 BE chromaticities (Rx, Ry, Gx, Gy, Bx, By)
        //     + 2×u16 BE white point (Wx, Wy)
        //     + u32 BE max_luminance + u32 BE min_luminance
        // = 24 bytes. u16 values in units of 0.00002 (same as MasteringDisplay).
        let read_u16 = |off: usize| u16::from_be_bytes(m[off..off + 2].try_into().unwrap());
        let read_u32 = |off: usize| u32::from_be_bytes(m[off..off + 4].try_into().unwrap());

        // mDCV u16 chromaticities are in units of 0.00002; luminance u32 in units of 0.0001 cd/m²
        let xy = |off: usize| read_u16(off) as f32 * 0.00002;
        let lum = |off: usize| read_u32(off) as f32 * 0.0001;
        Some(MasteringDisplay::new(
            [
                [xy(0), xy(2)],   // Red
                [xy(4), xy(6)],   // Green
                [xy(8), xy(10)],  // Blue
            ],
            [xy(12), xy(14)],     // White point
            lum(16),              // max_luminance
            lum(20),              // min_luminance
        ))
    });

    PngInfo {
        width: ihdr.width,
        height: ihdr.height,
        has_alpha,
        sequence,
        bit_depth: ihdr.bit_depth,
        color_type: ihdr.color_type,
        icc_profile: ancillary.icc_profile.clone(),
        exif: ancillary.exif.clone(),
        xmp: ancillary.xmp.clone(),
        source_gamma,
        srgb_intent,
        chromaticities,
        cicp,
        content_light_level,
        mastering_display,
    }
}

// ── Probe helper ────────────────────────────────────────────────────

/// Probe PNG metadata without decoding pixels.
pub(crate) fn probe_png(data: &[u8]) -> Result<PngInfo, PngError> {
    if data.len() < 8 || data[..8] != PNG_SIGNATURE {
        return Err(PngError::Decode("not a PNG file".into()));
    }

    let mut chunks = ChunkIter::new(data);

    let ihdr_chunk = chunks
        .next()
        .ok_or_else(|| PngError::Decode("empty PNG".into()))??;
    if ihdr_chunk.chunk_type != *b"IHDR" {
        return Err(PngError::Decode("first chunk is not IHDR".into()));
    }
    let ihdr = Ihdr::parse(ihdr_chunk.data)?;

    let mut ancillary = PngAncillary::default();
    for chunk_result in &mut chunks {
        let chunk = chunk_result?;
        match &chunk.chunk_type {
            b"IDAT" => break,
            b"IEND" => break,
            _ => {
                ancillary.collect(&chunk)?;
            }
        }
    }

    // Also scan post-IDAT chunks for late metadata
    for chunk_result in chunks {
        let chunk = chunk_result?;
        if chunk.chunk_type == *b"IEND" {
            break;
        }
        ancillary.collect_late(&chunk);
    }

    Ok(build_png_info(&ihdr, &ancillary))
}

// ── Full decode ─────────────────────────────────────────────────────

/// Decode PNG to pixels using our own decoder.
pub(crate) fn decode_png(
    data: &[u8],
    limits: &crate::decode::PngDecodeConfig,
    cancel: &dyn Stop,
) -> Result<PngDecodeOutput, PngError> {
    // Check for interlacing first
    if data.len() >= 29 && data[..8] == PNG_SIGNATURE {
        let interlace = data[28]; // IHDR interlace byte
        if interlace == 1 {
            return decode_interlaced_to_output(data, limits, cancel);
        }
    }

    let mut reader = RowDecoder::new(data, limits)?;
    let ihdr = *reader.ihdr();
    let has_trns = reader.ancillary().trns.is_some();

    let w = ihdr.width as usize;
    let h = ihdr.height as usize;

    // Fast path: RGBA8 or RGB8 without tRNS — raw unfiltered data IS the output.
    // Skip post_process_row (passthrough copy), skip build_pixel_data (cast + clone).
    let is_passthrough =
        !has_trns && ihdr.bit_depth == 8 && (ihdr.color_type == 6 || ihdr.color_type == 2); // RGBA8 or RGB8

    if is_passthrough {
        let raw_row_bytes = ihdr.raw_row_bytes()?;
        let total = raw_row_bytes * h;
        let stride = ihdr.stride()?; // raw_row_bytes + 1 (filter byte)
        let bpp = ihdr.filter_bpp();

        // Try stored-block fast path: if the zlib stream is entirely stored
        // blocks (Compression::None), we can bypass the decompressor and just
        // strip headers + copy + unfilter.
        let first_idat_pos = reader.first_idat_pos();
        let skip_crc = limits.skip_critical_chunk_crc;

        let skip_adler = limits.skip_decompression_checksum;
        if let Some(all_pixels) = try_decode_stored(
            data,
            first_idat_pos,
            skip_crc,
            skip_adler,
            h,
            stride,
            raw_row_bytes,
            bpp,
            cancel,
        ) {
            let all_pixels = all_pixels?;
            reader.finish_metadata();
            let mut warnings = reader.collect_decode_warnings();
            let ancillary = reader.ancillary();
            let info = build_png_info(&ihdr, ancillary);
            warnings.extend(crate::decode::detect_color_warnings(
                ancillary.srgb_intent,
                ancillary.gamma,
                ancillary.chrm.as_ref(),
                ancillary.cicp.as_ref(),
                ancillary.icc_profile.as_deref(),
            ));
            let pixels: PixelBuffer = if ihdr.color_type == 6 {
                PixelBuffer::from_imgvec(ImgVec::new(vec_u8_to_rgba8(all_pixels), w, h)).into()
            } else {
                PixelBuffer::from_imgvec(ImgVec::new(vec_u8_to_rgb8(all_pixels), w, h)).into()
            };
            return Ok(PngDecodeOutput {
                pixels,
                info,
                warnings,
            });
        }

        // Standard streaming path
        let mut all_pixels = vec![0u8; total];

        // Row 0: prev is zeros (already zeroed by vec![0u8; total])
        if h > 0 {
            let zeros = vec![0u8; raw_row_bytes];
            match reader.next_raw_row_direct(&mut all_pixels[..raw_row_bytes], &zeros) {
                Some(Ok(())) => {}
                Some(Err(e)) => return Err(e),
                None => {
                    return Err(PngError::Decode(
                        "unexpected end of image data at row 0".into(),
                    ));
                }
            }
            cancel.check()?;
        }

        // Rows 1..h: prev is the previous row in the output buffer
        for y in 1..h {
            let (prev_part, cur_part) = all_pixels.split_at_mut(y * raw_row_bytes);
            let prev = &prev_part[(y - 1) * raw_row_bytes..];
            let dest = &mut cur_part[..raw_row_bytes];
            match reader.next_raw_row_direct(dest, prev) {
                Some(Ok(())) => {}
                Some(Err(e)) => return Err(e),
                None => {
                    return Err(PngError::Decode(alloc::format!(
                        "unexpected end of image data at row {y}"
                    )));
                }
            }
            cancel.check()?;
        }

        reader.finish_metadata();

        let mut warnings = reader.collect_decode_warnings();
        let ancillary = reader.ancillary();
        let info = build_png_info(&ihdr, ancillary);

        warnings.extend(crate::decode::detect_color_warnings(
            ancillary.srgb_intent,
            ancillary.gamma,
            ancillary.chrm.as_ref(),
            ancillary.cicp.as_ref(),
            ancillary.icc_profile.as_deref(),
        ));

        // Reinterpret bytes as typed pixels without copying
        let pixels: PixelBuffer = if ihdr.color_type == 6 {
            // RGBA8 — reinterpret Vec<u8> as Vec<Rgba<u8>> (same layout, no copy)
            let rgba = vec_u8_to_rgba8(all_pixels);
            PixelBuffer::from_imgvec(ImgVec::new(rgba, w, h)).into()
        } else {
            // RGB8
            let rgb = vec_u8_to_rgb8(all_pixels);
            PixelBuffer::from_imgvec(ImgVec::new(rgb, w, h)).into()
        };

        return Ok(PngDecodeOutput {
            pixels,
            info,
            warnings,
        });
    }

    // General path for all other formats
    let fmt = OutputFormat::from_ihdr(&ihdr, reader.ancillary());
    let pixel_bytes = fmt.channels * fmt.bytes_per_channel;
    let out_row_bytes = w * pixel_bytes;

    let mut all_pixels = Vec::with_capacity(out_row_bytes * h);
    let mut row_buf = Vec::new();
    let mut raw_copy = vec![0u8; ihdr.raw_row_bytes()?];

    while let Some(result) = reader.next_raw_row() {
        let raw = result?;
        cancel.check()?;
        raw_copy[..raw.len()].copy_from_slice(raw);
        post_process_row(
            &raw_copy[..raw.len()],
            &ihdr,
            reader.ancillary(),
            &mut row_buf,
        );
        all_pixels.extend_from_slice(&row_buf);
    }

    reader.finish_metadata();

    let mut warnings = reader.collect_decode_warnings();

    let ancillary = reader.ancillary();
    let info = build_png_info(&ihdr, ancillary);
    let pixels = build_pixel_data(&ihdr, ancillary, all_pixels, w, h)?;

    warnings.extend(crate::decode::detect_color_warnings(
        ancillary.srgb_intent,
        ancillary.gamma,
        ancillary.chrm.as_ref(),
        ancillary.cicp.as_ref(),
        ancillary.icc_profile.as_deref(),
    ));

    Ok(PngDecodeOutput {
        pixels,
        info,
        warnings,
    })
}

/// Reinterpret `Vec<u8>` as `Vec<Rgba<u8>>` without copying.
fn vec_u8_to_rgba8(bytes: Vec<u8>) -> Vec<rgb::Rgba<u8>> {
    bytemuck::cast_vec(bytes)
}

/// Reinterpret `Vec<u8>` as `Vec<Rgb<u8>>` without copying.
fn vec_u8_to_rgb8(bytes: Vec<u8>) -> Vec<rgb::Rgb<u8>> {
    bytemuck::cast_vec(bytes)
}

/// Try to decode a stored-block (uncompressed) zlib stream directly, bypassing
/// the full inflate engine. Returns `None` if the stream uses actual compression.
///
/// Stored DEFLATE blocks are: `[BFINAL|0x00] [LEN_LO LEN_HI NLEN_LO NLEN_HI] [data...]`
/// The zlib wrapper adds a 2-byte header and 4-byte Adler-32 trailer.
///
/// This strips block headers and copies pixel data directly from the zlib stream
/// into the output buffer, then unfilters in-place.
/// Result from the stored-block fast path.
#[allow(clippy::too_many_arguments)]
fn try_decode_stored(
    file_data: &[u8],
    first_idat_pos: usize,
    skip_crc: bool,
    skip_adler: bool,
    height: usize,
    stride: usize, // raw_row_bytes + 1 (filter byte)
    raw_row_bytes: usize,
    bpp: usize,
    cancel: &dyn Stop,
) -> Option<Result<Vec<u8>, PngError>> {
    // Collect IDAT chunk payload slices (the zlib stream, possibly split across chunks).
    let mut idat_slices: Vec<&[u8]> = Vec::new();
    let mut pos = first_idat_pos;
    while pos + 12 <= file_data.len() {
        let length = u32::from_be_bytes(file_data[pos..pos + 4].try_into().unwrap()) as usize;
        let chunk_type: [u8; 4] = file_data[pos + 4..pos + 8].try_into().unwrap();
        let data_start = pos + 8;
        let Some(data_end) = data_start.checked_add(length) else {
            break;
        };
        let Some(crc_end) = data_end.checked_add(4) else {
            break;
        };
        if crc_end > file_data.len() {
            break;
        }
        if chunk_type != *b"IDAT" {
            break;
        }
        if !skip_crc {
            let stored_crc = u32::from_be_bytes(file_data[data_end..crc_end].try_into().unwrap());
            let computed = zenflate::crc32(
                zenflate::crc32(0, &chunk_type),
                &file_data[data_start..data_end],
            );
            if stored_crc != computed {
                return Some(Err(PngError::Decode("CRC mismatch in IDAT chunk".into())));
            }
        }
        idat_slices.push(&file_data[data_start..data_end]);
        pos = crc_end;
    }

    if idat_slices.is_empty() {
        return None;
    }

    // Build a contiguous view of the zlib stream.
    // For single-IDAT PNGs (common), this is zero-copy.
    let zlib_owned: Vec<u8>;
    let zlib: &[u8] = if idat_slices.len() == 1 {
        idat_slices[0]
    } else {
        zlib_owned = {
            let total: usize = idat_slices.iter().map(|s| s.len()).sum();
            let mut v = Vec::with_capacity(total);
            for s in &idat_slices {
                v.extend_from_slice(s);
            }
            v
        };
        &zlib_owned
    };

    // Need at least: zlib header (2) + block header (5) + adler32 (4)
    if zlib.len() < 11 {
        return None;
    }

    // Check zlib header: CM must be 8 (deflate)
    if zlib[0] & 0x0F != 8 {
        return None;
    }

    // Check first block is stored (BTYPE=00, bits 1-2 of first block byte)
    if zlib[2] & 0x06 != 0 {
        return None; // compressed — use full inflate
    }

    // Parse all stored blocks, collecting (offset, len) spans of payload data
    // within `zlib`. This avoids any intermediate allocation.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut zpos = 2; // past zlib header
    let zlib_end = zlib.len() - 4; // before adler32 trailer

    loop {
        if zpos >= zlib_end {
            return Some(Err(PngError::Decode("truncated stored block".into())));
        }
        let bfinal = zlib[zpos] & 0x01;
        let btype = (zlib[zpos] >> 1) & 0x03;
        if btype != 0 {
            return None; // not all stored blocks
        }
        zpos += 1;
        if zpos + 4 > zlib_end {
            return Some(Err(PngError::Decode(
                "truncated stored block header".into(),
            )));
        }
        let len = u16::from_le_bytes([zlib[zpos], zlib[zpos + 1]]) as usize;
        let nlen = u16::from_le_bytes([zlib[zpos + 2], zlib[zpos + 3]]) as usize;
        if len != (!nlen & 0xFFFF) {
            return Some(Err(PngError::Decode(
                "stored block LEN/NLEN mismatch".into(),
            )));
        }
        zpos += 4;
        if zpos + len > zlib_end {
            return Some(Err(PngError::Decode("truncated stored block data".into())));
        }
        if len > 0 {
            spans.push((zpos, len));
        }
        zpos += len;
        if bfinal != 0 {
            break;
        }
    }

    // Verify Adler-32 checksum from zlib trailer (skip entirely when not needed).
    if !skip_adler {
        let stored_adler = u32::from_be_bytes(zlib[zlib_end..zlib_end + 4].try_into().unwrap());
        let mut computed = zenflate::adler32(1, &[]);
        for &(start, len) in &spans {
            computed = zenflate::adler32(computed, &zlib[start..start + len]);
        }
        if stored_adler != computed {
            return Some(Err(PngError::Decode("Adler-32 checksum mismatch".into())));
        }
    }

    // Walk spans linearly with a cursor, copying row data into the output buffer.

    let total_payload: usize = spans.iter().map(|&(_, l)| l).sum();
    let expected = stride * height;
    if total_payload < expected {
        return Some(Err(PngError::Decode(alloc::format!(
            "stored data too short: {total_payload} < {expected}"
        ))));
    }

    let total_pixels = raw_row_bytes * height;
    let mut all_pixels = vec![0u8; total_pixels];

    let mut cursor = SpanCursor::new(&spans);
    let zeros = vec![0u8; raw_row_bytes];

    for y in 0..height {
        // Read filter byte
        let fb = cursor.read_byte(zlib);

        // Copy row data into output
        let dest_start = y * raw_row_bytes;
        cursor.read_into(
            zlib,
            &mut all_pixels[dest_start..dest_start + raw_row_bytes],
        );

        // Unfilter if needed
        if fb != 0 {
            if y == 0 {
                if let Err(e) = row::unfilter_row(fb, &mut all_pixels[..raw_row_bytes], &zeros, bpp)
                {
                    return Some(Err(e));
                }
            } else {
                let (prev_part, cur_part) = all_pixels.split_at_mut(y * raw_row_bytes);
                let prev = &prev_part[(y - 1) * raw_row_bytes..];
                if let Err(e) = row::unfilter_row(fb, &mut cur_part[..raw_row_bytes], prev, bpp) {
                    return Some(Err(e));
                }
            }
        }
        cancel.check().ok()?;
    }

    Some(Ok(all_pixels))
}

/// Linear cursor over payload spans within a contiguous zlib buffer.
struct SpanCursor<'a> {
    spans: &'a [(usize, usize)],
    idx: usize,
    off: usize, // offset within current span
}

impl<'a> SpanCursor<'a> {
    fn new(spans: &'a [(usize, usize)]) -> Self {
        Self {
            spans,
            idx: 0,
            off: 0,
        }
    }

    fn read_byte(&mut self, zlib: &[u8]) -> u8 {
        while self.idx < self.spans.len() {
            let (start, len) = self.spans[self.idx];
            if self.off < len {
                let b = zlib[start + self.off];
                self.off += 1;
                return b;
            }
            self.idx += 1;
            self.off = 0;
        }
        0
    }

    fn read_into(&mut self, zlib: &[u8], dest: &mut [u8]) {
        let mut written = 0;
        while written < dest.len() && self.idx < self.spans.len() {
            let (start, len) = self.spans[self.idx];
            let avail = len - self.off;
            let n = avail.min(dest.len() - written);
            dest[written..written + n]
                .copy_from_slice(&zlib[start + self.off..start + self.off + n]);
            written += n;
            self.off += n;
            if self.off >= len {
                self.idx += 1;
                self.off = 0;
            }
        }
    }
}

/// Decode interlaced PNG to PngDecodeOutput.
fn decode_interlaced_to_output(
    data: &[u8],
    config: &crate::decode::PngDecodeConfig,
    cancel: &dyn Stop,
) -> Result<PngDecodeOutput, PngError> {
    let (ihdr, ancillary, pixels, _fmt, mut warnings) = decode_interlaced(data, config, cancel)?;
    let w = ihdr.width as usize;
    let h = ihdr.height as usize;
    let info = build_png_info(&ihdr, &ancillary);
    let pixel_data = build_pixel_data(&ihdr, &ancillary, pixels, w, h)?;

    warnings.extend(crate::decode::detect_color_warnings(
        ancillary.srgb_intent,
        ancillary.gamma,
        ancillary.chrm.as_ref(),
        ancillary.cicp.as_ref(),
        ancillary.icc_profile.as_deref(),
    ));

    Ok(PngDecodeOutput {
        pixels: pixel_data,
        info,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::ihdr::Ihdr;
    use crate::decoder::postprocess::scale_to_8bit;
    use crate::decoder::row::unfilter_row;
    use enough::Unstoppable;
    use imgref::ImgVec;
    use rgb::{Gray, Rgb, Rgba};
    use zenpixels::{ChannelLayout, ChannelType, GrayAlpha16, PixelBuffer};

    #[test]
    fn chunk_parser_validates_signature() {
        let result = decode_png(
            b"not a png",
            &crate::decode::PngDecodeConfig::none(),
            &Unstoppable,
        );
        assert!(result.is_err());
    }

    #[test]
    fn unfilter_none() {
        let mut row = vec![10, 20, 30];
        let prev = vec![0, 0, 0];
        unfilter_row(0, &mut row, &prev, 1).unwrap();
        assert_eq!(row, vec![10, 20, 30]);
    }

    #[test]
    fn unfilter_sub() {
        let mut row = vec![10, 5, 3];
        let prev = vec![0, 0, 0];
        unfilter_row(1, &mut row, &prev, 1).unwrap();
        assert_eq!(row, vec![10, 15, 18]);
    }

    #[test]
    fn unfilter_up() {
        let mut row = vec![10, 20, 30];
        let prev = vec![5, 10, 15];
        unfilter_row(2, &mut row, &prev, 1).unwrap();
        assert_eq!(row, vec![15, 30, 45]);
    }

    #[test]
    fn unfilter_average() {
        let mut row = vec![10, 5, 3];
        let prev = vec![0, 0, 0];
        unfilter_row(3, &mut row, &prev, 1).unwrap();
        // i=0: row[0] += prev[0] >> 1 = 10 + 0 = 10
        // i=1: row[1] += floor((row[0] + prev[1]) / 2) = 5 + 5 = 10
        // i=2: row[2] += floor((row[1] + prev[2]) / 2) = 3 + 5 = 8
        assert_eq!(row, vec![10, 10, 8]);
    }

    #[test]
    fn unfilter_paeth() {
        let mut row = vec![10, 5, 3];
        let prev = vec![0, 0, 0];
        unfilter_row(4, &mut row, &prev, 1).unwrap();
        // i=0: paeth(0, 0, 0) = 0, so 10 + 0 = 10
        // i=1: paeth(10, 0, 0) = 10, so 5 + 10 = 15
        // i=2: paeth(15, 0, 0) = 15, so 3 + 15 = 18
        assert_eq!(row, vec![10, 15, 18]);
    }

    #[test]
    fn ihdr_validates_color_type_bit_depth() {
        // Valid: Gray 8-bit
        assert!(Ihdr::parse(&make_ihdr(1, 1, 8, 0, 0)).is_ok());
        // Valid: Indexed 4-bit
        assert!(Ihdr::parse(&make_ihdr(1, 1, 4, 3, 0)).is_ok());
        // Invalid: RGB 4-bit
        assert!(Ihdr::parse(&make_ihdr(1, 1, 4, 2, 0)).is_err());
        // Invalid: Indexed 16-bit
        assert!(Ihdr::parse(&make_ihdr(1, 1, 16, 3, 0)).is_err());
    }

    fn make_ihdr(w: u32, h: u32, bit_depth: u8, color_type: u8, interlace: u8) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&w.to_be_bytes());
        data.extend_from_slice(&h.to_be_bytes());
        data.push(bit_depth);
        data.push(color_type);
        data.push(0); // compression
        data.push(0); // filter
        data.push(interlace);
        data
    }

    #[test]
    fn scale_to_8bit_values() {
        assert_eq!(scale_to_8bit(0, 1), 0);
        assert_eq!(scale_to_8bit(1, 1), 255);
        assert_eq!(scale_to_8bit(0, 2), 0);
        assert_eq!(scale_to_8bit(1, 2), 85);
        assert_eq!(scale_to_8bit(3, 2), 255);
        assert_eq!(scale_to_8bit(0, 4), 0);
        assert_eq!(scale_to_8bit(15, 4), 255);
    }

    /// Regression test: decode all local Tier 1 fixtures from tests/regression/.
    #[test]
    fn regression_fixtures_decode() {
        let fixture_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/regression");
        assert!(
            fixture_dir.exists(),
            "regression fixture directory not found: {}",
            fixture_dir.display()
        );

        let mut tested = 0;
        let mut failures = Vec::new();

        for entry in std::fs::read_dir(&fixture_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("png") {
                continue;
            }

            let filename = path.file_name().unwrap().to_str().unwrap().to_string();
            let data = std::fs::read(&path).unwrap();

            // Known-corrupt fixtures where our stricter validation is expected
            let known_corrupt = filename.contains("badadler");

            let our_result =
                decode_png(&data, &crate::decode::PngDecodeConfig::none(), &Unstoppable);
            let ref_result = decode_with_png_crate(&data);

            match (our_result, ref_result) {
                (Ok(ours), Ok(reference)) => {
                    let our_bytes = pixel_data_to_bytes(&ours.pixels);
                    let ref_bytes = pixel_data_to_bytes(&reference.pixels);
                    if our_bytes != ref_bytes {
                        let our_desc = format_pixel_data(&ours.pixels);
                        let ref_desc = format_pixel_data(&reference.pixels);
                        failures.push(alloc::format!(
                            "{}: pixel mismatch (ours={}, ref={}, our_len={}, ref_len={})",
                            filename,
                            our_desc,
                            ref_desc,
                            our_bytes.len(),
                            ref_bytes.len()
                        ));
                    } else {
                        tested += 1;
                    }
                }
                (Err(_), Err(_)) => {
                    tested += 1;
                }
                (Ok(_), Err(_)) => {
                    tested += 1;
                }
                (Err(e), Ok(_)) if known_corrupt => {
                    eprintln!(
                        "  {}: we reject, ref accepts (known corrupt): {}",
                        filename, e
                    );
                    tested += 1;
                }
                (Err(e), Ok(_)) => {
                    failures.push(alloc::format!(
                        "{}: we failed but ref succeeded: {}",
                        filename,
                        e
                    ));
                }
            }
        }

        eprintln!(
            "Regression fixtures: {} tested, {} failures",
            tested,
            failures.len()
        );
        if !failures.is_empty() {
            for f in &failures {
                eprintln!("  FAIL: {}", f);
            }
            panic!("{} regression fixture failures", failures.len());
        }
        assert!(
            tested >= 8,
            "expected at least 8 fixtures, found {}",
            tested
        );
    }

    /// Compare our decoder's pixel output against the reference png crate
    /// for every PNGSuite file, fetched via codec-corpus.
    #[test]
    fn pngsuite_comparison() {
        let corpus = match codec_corpus::Corpus::new() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Skipping PNGSuite comparison: {e}");
                return;
            }
        };
        let suite_dir = match corpus.get("pngsuite") {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Skipping PNGSuite comparison: {e}");
                return;
            }
        };

        let mut tested = 0;
        let mut skipped = 0;
        let mut failures = Vec::new();

        for entry in std::fs::read_dir(suite_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("png") {
                continue;
            }

            let filename = path.file_name().unwrap().to_str().unwrap().to_string();

            // Skip files that are intentionally corrupt (start with 'x')
            if filename.starts_with('x') {
                skipped += 1;
                continue;
            }

            let data = std::fs::read(&path).unwrap();

            let our_result =
                decode_png(&data, &crate::decode::PngDecodeConfig::none(), &Unstoppable);
            let ref_result = decode_with_png_crate(&data);

            match (our_result, ref_result) {
                (Ok(ours), Ok(reference)) => {
                    let our_bytes = pixel_data_to_bytes(&ours.pixels);
                    let ref_bytes = pixel_data_to_bytes(&reference.pixels);

                    if our_bytes != ref_bytes {
                        let our_desc = format_pixel_data(&ours.pixels);
                        let ref_desc = format_pixel_data(&reference.pixels);
                        failures.push(alloc::format!(
                            "{}: pixel mismatch (ours={}, ref={}, our_len={}, ref_len={})",
                            filename,
                            our_desc,
                            ref_desc,
                            our_bytes.len(),
                            ref_bytes.len()
                        ));
                    } else {
                        tested += 1;
                    }
                }
                (Err(e), Ok(_)) => {
                    failures.push(alloc::format!(
                        "{}: we failed but ref succeeded: {}",
                        filename,
                        e
                    ));
                }
                (Ok(_), Err(_)) => {
                    tested += 1;
                }
                (Err(_), Err(_)) => {
                    skipped += 1;
                }
            }
        }

        eprintln!(
            "PNGSuite: {} matched, {} skipped, {} failures",
            tested,
            skipped,
            failures.len()
        );
        if !failures.is_empty() {
            for f in &failures {
                eprintln!("  FAIL: {}", f);
            }
            panic!(
                "{} PNGSuite comparison failures (see stderr)",
                failures.len()
            );
        }
    }

    /// Compare our decoder against the reference png crate on the
    /// png-conformance corpus.
    #[test]
    fn png_conformance_corpus() {
        let corpus = match codec_corpus::Corpus::new() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Skipping png-conformance comparison: {e}");
                return;
            }
        };
        let conf_dir = match corpus.get("png-conformance") {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Skipping png-conformance comparison: {e}");
                return;
            }
        };

        let mut tested = 0;
        let mut failures = Vec::new();

        for entry in std::fs::read_dir(&conf_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("png") {
                continue;
            }

            let filename = path.file_name().unwrap().to_str().unwrap().to_string();
            let data = std::fs::read(&path).unwrap();

            let known_corrupt = filename.contains("badadler");

            let our_result =
                decode_png(&data, &crate::decode::PngDecodeConfig::none(), &Unstoppable);
            let ref_result = decode_with_png_crate(&data);

            match (our_result, ref_result) {
                (Ok(ours), Ok(reference)) => {
                    let our_bytes = pixel_data_to_bytes(&ours.pixels);
                    let ref_bytes = pixel_data_to_bytes(&reference.pixels);
                    if our_bytes != ref_bytes {
                        let our_desc = format_pixel_data(&ours.pixels);
                        let ref_desc = format_pixel_data(&reference.pixels);
                        failures.push(alloc::format!(
                            "{}: pixel mismatch (ours={}, ref={}, our_len={}, ref_len={})",
                            filename,
                            our_desc,
                            ref_desc,
                            our_bytes.len(),
                            ref_bytes.len()
                        ));
                    } else {
                        tested += 1;
                    }
                }
                (Err(_), Err(_)) => {
                    tested += 1;
                }
                (Ok(_), Err(_)) => {
                    tested += 1;
                }
                (Err(e), Ok(_)) if known_corrupt => {
                    eprintln!(
                        "  {}: we reject, ref accepts (known corrupt): {}",
                        filename, e
                    );
                    tested += 1;
                }
                (Err(e), Ok(_)) => {
                    failures.push(alloc::format!(
                        "{}: we failed but ref succeeded: {}",
                        filename,
                        e
                    ));
                }
            }
        }

        eprintln!(
            "png-conformance: {} tested, {} failures",
            tested,
            failures.len()
        );
        if !failures.is_empty() {
            for f in &failures {
                eprintln!("  FAIL: {}", f);
            }
            panic!(
                "{} png-conformance comparison failures (see stderr)",
                failures.len()
            );
        }
        assert!(
            tested >= 11,
            "expected at least 11 png-conformance files, found {}",
            tested
        );
    }

    /// Decode using the reference png crate.
    fn decode_with_png_crate(data: &[u8]) -> Result<PngDecodeOutput, String> {
        use std::io::Cursor;

        let cursor = Cursor::new(data);
        let mut decoder = png::Decoder::new(cursor);
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let w = reader.info().width as usize;
        let h = reader.info().height as usize;
        let src_bit_depth = reader.info().bit_depth as u8;

        let (ct, bd) = reader.output_color_type();
        let buffer_size = reader.output_buffer_size().ok_or("no buffer size")?;
        let mut raw_pixels = vec![0u8; buffer_size];
        let output_info = reader
            .next_frame(&mut raw_pixels)
            .map_err(|e| e.to_string())?;
        raw_pixels.truncate(output_info.buffer_size());

        // Convert to native endian for 16-bit
        let pixels: PixelBuffer = match (ct, bd) {
            (png::ColorType::Rgba, png::BitDepth::Sixteen) => {
                let native = be_to_native_16_ref(&raw_pixels);
                let rgba: &[Rgba<u16>] = bytemuck::cast_slice(&native);
                PixelBuffer::from_imgvec(ImgVec::new(rgba.to_vec(), w, h)).into()
            }
            (png::ColorType::Rgba, _) => {
                let rgba: &[Rgba<u8>] = bytemuck::cast_slice(&raw_pixels);
                PixelBuffer::from_imgvec(ImgVec::new(rgba.to_vec(), w, h)).into()
            }
            (png::ColorType::Rgb, png::BitDepth::Sixteen) => {
                let native = be_to_native_16_ref(&raw_pixels);
                let rgb: &[Rgb<u16>] = bytemuck::cast_slice(&native);
                PixelBuffer::from_imgvec(ImgVec::new(rgb.to_vec(), w, h)).into()
            }
            (png::ColorType::Rgb, _) => {
                let rgb: &[Rgb<u8>] = bytemuck::cast_slice(&raw_pixels);
                PixelBuffer::from_imgvec(ImgVec::new(rgb.to_vec(), w, h)).into()
            }
            (png::ColorType::GrayscaleAlpha, png::BitDepth::Sixteen) => {
                let native = be_to_native_16_ref(&raw_pixels);
                let ga: &[[u16; 2]] = bytemuck::cast_slice(&native);
                let pixels: Vec<GrayAlpha16> =
                    ga.iter().map(|&[v, a]| GrayAlpha16::new(v, a)).collect();
                PixelBuffer::from_imgvec(ImgVec::new(pixels, w, h)).into()
            }
            (png::ColorType::GrayscaleAlpha, _) => {
                // GA8 → RGBA8 (matches our decoder behavior)
                let rgba: Vec<Rgba<u8>> = raw_pixels
                    .chunks_exact(2)
                    .map(|ga| Rgba {
                        r: ga[0],
                        g: ga[0],
                        b: ga[0],
                        a: ga[1],
                    })
                    .collect();
                PixelBuffer::from_imgvec(ImgVec::new(rgba, w, h)).into()
            }
            (png::ColorType::Grayscale, png::BitDepth::Sixteen) => {
                let native = be_to_native_16_ref(&raw_pixels);
                let gray: &[Gray<u16>] = bytemuck::cast_slice(&native);
                PixelBuffer::from_imgvec(ImgVec::new(gray.to_vec(), w, h)).into()
            }
            (png::ColorType::Grayscale, _) => {
                let gray: Vec<Gray<u8>> = raw_pixels.iter().map(|&g| Gray(g)).collect();
                PixelBuffer::from_imgvec(ImgVec::new(gray, w, h)).into()
            }
            (png::ColorType::Indexed, _) => {
                return Err("indexed not expanded".into());
            }
        };

        let src_color_type = match reader.info().color_type {
            png::ColorType::Grayscale => 0,
            png::ColorType::Rgb => 2,
            png::ColorType::Indexed => 3,
            png::ColorType::GrayscaleAlpha => 4,
            png::ColorType::Rgba => 6,
        };
        let info = PngInfo {
            width: w as u32,
            height: h as u32,
            has_alpha: false,
            sequence: zencodec::ImageSequence::Single,
            bit_depth: src_bit_depth,
            color_type: src_color_type,
            icc_profile: None,
            exif: None,
            xmp: None,
            source_gamma: None,
            srgb_intent: None,
            chromaticities: None,
            cicp: None,
            content_light_level: None,
            mastering_display: None,
        };

        Ok(PngDecodeOutput {
            pixels,
            info,
            warnings: Vec::new(),
        })
    }

    fn be_to_native_16_ref(bytes: &[u8]) -> Vec<u8> {
        if cfg!(target_endian = "big") {
            return bytes.to_vec();
        }
        let mut out = bytes.to_vec();
        for chunk in out.chunks_exact_mut(2) {
            chunk.swap(0, 1);
        }
        out
    }

    fn pixel_data_to_bytes(pixels: &PixelBuffer) -> Vec<u8> {
        pixels.copy_to_contiguous_bytes()
    }

    fn format_pixel_data(pixels: &PixelBuffer) -> &'static str {
        let desc = pixels.descriptor();
        match (desc.layout(), desc.channel_type()) {
            (ChannelLayout::Rgb, ChannelType::U8) => "Rgb8",
            (ChannelLayout::Rgba, ChannelType::U8) => "Rgba8",
            (ChannelLayout::Gray, ChannelType::U8) => "Gray8",
            (ChannelLayout::Rgb, ChannelType::U16) => "Rgb16",
            (ChannelLayout::Rgba, ChannelType::U16) => "Rgba16",
            (ChannelLayout::Gray, ChannelType::U16) => "Gray16",
            (ChannelLayout::GrayAlpha, ChannelType::U16) => "GrayAlpha16",
            _ => "Other",
        }
    }

    /// Walk a directory tree collecting all .png files.
    fn collect_pngs(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut pngs = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            for entry in entries {
                let Ok(entry) = entry else { continue };
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().and_then(|e| e.to_str()) == Some("png") {
                    pngs.push(path);
                }
            }
        }
        pngs.sort();
        pngs
    }

    /// Mass-test our decoder against the png crate on all PNG corpuses.
    #[test]
    fn corpus_comparison_vs_png_crate() {
        let codec_corpus = std::env::var("CODEC_CORPUS_DIR")
            .unwrap_or_else(|_| "/home/lilith/work/codec-corpus".to_string());
        let discover =
            std::env::var("DISCOVER_DIR").unwrap_or_else(|_| "/mnt/v/discover".to_string());
        let jpeg_encoder = std::env::var("JPEG_ENCODER_DIR")
            .unwrap_or_else(|_| "/home/lilith/work/jpeg-encoder".to_string());
        let corpus_dirs: Vec<(&str, String)> = vec![
            ("codec-corpus", codec_corpus),
            ("discover", format!("{discover}/images")),
            ("kodak", format!("{discover}/kodak/images")),
            ("image-png", format!("{jpeg_encoder}/external/image-png")),
        ];

        let mut total_tested = 0u32;
        let mut total_skipped = 0u32;
        let mut total_both_err = 0u32;
        let mut failures: Vec<String> = Vec::new();

        for (corpus_name, dir) in &corpus_dirs {
            let dir_path = std::path::Path::new(dir);
            if !dir_path.exists() {
                eprintln!("Corpus '{}' not found at {}, skipping", corpus_name, dir);
                continue;
            }

            let pngs = collect_pngs(dir_path);
            eprintln!("Corpus '{}': {} PNG files found", corpus_name, pngs.len());

            let mut corpus_tested = 0u32;
            let mut corpus_skipped = 0u32;
            let mut corpus_both_err = 0u32;

            let progress_interval = if pngs.len() > 1000 {
                pngs.len() / 20
            } else {
                usize::MAX
            };

            for (idx, path) in pngs.iter().enumerate() {
                if idx > 0 && idx % progress_interval == 0 {
                    eprintln!(
                        "  [{}/{}] {} matched, {} failures so far",
                        idx,
                        pngs.len(),
                        corpus_tested,
                        failures.len()
                    );
                }

                let filename = path.strip_prefix(dir_path).unwrap_or(path);
                let filename_str = filename.display().to_string();

                // Skip intentionally corrupt files
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                    && ((stem.starts_with('x') && filename_str.contains("pngsuite"))
                        || stem == "badadler")
                {
                    corpus_skipped += 1;
                    continue;
                }

                let data = match std::fs::read(path) {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("  read error {}: {}", filename_str, e);
                        corpus_skipped += 1;
                        continue;
                    }
                };

                let our_result =
                    decode_png(&data, &crate::decode::PngDecodeConfig::none(), &Unstoppable);
                let ref_result = decode_with_png_crate(&data);

                match (our_result, ref_result) {
                    (Ok(ours), Ok(reference)) => {
                        let our_bytes = pixel_data_to_bytes(&ours.pixels);
                        let ref_bytes = pixel_data_to_bytes(&reference.pixels);

                        if our_bytes != ref_bytes {
                            let our_desc = format_pixel_data(&ours.pixels);
                            let ref_desc = format_pixel_data(&reference.pixels);
                            failures.push(alloc::format!(
                                "[{}] {}: pixel mismatch (ours={}, ref={}, our_len={}, ref_len={})",
                                corpus_name,
                                filename_str,
                                our_desc,
                                ref_desc,
                                our_bytes.len(),
                                ref_bytes.len()
                            ));
                        } else {
                            corpus_tested += 1;
                        }
                    }
                    (Err(e), Ok(_)) => {
                        failures.push(alloc::format!(
                            "[{}] {}: we failed but ref succeeded: {}",
                            corpus_name,
                            filename_str,
                            e
                        ));
                    }
                    (Ok(_), Err(_)) => {
                        corpus_tested += 1;
                    }
                    (Err(_), Err(_)) => {
                        corpus_both_err += 1;
                    }
                }
            }

            eprintln!(
                "  {} matched, {} skipped, {} both-err, {} failures so far",
                corpus_tested,
                corpus_skipped,
                corpus_both_err,
                failures.len()
            );
            total_tested += corpus_tested;
            total_skipped += corpus_skipped;
            total_both_err += corpus_both_err;
        }

        eprintln!(
            "\n=== TOTAL: {} matched, {} skipped, {} both-err, {} failures ===",
            total_tested,
            total_skipped,
            total_both_err,
            failures.len()
        );

        if !failures.is_empty() {
            eprintln!("\nFailures:");
            for f in &failures {
                eprintln!("  FAIL: {}", f);
            }
            panic!("{} corpus comparison failures (see stderr)", failures.len());
        }
    }

    /// Mass-test against corpus-builder scraped PNGs (85K+ files).
    /// Run with: cargo test --release corpus_builder -- --nocapture --ignored
    #[test]
    #[ignore]
    fn corpus_builder_comparison() {
        use std::io::Write;

        let cb_base = std::env::var("CORPUS_BUILDER_OUTPUT_DIR")
            .unwrap_or_else(|_| "/mnt/v/output/corpus-builder".to_string());
        let corpus_dirs: Vec<(&str, String)> = vec![
            ("png-8", format!("{cb_base}/png-8")),
            ("png-24-32", format!("{cb_base}/png-24-32")),
            ("apng", format!("{cb_base}/apng")),
            ("repro", format!("{cb_base}/repro-images")),
        ];

        let results_base = std::env::var("ZENPNG_OUTPUT_DIR")
            .unwrap_or_else(|_| "/mnt/v/output/zenpng".to_string());
        let results_dir = std::path::PathBuf::from(&results_base);
        std::fs::create_dir_all(&results_dir).unwrap();
        let results_path = results_dir.join("corpus_results.jsonl");

        // Load already-tested paths for resumption
        let mut done: std::collections::HashSet<String> = std::collections::HashSet::new();
        if results_path.exists()
            && let Ok(contents) = std::fs::read_to_string(&results_path)
        {
            for line in contents.lines() {
                if let Some(path_start) = line.find("\"path\":\"") {
                    let rest = &line[path_start + 8..];
                    if let Some(end) = rest.find('"') {
                        done.insert(rest[..end].to_string());
                    }
                }
            }
        }
        eprintln!("Loaded {} already-tested results", done.len());

        let mut results_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&results_path)
            .unwrap();

        let mut total_tested = 0u32;
        let mut total_we_fail = 0u32;
        let mut total_both_err = 0u32;
        let mut total_pixel_mismatch = 0u32;
        let mut total_skipped = 0u32;
        let mut total_timeout = 0u32;

        let timeout_dur = std::time::Duration::from_secs(30);

        for (corpus_name, dir) in &corpus_dirs {
            let dir_path = std::path::Path::new(dir);
            if !dir_path.exists() {
                eprintln!("Corpus '{}' not found at {}, skipping", corpus_name, dir);
                continue;
            }

            let pngs = collect_pngs(dir_path);
            eprintln!("Corpus '{}': {} PNG files found", corpus_name, pngs.len());

            let mut corpus_tested = 0u32;
            let mut corpus_we_fail = 0u32;
            let mut corpus_both_err = 0u32;
            let progress_interval = if pngs.len() > 500 {
                pngs.len() / 20
            } else {
                usize::MAX
            };

            for (idx, path) in pngs.iter().enumerate() {
                if idx > 0 && idx % progress_interval == 0 {
                    eprintln!(
                        "  [{}/{}] {} ok, {} we-fail, {} mismatch, {} timeout",
                        idx,
                        pngs.len(),
                        corpus_tested,
                        corpus_we_fail,
                        total_pixel_mismatch,
                        total_timeout
                    );
                }

                let path_str = path.display().to_string();

                // Skip already-tested files
                if done.contains(&path_str) {
                    total_skipped += 1;
                    continue;
                }

                let data = match std::fs::read(path) {
                    Ok(d) => d,
                    Err(_) => continue,
                };

                // Skip very large files (>50MB)
                if data.len() > 50_000_000 {
                    let _ = writeln!(
                        results_file,
                        "{{\"path\":\"{}\",\"corpus\":\"{}\",\"result\":\"skipped\",\"reason\":\"too_large\",\"size\":{}}}",
                        path_str,
                        corpus_name,
                        data.len()
                    );
                    corpus_we_fail += 1;
                    continue;
                }

                // Decode with hard thread-based timeout
                let data_clone = data.clone();
                let handle = std::thread::spawn(move || {
                    decode_png(
                        &data_clone,
                        &crate::decode::PngDecodeConfig::none(),
                        &Unstoppable,
                    )
                });
                let deadline = std::time::Instant::now() + timeout_dur;
                let our_result = loop {
                    if handle.is_finished() {
                        break handle.join().unwrap();
                    }
                    if std::time::Instant::now() >= deadline {
                        let _ = writeln!(
                            results_file,
                            "{{\"path\":\"{}\",\"corpus\":\"{}\",\"result\":\"timeout\",\"size\":{}}}",
                            path_str,
                            corpus_name,
                            data.len()
                        );
                        total_timeout += 1;
                        corpus_we_fail += 1;
                        break Err(PngError::Decode("timeout".into()));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                };
                if our_result
                    .as_ref()
                    .is_err_and(|e| alloc::format!("{e}").contains("timeout"))
                {
                    continue;
                }

                let ref_result = decode_with_png_crate(&data);

                let result_str = match (&our_result, &ref_result) {
                    (Ok(ours), Ok(reference)) => {
                        let our_bytes = pixel_data_to_bytes(&ours.pixels);
                        let ref_bytes = pixel_data_to_bytes(&reference.pixels);

                        if our_bytes != ref_bytes {
                            total_pixel_mismatch += 1;
                            alloc::format!(
                                "{{\"path\":\"{}\",\"corpus\":\"{}\",\"result\":\"pixel_mismatch\",\"ours\":\"{}\",\"ref\":\"{}\",\"our_len\":{},\"ref_len\":{}}}",
                                path_str,
                                corpus_name,
                                format_pixel_data(&ours.pixels),
                                format_pixel_data(&reference.pixels),
                                our_bytes.len(),
                                ref_bytes.len()
                            )
                        } else {
                            corpus_tested += 1;
                            alloc::format!(
                                "{{\"path\":\"{}\",\"corpus\":\"{}\",\"result\":\"ok\"}}",
                                path_str,
                                corpus_name
                            )
                        }
                    }
                    (Err(e), Ok(_)) => {
                        corpus_we_fail += 1;
                        let err_msg = alloc::format!("{}", e).replace('"', "'");
                        alloc::format!(
                            "{{\"path\":\"{}\",\"corpus\":\"{}\",\"result\":\"we_fail\",\"error\":\"{}\"}}",
                            path_str,
                            corpus_name,
                            err_msg
                        )
                    }
                    (Ok(_), Err(_)) => {
                        corpus_tested += 1;
                        alloc::format!(
                            "{{\"path\":\"{}\",\"corpus\":\"{}\",\"result\":\"ok_ref_fail\"}}",
                            path_str,
                            corpus_name
                        )
                    }
                    (Err(_), Err(_)) => {
                        corpus_both_err += 1;
                        alloc::format!(
                            "{{\"path\":\"{}\",\"corpus\":\"{}\",\"result\":\"both_fail\"}}",
                            path_str,
                            corpus_name
                        )
                    }
                };

                let _ = writeln!(results_file, "{}", result_str);
            }

            eprintln!(
                "  {} ok, {} we-fail, {} both-err",
                corpus_tested, corpus_we_fail, corpus_both_err
            );
            total_tested += corpus_tested;
            total_we_fail += corpus_we_fail;
            total_both_err += corpus_both_err;
        }

        drop(results_file);

        eprintln!(
            "\n=== TOTAL: {} ok, {} we-fail, {} both-err, {} pixel-mismatch, {} timeout, {} skipped(resumed) ===",
            total_tested,
            total_we_fail,
            total_both_err,
            total_pixel_mismatch,
            total_timeout,
            total_skipped
        );
        eprintln!("Results written to {}", results_path.display());

        if total_pixel_mismatch > 0 {
            panic!(
                "{} pixel mismatches (see {})",
                total_pixel_mismatch,
                results_path.display()
            );
        }
    }

    /// Targeted test: compare zenflate batch vs streaming decompressor.
    #[test]
    #[ignore]
    fn streaming_vs_batch_decompressor() {
        use enough::Unstoppable;

        let cb = std::env::var("CORPUS_BUILDER_OUTPUT_DIR")
            .unwrap_or_else(|_| "/mnt/v/output/corpus-builder".to_string());
        let test_files = [
            format!("{cb}/png-8/g8_dff1977698eea27b.png"),
            format!("{cb}/png-8/g8_f977635ec6266135.png"),
            format!("{cb}/png-8/google_hudsonvalleyseed_com_56490ab04e5742ee.png"),
        ];

        for path in &test_files {
            let data = match std::fs::read(path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Skipping {}: {}", path, e);
                    continue;
                }
            };

            // Extract IHDR and concatenated IDAT data
            let mut pos = 8usize;
            let mut idat_data = Vec::new();
            let mut ihdr_data = None;
            while pos + 12 <= data.len() {
                let length = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
                let chunk_type: [u8; 4] = data[pos + 4..pos + 8].try_into().unwrap();
                let data_start = pos + 8;
                let Some(data_end) = data_start.checked_add(length) else {
                    break;
                };
                let Some(crc_end) = data_end.checked_add(4) else {
                    break;
                };
                if crc_end > data.len() {
                    break;
                }
                if &chunk_type == b"IHDR" {
                    ihdr_data = Some(data[data_start..data_end].to_vec());
                } else if &chunk_type == b"IDAT" {
                    idat_data.extend_from_slice(&data[data_start..data_end]);
                } else if &chunk_type == b"IEND" {
                    break;
                }
                pos = crc_end;
            }

            let ihdr = ihdr_data.unwrap();
            let w = u32::from_be_bytes(ihdr[0..4].try_into().unwrap());
            let h = u32::from_be_bytes(ihdr[4..8].try_into().unwrap());
            let depth = ihdr[8];
            let ctype = ihdr[9];
            let channels: usize = match ctype {
                0 => 1,
                2 => 3,
                3 => 1,
                4 => 2,
                6 => 4,
                _ => panic!("unknown color type"),
            };
            let stride = 1 + w as usize * channels * (depth as usize / 8).max(1);
            let expected_size = stride * h as usize;

            eprintln!(
                "\n{}: {}x{} depth={} ctype={}, IDAT={} bytes, expected decompressed={}",
                path,
                w,
                h,
                depth,
                ctype,
                idat_data.len(),
                expected_size
            );

            // Test 1: Batch decompressor (whole-buffer)
            let mut output = vec![0u8; expected_size + 65536]; // extra space
            let mut batch = zenflate::Decompressor::new();
            let batch_result = batch.zlib_decompress(&idat_data, &mut output, Unstoppable);
            match &batch_result {
                Ok(outcome) => eprintln!("  Batch: OK, {} bytes output", outcome.output_written),
                Err(e) => eprintln!("  Batch: FAILED: {:?}", e),
            }

            // Test 2: Streaming decompressor
            struct SliceSource<'a> {
                data: &'a [u8],
                pos: usize,
            }
            impl zenflate::InputSource for SliceSource<'_> {
                type Error = std::io::Error;
                fn fill_buf(&mut self) -> Result<&[u8], Self::Error> {
                    Ok(&self.data[self.pos..])
                }
                fn consume(&mut self, amt: usize) {
                    self.pos += amt;
                }
            }

            let source = SliceSource {
                data: &idat_data,
                pos: 0,
            };
            let mut stream = zenflate::StreamDecompressor::zlib(source, stride * 2);
            let mut total_output = 0usize;
            let mut stream_err = None;
            loop {
                if stream.is_done() {
                    break;
                }
                match stream.fill() {
                    Ok(_) => {}
                    Err(e) => {
                        stream_err = Some(alloc::format!("{:?}", e));
                        break;
                    }
                }
                let peeked = stream.peek().len();
                total_output += peeked;
                stream.advance(peeked);
            }
            match &stream_err {
                None => eprintln!("  Stream(zlib): OK, {} bytes output", total_output),
                Some(e) => eprintln!("  Stream(zlib): FAILED at {} bytes: {}", total_output, e),
            }

            // Test 2b: Raw deflate streaming (skip zlib header 2 bytes, footer 4 bytes)
            let raw_deflate = &idat_data[2..idat_data.len() - 4];
            let source = SliceSource {
                data: raw_deflate,
                pos: 0,
            };
            let mut stream = zenflate::StreamDecompressor::deflate(source, stride * 2);
            let mut total_output = 0usize;
            let mut stream_err = None;
            loop {
                if stream.is_done() {
                    break;
                }
                match stream.fill() {
                    Ok(_) => {}
                    Err(e) => {
                        stream_err = Some(alloc::format!("{:?}", e));
                        break;
                    }
                }
                let peeked = stream.peek().len();
                total_output += peeked;
                stream.advance(peeked);
            }
            match &stream_err {
                None => eprintln!("  Stream(raw): OK, {} bytes output", total_output),
                Some(e) => eprintln!("  Stream(raw): FAILED at {} bytes: {}", total_output, e),
            }

            // Test 3: Streaming with small chunk source (simulates IDAT chunks)
            struct ChunkedSource<'a> {
                data: &'a [u8],
                pos: usize,
                chunk_size: usize,
            }
            impl zenflate::InputSource for ChunkedSource<'_> {
                type Error = std::io::Error;
                fn fill_buf(&mut self) -> Result<&[u8], Self::Error> {
                    let end = (self.pos + self.chunk_size).min(self.data.len());
                    Ok(&self.data[self.pos..end])
                }
                fn consume(&mut self, amt: usize) {
                    self.pos += amt;
                }
            }

            for chunk_size in [256, 512, 1024, 4096, 8192] {
                let source = ChunkedSource {
                    data: &idat_data,
                    pos: 0,
                    chunk_size,
                };
                let mut stream = zenflate::StreamDecompressor::zlib(source, stride * 2);
                let mut total = 0usize;
                let mut err = None;
                loop {
                    if stream.is_done() {
                        break;
                    }
                    match stream.fill() {
                        Ok(_) => {}
                        Err(e) => {
                            err = Some(alloc::format!("{:?}", e));
                            break;
                        }
                    }
                    let p = stream.peek().len();
                    total += p;
                    stream.advance(p);
                }
                match &err {
                    None => eprintln!("  Stream(chunk={}): OK, {} bytes", chunk_size, total),
                    Some(e) => eprintln!(
                        "  Stream(chunk={}): FAILED at {} bytes: {}",
                        chunk_size, total, e
                    ),
                }
            }

            // Verify batch succeeded
            assert!(
                batch_result.is_ok(),
                "Batch decompressor should succeed on {}",
                path
            );
        }
    }

    /// Compare pixel output for a specific file that showed a pixel mismatch.
    #[test]
    #[ignore]
    fn debug_pixel_mismatch() {
        let cb = std::env::var("CORPUS_BUILDER_OUTPUT_DIR")
            .unwrap_or_else(|_| "/mnt/v/output/corpus-builder".to_string());
        let path = format!(
            "{cb}/repro-images/image-rs_image-png/346/177061203-3c6b1002-fb61-4f86-97f6-f0470cb03d84.png"
        );
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Skipping: {}", e);
                return;
            }
        };

        let our = decode_png(
            &data,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();
        let our_bytes = pixel_data_to_bytes(&our.pixels);
        eprintln!(
            "Our format: {}, {} bytes",
            format_pixel_data(&our.pixels),
            our_bytes.len()
        );
        eprintln!(
            "Our first 32 bytes: {:?}",
            &our_bytes[..32.min(our_bytes.len())]
        );

        let reference = decode_with_png_crate(&data).unwrap();
        let ref_bytes = pixel_data_to_bytes(&reference.pixels);
        eprintln!(
            "Ref format: {}, {} bytes",
            format_pixel_data(&reference.pixels),
            ref_bytes.len()
        );
        eprintln!(
            "Ref first 32 bytes: {:?}",
            &ref_bytes[..32.min(ref_bytes.len())]
        );

        let mut diffs = 0;
        for (i, (a, b)) in our_bytes.iter().zip(ref_bytes.iter()).enumerate() {
            if a != b {
                if diffs < 20 {
                    let px = i / 4;
                    let ch = ["R", "G", "B", "A"][i % 4];
                    eprintln!(
                        "  diff byte {}: pixel {} {}: ours={} ref={}",
                        i, px, ch, a, b
                    );
                }
                diffs += 1;
            }
        }
        eprintln!("Total diffs: {} of {} bytes", diffs, our_bytes.len());
        assert_eq!(diffs, 0, "Pixel data should match");
    }

    /// Compare streaming vs batch decompressor output byte-by-byte.
    #[test]
    #[ignore]
    fn debug_streaming_divergence() {
        let cb = std::env::var("CORPUS_BUILDER_OUTPUT_DIR")
            .unwrap_or_else(|_| "/mnt/v/output/corpus-builder".to_string());
        let path = format!("{cb}/png-8/wm_upload_wikimedia_org_13efdd48e85b970e.png");
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Skipping: {}", e);
                return;
            }
        };

        // Extract IDAT data
        let mut pos = 8usize;
        let mut idat_data = Vec::new();
        while pos + 12 <= data.len() {
            let length = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
            let chunk_type: [u8; 4] = data[pos + 4..pos + 8].try_into().unwrap();
            let data_start = pos + 8;
            let Some(data_end) = data_start.checked_add(length) else {
                break;
            };
            let Some(crc_end) = data_end.checked_add(4) else {
                break;
            };
            if crc_end > data.len() {
                break;
            }
            if &chunk_type == b"IDAT" {
                idat_data.extend_from_slice(&data[data_start..data_end]);
            } else if &chunk_type == b"IEND" {
                break;
            }
            pos = crc_end;
        }
        eprintln!("IDAT: {} bytes compressed", idat_data.len());

        // Batch decompress (reference)
        let mut batch_output = vec![0u8; 128 * 1024 * 1024];
        let mut batch = zenflate::Decompressor::new();
        let batch_result =
            batch.zlib_decompress(&idat_data, &mut batch_output, enough::Unstoppable);
        let batch_len = match &batch_result {
            Ok(outcome) => {
                eprintln!("Batch: OK, {} bytes", outcome.output_written);
                outcome.output_written
            }
            Err(e) => {
                eprintln!("Batch: FAILED {:?}", e);
                let last_nonzero = batch_output.iter().rposition(|&b| b != 0).unwrap_or(0);
                eprintln!("  Batch wrote up to ~{} bytes before failure", last_nonzero);
                last_nonzero + 1
            }
        };

        // Streaming decompress
        struct SliceSource<'a> {
            data: &'a [u8],
            pos: usize,
        }
        impl zenflate::InputSource for SliceSource<'_> {
            type Error = std::io::Error;
            fn fill_buf(&mut self) -> Result<&[u8], Self::Error> {
                Ok(&self.data[self.pos..])
            }
            fn consume(&mut self, amt: usize) {
                self.pos += amt;
            }
        }

        let source = SliceSource {
            data: &idat_data,
            pos: 0,
        };
        let mut stream = zenflate::StreamDecompressor::zlib(source, 65536);
        let mut stream_output = Vec::with_capacity(batch_len);
        let mut stream_err = None;
        loop {
            if stream.is_done() {
                break;
            }
            match stream.fill() {
                Ok(_) => {}
                Err(e) => {
                    stream_err = Some(alloc::format!("{:?}", e));
                    break;
                }
            }
            let peeked = stream.peek();
            stream_output.extend_from_slice(peeked);
            stream.advance(peeked.len());
        }
        eprintln!(
            "Stream: {} bytes, err={:?}",
            stream_output.len(),
            stream_err
        );

        // Compare byte-by-byte
        let cmp_len = batch_len.min(stream_output.len());
        let mut first_diff = None;
        for i in 0..cmp_len {
            if batch_output[i] != stream_output[i] {
                first_diff = Some(i);
                break;
            }
        }

        if let Some(diff_pos) = first_diff {
            eprintln!("\nFIRST DIVERGENCE at byte {}", diff_pos);
            eprintln!(
                "  batch[{}..]: {:?}",
                diff_pos,
                &batch_output[diff_pos..diff_pos + 20.min(batch_len - diff_pos)]
            );
            eprintln!(
                "  stream[{}..]: {:?}",
                diff_pos,
                &stream_output[diff_pos..diff_pos + 20.min(stream_output.len() - diff_pos)]
            );
            let stride = 12316;
            let row = diff_pos / stride;
            let col = diff_pos % stride;
            eprintln!("  Row {}, col {} (stride={})", row, col, stride);
        } else if batch_len != stream_output.len() {
            eprintln!(
                "Same up to {} bytes, but lengths differ: batch={} stream={}",
                cmp_len,
                batch_len,
                stream_output.len()
            );
        } else {
            eprintln!("IDENTICAL: {} bytes match perfectly", batch_len);
        }
    }

    /// Test all previously-failing corpus files after fastloop refill fix.
    #[test]
    #[ignore]
    fn debug_remaining_we_fail() {
        let cb = std::env::var("CORPUS_BUILDER_OUTPUT_DIR")
            .unwrap_or_else(|_| "/mnt/v/output/corpus-builder".to_string());
        let files = [
            format!("{cb}/png-8/wm_upload_wikimedia_org_13efdd48e85b970e.png"),
            format!("{cb}/png-24-32/wm_upload_wikimedia_org_45634e241d7821a3.png"),
            format!("{cb}/png-24-32/wm_upload_wikimedia_org_72ec2889934b6b15.png"),
            format!("{cb}/png-24-32/wm_upload_wikimedia_org_a119af42024ad225.png"),
            format!("{cb}/png-24-32/wm_upload_wikimedia_org_a23d1e831e128dff.png"),
            format!("{cb}/png-24-32/wm_upload_wikimedia_org_c8a458b0cef3d942.png"),
            format!(
                "{cb}/repro-images/libvips_libvips/1567/76076711-703ab580-5fb0-11ea-8562-27cd30e8e653.png"
            ),
            format!(
                "{cb}/repro-images/libvips_libvips/3123/199550258-f3b4ad36-10f2-47d8-af1f-9972b62b99be.png"
            ),
            format!(
                "{cb}/repro-images/libvips_libvips/3144/200133882-34bc8d61-4dbd-42de-a88a-0eaa1dae99ac.png"
            ),
        ];
        let mut failures = Vec::new();
        for path in &files {
            let name = path.rsplit('/').next().unwrap();
            let data = match std::fs::read(path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("{name}: SKIP ({e})");
                    continue;
                }
            };
            let our = decode_png(
                &data,
                &crate::decode::PngDecodeConfig::none(),
                &enough::Unstoppable,
            );
            let reference = decode_with_png_crate(&data);
            match (&our, &reference) {
                (Ok(o), Ok(r)) => {
                    let ob = pixel_data_to_bytes(&o.pixels);
                    let rb = pixel_data_to_bytes(&r.pixels);
                    if ob == rb {
                        eprintln!("{name}: OK ({} bytes)", ob.len());
                    } else {
                        eprintln!(
                            "{name}: PIXEL MISMATCH (ours={} ref={})",
                            ob.len(),
                            rb.len()
                        );
                        failures.push(name);
                    }
                }
                (Err(e), Ok(_)) => {
                    eprintln!("{name}: WE FAIL: {e}");
                    failures.push(name);
                }
                (Ok(_), Err(e)) => {
                    eprintln!("{name}: THEY FAIL: {e}");
                }
                (Err(e1), Err(e2)) => {
                    eprintln!("{name}: BOTH FAIL: us={e1} them={e2}");
                }
            }
        }
        assert!(failures.is_empty(), "Failures: {failures:?}");
    }

    /// Craft a minimal valid 1×1 RGBA8 PNG and return its bytes.
    fn craft_valid_1x1_png() -> Vec<u8> {
        let img = imgref::ImgVec::new(
            vec![rgb::Rgba {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }],
            1,
            1,
        );
        crate::encode::encode_rgba8(
            img.as_ref(),
            None,
            &crate::encode::EncodeConfig::default(),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap()
    }

    /// Corrupt a specific chunk's CRC in a PNG file.
    fn corrupt_chunk_crc(png: &[u8], target_type: &[u8; 4]) -> Vec<u8> {
        let mut result = png.to_vec();
        let mut pos = 8; // skip signature
        while pos + 12 <= result.len() {
            let length = u32::from_be_bytes(result[pos..pos + 4].try_into().unwrap()) as usize;
            let chunk_type: [u8; 4] = result[pos + 4..pos + 8].try_into().unwrap();
            let Some(crc_start) = (pos + 8).checked_add(length) else {
                break;
            };
            let Some(crc_end) = crc_start.checked_add(4) else {
                break;
            };
            if crc_end > result.len() {
                break;
            }
            if &chunk_type == target_type {
                // Flip a bit in the CRC
                result[crc_start] ^= 0x01;
                return result;
            }
            pos = crc_end;
        }
        panic!(
            "chunk type {:?} not found",
            core::str::from_utf8(target_type)
        );
    }

    /// Corrupt the zlib Adler-32 checksum inside IDAT data.
    fn corrupt_idat_adler(png: &[u8]) -> Vec<u8> {
        let mut result = png.to_vec();
        let mut last_idat_data_end = None;
        let mut pos = 8;
        while pos + 12 <= result.len() {
            let length = u32::from_be_bytes(result[pos..pos + 4].try_into().unwrap()) as usize;
            let chunk_type: [u8; 4] = result[pos + 4..pos + 8].try_into().unwrap();
            let data_start = pos + 8;
            let Some(data_end) = data_start.checked_add(length) else {
                break;
            };
            let Some(crc_end) = data_end.checked_add(4) else {
                break;
            };
            if crc_end > result.len() {
                break;
            }
            if chunk_type == *b"IDAT" && length > 0 {
                last_idat_data_end = Some(data_end);
            }
            pos = crc_end;
        }
        let data_end = last_idat_data_end.expect("no IDAT with data");
        result[data_end - 1] ^= 0x01;
        // Re-compute the IDAT chunk's CRC
        pos = 8;
        while pos + 12 <= result.len() {
            let length = u32::from_be_bytes(result[pos..pos + 4].try_into().unwrap()) as usize;
            let chunk_type: [u8; 4] = result[pos + 4..pos + 8].try_into().unwrap();
            let data_start = pos + 8;
            let Some(data_end_chunk) = data_start.checked_add(length) else {
                break;
            };
            let crc_start = data_end_chunk;
            let Some(crc_end) = crc_start.checked_add(4) else {
                break;
            };
            if crc_end > result.len() {
                break;
            }
            if chunk_type == *b"IDAT" && data_end_chunk == data_end {
                let crc = zenflate::crc32(
                    zenflate::crc32(0, &chunk_type),
                    &result[data_start..data_end_chunk],
                );
                result[crc_start..crc_end].copy_from_slice(&crc.to_be_bytes());
                break;
            }
            pos = crc_end;
        }
        result
    }

    #[test]
    fn corrupt_ihdr_crc_rejected_with_strict() {
        let png = craft_valid_1x1_png();
        let corrupt = corrupt_chunk_crc(&png, b"IHDR");
        let result = decode_png(
            &corrupt,
            &crate::decode::PngDecodeConfig::strict(),
            &Unstoppable,
        );
        assert!(result.is_err(), "corrupt IHDR CRC should be rejected");
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("CRC mismatch"), "error: {err_msg}");
    }

    #[test]
    fn corrupt_ihdr_crc_accepted_by_default() {
        let png = craft_valid_1x1_png();
        let corrupt = corrupt_chunk_crc(&png, b"IHDR");
        let result = decode_png(
            &corrupt,
            &crate::decode::PngDecodeConfig::none(),
            &Unstoppable,
        );
        assert!(
            result.is_ok(),
            "corrupt IHDR CRC should be accepted by default: {:?}",
            result.err()
        );
    }

    #[test]
    fn corrupt_idat_crc_rejected_with_strict() {
        let png = craft_valid_1x1_png();
        let corrupt = corrupt_chunk_crc(&png, b"IDAT");
        let result = decode_png(
            &corrupt,
            &crate::decode::PngDecodeConfig::strict(),
            &Unstoppable,
        );
        assert!(result.is_err(), "corrupt IDAT CRC should be rejected");
    }

    #[test]
    fn corrupt_idat_crc_accepted_by_default() {
        let png = craft_valid_1x1_png();
        let corrupt = corrupt_chunk_crc(&png, b"IDAT");
        let result = decode_png(
            &corrupt,
            &crate::decode::PngDecodeConfig::none(),
            &Unstoppable,
        );
        assert!(
            result.is_ok(),
            "corrupt IDAT CRC should be accepted by default: {:?}",
            result.err()
        );
    }

    #[test]
    fn corrupt_adler32_rejected_with_strict() {
        let png = craft_valid_1x1_png();
        let corrupt = corrupt_idat_adler(&png);
        let result = decode_png(
            &corrupt,
            &crate::decode::PngDecodeConfig::strict(),
            &Unstoppable,
        );
        assert!(
            result.is_err(),
            "corrupt Adler-32 should be rejected with strict"
        );
    }

    #[test]
    fn corrupt_adler32_accepted_by_default() {
        let png = craft_valid_1x1_png();
        let corrupt = corrupt_idat_adler(&png);
        let result = decode_png(
            &corrupt,
            &crate::decode::PngDecodeConfig::none(),
            &Unstoppable,
        );
        assert!(
            result.is_ok(),
            "corrupt Adler-32 should be accepted by default: {:?}",
            result.err()
        );
    }

    #[test]
    fn default_accepts_all_corruption() {
        let png = craft_valid_1x1_png();

        // Corrupt both IHDR CRC and Adler-32
        let mut corrupt = corrupt_chunk_crc(&png, b"IHDR");
        corrupt = corrupt_idat_adler(&corrupt);

        let result = decode_png(
            &corrupt,
            &crate::decode::PngDecodeConfig::none(),
            &Unstoppable,
        );
        assert!(
            result.is_ok(),
            "default should accept all corruption: {:?}",
            result.err()
        );
    }

    #[test]
    fn badadler_fixture_rejected_with_strict() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/regression/badadler.png");
        let data = std::fs::read(&path).unwrap();
        let result = decode_png(
            &data,
            &crate::decode::PngDecodeConfig::strict(),
            &Unstoppable,
        );
        assert!(
            result.is_err(),
            "badadler.png should be rejected with strict"
        );
    }

    #[test]
    fn badadler_fixture_accepted_by_default() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/regression/badadler.png");
        let data = std::fs::read(&path).unwrap();
        let result = decode_png(&data, &crate::decode::PngDecodeConfig::none(), &Unstoppable);
        assert!(
            result.is_ok(),
            "badadler.png should decode by default (checksums skipped): {:?}",
            result.err()
        );
        let output = result.unwrap();
        assert!(
            output
                .warnings
                .contains(&crate::decode::PngWarning::DecompressionChecksumSkipped),
            "should have DecompressionChecksumSkipped warning"
        );
        // Verify we actually got pixel data
        assert!(output.info.width > 0 && output.info.height > 0);
    }

    #[test]
    fn valid_png_no_decode_warnings() {
        let png = craft_valid_1x1_png();
        let result = decode_png(&png, &crate::decode::PngDecodeConfig::none(), &Unstoppable);
        let output = result.unwrap();
        assert!(
            output.warnings.is_empty(),
            "valid PNG should have no warnings, got: {:?}",
            output.warnings,
        );
    }

    // ── Scalar dispatch tests (archmage for_each_token_permutation) ──

    #[cfg(feature = "_dev")]
    #[test]
    fn decode_scalar_regression_pngs() {
        use archmage::testing::{CompileTimePolicy, for_each_token_permutation};

        // Load regression test PNGs
        let regression_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("regression");
        if !regression_dir.exists() {
            eprintln!("Skipping: {} not found", regression_dir.display());
            return;
        }

        // Collect all PNG files
        let mut png_files: Vec<std::path::PathBuf> = Vec::new();
        for entry in std::fs::read_dir(&regression_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) == Some("png") {
                png_files.push(path);
            }
        }
        assert!(
            !png_files.is_empty(),
            "no test PNGs found in {}",
            regression_dir.display()
        );

        // Read all files once
        let test_data: Vec<(String, Vec<u8>)> = png_files
            .iter()
            .map(|p| {
                let name = p.file_name().unwrap().to_str().unwrap().to_string();
                let data = std::fs::read(p).unwrap();
                (name, data)
            })
            .collect();

        let report = for_each_token_permutation(CompileTimePolicy::Warn, |perm| {
            for (name, data) in &test_data {
                let is_apng = name.starts_with("apng_");
                if is_apng {
                    // Decode as APNG
                    match crate::decode::decode_apng(
                        data,
                        &crate::decode::PngDecodeConfig::none(),
                        &Unstoppable,
                    ) {
                        Ok(result) => {
                            assert!(
                                !result.frames.is_empty(),
                                "{name}: APNG decoded 0 frames (perm {perm:?})"
                            );
                        }
                        Err(e) => {
                            panic!("{name}: APNG decode failed (perm {perm:?}): {e}");
                        }
                    }
                } else {
                    // Decode as static PNG
                    match decode_png(data, &crate::decode::PngDecodeConfig::none(), &Unstoppable) {
                        Ok(output) => {
                            assert!(
                                output.info.width > 0 && output.info.height > 0,
                                "{name}: decoded 0×0 (perm {perm:?})"
                            );
                        }
                        Err(e) => {
                            // badadler.png is expected to fail with strict decode
                            if !name.contains("badadler") {
                                panic!("{name}: decode failed (perm {perm:?}): {e}");
                            }
                        }
                    }
                }
            }
        });
        eprintln!("decode scalar regression: {report}");
    }

    #[cfg(feature = "_dev")]
    #[test]
    fn encode_decode_roundtrip_all_tiers() {
        use archmage::testing::{CompileTimePolicy, for_each_token_permutation};

        // Create a small test image with patterns that exercise all filter types
        let mut pixels = Vec::new();
        for y in 0..8u8 {
            for x in 0..8u8 {
                pixels.push(Rgba {
                    r: x.wrapping_mul(31),
                    g: y.wrapping_mul(37),
                    b: x.wrapping_add(y).wrapping_mul(17),
                    a: 255,
                });
            }
        }
        let img = ImgVec::new(pixels, 8, 8);

        // Encode once (at default effort)
        let encoded = crate::encode::encode_rgba8(
            img.as_ref(),
            None,
            &crate::encode::EncodeConfig::default(),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();

        let report = for_each_token_permutation(CompileTimePolicy::Warn, |perm| {
            // Decode with each SIMD tier permutation
            let result = decode_png(
                &encoded,
                &crate::decode::PngDecodeConfig::none(),
                &Unstoppable,
            );
            let output = result.unwrap_or_else(|e| {
                panic!("decode failed (perm {perm:?}): {e}");
            });
            assert_eq!(output.info.width, 8);
            assert_eq!(output.info.height, 8);
        });
        eprintln!("encode-decode roundtrip tiers: {report}");
    }

    // ── Decode error paths ──────────────────────────────────────────

    #[test]
    fn decode_empty_data_errors() {
        let result = decode_png(&[], &crate::decode::PngDecodeConfig::none(), &Unstoppable);
        assert!(result.is_err());
    }

    #[test]
    fn decode_truncated_signature_errors() {
        let result = decode_png(
            &[0x89, 0x50, 0x4E],
            &crate::decode::PngDecodeConfig::none(),
            &Unstoppable,
        );
        assert!(result.is_err());
    }

    #[test]
    fn decode_valid_signature_but_no_ihdr() {
        let result = decode_png(
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
            &crate::decode::PngDecodeConfig::none(),
            &Unstoppable,
        );
        assert!(result.is_err());
    }

    #[test]
    fn probe_empty_data_errors() {
        let result = probe_png(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn probe_valid_png_returns_info() {
        // Encode a small image and probe it
        let pixels: Vec<Rgb<u8>> = vec![Rgb { r: 0, g: 0, b: 0 }; 4];
        let img = imgref::Img::new(pixels, 2, 2);
        let encoded = crate::encode::encode_rgb8(
            img.as_ref(),
            None,
            &crate::encode::EncodeConfig::default()
                .with_compression(crate::types::Compression::Fastest),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let info = probe_png(&encoded).unwrap();
        assert_eq!(info.width, 2);
        assert_eq!(info.height, 2);
    }

    // ── 16-bit encode+decode roundtrips for all color types ─────────

    #[test]
    fn roundtrip_gray16_strict() {
        // Use values with nonzero low byte to prevent 16→8 reduction
        let pixels: Vec<rgb::Gray<u16>> = (0..16).map(|i| rgb::Gray(i * 4096 + 1)).collect();
        let img = imgref::Img::new(pixels.clone(), 4, 4);
        let encoded = crate::encode::encode_gray16(
            img.as_ref(),
            None,
            &crate::encode::EncodeConfig::default()
                .with_compression(crate::types::Compression::Balanced),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let decoded = decode_png(
            &encoded,
            &crate::decode::PngDecodeConfig::strict(),
            &Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 4);
        assert_eq!(decoded.info.height, 4);
        assert_eq!(decoded.info.bit_depth, 16);
        assert!(!decoded.info.has_alpha);
    }

    #[test]
    fn roundtrip_rgb16_strict() {
        let pixels: Vec<Rgb<u16>> = (0..16)
            .map(|i| Rgb {
                r: i * 4096 + 1,
                g: i * 2048 + 3,
                b: i * 1024 + 7,
            })
            .collect();
        let img = imgref::Img::new(pixels, 4, 4);
        let encoded = crate::encode::encode_rgb16(
            img.as_ref(),
            None,
            &crate::encode::EncodeConfig::default()
                .with_compression(crate::types::Compression::Balanced),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let decoded = decode_png(
            &encoded,
            &crate::decode::PngDecodeConfig::strict(),
            &Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 4);
        assert_eq!(decoded.info.bit_depth, 16);
    }

    #[test]
    fn roundtrip_rgba16_strict() {
        let pixels: Vec<Rgba<u16>> = (0..16)
            .map(|i| Rgba {
                r: i * 4096 + 1,
                g: i * 2048 + 3,
                b: i * 1024 + 7,
                a: 65535 - i * 1000,
            })
            .collect();
        let img = imgref::Img::new(pixels, 4, 4);
        let encoded = crate::encode::encode_rgba16(
            img.as_ref(),
            None,
            &crate::encode::EncodeConfig::default()
                .with_compression(crate::types::Compression::Balanced),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let decoded = decode_png(
            &encoded,
            &crate::decode::PngDecodeConfig::strict(),
            &Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 4);
        assert_eq!(decoded.info.bit_depth, 16);
        assert!(decoded.info.has_alpha);
    }

    // ── All 8-bit color types roundtrip ─────────────────────────────

    #[test]
    fn roundtrip_gray8_strict() {
        let pixels: Vec<Gray<u8>> = (0..16).map(|i| Gray((i * 16) as u8)).collect();
        let img = imgref::Img::new(pixels, 4, 4);
        let encoded = crate::encode::encode_gray8(
            img.as_ref(),
            None,
            &crate::encode::EncodeConfig::default()
                .with_compression(crate::types::Compression::Balanced),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let decoded = decode_png(
            &encoded,
            &crate::decode::PngDecodeConfig::strict(),
            &Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 4);
        assert_eq!(decoded.info.bit_depth, 8);
    }

    #[test]
    fn roundtrip_rgb8_strict() {
        let pixels: Vec<Rgb<u8>> = (0..16)
            .map(|i| Rgb {
                r: (i * 16) as u8,
                g: (i * 8) as u8,
                b: (i * 4) as u8,
            })
            .collect();
        let img = imgref::Img::new(pixels, 4, 4);
        let encoded = crate::encode::encode_rgb8(
            img.as_ref(),
            None,
            &crate::encode::EncodeConfig::default()
                .with_compression(crate::types::Compression::Balanced),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let decoded = decode_png(
            &encoded,
            &crate::decode::PngDecodeConfig::strict(),
            &Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 4);
    }

    #[test]
    fn roundtrip_rgba8_strict() {
        let pixels: Vec<Rgba<u8>> = (0..16)
            .map(|i| Rgba {
                r: (i * 16) as u8,
                g: (i * 8) as u8,
                b: (i * 4) as u8,
                a: 200,
            })
            .collect();
        let img = imgref::Img::new(pixels, 4, 4);
        let encoded = crate::encode::encode_rgba8(
            img.as_ref(),
            None,
            &crate::encode::EncodeConfig::default()
                .with_compression(crate::types::Compression::Balanced),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let decoded = decode_png(
            &encoded,
            &crate::decode::PngDecodeConfig::strict(),
            &Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 4);
        assert!(decoded.info.has_alpha);
    }

    // ── High effort encode+decode (exercises brute-force compress paths) ──

    #[test]
    fn roundtrip_rgb8_effort_24_intense() {
        let pixels: Vec<Rgb<u8>> = (0..64)
            .map(|i| Rgb {
                r: (i * 4) as u8,
                g: (i * 3) as u8,
                b: (i * 2) as u8,
            })
            .collect();
        let img = imgref::Img::new(pixels, 8, 8);
        let encoded = crate::encode::encode_rgb8(
            img.as_ref(),
            None,
            &crate::encode::EncodeConfig::default()
                .with_compression(crate::types::Compression::Intense),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let decoded = decode_png(
            &encoded,
            &crate::decode::PngDecodeConfig::strict(),
            &Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 8);
        assert_eq!(decoded.info.height, 8);
    }

    #[test]
    fn roundtrip_rgba8_effort_27_crush() {
        let pixels: Vec<Rgba<u8>> = (0..64)
            .map(|i| Rgba {
                r: (i * 4) as u8,
                g: (i * 3) as u8,
                b: (i * 2) as u8,
                a: if i % 3 == 0 { 0 } else { 255 },
            })
            .collect();
        let img = imgref::Img::new(pixels, 8, 8);
        let encoded = crate::encode::encode_rgba8(
            img.as_ref(),
            None,
            &crate::encode::EncodeConfig::default()
                .with_compression(crate::types::Compression::Crush),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let decoded = decode_png(
            &encoded,
            &crate::decode::PngDecodeConfig::strict(),
            &Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 8);
    }

    #[test]
    fn roundtrip_gray8_effort_30_maniac() {
        let pixels: Vec<Gray<u8>> = (0..64).map(|i| Gray((i * 4) as u8)).collect();
        let img = imgref::Img::new(pixels, 8, 8);
        let encoded = crate::encode::encode_gray8(
            img.as_ref(),
            None,
            &crate::encode::EncodeConfig::default()
                .with_compression(crate::types::Compression::Maniac),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let decoded = decode_png(
            &encoded,
            &crate::decode::PngDecodeConfig::strict(),
            &Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 8);
    }

    #[test]
    fn roundtrip_rgb16_effort_22_aggressive() {
        let pixels: Vec<Rgb<u16>> = (0..16)
            .map(|i| Rgb {
                r: i * 4096 + 1,
                g: i * 2048 + 3,
                b: i * 1024 + 7,
            })
            .collect();
        let img = imgref::Img::new(pixels, 4, 4);
        let encoded = crate::encode::encode_rgb16(
            img.as_ref(),
            None,
            &crate::encode::EncodeConfig::default()
                .with_compression(crate::types::Compression::Aggressive),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let decoded = decode_png(
            &encoded,
            &crate::decode::PngDecodeConfig::strict(),
            &Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 4);
    }

    // ── Decode config builder ───────────────────────────────────────

    #[test]
    fn decode_config_builders_chain() {
        let config = crate::decode::PngDecodeConfig::none()
            .with_max_pixels(1000)
            .with_max_memory(50000)
            .with_skip_decompression_checksum(false)
            .with_skip_critical_chunk_crc(false);
        assert_eq!(config.max_pixels, Some(1000));
        assert_eq!(config.max_memory_bytes, Some(50000));
        assert!(!config.skip_decompression_checksum);
        assert!(!config.skip_critical_chunk_crc);
    }

    // ── build_png_info covers metadata paths ────────────────────────

    #[test]
    fn build_png_info_with_cicp() {
        let ihdr = Ihdr::parse(&make_ihdr(8, 8, 8, 2, 0)).unwrap();
        let mut ancillary = crate::chunk::ancillary::PngAncillary::default();
        ancillary.cicp = Some([1, 13, 0, 1]);
        let info = build_png_info(&ihdr, &ancillary);
        assert!(info.cicp.is_some());
    }

    #[test]
    fn build_png_info_minimal() {
        let ihdr = Ihdr::parse(&make_ihdr(4, 4, 8, 6, 0)).unwrap();
        let ancillary = crate::chunk::ancillary::PngAncillary::default();
        let info = build_png_info(&ihdr, &ancillary);
        assert_eq!(info.width, 4);
        assert_eq!(info.height, 4);
        assert!(info.has_alpha);
        assert_eq!(info.sequence, zencodec::ImageSequence::Single);
        assert!(info.icc_profile.is_none());
        assert!(info.exif.is_none());
        assert!(info.xmp.is_none());
        assert!(info.source_gamma.is_none());
        assert!(info.srgb_intent.is_none());
        assert!(info.chromaticities.is_none());
        assert!(info.cicp.is_none());
    }
}
