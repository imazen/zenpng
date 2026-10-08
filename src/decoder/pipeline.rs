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

/// [`PIPELINE_MIN_BYTES`], or with the `_dev` feature the
/// `ZENPNG_PIPELINE_MIN_BYTES` override (threshold sweeps).
fn pipeline_min_bytes() -> usize {
    #[cfg(feature = "_dev")]
    if let Some(v) = std::env::var("ZENPNG_PIPELINE_MIN_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return v;
    }
    PIPELINE_MIN_BYTES
}

/// Whether decoding `ihdr`'s image with `max_threads` should pipeline.
///
/// Palette and sub-byte gray images don't: their rows expand to 3-8 output
/// bytes per filtered byte on the consumer thread, so it is the bottleneck
/// and the handoff only adds cost (pal8 at 1024 px ran 1.32x slower
/// pipelined on i265 P-cores, `benchmarks/pareto_x86_decode_2026-10-07.md`;
/// RGB8 0.83x, RGBA8 0.78x, RGB16 0.92x, gray8 0.99x).
pub(crate) fn worth_it(ihdr: &crate::chunk::ihdr::Ihdr, max_threads: usize) -> bool {
    let Ok(raw) = ihdr.raw_row_bytes() else {
        return false;
    };
    let filtered_bytes = (ihdr.height as usize).saturating_mul(raw + 1);
    !cfg!(target_arch = "wasm32")
        && max_threads != 1
        && ihdr.color_type != 3
        && ihdr.bit_depth >= 8
        && filtered_bytes >= pipeline_min_bytes()
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

/// [`run`]'s producer on an owned reader, for pull-style consumers (the
/// zencodec streaming decoder): a second thread inflates filtered rows while
/// the caller takes them one at a time with [`Piped::next_row`].
pub(crate) struct Piped {
    full_rx: mpsc::Receiver<Result<(Vec<u8>, usize)>>,
    empty_tx: Option<mpsc::Sender<Vec<u8>>>,
    producer: Option<std::thread::JoinHandle<bool>>,
    /// The chunk being read: buffer, rows in it, next row index.
    chunk: Option<(Vec<u8>, usize, usize)>,
    stride: usize,
}

impl Piped {
    /// Start inflating `rows` rows of `reader` on a new thread.
    pub(crate) fn spawn(mut reader: RowDecoder<'static>, rows: usize) -> Self {
        let stride = reader.stride();
        let per_chunk = (CHUNK_BYTES / stride).max(1);
        let (full_tx, full_rx) = mpsc::sync_channel::<Result<(Vec<u8>, usize)>>(CHUNKS);
        let (empty_tx, empty_rx) = mpsc::channel::<Vec<u8>>();
        for _ in 0..CHUNKS {
            empty_tx
                .send(alloc::vec![0u8; per_chunk * stride])
                .expect("receiver alive");
        }
        let producer = std::thread::spawn(move || -> bool {
            let mut y = 0;
            while y < rows {
                let Ok(mut buf) = empty_rx.recv() else {
                    return false; // consumer gone
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
                    return false;
                }
                y += n;
            }
            match reader.finish_stream() {
                Ok(()) => true,
                Err(e) => {
                    let _ = full_tx.send(Err(e));
                    false
                }
            }
        });
        Self {
            full_rx,
            empty_tx: Some(empty_tx),
            producer: Some(producer),
            chunk: None,
            stride,
        }
    }

    /// The next filtered row (filter byte + row data), in order.
    pub(crate) fn next_row(&mut self) -> Result<&[u8]> {
        if let Some((buf, n, next)) = self.chunk.take() {
            if next < n {
                self.chunk = Some((buf, n, next));
            } else if let Some(tx) = &self.empty_tx {
                let _ = tx.send(buf);
            }
        }
        if self.chunk.is_none() {
            let (buf, n) = match self.full_rx.recv() {
                Ok(Ok(chunk)) => chunk,
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(at!(PngError::Internal(
                        zencodec::InternalKind::Bug,
                        "decode pipeline producer stopped early".into()
                    )));
                }
            };
            self.chunk = Some((buf, n, 0));
        }
        let (buf, _, next) = self.chunk.as_mut().expect("chunk set above");
        let r = *next;
        *next += 1;
        Ok(&buf[r * self.stride..(r + 1) * self.stride])
    }

    /// After the last row: wait for the producer's drain of the zlib stream
    /// (it verifies Adler-32 in strict mode) and report its error, if any.
    pub(crate) fn finish(&mut self) -> Result<()> {
        self.empty_tx = None;
        let Some(producer) = self.producer.take() else {
            return Ok(());
        };
        let drained = producer
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
        if !drained && let Ok(Err(e)) = self.full_rx.recv() {
            return Err(e);
        }
        Ok(())
    }
}

impl Drop for Piped {
    fn drop(&mut self) {
        // Hanging up both channels stops the producer wherever it waits; a
        // dropped decoder doesn't wait for it.
        self.empty_tx = None;
    }
}
