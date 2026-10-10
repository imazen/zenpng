//! Worst-case cost of the zlib-end placement (`just inventory-cost`). Opt-in by
//! `INVENTORY_COST_CASE=<name>` (one case per process, so `/usr/bin/time -v` reports that
//! case's peak RSS); unset means "not requested".

use zencodec::decode::{DecodeJob, DecoderConfig};
use zenpng::PngDecoderConfig;

const SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = (data.len() as u32).to_be_bytes().to_vec();
    v.extend_from_slice(ty);
    v.extend_from_slice(data);
    let crc = zenflate::crc32(zenflate::crc32(0, ty), data);
    v.extend_from_slice(&crc.to_be_bytes());
    v
}

fn ihdr(w: u32, h: u32, depth: u8, color: u8) -> Vec<u8> {
    let mut d = w.to_be_bytes().to_vec();
    d.extend_from_slice(&h.to_be_bytes());
    d.extend_from_slice(&[depth, color, 0, 0, 0]);
    chunk(b"IHDR", &d)
}

/// zlib stream of non-final stored blocks totalling `n` bytes of zeros, optionally finished
/// with a final empty block and a (possibly wrong) Adler-32.
fn stored_stream(first: &[u8], n: usize, finish: Option<u32>, broken: bool) -> Vec<u8> {
    let mut s = vec![0x78, 0x01];
    let mut push = |b: &[u8], fin: bool| {
        s.push(u8::from(fin));
        s.extend_from_slice(&(b.len() as u16).to_le_bytes());
        s.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
        s.extend_from_slice(b);
    };
    push(first, false);
    let zeros = vec![0u8; 65535];
    let mut left = n;
    while left > 0 {
        let k = left.min(65535);
        push(&zeros[..k], false);
        left -= k;
    }
    if broken {
        s.push(0xFF);
    } else if let Some(adler) = finish {
        push(&[], true);
        s.extend_from_slice(&adler.to_be_bytes());
    }
    s
}

fn png(w: u32, h: u32, depth: u8, color: u8, idat: &[u8]) -> Vec<u8> {
    let mut v = SIG.to_vec();
    v.extend(ihdr(w, h, depth, color));
    v.extend(chunk(b"IDAT", idat));
    v.extend(chunk(b"IEND", &[]));
    v
}

fn zlib_zeros(n: usize) -> Vec<u8> {
    let mut c = zenflate::Compressor::new(zenflate::CompressionLevel::fastest());
    let zeros = vec![0u8; n];
    let mut out = vec![0u8; zenflate::Compressor::zlib_compress_bound(n)];
    let k = c
        .zlib_compress(&zeros, &mut out, zenflate::Unstoppable)
        .unwrap();
    out.truncate(k);
    out
}

#[test]
fn inventory_cost() {
    let Some(case) = std::env::var_os("INVENTORY_COST_CASE") else {
        eprintln!("INVENTORY_COST_CASE unset: cost probe not requested (just inventory-cost)");
        return;
    };
    let case = case.to_string_lossy().into_owned();
    const MIB: usize = 1 << 20;
    let sixteen = 16 * MIB - 1024; // just under RUN_SEARCH_CAP
    let bytes = match case.as_str() {
        // Rows of a 2x1 image, then ~16 MiB of valid stored excess, proper end.
        "excess_stored" => png(
            2,
            1,
            8,
            2,
            &stored_stream(&[0, 1, 2, 3, 4, 5, 6], sixteen, Some(1), false),
        ),
        // The same, but the stream breaks after the excess.
        "broken_after_rows" => png(
            2,
            1,
            8,
            2,
            &stored_stream(&[0, 1, 2, 3, 4, 5, 6], sixteen, None, true),
        ),
        // A full 4096x4096 gray8 image in ~16 MiB of stored blocks with a wrong Adler-32.
        "wrong_adler_full_image" => {
            let raw = 4096 * 4097;
            png(
                4096,
                4096,
                8,
                0,
                &stored_stream(&[], raw, Some(0xdead_beef), false),
            )
        }
        // Deflate bomb: ~1 GiB of zeros (about 1 MB compressed) behind a 2x1 header.
        "bomb_excess" => png(2, 1, 8, 2, &zlib_zeros(1 << 30)),
        // Deflate bomb under a huge IHDR (40000x30000 gray8 = 1.2 GB rows), so no excess:
        // the whole 1 GiB inflates and the end search runs (checksum is valid here).
        "bomb_huge_ihdr" => png(40000, 30000, 8, 0, &zlib_zeros(1 << 30)),
        // The same with a wrong Adler-32 (the binary-search fallback).
        "bomb_huge_ihdr_wrong_adler" => {
            let mut z = zlib_zeros(1 << 30);
            *z.last_mut().unwrap() ^= 0xff;
            png(40000, 30000, 8, 0, &z)
        }
        // 128 MiB of patterned (deflate-compressible, not trivially zero) rows with a wrong Adler-32:
        // the end search's binary-search fallback over real deflate data.
        "wrong_adler_deflate_128m" => {
            let (w, h) = (11585u32, 11585u32);
            let n = h as usize * (w as usize + 1);
            let raw: Vec<u8> = (0..n).map(|i| ((i % 1009) * 31 % 251) as u8).collect();
            let mut c = zenflate::Compressor::new(zenflate::CompressionLevel::fastest());
            let mut out = vec![0u8; zenflate::Compressor::zlib_compress_bound(n)];
            let k = c
                .zlib_compress(&raw, &mut out, zenflate::Unstoppable)
                .unwrap();
            out.truncate(k);
            *out.last_mut().unwrap() ^= 0xff;
            eprintln!("deflate stream {} B for {} B of rows", out.len(), n);
            png(w, h, 8, 0, &out)
        }
        other => panic!("unknown case {other}"),
    };
    let t = std::time::Instant::now();
    let inv = PngDecoderConfig::new()
        .job()
        .inventory(&bytes)
        .unwrap()
        .unwrap();
    eprintln!(
        "COST case={case} input={} B parts={} wall={:?}",
        bytes.len(),
        inv.parts().len(),
        t.elapsed()
    );
}
