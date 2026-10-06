//! Two-thread decode for ordinary (non-`iDOT`) PNGs: one thread inflates
//! filtered rows while the caller's thread unfilters and expands the rows
//! before them.
//!
//! A zlib stream decodes serially, but on large images unfiltering is about
//! as expensive as inflating (39-47% of decode instructions on RGB8 photos,
//! callgrind 2026-10-06), so overlapping the two stages saves time whenever a
//! second core is free. Output is identical to the serial decoder: rows are
//! unfiltered in order by one thread, and the producer drains the zlib stream
//! (verifying Adler-32 in strict mode) exactly as the serial path does.

use alloc::vec::Vec;
use std::sync::mpsc;

use enough::Stop;
use whereat::at;

use super::row::RowDecoder;
use crate::error::{PngError, Result};

/// Smallest filtered stream (bytes) worth a second thread. Crossover on i265
/// E-cores (2026-10-06, `decode_only`, RGB8 at 256-768 px): a photo gains from
/// ~324 KiB (0.83x at 384 px), a document and a screenshot break even at
/// ~480-600 KiB (0.98-1.01x) and gain from ~750 KiB (0.79-0.88x).
pub(crate) const PIPELINE_MIN_BYTES: usize = 512 * 1024;

/// Filtered bytes per chunk handed between the threads.
const CHUNK_BYTES: usize = 128 * 1024;

/// Chunks in flight; the producer gets at most this far ahead.
const CHUNKS: usize = 4;

/// Whether a decode of `filtered_bytes` with `max_threads` should pipeline.
pub(crate) fn worth_it(filtered_bytes: usize, max_threads: usize) -> bool {
    !cfg!(target_arch = "wasm32")
        && max_threads != 1
        && filtered_bytes >= PIPELINE_MIN_BYTES
        && std::thread::available_parallelism().is_ok_and(|n| n.get() >= 2)
}

/// Inflate all `rows` filtered rows of `reader` on a second thread and call
/// `consume(y, filtered_row)` for each, in order, on this thread. Drains the
/// zlib stream after the last row (`RowDecoder::finish_stream`). Returns the
/// reader for metadata collection.
pub(crate) fn run<'a>(
    mut reader: RowDecoder<'a>,
    rows: usize,
    cancel: &dyn Stop,
    mut consume: impl FnMut(usize, &[u8]) -> Result<()>,
) -> Result<RowDecoder<'a>> {
    let stride = reader.stride();
    let per_chunk = (CHUNK_BYTES / stride).max(1);
    let (full_tx, full_rx) = mpsc::sync_channel::<Result<(Vec<u8>, usize)>>(CHUNKS);
    let (empty_tx, empty_rx) = mpsc::channel::<Vec<u8>>();
    for _ in 0..CHUNKS {
        // Small and bounded by the row width, not by untrusted totals.
        empty_tx
            .send(alloc::vec![0u8; per_chunk * stride])
            .expect("receiver alive");
    }

    std::thread::scope(|s| {
        let producer = s.spawn(move || -> (RowDecoder<'a>, bool) {
            let mut y = 0;
            while y < rows {
                let Ok(mut buf) = empty_rx.recv() else {
                    return (reader, false); // consumer stopped
                };
                let n = per_chunk.min(rows - y);
                let mut res = Ok(());
                for r in 0..n {
                    match reader.next_filtered_row(&mut buf[r * stride..(r + 1) * stride]) {
                        Some(Ok(())) => {}
                        Some(Err(e)) => {
                            res = Err(e);
                            break;
                        }
                        None => {
                            res = Err(at!(PngError::Truncated(alloc::format!(
                                "image data ends at row {}",
                                y + r
                            ))));
                            break;
                        }
                    }
                }
                let failed = res.is_err();
                if full_tx.send(res.map(|()| (buf, n))).is_err() || failed {
                    return (reader, false);
                }
                y += n;
            }
            let drained = reader.finish_stream();
            let ok = drained.is_ok();
            if let Err(e) = drained {
                let _ = full_tx.send(Err(e));
            }
            (reader, ok)
        });

        let mut y = 0;
        let mut outcome = Ok(());
        while y < rows {
            let (buf, n) = match full_rx.recv() {
                Ok(Ok(chunk)) => chunk,
                Ok(Err(e)) => {
                    outcome = Err(e);
                    break;
                }
                Err(_) => {
                    outcome = Err(at!(PngError::Internal(
                        zencodec::InternalKind::Bug,
                        "decode pipeline producer stopped early".into()
                    )));
                    break;
                }
            };
            // Cancellation is checked here, on the caller's thread; the
            // producer stops when this side hangs up.
            if let Err(e) = cancel.check() {
                outcome = Err(at!(PngError::from(e)));
                break;
            }
            for r in 0..n {
                if let Err(e) = consume(y + r, &buf[r * stride..(r + 1) * stride]) {
                    outcome = Err(e);
                    break;
                }
            }
            if outcome.is_err() {
                break;
            }
            y += n;
            let _ = empty_tx.send(buf);
        }
        // Stop the producer if we bailed, then collect it.
        drop(empty_tx);
        let (reader, drained) = producer
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
        outcome?;
        if !drained {
            // The drain error was sent after the last chunk.
            if let Ok(Err(e)) = full_rx.recv() {
                return Err(e);
            }
        }
        Ok(reader)
    })
}

/// Unfilter `filtered` (filter byte + row) into row `y` of `out`, whose rows
/// are `row_bytes` long; the filter's predecessor is row `y - 1` of `out`, or
/// `zeros` for the first row.
pub(crate) fn unfilter_into(
    out: &mut [u8],
    y: usize,
    row_bytes: usize,
    filtered: &[u8],
    zeros: &[u8],
    bpp: usize,
) -> Result<()> {
    let (done, rest) = out.split_at_mut(y * row_bytes);
    let row = &mut rest[..row_bytes];
    row.copy_from_slice(&filtered[1..]);
    let prev = if y == 0 {
        zeros
    } else {
        &done[(y - 1) * row_bytes..]
    };
    super::row::unfilter_row(filtered[0], row, prev, bpp)
}
