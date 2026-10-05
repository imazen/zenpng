//! Parallel decode of PNGs carrying Apple's `iDOT` segment table.
//!
//! `iDOT` (unregistered, ancillary, written by Apple software since ~2011)
//! records where the zlib stream can be split into horizontal strips that
//! inflate independently. Layout, verified on real Apple files — see
//! `docs/IDOT_PARALLEL_PNG.md`:
//!
//! ```text
//! u32 N                                   (big-endian)
//! N × { u32 first_row, u32 row_count, u32 offset }
//! ```
//!
//! `offset` is the byte offset of the segment's first IDAT chunk header,
//! measured from the first byte of the `iDOT` chunk's length field.
//!
//! The parallel path is an optimisation of the serial decode, never a second
//! interpretation of the file: every check below exists so that an accepted
//! parallel result is byte-identical to what [`super::RowDecoder`] would
//! produce. Anything unexpected returns [`Outcome::Fallback`] and the caller
//! runs the unchanged serial decoder.

use alloc::borrow::Cow;
use alloc::vec::Vec;

use enough::Stop;
#[allow(unused_imports)]
use whereat::at;

use crate::chunk::ancillary::PngAncillary;
use crate::chunk::ihdr::Ihdr;
use crate::error::PngError;

use super::postprocess::post_process_row;
use super::row::{IdatSource, unfilter_row};

/// Upper bound on accepted segment counts. Apple writes 2; zenpng's encoder
/// writes at most this many.
pub(crate) const MAX_SEGMENTS: usize = 64;

/// One validated `iDOT` segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IdotSegment {
    /// First image row of the segment.
    pub first_row: u32,
    /// Number of rows.
    pub rows: u32,
    /// File offset of the segment's first IDAT chunk header.
    pub chunk_pos: usize,
    /// File offset of the next segment's first IDAT chunk header
    /// (`usize::MAX` for the last segment).
    pub end_pos: usize,
}

/// Parse and statically validate an `iDOT` chunk.
///
/// Returns `None` (decode serially) unless the table tiles the image exactly
/// and every offset lands on an IDAT chunk header inside the contiguous IDAT
/// run, in increasing order, with the first one at the first IDAT.
pub(crate) fn validate(
    file: &[u8],
    idot_pos: usize,
    data: &[u8],
    ihdr: &Ihdr,
    first_idat_pos: usize,
) -> Option<Vec<IdotSegment>> {
    if ihdr.interlace != 0 || data.len() < 4 || !(data.len() - 4).is_multiple_of(12) {
        return None;
    }
    let word = |i: usize| u32::from_be_bytes(data[i * 4..i * 4 + 4].try_into().unwrap());
    let n = word(0) as usize;
    if !(2..=MAX_SEGMENTS).contains(&n) || data.len() != 4 + 12 * n || n > ihdr.height as usize {
        return None;
    }

    // Positions of every chunk header in the contiguous IDAT run.
    let mut idat_run = Vec::new();
    let mut pos = first_idat_pos;
    while let Some(hdr) = file.get(pos..pos.checked_add(8)?) {
        if hdr[4..8] != *b"IDAT" {
            break;
        }
        let len = u32::from_be_bytes(hdr[..4].try_into().unwrap()) as usize;
        idat_run.push(pos);
        pos = pos.checked_add(12)?.checked_add(len)?;
    }

    let mut segs = Vec::with_capacity(n);
    let mut next_row = 0u32;
    let mut run_idx = 0usize;
    for k in 0..n {
        let first_row = word(1 + 3 * k);
        let rows = word(2 + 3 * k);
        let abs = idot_pos.checked_add(word(3 + 3 * k) as usize)?;
        if first_row != next_row || rows == 0 {
            return None;
        }
        next_row = next_row.checked_add(rows)?;
        // Offsets must be strictly increasing chunk headers in the run.
        while run_idx < idat_run.len() && idat_run[run_idx] < abs {
            run_idx += 1;
        }
        if idat_run.get(run_idx) != Some(&abs) || (k == 0 && abs != first_idat_pos) {
            return None;
        }
        run_idx += 1;
        segs.push(IdotSegment {
            first_row,
            rows,
            chunk_pos: abs,
            end_pos: usize::MAX,
        });
    }
    if next_row != ihdr.height {
        return None;
    }
    for k in 0..n - 1 {
        segs[k].end_pos = segs[k + 1].chunk_pos;
    }
    Some(segs)
}

/// Benchmark/test counters: parallel decodes completed and fallbacks taken.
#[cfg(feature = "_dev")]
pub(crate) static STATS: [core::sync::atomic::AtomicUsize; 2] = [
    core::sync::atomic::AtomicUsize::new(0),
    core::sync::atomic::AtomicUsize::new(0),
];

#[cfg(feature = "_dev")]
fn count(i: usize) {
    STATS[i].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}
#[cfg(not(feature = "_dev"))]
fn count(_: usize) {}

/// Result of a parallel attempt.
pub(crate) enum Outcome {
    /// Every row was written and is identical to the serial decode.
    Done,
    /// Not attempted, or a check failed: decode serially.
    Fallback,
}

/// How each worker writes its rows.
#[derive(Clone, Copy)]
pub(crate) enum Sink<'a> {
    /// Unfiltered raw rows (`raw_row_bytes` each) are the output.
    Raw,
    /// Rows go through [`post_process_row`]; output rows are `out_row_bytes`.
    Post {
        ancillary: &'a PngAncillary,
        out_row_bytes: usize,
    },
}

/// Filtered bytes (rows × stride) per worker for the first two workers.
///
/// Measured on a Core Ultra 7 265K (`examples/idot_bench.rs`, 2026-10-02,
/// `benchmarks/idot_decode_2026-10-02.log`): two workers start paying off at
/// about 1 MiB each (1024² RGBA, 4 MiB: 1.33–1.43×), while at 0.5 MiB each
/// (512² RGBA) they were 1.02–1.19× and noisy. Workers beyond two only helped
/// on large images — four was best at 4.5–5 MP, eight only at 7.6 MP — so
/// each extra worker needs [`EXTRA_WORKER_BYTES`] more.
pub(crate) const MIN_BYTES_PER_WORKER: usize = 1 << 20;

/// Test/benchmark override of [`MIN_BYTES_PER_WORKER`] (0 = none), so tiny
/// fixtures can exercise the parallel path. `_dev` only.
#[cfg(feature = "_dev")]
pub(crate) static MIN_BYTES_OVERRIDE: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

fn min_bytes_per_worker() -> usize {
    #[cfg(feature = "_dev")]
    {
        let v = MIN_BYTES_OVERRIDE.load(core::sync::atomic::Ordering::Relaxed);
        if v != 0 {
            return v;
        }
    }
    #[cfg(feature = "_dev")]
    if let Some(v) = std::env::var("ZENPNG_IDOT_MIN_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return v;
    }
    MIN_BYTES_PER_WORKER
}

/// Additional filtered bytes required per worker beyond the second.
pub(crate) const EXTRA_WORKER_BYTES: usize = 4 << 20;

/// How many workers `total_bytes` of filtered data justifies (0 or 1 = none).
pub(crate) fn workers_for_bytes(total_bytes: usize) -> usize {
    let min = min_bytes_per_worker().max(1);
    if total_bytes < 2 * min {
        return 0;
    }
    let extra = if min == MIN_BYTES_PER_WORKER {
        EXTRA_WORKER_BYTES
    } else {
        // Benchmark override scales both thresholds together.
        4 * min
    };
    2 + (total_bytes - 2 * min) / extra
}

/// Decide how many workers to use; `None` when parallel decode isn't worth it.
///
/// `tier_cpus` caps the count when workers will be pinned to a core tier, so
/// pinned workers never outnumber the cores they may run on.
pub(crate) fn plan_workers(
    segs: &[IdotSegment],
    stride: usize,
    max_threads: usize,
    tier_cpus: Option<usize>,
) -> Option<usize> {
    if max_threads == 1 {
        return None;
    }
    let available = std::thread::available_parallelism().map_or(1, |n| n.get());
    let threads = if max_threads == 0 {
        available
    } else {
        max_threads.min(available)
    };
    let total_rows: usize = segs.iter().map(|s| s.rows as usize).sum();
    let total_bytes = total_rows.saturating_mul(stride);
    let by_size = workers_for_bytes(total_bytes);
    let workers = threads
        .min(segs.len())
        .min(by_size)
        .min(tier_cpus.unwrap_or(usize::MAX));
    (workers >= 2).then_some(workers)
}

/// Per-worker result: (Adler-32, length) of each segment's filtered bytes,
/// plus the zlib footer if this worker owns the last segment.
struct WorkerOk {
    adlers: Vec<(u32, usize)>,
    footer: Option<u32>,
}

enum WorkerErr {
    Fallback,
    Cancelled(whereat::At<PngError>),
}

/// Geometry shared by all workers.
#[derive(Clone, Copy)]
struct Geometry {
    stride: usize,
    raw_row_bytes: usize,
    bpp: usize,
    out_row_bytes: usize,
    capacity: usize,
    skip_crc: bool,
    ihdr: Ihdr,
}

/// Decode `segs` in parallel into `out` (`height × out_row_bytes` bytes).
///
/// `out_row_bytes` must equal `raw_row_bytes` for [`Sink::Raw`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_parallel(
    file: &[u8],
    ihdr: &Ihdr,
    segs: &[IdotSegment],
    workers: usize,
    sink: Sink<'_>,
    out: &mut [u8],
    skip_crc: bool,
    pin: Option<&crate::affinity::Pin>,
    cancel: &dyn Stop,
) -> Result<Outcome, whereat::At<PngError>> {
    let raw_row_bytes = ihdr.raw_row_bytes()?;
    let stride = ihdr.stride()?;
    let out_row_bytes = match sink {
        Sink::Raw => raw_row_bytes,
        Sink::Post { out_row_bytes, .. } => out_row_bytes,
    };
    let geo = Geometry {
        stride,
        raw_row_bytes,
        bpp: ihdr.filter_bpp(),
        out_row_bytes,
        capacity: crate::alloc_util::stream_capacity(stride)?,
        skip_crc,
        ihdr: *ihdr,
    };
    debug_assert_eq!(out.len(), ihdr.height as usize * out_row_bytes);

    // One work unit per segment, claimed dynamically: a worker on a fast
    // core simply takes more segments.
    let mut units = Vec::with_capacity(segs.len());
    let mut rest = out;
    for (i, seg) in segs.iter().enumerate() {
        let (a, b) = rest.split_at_mut(seg.rows as usize * out_row_bytes);
        units.push((i, a));
        rest = b;
    }
    let queue = std::sync::Mutex::new(units.into_iter());
    let abort = core::sync::atomic::AtomicBool::new(false);
    let n = segs.len();

    #[cfg(feature = "_dev")]
    let trace_t0 = std::time::Instant::now();
    #[cfg(feature = "_dev")]
    let trace = std::env::var_os("ZENPNG_IDOT_TRACE").is_some();

    let work = || {
        let mut done: Vec<(usize, Result<WorkerOk, WorkerErr>)> = Vec::new();
        loop {
            if abort.load(core::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let Some((i, slice)) = queue.lock().ok().and_then(|mut q| q.next()) else {
                break;
            };
            let r = run_worker(
                file,
                &segs[i..=i],
                i == 0,
                i + 1 == n,
                geo,
                sink,
                slice,
                cancel,
            );
            if r.is_err() {
                abort.store(true, core::sync::atomic::Ordering::Relaxed);
            }
            done.push((i, r));
        }
        done
    };

    let _caller_guard = crate::affinity::caller_guard(pin);
    let mut results: Vec<(usize, Result<WorkerOk, WorkerErr>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (1..workers)
            .map(|_| {
                let work = &work;
                scope.spawn(move || {
                    if let Some(p) = pin {
                        p.apply_to_current_thread();
                    }
                    #[cfg(feature = "_dev")]
                    let ts = trace_t0.elapsed();
                    let r = work();
                    #[cfg(feature = "_dev")]
                    if trace {
                        std::eprintln!(
                            "  worker start {:.3} end {:.3} ms, {} segments",
                            ts.as_secs_f64() * 1e3,
                            trace_t0.elapsed().as_secs_f64() * 1e3,
                            r.len()
                        );
                    }
                    r
                })
            })
            .collect();
        let mut all = work();
        #[cfg(feature = "_dev")]
        if trace {
            std::eprintln!(
                "  caller end {:.3} ms, {} segments",
                trace_t0.elapsed().as_secs_f64() * 1e3,
                all.len()
            );
        }
        for h in handles {
            match h.join() {
                Ok(v) => all.extend(v),
                Err(_) => all.push((usize::MAX, Err(WorkerErr::Fallback))),
            }
        }
        all
    });
    results.sort_by_key(|r| r.0);
    if results.len() != n {
        // Aborted before every segment ran; the failure is in `results`.
        if let Some(pos) = results
            .iter()
            .position(|r| matches!(r.1, Err(WorkerErr::Cancelled(_))))
            && let (_, Err(WorkerErr::Cancelled(e))) = results.swap_remove(pos)
        {
            return Err(e);
        }
        count(1);
        return Ok(Outcome::Fallback);
    }
    let results = results.into_iter().map(|r| r.1);

    // Serial equivalence: join per-segment Adler-32s and compare with the
    // footer. A mismatch is rare (corruption) and the serial path decides
    // what to report, so fall back.
    let mut adler = 1u32;
    let mut footer = None;
    for r in results {
        match r {
            Ok(w) => {
                for (a, len) in w.adlers {
                    adler = zenflate::adler32_combine(adler, a, len);
                }
                if w.footer.is_some() {
                    footer = w.footer;
                }
            }
            Err(WorkerErr::Cancelled(e)) => return Err(e),
            Err(WorkerErr::Fallback) => {
                count(1);
                return Ok(Outcome::Fallback);
            }
        }
    }
    if footer != Some(adler) {
        count(1);
        return Ok(Outcome::Fallback);
    }
    count(0);
    #[cfg(feature = "_dev")]
    if trace {
        std::eprintln!(
            "  joined+verified {:.3} ms",
            trace_t0.elapsed().as_secs_f64() * 1e3
        );
    }
    Ok(Outcome::Done)
}

/// Decode a contiguous run of segments (in practice one) into `out`.
#[allow(clippy::too_many_arguments)]
fn run_worker(
    file: &[u8],
    segs: &[IdotSegment],
    first_group: bool,
    last_group: bool,
    geo: Geometry,
    sink: Sink<'_>,
    out: &mut [u8],
    cancel: &dyn Stop,
) -> Result<WorkerOk, WorkerErr> {
    let mut adlers = Vec::with_capacity(segs.len());
    let mut footer = None;
    // Raw-row scratch for Sink::Post (prev + current), and the post-processed row.
    let (mut prev_raw, mut cur_raw, mut row_buf) = match sink {
        Sink::Raw => (Vec::new(), Vec::new(), Vec::new()),
        Sink::Post { .. } => (
            alloc::vec![0u8; geo.raw_row_bytes],
            alloc::vec![0u8; geo.raw_row_bytes],
            Vec::with_capacity(geo.out_row_bytes),
        ),
    };
    let zero_row = match sink {
        Sink::Raw => alloc::vec![0u8; geo.raw_row_bytes],
        Sink::Post { .. } => Vec::new(),
    };
    let mut out_row = 0usize; // row index within `out`

    for (i, seg) in segs.iter().enumerate() {
        let is_image_first = first_group && i == 0;
        let is_image_last = last_group && i + 1 == segs.len();
        let source = IdatSource::new_bounded(
            Cow::Borrowed(file),
            seg.chunk_pos,
            seg.end_pos,
            geo.skip_crc,
        )
        .map_err(|_| WorkerErr::Fallback)?;
        let dec = if is_image_first {
            zenflate::StreamDecompressor::zlib(source, geo.capacity)
        } else {
            zenflate::StreamDecompressor::zlib_continuation(source, geo.capacity)
        };
        let mut dec = dec.with_segment_end(true).with_skip_checksum(true);

        for r in 0..seg.rows as usize {
            // Fill until a whole filtered row is available.
            while dec.peek().len() < geo.stride {
                if dec.is_done() {
                    return Err(WorkerErr::Fallback);
                }
                dec.fill().map_err(|_| WorkerErr::Fallback)?;
            }
            let peeked = dec.peek();
            let filter = peeked[0];
            // The first row a worker decodes has no predecessor here. That is
            // only the serial result if the row ignores its predecessor
            // (None or Sub) or is the image's first row.
            let worker_first = out_row == 0;
            if worker_first && !is_image_first && filter > 1 {
                return Err(WorkerErr::Fallback);
            }
            match sink {
                Sink::Raw => {
                    let at = out_row * geo.raw_row_bytes;
                    let (done, rest) = out.split_at_mut(at);
                    let dest = &mut rest[..geo.raw_row_bytes];
                    dest.copy_from_slice(&peeked[1..geo.stride]);
                    let prev = if worker_first {
                        &zero_row[..]
                    } else {
                        &done[at - geo.raw_row_bytes..]
                    };
                    unfilter_row(filter, dest, prev, geo.bpp).map_err(|_| WorkerErr::Fallback)?;
                }
                Sink::Post {
                    ancillary,
                    out_row_bytes,
                } => {
                    cur_raw.copy_from_slice(&peeked[1..geo.stride]);
                    if worker_first {
                        prev_raw.fill(0);
                    }
                    unfilter_row(filter, &mut cur_raw, &prev_raw, geo.bpp)
                        .map_err(|_| WorkerErr::Fallback)?;
                    post_process_row(&cur_raw, &geo.ihdr, ancillary, &mut row_buf);
                    if row_buf.len() != out_row_bytes {
                        return Err(WorkerErr::Fallback);
                    }
                    out[out_row * out_row_bytes..(out_row + 1) * out_row_bytes]
                        .copy_from_slice(&row_buf);
                    core::mem::swap(&mut prev_raw, &mut cur_raw);
                }
            }
            dec.advance(geo.stride);
            out_row += 1;
            if r % 16 == 0 {
                cancel
                    .check()
                    .map_err(|e| WorkerErr::Cancelled(at!(PngError::from(e))))?;
            }
        }

        // Drain: the segment must end exactly here, with no extra rows.
        while !dec.is_done() {
            let n = dec.fill().map_err(|_| WorkerErr::Fallback)?.len();
            if n > 0 {
                return Err(WorkerErr::Fallback);
            }
        }
        if !dec.peek().is_empty() {
            return Err(WorkerErr::Fallback);
        }
        if is_image_last {
            if dec.ended_at_segment_boundary() {
                return Err(WorkerErr::Fallback);
            }
            footer = dec.footer_checksum();
        } else if !dec.ended_at_segment_boundary() {
            return Err(WorkerErr::Fallback);
        }
        adlers.push((dec.running_checksum(), seg.rows as usize * geo.stride));
    }
    Ok(WorkerOk { adlers, footer })
}
