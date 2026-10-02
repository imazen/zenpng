//! Probe Apple `iDOT` parallel-decode PNGs: dump the chunk, check that the
//! parallel interpretation matches the serial one, and time serial vs
//! segment-parallel inflate + unfilter.
//!
//! The iDOT layout (verified on real Apple files, see
//! `docs/IDOT_PARALLEL_PNG.md`): `u32 N`, then N × `{u32 first_row,
//! u32 row_count, u32 offset}` big-endian, where `offset` is the byte offset
//! of that segment's first IDAT chunk measured from the start of the iDOT
//! chunk's length field.
//!
//! This is a measurement harness, not the production decoder: it only
//! handles non-interlaced 8-bit-or-wider images and assumes each non-first
//! segment's first row uses filter None or Sub (it checks and reports).
//!
//! Usage:
//!   cargo run --release --example idot_probe --features _dev -- a.png [b.png ...]

use std::time::{Duration, Instant};

use zenflate::{CompressionLevel, Compressor, Decompressor, StreamDecompressor, Unstoppable};

struct Chunk {
    pos: usize,
    ty: [u8; 4],
    data: std::ops::Range<usize>,
}

fn chunks(b: &[u8]) -> Vec<Chunk> {
    let mut out = Vec::new();
    let mut p = 8;
    while p + 12 <= b.len() {
        let n = u32::from_be_bytes(b[p..p + 4].try_into().unwrap()) as usize;
        let ty: [u8; 4] = b[p + 4..p + 8].try_into().unwrap();
        if p + 12 + n > b.len() {
            break;
        }
        out.push(Chunk {
            pos: p,
            ty,
            data: p + 8..p + 8 + n,
        });
        p += 12 + n;
    }
    out
}

/// Concatenate IDAT payloads of chunks whose header position is in `[from, to)`.
fn idat_bytes(b: &[u8], cs: &[Chunk], from: usize, to: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for c in cs {
        if &c.ty == b"IDAT" && c.pos >= from && c.pos < to {
            v.extend_from_slice(&b[c.data.clone()]);
        }
    }
    v
}

/// Unfilter `rows` filtered rows (each `1 + stride` bytes, in `raw`) into
/// `out` (`rows * stride` bytes). Row 0's predecessor is all-zero, which is
/// only the true predecessor for the image's first row, or for a segment
/// whose first row uses filter None or Sub (the probe checks this).
fn unfilter_into(raw: &mut [u8], out: &mut [u8], stride: usize, bpp: usize, rows: usize) {
    let zero = vec![0u8; stride];
    for r in 0..rows {
        let (done, rest) = out.split_at_mut(r * stride);
        let prev = if r == 0 {
            &zero[..]
        } else {
            &done[(r - 1) * stride..]
        };
        let src = &mut raw[r * (stride + 1)..(r + 1) * (stride + 1)];
        let ft = src[0];
        zenpng::__bench_unfilter_row(ft, &mut src[1..], prev, bpp);
        rest[..stride].copy_from_slice(&src[1..]);
    }
}

/// Fused streaming inflate + unfilter (the shape `zenpng::decode` uses):
/// rows are unfiltered while still cache-hot, straight into `out`.
/// Returns the first row's filter type.
fn fused_into(
    input: &[u8],
    zlib: bool,
    out: &mut [u8],
    stride: usize,
    bpp: usize,
    rows: usize,
) -> u8 {
    let mut dec = if zlib {
        StreamDecompressor::zlib(input, 256 * 1024)
    } else {
        StreamDecompressor::deflate(input, 256 * 1024)
    }
    .with_skip_checksum(true);
    let zero = vec![0u8; stride];
    let mut row_buf = vec![0u8; stride + 1];
    let mut r = 0;
    let mut first_filter = 0;
    while r < rows {
        let avail_len = dec.fill().unwrap().len();
        if avail_len < stride + 1 {
            assert!(!dec.is_done(), "stream ended early at row {r}");
            if avail_len == 0 {
                continue;
            }
            // Partial row: copy what we have and keep filling.
            let mut have = 0;
            while have < stride + 1 {
                let a = dec.fill().unwrap();
                let take = a.len().min(stride + 1 - have);
                row_buf[have..have + take].copy_from_slice(&a[..take]);
                have += take;
                dec.advance(take);
            }
        } else {
            row_buf.copy_from_slice(&dec.peek()[..stride + 1]);
            dec.advance(stride + 1);
        }
        if r == 0 {
            first_filter = row_buf[0];
        }
        let (done, rest) = out.split_at_mut(r * stride);
        let prev = if r == 0 {
            &zero[..]
        } else {
            &done[(r - 1) * stride..]
        };
        let ft = row_buf[0];
        zenpng::__bench_unfilter_row(ft, &mut row_buf[1..], prev, bpp);
        rest[..stride].copy_from_slice(&row_buf[1..]);
        r += 1;
    }
    first_filter
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn main() {
    let iters = 9;
    for path in std::env::args().skip(1) {
        let b = std::fs::read(&path).unwrap();
        let cs = chunks(&b);
        let ihdr = &b[cs[0].data.clone()];
        let w = u32::from_be_bytes(ihdr[0..4].try_into().unwrap()) as usize;
        let h = u32::from_be_bytes(ihdr[4..8].try_into().unwrap()) as usize;
        let (bd, ct, il) = (ihdr[8] as usize, ihdr[9], ihdr[12]);
        let ch = match ct {
            0 | 3 => 1,
            2 => 3,
            4 => 2,
            6 => 4,
            _ => panic!("bad color type"),
        };
        let stride = (w * ch * bd).div_ceil(8);
        let bpp = (ch * bd / 8).max(1);
        println!("== {path}: {w}x{h} bitdepth={bd} color_type={ct} interlace={il}");
        let Some(idot) = cs.iter().find(|c| &c.ty == b"iDOT") else {
            println!("   no iDOT");
            continue;
        };
        if il != 0 || bd < 8 {
            println!("   skipped: probe only handles non-interlaced >=8-bit");
            continue;
        }
        let d = &b[idot.data.clone()];
        let word = |i: usize| u32::from_be_bytes(d[i * 4..i * 4 + 4].try_into().unwrap()) as usize;
        let n = word(0);
        let segs: Vec<(usize, usize, usize)> = (0..n)
            .map(|k| (word(1 + 3 * k), word(2 + 3 * k), idot.pos + word(3 + 3 * k)))
            .collect();
        println!("   iDOT N={n} segments (first_row,row_count,abs_offset)={segs:?}");

        // ── Serial reference: production decoder ──
        let cfg = zenpng::PngDecodeConfig::default();
        let reference = zenpng::decode(&b, &cfg, &Unstoppable).unwrap();
        let mut t_full = Vec::new();
        for _ in 0..iters {
            let t = Instant::now();
            let o = zenpng::decode(&b, &cfg, &Unstoppable).unwrap();
            t_full.push(t.elapsed());
            std::hint::black_box(o);
        }
        let median_full = median(t_full.clone());

        // ── Serial manual pipeline: one-shot inflate + unfilter ──
        let all = idat_bytes(&b, &cs, 0, usize::MAX);
        let raw_len = h * (stride + 1);
        let mut t_inf = Vec::new();
        let mut t_unf = Vec::new();
        let mut serial_pixels = Vec::new();
        for _ in 0..iters {
            let mut raw = vec![0u8; raw_len];
            let t = Instant::now();
            Decompressor::new()
                .with_skip_checksum(true)
                .zlib_decompress(&all, &mut raw, Unstoppable)
                .unwrap();
            t_inf.push(t.elapsed());
            let t = Instant::now();
            let mut out = vec![0u8; h * stride];
            unfilter_into(&mut raw, &mut out, stride, bpp, h);
            t_unf.push(t.elapsed());
            serial_pixels = out;
        }

        // ── Segment-parallel: each thread inflates + unfilters its segment ──
        // Segment byte ranges: from its offset to the next segment's offset.
        let mut seg_inputs = Vec::new();
        for (k, &(_, _, off)) in segs.iter().enumerate() {
            let end = segs.get(k + 1).map_or(usize::MAX, |s| s.2);
            let mut bytes = idat_bytes(&b, &cs, off, end);
            if k == 0 {
                bytes.drain(..2); // zlib header
            }
            if k + 1 < n {
                // Non-final segment ends at a byte-aligned block boundary;
                // terminate it with an empty final stored block so a
                // one-shot raw-deflate decoder accepts it.
                bytes.extend_from_slice(&[0x01, 0x00, 0x00, 0xff, 0xff]);
            }
            seg_inputs.push(bytes);
        }
        let mut t_par = Vec::new();
        let mut par_pixels = Vec::new();
        let mut start_filters = Vec::new();
        for _ in 0..iters {
            let t = Instant::now();
            let mut out = vec![0u8; h * stride];
            // Split the output into one disjoint slice per segment.
            let mut slices = Vec::new();
            let mut rest = &mut out[..];
            for &(_, rows, _) in &segs {
                let (a, b) = rest.split_at_mut(rows * stride);
                slices.push(a);
                rest = b;
            }
            start_filters = std::thread::scope(|s| {
                let hs: Vec<_> = segs
                    .iter()
                    .zip(&seg_inputs)
                    .zip(slices)
                    .map(|((&(_, rows, _), input), dst)| {
                        s.spawn(move || {
                            let t0 = t.elapsed();
                            let mut raw = vec![0u8; rows * (stride + 1)];
                            let t1 = t.elapsed();
                            let o = Decompressor::new()
                                .deflate_decompress(input, &mut raw, Unstoppable)
                                .unwrap();
                            assert_eq!(o.output_written, raw.len(), "segment size mismatch");
                            let t2 = t.elapsed();
                            let f0 = raw[0];
                            unfilter_into(&mut raw, dst, stride, bpp, rows);
                            let t3 = t.elapsed();
                            (f0, [t0, t1, t2, t3])
                        })
                    })
                    .collect();
                hs.into_iter().map(|h| h.join().unwrap()).collect::<Vec<_>>()
            })
            .into_iter()
            .map(|(f0, ts)| {
                if std::env::var_os("IDOT_TRACE").is_some() {
                    let ms = ts.map(|d| d.as_secs_f64() * 1e3);
                    eprintln!(
                        "     thread: spawned@{:.2} alloc-done@{:.2} inflate-done@{:.2} unfilter-done@{:.2} ms",
                        ms[0], ms[1], ms[2], ms[3]
                    );
                }
                f0
            })
            .collect();
            t_par.push(t.elapsed());
            par_pixels = out;
        }
        // ── Fused variants ──
        let mut t_fs = Vec::new();
        let mut fused_serial = Vec::new();
        for _ in 0..iters {
            let t = Instant::now();
            let mut out = vec![0u8; h * stride];
            fused_into(&all, true, &mut out, stride, bpp, h);
            t_fs.push(t.elapsed());
            fused_serial = out;
        }
        let mut t_fp = Vec::new();
        let mut fused_par = Vec::new();
        for _ in 0..iters {
            let t = Instant::now();
            let mut out = vec![0u8; h * stride];
            let mut slices = Vec::new();
            let mut rest = &mut out[..];
            for &(_, rows, _) in &segs {
                let (a, b) = rest.split_at_mut(rows * stride);
                slices.push(a);
                rest = b;
            }
            std::thread::scope(|s| {
                for ((&(_, rows, _), input), dst) in segs.iter().zip(&seg_inputs).zip(slices) {
                    s.spawn(move || fused_into(input, false, dst, stride, bpp, rows));
                }
            });
            t_fp.push(t.elapsed());
            fused_par = out;
        }
        let (fs, fp) = (median(t_fs), median(t_fp));
        println!(
            "   FUSED: serial {:.2} ms | {n}-thread segment-parallel {:.2} ms -> {:.2}x vs fused serial, {:.2}x vs zenpng::decode (identical: {})",
            fs.as_secs_f64() * 1e3,
            fp.as_secs_f64() * 1e3,
            fs.as_secs_f64() / fp.as_secs_f64(),
            median_full.as_secs_f64() / fp.as_secs_f64(),
            fused_par == serial_pixels && fused_serial == serial_pixels,
        );
        // ── Encode-side cost of segmenting: compress the file's own filtered
        // stream as 1/2/4/8 independent segments (window + Huffman state
        // reset at each boundary; the filter bytes are left as Apple wrote
        // them). Sizes are raw-deflate payload sums.
        if std::env::var_os("IDOT_ENC").is_some() {
            let raw = {
                let mut raw = vec![0u8; raw_len];
                Decompressor::new()
                    .with_skip_checksum(true)
                    .zlib_decompress(&all, &mut raw, Unstoppable)
                    .unwrap();
                raw
            };
            for effort in [1u32, 7, 13, 19, 24] {
                let mut line = format!("   ENC effort {effort:>2}:");
                let mut base = 0usize;
                for nseg in [1usize, 2, 4, 8] {
                    let mut total = 0usize;
                    let mut wall = Duration::ZERO;
                    let per = h.div_ceil(nseg);
                    let ranges: Vec<_> = (0..nseg)
                        .map(|k| (k * per).min(h)..((k + 1) * per).min(h))
                        .filter(|r| !r.is_empty())
                        .collect();
                    let t = Instant::now();
                    let sizes: Vec<usize> = std::thread::scope(|s| {
                        let hs: Vec<_> = ranges
                            .iter()
                            .map(|r| {
                                let input = &raw[r.start * (stride + 1)..r.end * (stride + 1)];
                                s.spawn(move || {
                                    let mut c = Compressor::new(CompressionLevel::new(effort));
                                    let mut outb =
                                        vec![0u8; Compressor::deflate_compress_bound(input.len())];
                                    c.deflate_compress(input, &mut outb, Unstoppable).unwrap()
                                })
                            })
                            .collect();
                        hs.into_iter().map(|h| h.join().unwrap()).collect()
                    });
                    wall += t.elapsed();
                    total += sizes.iter().sum::<usize>();
                    if nseg == 1 {
                        base = total;
                    }
                    line += &format!(
                        "  N={nseg}: {total} B ({:+.3}%) {:.1} ms",
                        (total as f64 / base as f64 - 1.0) * 100.0,
                        wall.as_secs_f64() * 1e3
                    );
                }
                println!("{line}");
            }
        }
        let ok_start = start_filters.iter().skip(1).all(|&f| f <= 1);
        let ok_serial = par_pixels == serial_pixels;
        let ok_ref = serial_pixels == reference.pixels.copy_to_contiguous_bytes();
        let (f, i, u, p) = (median(t_full), median(t_inf), median(t_unf), median(t_par));
        let mp = (w * h) as f64 / 1e6;
        println!(
            "   segment start filters={start_filters:?} (independent: {ok_start}); parallel==serial: {ok_serial}; manual==zenpng::decode: {ok_ref}"
        );
        println!(
            "   zenpng::decode {:.2} ms ({:.0} MP/s) | manual serial inflate {:.2} ms + unfilter {:.2} ms = {:.2} ms | {n}-thread segment-parallel {:.2} ms -> {:.2}x vs manual serial",
            f.as_secs_f64() * 1e3,
            mp / f.as_secs_f64(),
            i.as_secs_f64() * 1e3,
            u.as_secs_f64() * 1e3,
            (i + u).as_secs_f64() * 1e3,
            p.as_secs_f64() * 1e3,
            (i + u).as_secs_f64() / p.as_secs_f64(),
        );
    }
}
