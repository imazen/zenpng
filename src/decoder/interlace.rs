//! Adam7 interlacing support.

use alloc::borrow::Cow;
use alloc::vec;
use alloc::vec::Vec;

use enough::Stop;

use crate::chunk::ancillary::PngAncillary;
use crate::chunk::ihdr::Ihdr;
use crate::chunk::{ChunkIter, ChunkRef, PNG_SIGNATURE};
use crate::error::PngError;
#[allow(unused_imports)]
use whereat::at;

use super::postprocess::{OutputFormat, post_process_row};
use super::row::{IdatSource, unfilter_row};

// ── Adam7 interlacing ───────────────────────────────────────────────

/// Adam7 pass parameters: (x_offset, y_offset, x_step, y_step).
const ADAM7_PASSES: [(usize, usize, usize, usize); 7] = [
    (0, 0, 8, 8), // pass 1
    (4, 0, 8, 8), // pass 2
    (0, 4, 4, 8), // pass 3
    (2, 0, 4, 4), // pass 4
    (0, 2, 2, 4), // pass 5
    (1, 0, 2, 2), // pass 6
    (0, 1, 1, 2), // pass 7
];

/// Compute dimensions of an Adam7 sub-image for a given pass.
fn adam7_pass_size(width: u32, height: u32, pass: usize) -> (u32, u32) {
    let (x_off, y_off, x_step, y_step) = ADAM7_PASSES[pass];
    let w = if width as usize > x_off {
        (width as usize - x_off).div_ceil(x_step)
    } else {
        0
    };
    let h = if height as usize > y_off {
        (height as usize - y_off).div_ceil(y_step)
    } else {
        0
    };
    (w as u32, h as u32)
}

/// Decode an interlaced PNG: decompress all 7 passes, unfilter, scatter to final image,
/// then return the assembled pixel rows and decode warnings.
#[allow(clippy::type_complexity)]
pub(crate) fn decode_interlaced(
    data: &'_ [u8],
    config: &crate::decode::PngDecodeConfig,
    cancel: &dyn Stop,
) -> Result<
    (
        Ihdr,
        PngAncillary,
        Vec<u8>,
        OutputFormat,
        Vec<crate::decode::PngWarning>,
    ),
    whereat::At<PngError>,
> {
    // Validate signature
    if data.len() < 8 || data[..8] != PNG_SIGNATURE {
        return Err(at!(PngError::Decode("not a PNG file".into())));
    }

    let mut chunks = ChunkIter::new_with_config(data, config.skip_critical_chunk_crc);

    // Parse IHDR
    let ihdr_chunk = chunks
        .next()
        .ok_or_else(|| at!(PngError::Decode("empty PNG".into())))??;
    if ihdr_chunk.chunk_type != *b"IHDR" {
        return Err(at!(PngError::Decode("first chunk is not IHDR".into())));
    }
    let ihdr = Ihdr::parse(ihdr_chunk.data)?;

    // Collect pre-IDAT metadata
    let mut ancillary = PngAncillary::default();
    let mut first_idat_pos = None;
    for chunk_result in &mut chunks {
        let chunk = chunk_result?;
        if chunk.chunk_type == *b"IDAT" {
            first_idat_pos = Some(chunks.pos() - 12 - chunk.data.len());
            break;
        }
        ancillary.collect(&chunk)?;
    }

    // Collect warnings from chunk CRC validation
    let mut decode_warnings = chunks.warnings;

    let first_idat_pos =
        first_idat_pos.ok_or_else(|| at!(PngError::Decode("no IDAT chunk found".into())))?;

    if ihdr.is_indexed() && ancillary.palette.is_none() {
        return Err(at!(PngError::Decode(
            "indexed color type requires PLTE chunk".into(),
        )));
    }

    let fmt = OutputFormat::from_ihdr(&ihdr, &ancillary)?;

    let out_bpp = (fmt.channels * fmt.bytes_per_channel) as u32;
    config.validate(ihdr.width, ihdr.height, out_bpp)?;

    let bpp = ihdr.filter_bpp();
    let width = ihdr.width;
    let height = ihdr.height;

    // Allocate final output image
    let out_row_bytes = width as usize * fmt.channels * fmt.bytes_per_channel;
    let mut final_pixels = vec![0u8; out_row_bytes * height as usize];

    // Create IDAT source and decompressor.
    // The capacity must be at least as large as the widest pass stride,
    // otherwise the fill loop cannot accumulate a full row and spins forever.
    // Pass 7 (x_step=1) gives the widest rows: full image width.
    let max_pass_stride = ihdr.stride()?; // 1 + raw_row_bytes for full width
    let capacity = max_pass_stride * 2;
    let source = IdatSource::new(
        Cow::Borrowed(data),
        first_idat_pos,
        config.skip_critical_chunk_crc,
    );
    let mut decompressor = zenflate::StreamDecompressor::zlib(source, capacity)
        .with_skip_checksum(config.skip_decompression_checksum);

    // Process each Adam7 pass
    for (pass, &(x_off, y_off, x_step, y_step)) in ADAM7_PASSES.iter().enumerate() {
        let (pw, ph) = adam7_pass_size(width, height, pass);
        if pw == 0 || ph == 0 {
            continue;
        }

        // Compute stride for this sub-image
        let bits_per_row = pw as usize * ihdr.channels() * ihdr.bit_depth as usize;
        let raw_row_bytes = bits_per_row.div_ceil(8);
        let pass_stride = 1 + raw_row_bytes;

        let mut prev_row = vec![0u8; raw_row_bytes];
        let mut current_row = vec![0u8; raw_row_bytes];
        let mut row_buf = Vec::new();

        for pass_y in 0..ph as usize {
            cancel.check().map_err(|e| at!(PngError::from(e)))?;
            // Fill decompressor until we have a full stride
            loop {
                let available = decompressor.peek().len();
                if available >= pass_stride {
                    break;
                }
                if decompressor.is_done() {
                    return Err(at!(PngError::Decode(alloc::format!(
                        "truncated interlaced data in pass {}",
                        pass + 1
                    ))));
                }
                decompressor.fill().map_err(|e| {
                    at!(PngError::Decode(alloc::format!(
                        "decompression error: {e:?}"
                    )))
                })?;
            }

            let peeked = decompressor.peek();
            let filter_byte = peeked[0];
            current_row[..raw_row_bytes].copy_from_slice(&peeked[1..pass_stride]);
            decompressor.advance(pass_stride);

            unfilter_row(
                filter_byte,
                &mut current_row[..raw_row_bytes],
                &prev_row,
                bpp,
            )?;

            // Post-process this sub-image row
            // Create a temporary Ihdr with the sub-image width for post-processing
            let sub_ihdr = Ihdr {
                width: pw,
                height: ph,
                ..ihdr
            };
            post_process_row(
                &current_row[..raw_row_bytes],
                &sub_ihdr,
                &ancillary,
                &mut row_buf,
            );

            // Scatter pixels to final positions
            let pixel_bytes = fmt.channels * fmt.bytes_per_channel;
            let dest_y = y_off + pass_y * y_step;
            if dest_y < height as usize {
                for px in 0..pw as usize {
                    let dest_x = x_off + px * x_step;
                    if dest_x < width as usize {
                        let src_offset = px * pixel_bytes;
                        let dst_offset = dest_y * out_row_bytes + dest_x * pixel_bytes;
                        if src_offset + pixel_bytes <= row_buf.len()
                            && dst_offset + pixel_bytes <= final_pixels.len()
                        {
                            final_pixels[dst_offset..dst_offset + pixel_bytes]
                                .copy_from_slice(&row_buf[src_offset..src_offset + pixel_bytes]);
                        }
                    }
                }
            }

            core::mem::swap(&mut current_row, &mut prev_row);
        }
    }

    // Collect post-IDAT metadata: scan forward from first_idat_pos, skip IDATs
    {
        let mut pos = first_idat_pos;
        // Skip IDAT chunks
        while pos + 12 <= data.len() {
            let length = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
            let chunk_type: [u8; 4] = data[pos + 4..pos + 8].try_into().unwrap();
            let Some(crc_end) = (pos + 8).checked_add(length).and_then(|v| v.checked_add(4)) else {
                break;
            };
            if crc_end > data.len() {
                break;
            }
            if chunk_type != *b"IDAT" {
                break;
            }
            pos = crc_end;
        }
        // Collect late metadata
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
            if chunk_type == *b"IEND" {
                break;
            }
            let chunk_data = &data[data_start..data_end];
            ancillary.collect_late(&ChunkRef {
                chunk_type,
                data: chunk_data,
            });
            pos = crc_end;
        }
    }

    // Collect decompressor warnings
    if decompressor.checksum_matched() == Some(false) {
        decode_warnings.push(crate::decode::PngWarning::DecompressionChecksumSkipped);
    }

    Ok((ihdr, ancillary, final_pixels, fmt, decode_warnings))
}
