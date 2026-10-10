//! Round-3 review probes for the PNG inventory (adopted, assertions flipped to the fixed
//! outcome): no consumed leaf part covers bytes the decoder never reads, and a chunk the
//! animation decoder reads as frame data is reported as read.

#![allow(dead_code, unused_imports)] // helpers shared by the round-3 probes

use zencodec::decode::{Decode, DecodeJob, DecodePolicy, DecoderConfig};
use zencodec::inventory::{Disposition as D, Inventory, PartTag};
use zenpng::PngDecoderConfig;

const SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&(data.len() as u32).to_be_bytes());
    v.extend_from_slice(ty);
    v.extend_from_slice(data);
    let crc = zenflate::crc32(zenflate::crc32(0, ty), data);
    v.extend_from_slice(&crc.to_be_bytes());
    v
}

fn bad_crc(mut c: Vec<u8>) -> Vec<u8> {
    *c.last_mut().unwrap() ^= 0xff;
    c
}

fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut v = vec![0x78, 0x01, 0x01];
    v.extend_from_slice(&(data.len() as u16).to_le_bytes());
    v.extend_from_slice(&(!(data.len() as u16)).to_le_bytes());
    v.extend_from_slice(data);
    v.extend_from_slice(&zenflate::adler32(1, data).to_be_bytes());
    v
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut c = zenflate::Compressor::new(zenflate::CompressionLevel::balanced());
    let mut out = vec![0u8; zenflate::Compressor::zlib_compress_bound(data.len())];
    let n = c
        .zlib_compress(data, &mut out, zenflate::Unstoppable)
        .unwrap();
    out.truncate(n);
    out
}

fn ihdr(w: u32, h: u32, depth: u8, color: u8) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend_from_slice(&w.to_be_bytes());
    d.extend_from_slice(&h.to_be_bytes());
    d.extend_from_slice(&[depth, color, 0, 0, 0]);
    chunk(b"IHDR", &d)
}

/// 2x1 RGB8 image data.
fn rgb_idat() -> Vec<u8> {
    chunk(b"IDAT", &zlib_stored(&[0, 1, 2, 3, 4, 5, 6]))
}

fn itxt(keyword: &str, flag: u8, text: &[u8]) -> Vec<u8> {
    let mut d = keyword.as_bytes().to_vec();
    d.extend_from_slice(&[0, flag, 0, 0, 0]);
    d.extend_from_slice(text);
    d
}

fn inv_with(bytes: &[u8], policy: Option<DecodePolicy>) -> Inventory {
    let mut job = PngDecoderConfig::new().job();
    if let Some(p) = policy {
        job = job.with_policy(p);
    }
    let inv = job.inventory(bytes).unwrap().unwrap();
    inv.validate()
        .unwrap_or_else(|e| panic!("invalid inventory: {e}\n{inv}"));
    inv
}

fn decode_with(
    bytes: &[u8],
    policy: Option<DecodePolicy>,
) -> Result<zencodec::decode::DecodeOutput, String> {
    let mut job = PngDecoderConfig::new().job();
    if let Some(p) = policy {
        job = job.with_policy(p);
    }
    job.decoder(bytes.into(), &[])
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())
}

fn parts_of(inv: &Inventory, ty: &[u8; 4]) -> Vec<zencodec::inventory::Part> {
    inv.parts()
        .iter()
        .filter(|p| p.tag == PartTag::FourCc(*ty))
        .cloned()
        .collect()
}

fn pixels(out: &zencodec::decode::DecodeOutput) -> Vec<u8> {
    let s = out.pixels();
    let mut v = Vec::new();
    for y in 0..s.rows() {
        v.extend_from_slice(s.row(y));
    }
    v
}

/// Every leaf part that overlaps `needle`'s occurrences in `file` must be unconsumed.
fn assert_unconsumed(inv: &Inventory, file: &[u8], needle: &[u8]) {
    let at = file
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("needle present") as u64;
    let range = at..at + needle.len() as u64;
    let mut has_child = vec![false; inv.parts().len()];
    for p in inv.parts() {
        if let Some(par) = p.parent {
            has_child[par.index()] = true;
        }
    }
    for (i, p) in inv.parts().iter().enumerate() {
        if !has_child[i] && p.range.start < range.end && p.range.end > range.start {
            assert!(
                !p.disposition.is_consumed(),
                "hidden bytes in a consumed part {:?} {:?}\n{inv}",
                p.range,
                p.disposition
            );
        }
    }
}
fn apng() -> Vec<u8> {
    let fctl = |seq: u32| {
        let mut d = seq.to_be_bytes().to_vec();
        d.extend_from_slice(&2u32.to_be_bytes());
        d.extend_from_slice(&1u32.to_be_bytes());
        d.extend_from_slice(&[0; 8]);
        d.extend_from_slice(&[0, 1, 0, 10, 0, 0]);
        d
    };
    let mut v = SIG.to_vec();
    v.extend(ihdr(2, 1, 8, 2));
    v.extend(chunk(b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0]));
    v.extend(chunk(b"fcTL", &fctl(0)));
    v.extend(rgb_idat());
    v.extend(chunk(b"fcTL", &fctl(1)));
    let mut fd = 2u32.to_be_bytes().to_vec();
    fd.extend(zlib_stored(&[0, 9, 9, 9, 8, 8, 8]));
    v.extend(chunk(b"fdAT", &fd));
    v.extend(chunk(b"IEND", &[]));
    v
}

fn frames(png: &[u8]) -> Vec<Vec<u8>> {
    use zencodec::decode::AnimationFrameDecoder;
    let mut d = PngDecoderConfig::new()
        .job()
        .animation_frame_decoder(png.into(), &[])
        .unwrap();
    let mut out = Vec::new();
    while let Some(f) = d.render_next_frame(None).unwrap() {
        let s = f.pixels();
        let mut v = Vec::new();
        for y in 0..s.rows() {
            v.extend_from_slice(s.row(y));
        }
        out.push(v);
    }
    out
}

const SECRET: &[u8] = b"SERIAL=ABC123;GPS=47.6,-122.3;OWNER=someone@example.com";

/// True when some leaf part overlapping `needle` in `file` is consumed (the defect).
fn consumed_over(inv: &Inventory, file: &[u8], needle: &[u8]) -> bool {
    let at = file
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("needle") as u64;
    let r = at..at + needle.len() as u64;
    let mut has_child = vec![false; inv.parts().len()];
    for p in inv.parts() {
        if let Some(par) = p.parent {
            has_child[par.index()] = true;
        }
    }
    inv.parts().iter().enumerate().any(|(i, p)| {
        !has_child[i]
            && p.range.start < r.end
            && p.range.end > r.start
            && p.disposition.is_consumed()
    })
}

fn leaf_over(inv: &Inventory, file: &[u8], needle: &[u8]) -> String {
    let at = file
        .windows(needle.len())
        .position(|w| w == needle)
        .unwrap() as u64;
    inv.parts()
        .iter()
        .filter(|p| p.range.start <= at && p.range.end > at)
        .map(|p| {
            format!(
                "{:?} {:?} {:?} {:?}",
                p.kind, p.range, p.disposition, p.detail
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn png_rgb(idats: &[Vec<u8>]) -> Vec<u8> {
    let mut v = SIG.to_vec();
    v.extend(ihdr(2, 1, 8, 2));
    for c in idats {
        v.extend_from_slice(c);
    }
    v.extend(chunk(b"IEND", &[]));
    v
}

// ── Round 3 probes ───────────────────────────────────────────────────────────

fn png_idx(depth: u8, pre: &[Vec<u8>], idat: &[u8]) -> Vec<u8> {
    let mut v = SIG.to_vec();
    v.extend(ihdr(2, 1, depth, 3));
    for c in pre {
        v.extend_from_slice(c);
    }
    v.extend(chunk(b"IDAT", &zlib_stored(idat)));
    v.extend(chunk(b"IEND", &[]));
    v
}

fn pal(n: usize) -> Vec<u8> {
    (0..n * 3).map(|i| (i * 37 % 256) as u8).collect()
}

/// Held tRNS flushed at the first PLTE; a later, smaller PLTE wins.
#[test]
fn r3_trns_held_then_smaller_plte_wins() {
    let with = png_idx(
        8,
        &[
            chunk(b"tRNS", SECRET),
            chunk(b"PLTE", &pal(60)),
            chunk(b"PLTE", &pal(2)),
        ],
        &[0, 0, 1],
    );
    let base = png_idx(
        8,
        &[
            chunk(b"tRNS", &SECRET[..2]),
            chunk(b"PLTE", &pal(60)),
            chunk(b"PLTE", &pal(2)),
        ],
        &[0, 0, 1],
    );
    let inv = inv_with(&with, None);
    let a = decode_with(&with, None).unwrap();
    let b = decode_with(&base, None).unwrap();
    eprintln!(
        "equal={} leaf: {}",
        pixels(&a) == pixels(&b),
        leaf_over(&inv, &with, &SECRET[2..])
    );
    assert_eq!(pixels(&a), pixels(&b));
    assert!(
        !consumed_over(&inv, &with, &SECRET[2..]),
        "hidden bytes in a consumed part\n{inv}"
    );
}

/// tRNS after a large PLTE, then a smaller PLTE replaces the palette.
#[test]
fn r3_trns_after_plte_then_smaller_plte_wins() {
    let with = png_idx(
        8,
        &[
            chunk(b"PLTE", &pal(60)),
            chunk(b"tRNS", SECRET),
            chunk(b"PLTE", &pal(2)),
        ],
        &[0, 0, 1],
    );
    let base = png_idx(
        8,
        &[
            chunk(b"PLTE", &pal(60)),
            chunk(b"tRNS", &SECRET[..2]),
            chunk(b"PLTE", &pal(2)),
        ],
        &[0, 0, 1],
    );
    let inv = inv_with(&with, None);
    let a = decode_with(&with, None).unwrap();
    let b = decode_with(&base, None).unwrap();
    eprintln!(
        "equal={} leaf: {}",
        pixels(&a) == pixels(&b),
        leaf_over(&inv, &with, &SECRET[2..])
    );
    assert_eq!(pixels(&a), pixels(&b));
    assert!(
        !consumed_over(&inv, &with, &SECRET[2..]),
        "hidden bytes in a consumed part\n{inv}"
    );
}

/// 1-bit indexed: tRNS entries past index 1 are unreachable.
#[test]
fn r3_trns_beyond_bit_depth() {
    let with = png_idx(
        1,
        &[chunk(b"PLTE", &pal(60)), chunk(b"tRNS", SECRET)],
        &[0, 0b0100_0000],
    );
    let base = png_idx(
        1,
        &[chunk(b"PLTE", &pal(60)), chunk(b"tRNS", &SECRET[..2])],
        &[0, 0b0100_0000],
    );
    let inv = inv_with(&with, None);
    let a = decode_with(&with, None).unwrap();
    let b = decode_with(&base, None).unwrap();
    eprintln!(
        "equal={} leaf: {}",
        pixels(&a) == pixels(&b),
        leaf_over(&inv, &with, &SECRET[2..])
    );
    assert_eq!(pixels(&a), pixels(&b));
    assert!(
        !consumed_over(&inv, &with, &SECRET[2..]),
        "hidden bytes in a consumed part\n{inv}"
    );
}

/// Non-final stored blocks: the rows, then ~1 MiB of valid excess, then an invalid block.
fn broken_after_rows(raw: &[u8]) -> Vec<u8> {
    let mut s = vec![0x78, 0x01];
    let mut blocks: Vec<Vec<u8>> = vec![raw.to_vec()];
    for _ in 0..16 {
        blocks.push(vec![0u8; 65535]);
    }
    blocks.push(SECRET.to_vec());
    for b in &blocks {
        s.push(0x00); // BFINAL=0, BTYPE=00
        s.extend_from_slice(&(b.len() as u16).to_le_bytes());
        s.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
        s.extend_from_slice(b);
    }
    s.push(0xFF); // BFINAL=1, BTYPE=11: invalid
    s.extend_from_slice(b"tail after the break");
    s
}

#[test]
fn r3_stream_breaks_after_the_last_row() {
    let raw = [0u8, 1, 2, 3, 4, 5, 6];
    let with = png_rgb(&[chunk(b"IDAT", &broken_after_rows(&raw))]);
    let base = png_rgb(&[chunk(b"IDAT", &zlib_stored(&raw))]);
    let inv = inv_with(&with, None);
    let a = decode_with(&with, None);
    let s = decode_with(&with, Some(DecodePolicy::none().with_strict(true)));
    let b = decode_with(&base, None).unwrap();
    let idat = &parts_of(&inv, b"IDAT")[0];
    eprintln!(
        "default ok={} strict ok={} detail={:?}\nleaf over secret: {}",
        a.is_ok(),
        s.is_ok(),
        idat.detail,
        leaf_over(&inv, &with, SECRET)
    );
    assert_eq!(pixels(&a.unwrap()), pixels(&b));
    // The fix's claim: nothing after the rows is consumed.
    assert!(
        !consumed_over(&inv, &with, SECRET),
        "secret consumed\n{inv}"
    );
    assert!(!consumed_over(&inv, &with, b"tail after the break"));
    let kids: Vec<_> = inv
        .parts()
        .iter()
        .filter(|p| p.parent.is_some())
        .map(|p| (p.range.clone(), p.disposition))
        .collect();
    eprintln!("children: {kids:?}");
}
