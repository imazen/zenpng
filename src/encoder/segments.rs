//! `iDOT`-segmented IDAT output, so decoders can inflate horizontal strips in
//! parallel (Apple ImageIO, and zenpng's own decoder).
//!
//! The image data stays one standard zlib stream: segment boundaries are
//! byte-aligned full flushes, and every segment starts with an empty window,
//! so decoders that ignore `iDOT` read it normally. See
//! `docs/IDOT_PARALLEL_PNG.md`.
//!
//! The compression pipeline picks filters and produces one zlib stream; this
//! module then
//! 1. inflates that stream to recover the chosen filter bytes,
//! 2. re-filters each non-first segment's first row as None or Sub (whichever
//!    has the smaller absolute byte sum) when it used Up/Average/Paeth, since
//!    those reference the previous segment,
//! 3. compresses the segments in parallel with the pipeline's final zenflate
//!    level, and
//! 4. writes `iDOT` immediately before the first IDAT, one IDAT per segment.

use alloc::vec::Vec;

use enough::Stop;
#[allow(unused_imports)]
use whereat::at;
use zenflate::png::StripCompressor;

use crate::chunk::write::write_chunk;
use crate::decoder::idot::{MAX_SEGMENTS, workers_for_bytes};
use crate::error::PngError;

/// One segment's compression result.
type SegResult = Result<Vec<u8>, zenflate::CompressionError>;

/// IDAT payload: one zlib stream, or the same stream split into segments.
pub(crate) enum Idat {
    Single(Vec<u8>),
    Segmented {
        /// Row count of each segment.
        rows: Vec<u32>,
        /// Compressed parts; part 0 starts with the zlib header, the last ends
        /// with the Adler-32 trailer. Their concatenation is the zlib stream.
        parts: Vec<Vec<u8>>,
    },
}

impl Idat {
    /// Bytes this will add to the file (chunks included).
    pub(crate) fn file_len(&self) -> usize {
        match self {
            Idat::Single(z) => 12 + z.len(),
            Idat::Segmented { parts, .. } => {
                12 + 4 + 12 * parts.len() + parts.iter().map(|p| 12 + p.len()).sum::<usize>()
            }
        }
    }
}

/// How many segments to emit for a `requested` count. zenpng's decoder would
/// never use more workers than [`workers_for_bytes`] allows, so more segments
/// would only cost bytes; images under about 2 MiB of filtered data get none.
/// Most segments the encoder writes: the largest count verified to decode
/// identically through Apple ImageIO's parallel path (macOS 27). The decoder
/// accepts up to [`MAX_SEGMENTS`].
pub(crate) const MAX_WRITTEN_SEGMENTS: usize = 16;

pub(crate) fn plan(requested: u32, row_bytes: usize, height: usize) -> usize {
    if requested < 2 {
        return 1;
    }
    let filtered = (row_bytes + 1).saturating_mul(height);
    let n = (requested as usize)
        .min(height)
        .min(MAX_SEGMENTS)
        .min(MAX_WRITTEN_SEGMENTS)
        .min(workers_for_bytes(filtered));
    if n < 2 { 1 } else { n }
}

/// Row counts for `n` segments: as even as possible, larger ones first
/// (Apple splits an even height into two equal halves).
pub(crate) fn split_rows(height: usize, n: usize) -> Vec<u32> {
    let base = height / n;
    let extra = height % n;
    (0..n)
        .map(|k| (base + usize::from(k < extra)) as u32)
        .collect()
}

/// Re-split a finished zlib stream into `n` independently decodable segments.
#[allow(clippy::too_many_arguments)]
pub(crate) fn segment(
    zlib: Vec<u8>,
    row_bytes: usize,
    height: usize,
    bpp: usize,
    n: usize,
    effort: u32,
    max_threads: usize,
    cancel: &dyn Stop,
) -> crate::error::Result<Idat> {
    if n < 2 || zlib.len() < 6 {
        return Ok(Idat::Single(zlib));
    }
    let stride = row_bytes + 1;
    let total = stride * height;

    // 1. Recover the filtered bytes the pipeline chose.
    let mut filtered = alloc::vec![0u8; total];
    let got = zenflate::Decompressor::new()
        .zlib_decompress(&zlib, &mut filtered, zenflate::Unstoppable)
        .map_err(|e| {
            at!(PngError::Internal(
                zencodec::InternalKind::Bug,
                alloc::format!("re-inflating our own IDAT failed: {e:?}")
            ))
        })?;
    if got.output_written != total {
        return Ok(Idat::Single(zlib));
    }

    // 2. Unfilter on the fly to know each boundary row and its predecessor,
    //    and re-filter boundary rows that reference the previous segment.
    let rows = split_rows(height, n);
    let mut boundaries = Vec::with_capacity(n - 1);
    let mut acc = 0usize;
    for r in &rows[..n - 1] {
        acc += *r as usize;
        boundaries.push(acc);
    }
    let mut prev = alloc::vec![0u8; row_bytes];
    let mut cur = alloc::vec![0u8; row_bytes];
    let mut next_boundary = 0;
    for y in 0..height {
        let row = &filtered[y * stride..(y + 1) * stride];
        cur.copy_from_slice(&row[1..]);
        crate::simd::unfilter_row(row[0], &mut cur, &prev, bpp)?;
        if boundaries.get(next_boundary) == Some(&y) {
            next_boundary += 1;
            if row[0] > 1 {
                refilter_none_or_sub(&cur, bpp, &mut filtered[y * stride..(y + 1) * stride]);
            }
        }
        core::mem::swap(&mut prev, &mut cur);
        if y % 64 == 0 {
            cancel.check().map_err(|e| at!(PngError::from(e)))?;
        }
    }

    // 3. Compress segments in parallel.
    let level = super::compress::segment_level(effort);
    let mut slices = Vec::with_capacity(n);
    let mut start = 0;
    for r in &rows {
        let end = start + *r as usize * stride;
        slices.push(&filtered[start..end]);
        start = end;
    }
    let threads = if max_threads == 0 {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    } else {
        max_threads
    }
    .clamp(1, n);
    let compress_one = |c: &mut StripCompressor, k: usize, data: &[u8]| -> SegResult {
        let mut out = alloc::vec![0u8; StripCompressor::bound(data.len())];
        let len = c.compress(data, k + 1 == n, &mut out, cancel)?;
        out.truncate(len);
        Ok(out)
    };
    // One compressor per worker; strips don't depend on what it compressed
    // before, so the output is the same for any thread count.
    let mut main = StripCompressor::new(level);
    let header = main.zlib_header();
    let mut parts: Vec<Option<SegResult>> = (0..n).map(|_| None).collect();
    if threads <= 1 {
        for (k, d) in slices.iter().enumerate() {
            parts[k] = Some(compress_one(&mut main, k, d));
        }
    } else {
        drop(main);
        // Workers take segments round-robin by index.
        let results: Vec<Vec<(usize, SegResult)>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    let slices = &slices;
                    let compress_one = &compress_one;
                    s.spawn(move || {
                        let mut c = StripCompressor::new(level);
                        (t..n)
                            .step_by(threads)
                            .map(|k| (k, compress_one(&mut c, k, slices[k])))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_default())
                .collect()
        });
        for (k, r) in results.into_iter().flatten() {
            parts[k] = Some(r);
        }
    }
    let mut out_parts = Vec::with_capacity(n);
    for p in parts {
        match p {
            Some(Ok(v)) => out_parts.push(v),
            Some(Err(zenflate::CompressionError::Stopped(reason))) => {
                return Err(at!(PngError::Stopped(reason)));
            }
            Some(Err(e)) => {
                return Err(at!(PngError::Internal(
                    zencodec::InternalKind::Dependency,
                    alloc::format!("zenflate segment compression failed: {e}")
                )));
            }
            None => return Ok(Idat::Single(zlib)),
        }
    }

    // 4. zlib framing: the strip level's header, combined Adler-32 trailer.
    let adler = zenflate::adler32(1, &filtered);
    let mut first = Vec::with_capacity(2 + out_parts[0].len());
    first.extend_from_slice(&header);
    first.extend_from_slice(&out_parts[0]);
    out_parts[0] = first;
    out_parts[n - 1].extend_from_slice(&adler.to_be_bytes());

    Ok(Idat::Segmented {
        rows,
        parts: out_parts,
    })
}

/// Replace a filtered row (filter byte + data) with None or Sub, whichever has
/// the smaller sum of absolute signed bytes. `raw` is the unfiltered row.
pub(crate) fn refilter_none_or_sub(raw: &[u8], bpp: usize, dst: &mut [u8]) {
    let none_cost: u64 = raw.iter().map(|&b| (b as i8).unsigned_abs() as u64).sum();
    let sub_cost: u64 = raw
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            let left = if i >= bpp { raw[i - bpp] } else { 0 };
            (b.wrapping_sub(left) as i8).unsigned_abs() as u64
        })
        .sum();
    if none_cost <= sub_cost {
        dst[0] = 0;
        dst[1..].copy_from_slice(raw);
    } else {
        dst[0] = 1;
        for i in 0..raw.len() {
            let left = if i >= bpp { raw[i - bpp] } else { 0 };
            dst[1 + i] = raw[i].wrapping_sub(left);
        }
    }
}

/// Write the IDAT payload: one chunk, or `iDOT` followed by one IDAT per
/// segment. Falls back to plain IDAT chunks (no `iDOT`) if an offset would
/// not fit the chunk's 32-bit fields.
pub(crate) fn write_idat(out: &mut Vec<u8>, idat: &Idat) {
    match idat {
        Idat::Single(z) => write_chunk(out, b"IDAT", z),
        Idat::Segmented { rows, parts } => {
            let n = parts.len();
            let idot_chunk_len = 12 + 4 + 12 * n;
            let mut table = Vec::with_capacity(4 + 12 * n);
            table.extend_from_slice(&(n as u32).to_be_bytes());
            let mut offset = idot_chunk_len as u64;
            let mut first_row = 0u64;
            let mut fits = true;
            for (r, p) in rows.iter().zip(parts) {
                fits &= offset <= u32::MAX as u64;
                table.extend_from_slice(&(first_row as u32).to_be_bytes());
                table.extend_from_slice(&r.to_be_bytes());
                table.extend_from_slice(&(offset as u32).to_be_bytes());
                first_row += *r as u64;
                offset += 12 + p.len() as u64;
            }
            if fits {
                write_chunk(out, b"iDOT", &table);
            }
            for p in parts {
                write_chunk(out, b"IDAT", p);
            }
        }
    }
}
