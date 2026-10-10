//! Round-2 review probes for the PNG inventory (adopted): each asserts that no consumed
//! leaf part covers bytes the decoder never reads (the reviewer's probes asserted the defect).

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
fn _assert_unconsumed(inv: &Inventory, file: &[u8], needle: &[u8]) {
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

#[test]
fn r2_wrong_adler_trailing_idat_is_skipped() {
    let mut stream = zlib_stored(&[0, 1, 2, 3, 4, 5, 6]);
    *stream.last_mut().unwrap() ^= 0xff; // wrong Adler-32
    let base = png_rgb(&[chunk(b"IDAT", &stream)]);
    let with = png_rgb(&[chunk(b"IDAT", &stream), chunk(b"IDAT", SECRET)]);
    let inv = inv_with(&with, None);
    let a = decode_with(&with, None).expect("default decode skips the Adler check");
    let b = decode_with(&base, None).unwrap();
    eprintln!("leaf over secret: {}", leaf_over(&inv, &with, SECRET));
    assert_eq!(pixels(&a), pixels(&b));
    assert!(
        !consumed_over(&inv, &with, SECRET),
        "hidden bytes in a consumed part\n{inv}"
    );
}

#[test]
fn r2_excess_decompressed_idat_data_is_not_consumed() {
    let raw = [0u8, 1, 2, 3, 4, 5, 6];
    let base = png_rgb(&[chunk(b"IDAT", &zlib_stored(&raw))]);
    let with = png_rgb(&[chunk(b"IDAT", &zlib_stored(&[&raw[..], SECRET].concat()))]);
    let inv = inv_with(&with, None);
    for policy in [None, Some(DecodePolicy::none().with_strict(true))] {
        let a = decode_with(&with, policy).unwrap();
        let b = decode_with(&base, policy).unwrap();
        assert_eq!(pixels(&a), pixels(&b));
    }
    eprintln!("leaf over secret: {}", leaf_over(&inv, &with, SECRET));
    assert!(
        !consumed_over(&inv, &with, SECRET),
        "hidden bytes in a consumed part\n{inv}"
    );
}

#[test]
fn r2_frames_beyond_actl_count_are_not_consumed() {
    let base = apng();
    let iend = base.len() - 12;
    let mut f3 = 3u32.to_be_bytes().to_vec();
    f3.extend_from_slice(&2u32.to_be_bytes());
    f3.extend_from_slice(&1u32.to_be_bytes());
    f3.extend_from_slice(&[0; 8]);
    f3.extend_from_slice(&[0, 1, 0, 10, 0, 0]);
    let mut fd = 4u32.to_be_bytes().to_vec();
    fd.extend_from_slice(SECRET);
    let mut with = base[..iend].to_vec();
    with.extend(chunk(b"fcTL", &f3));
    with.extend(chunk(b"fdAT", &fd));
    with.extend_from_slice(&base[iend..]);
    let inv = inv_with(&with, None);
    assert_eq!(frames(&with), frames(&base));
    eprintln!(
        "frames {}; leaf over secret: {}",
        frames(&with).len(),
        leaf_over(&inv, &with, SECRET)
    );
    assert!(
        !consumed_over(&inv, &with, SECRET),
        "hidden bytes in a consumed part\n{inv}"
    );
}

#[test]
fn r2_fdat_after_frame_stream_end_is_not_consumed() {
    let base = apng();
    let iend = base.len() - 12;
    let mut fd = 3u32.to_be_bytes().to_vec();
    fd.extend_from_slice(SECRET);
    let mut with = base[..iend].to_vec();
    with.extend(chunk(b"fdAT", &fd));
    with.extend_from_slice(&base[iend..]);
    let inv = inv_with(&with, None);
    assert_eq!(frames(&with), frames(&base));
    eprintln!("leaf over secret: {}", leaf_over(&inv, &with, SECRET));
    assert!(
        !consumed_over(&inv, &with, SECRET),
        "hidden bytes in a consumed part\n{inv}"
    );
}

#[test]
fn r2_long_iccp_profile_name_is_not_consumed() {
    let profile = b"fake icc profile bytes";
    let mut name = Vec::new();
    while name.len() < 120 {
        name.extend_from_slice(SECRET);
    }
    let mut body = name.clone();
    body.extend_from_slice(&[0, 0]);
    body.extend(zlib_stored(profile));
    let with = png_rgb(&[chunk(b"iCCP", &body), rgb_idat()]);
    // png_rgb puts IDATs after IHDR; iCCP must precede IDAT, so build directly.
    let mut v = SIG.to_vec();
    v.extend(ihdr(2, 1, 8, 2));
    v.extend(chunk(b"iCCP", &body));
    v.extend(rgb_idat());
    v.extend(chunk(b"IEND", &[]));
    let _ = with;
    let inv = inv_with(&v, None);
    let out = decode_with(&v, None).unwrap();
    assert_eq!(
        out.info().metadata().icc_profile.as_deref(),
        Some(&profile[..])
    );
    eprintln!("leaf over secret: {}", leaf_over(&inv, &v, SECRET));
    assert!(
        !consumed_over(&inv, &v, SECRET),
        "hidden bytes in a consumed part\n{inv}"
    );
}

#[test]
fn r2_xmp_language_tag_is_not_consumed() {
    let mut d = b"XML:com.adobe.xmp\0".to_vec();
    d.extend_from_slice(&[0, 0]);
    d.extend_from_slice(SECRET); // language tag
    d.push(0);
    d.push(0); // empty translated keyword
    d.extend_from_slice(b"<x:xmpmeta/>");
    let mut v = SIG.to_vec();
    v.extend(ihdr(2, 1, 8, 2));
    v.extend(chunk(b"iTXt", &d));
    v.extend(rgb_idat());
    v.extend(chunk(b"IEND", &[]));
    let inv = inv_with(&v, None);
    let out = decode_with(&v, None).unwrap();
    assert_eq!(
        out.info().metadata().xmp.as_deref(),
        Some(&b"<x:xmpmeta/>"[..])
    );
    eprintln!("leaf over secret: {}", leaf_over(&inv, &v, SECRET));
    assert!(
        !consumed_over(&inv, &v, SECRET),
        "hidden bytes in a consumed part\n{inv}"
    );
}

#[test]
fn r2_trns_before_plte_longer_than_palette_is_not_consumed() {
    let mk = |trns: &[u8]| {
        let mut v = SIG.to_vec();
        v.extend(ihdr(2, 1, 8, 3));
        v.extend(chunk(b"tRNS", trns));
        v.extend(chunk(b"PLTE", &[255, 0, 0, 0, 255, 0]));
        v.extend(chunk(b"IDAT", &zlib_stored(&[0, 0, 1])));
        v.extend(chunk(b"IEND", &[]));
        v
    };
    let with = mk(SECRET);
    let short = mk(&SECRET[..2]);
    let inv = inv_with(&with, None);
    let a = decode_with(&with, None).unwrap();
    let b = decode_with(&short, None).unwrap();
    eprintln!(
        "equal={} leaf over secret tail: {}",
        pixels(&a) == pixels(&b),
        leaf_over(&inv, &with, &SECRET[2..])
    );
    assert_eq!(pixels(&a), pixels(&b));
    assert!(
        !consumed_over(&inv, &with, &SECRET[2..]),
        "hidden bytes in a consumed part\n{inv}"
    );
}

#[test]
fn r2_plte_entries_beyond_bit_depth_are_not_consumed() {
    let mk = |extra: &[u8]| {
        let mut pal = vec![255, 0, 0, 0, 255, 0];
        pal.extend_from_slice(extra);
        let mut v = SIG.to_vec();
        v.extend(ihdr(2, 1, 1, 3)); // 1-bit indexed: indices 0..=1 only
        v.extend(chunk(b"PLTE", &pal));
        v.extend(chunk(b"IDAT", &zlib_stored(&[0, 0b0100_0000])));
        v.extend(chunk(b"IEND", &[]));
        v
    };
    let mut extra = SECRET.to_vec();
    while !extra.len().is_multiple_of(3) {
        extra.push(b' ');
    }
    let with = mk(&extra);
    let base = mk(&[]);
    let inv = inv_with(&with, None);
    let a = decode_with(&with, None).unwrap();
    let b = decode_with(&base, None).unwrap();
    eprintln!(
        "equal={} leaf over secret: {}",
        pixels(&a) == pixels(&b),
        leaf_over(&inv, &with, SECRET)
    );
    assert_eq!(pixels(&a), pixels(&b));
    assert!(
        !consumed_over(&inv, &with, SECRET),
        "hidden bytes in a consumed part\n{inv}"
    );
}

#[test]
fn r2_post_idat_bad_crc_detail_does_not_claim_strict_skips_it() {
    let mut png = SIG.to_vec();
    png.extend(ihdr(2, 1, 8, 2));
    png.extend(rgb_idat());
    png.extend(bad_crc(chunk(b"eXIf", b"II*\0\x08\0\0\0\0\0")));
    png.extend(chunk(b"IEND", &[]));
    let inv = inv_with(&png, Some(DecodePolicy::none().with_strict(true)));
    let d = parts_of(&inv, b"eXIf")[0].detail.clone().unwrap();
    assert!(d.contains("decode() reads it unchecked"), "{d}");
    assert!(
        d.contains("probe() and the animation decoder skip it"),
        "{d}"
    );
    // probe() checks the CRC, so it drops the chunk the still decode reads.
    assert!(
        PngDecoderConfig::new()
            .job()
            .probe(&png)
            .unwrap()
            .metadata()
            .exif
            .is_none()
    );
    assert!(!d.contains("strict policy skips"), "{d}");
}

/// Excess data inside a *compressed* stream (not stored blocks).
#[test]
fn r2_excess_compressed_data_is_not_consumed() {
    let (w, h) = (64u32, 64u32);
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0u8);
        for x in 0..w * 3 {
            raw.push(((x * 7 + y * 13) % 251) as u8);
        }
    }
    let mk = |extra: &[u8]| {
        let mut v = SIG.to_vec();
        v.extend(ihdr(w, h, 8, 2));
        v.extend(chunk(b"IDAT", &zlib(&[&raw[..], extra].concat())));
        v.extend(chunk(b"IEND", &[]));
        v
    };
    let with = mk(&SECRET.repeat(40));
    let base = mk(&[]);
    let inv = inv_with(&with, None);
    assert_eq!(
        pixels(&decode_with(&with, None).unwrap()),
        pixels(&decode_with(&base, None).unwrap())
    );
    // The compressed secret is not literally present; the IDAT must carry a split.
    let idat = parts_of(&inv, b"IDAT");
    assert!(
        inv.parts()
            .iter()
            .any(|p| p.parent.is_some() && p.disposition == D::Unreferenced),
        "{inv}"
    );
    let _ = idat;
}
