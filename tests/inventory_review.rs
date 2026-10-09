//! Adversarial probes from the independent review of the PNG inventory (round 1),
//! adopted as regression tests. Each asserts the corrected behaviour: no consumed
//! part holds bytes the decoder never reads, and policy-dependent dispositions
//! agree with what `decode()` returns.

use zencodec::decode::{Decode, DecodeJob, DecodePolicy, DecoderConfig};
use zencodec::inventory::{Disposition as D, Inventory, MetadataKind as M, PartTag};
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

// ── 1. Strict policy: post-IDAT bad-CRC chunks are still collected ──────────

fn strict_crc_only() -> DecodePolicy {
    DecodePolicy::none().with_strict(true)
}

#[test]
fn strict_post_idat_bad_crc_exif_is_consumed() {
    let exif = b"II*\0\x08\0\0\0\0\0";
    let mut png = SIG.to_vec();
    png.extend(ihdr(2, 1, 8, 2));
    png.extend(rgb_idat());
    png.extend(bad_crc(chunk(b"eXIf", exif)));
    png.extend(chunk(b"IEND", &[]));

    let inv = inv_with(&png, Some(strict_crc_only()));
    let p = &parts_of(&inv, b"eXIf")[0];
    eprintln!(
        "inventory (strict crc): {:?} / {:?}",
        p.disposition, p.detail
    );
    let out = decode_with(&png, Some(strict_crc_only())).expect("strict decode ok");
    let got = out.info().metadata().exif;
    eprintln!("strict-crc decode exif: {:?}", got.as_deref());
    assert_eq!(
        p.disposition,
        D::Metadata(M::Exif),
        "post-IDAT chunks are read unchecked"
    );
    assert_eq!(
        got.as_deref(),
        Some(&exif[..]),
        "decode surfaces the bad-CRC eXIf"
    );
}

/// `DecodePolicy::strict()` also sets allow_exif=false, which hid CRC handling in the
/// first version of the author's test; use strict CRC alone and check both CRC states.
#[test]
fn strict_crc_alone_separates_good_and_bad_pre_idat_exif() {
    let exif = b"II*\0\x08\0\0\0\0\0";
    let mk = |c: Vec<u8>| {
        let mut v = SIG.to_vec();
        v.extend(ihdr(2, 1, 8, 2));
        v.extend(c);
        v.extend(rgb_idat());
        v.extend(chunk(b"IEND", &[]));
        v
    };
    let good = mk(chunk(b"eXIf", exif));
    let out = decode_with(&good, Some(strict_crc_only())).unwrap();
    assert!(out.info().metadata().exif.is_some());
    assert_eq!(
        parts_of(&inv_with(&good, Some(strict_crc_only())), b"eXIf")[0].disposition,
        D::Metadata(M::Exif)
    );
    let bad = mk(bad_crc(chunk(b"eXIf", exif)));
    let out = decode_with(&bad, Some(strict_crc_only())).unwrap();
    assert!(out.info().metadata().exif.is_none());
    assert_eq!(
        parts_of(&inv_with(&bad, Some(strict_crc_only())), b"eXIf")[0].disposition,
        D::Dropped
    );
    // strict() also bans EXIF outright, so a good chunk is Dropped for that reason.
    let out = decode_with(&good, Some(DecodePolicy::strict())).unwrap();
    assert!(out.info().metadata().exif.is_none());
    assert_eq!(
        parts_of(&inv_with(&good, Some(DecodePolicy::strict())), b"eXIf")[0].disposition,
        D::Dropped
    );
}

#[test]
fn strict_bad_crc_chunk_before_ihdr_leaves_ihdr_intact() {
    // A bad-CRC ancillary chunk before IHDR: strict ChunkIter skips it, so the
    // decoder sees IHDR first. The walker has already consumed its "first" flag.
    let mut png = SIG.to_vec();
    png.extend(bad_crc(chunk(b"tEXt", b"Key\0v")));
    png.extend(ihdr(2, 1, 8, 3));
    png.extend(chunk(b"PLTE", &[255, 0, 0, 0, 255, 0]));
    png.extend(chunk(b"IDAT", &zlib_stored(&[0, 0, 1])));
    png.extend(chunk(b"IEND", &[]));
    let inv = inv_with(&png, Some(strict_crc_only()));
    eprintln!("{inv}");
    let strict = decode_with(&png, Some(strict_crc_only()));
    let lenient = decode_with(&png, None);
    eprintln!(
        "strict decode ok={} lenient decode ok={}",
        strict.is_ok(),
        lenient.is_ok()
    );
    assert!(strict.is_ok());
    assert_eq!(parts_of(&inv, b"IHDR")[0].disposition, D::Structure);
    assert_eq!(parts_of(&inv, b"PLTE")[0].disposition, D::ImageData);
    assert_eq!(parts_of(&inv, b"tEXt")[0].disposition, D::Dropped);
}

// ── 2. EXIF suppressed by policy still feeds ImageInfo::orientation ─────────

#[test]
fn exif_suppressed_by_policy_keeps_orientation_disposition() {
    // TIFF LE, one IFD entry: Orientation (0x0112) SHORT = 6.
    let exif: &[u8] = &[
        b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
    ];
    let mut png = SIG.to_vec();
    png.extend(ihdr(2, 1, 8, 2));
    png.extend(chunk(b"eXIf", exif));
    png.extend(rgb_idat());
    png.extend(chunk(b"IEND", &[]));
    let policy = || DecodePolicy::none().with_allow_exif(false);
    let inv = inv_with(&png, Some(policy()));
    let p = &parts_of(&inv, b"eXIf")[0];
    let out = decode_with(&png, Some(policy())).unwrap();
    let plain = decode_with(&png, None).unwrap();
    let none_png = {
        let mut v = SIG.to_vec();
        v.extend(ihdr(2, 1, 8, 2));
        v.extend(rgb_idat());
        v.extend(chunk(b"IEND", &[]));
        v
    };
    let baseline = decode_with(&none_png, None).unwrap();
    eprintln!(
        "inventory: {:?}; decode(policy) exif={:?} orientation={:?}; no-eXIf orientation={:?}; default-policy orientation={:?}",
        p.disposition,
        out.info().metadata().exif.is_some(),
        out.info().orientation,
        baseline.info().orientation,
        plain.info().orientation,
    );
    assert!(out.info().metadata().exif.is_none());
    assert_ne!(
        format!("{:?}", out.info().orientation),
        format!("{:?}", baseline.info().orientation),
        "orientation from the suppressed eXIf reaches ImageInfo"
    );
    assert_eq!(p.disposition, D::Metadata(M::Orientation));
    // Without an Orientation tag the blob is simply dropped.
    let mut plain_exif = SIG.to_vec();
    plain_exif.extend(ihdr(2, 1, 8, 2));
    plain_exif.extend(chunk(b"eXIf", b"II*\0\x08\0\0\0\0\0"));
    plain_exif.extend(rgb_idat());
    plain_exif.extend(chunk(b"IEND", &[]));
    let inv = inv_with(&plain_exif, Some(policy()));
    assert_eq!(parts_of(&inv, b"eXIf")[0].disposition, D::Dropped);
}

// ── 3. Bytes hidden inside consumed parts ───────────────────────────────────

const SECRET: &[u8] = b"SERIAL=ABC123;GPS=47.6,-122.3;OWNER=someone@example.com";

#[test]
fn trns_tail_is_unreferenced() {
    // Gray8 2x1, tRNS = [0, 5] + secret. The decoder reads the first 2 bytes.
    let mk = |trns: &[u8]| {
        let mut v = SIG.to_vec();
        v.extend(ihdr(2, 1, 8, 0));
        v.extend(chunk(b"tRNS", trns));
        v.extend(chunk(b"IDAT", &zlib_stored(&[0, 5, 9])));
        v.extend(chunk(b"IEND", &[]));
        v
    };
    let with = mk(&[&[0, 5][..], SECRET].concat());
    let without = mk(&[0, 5]);
    let inv = inv_with(&with, None);
    let p = &parts_of(&inv, b"tRNS")[0];
    let a = pixels(&decode_with(&with, None).unwrap());
    let b = pixels(&decode_with(&without, None).unwrap());
    eprintln!(
        "tRNS part {:?} len {} disposition {:?}; pixels equal: {}",
        p.range,
        p.len(),
        p.disposition,
        a == b
    );
    assert_eq!(p.disposition, D::ImageData);
    assert_eq!(a, b, "the secret tail of tRNS never affects pixels");
    assert_unconsumed(&inv, &with, SECRET);
}

#[test]
fn oversized_indexed_trns_bytes_are_unreferenced() {
    // Indexed, 2-entry palette, tRNS longer than the palette: the decoder keeps
    // presence only (alpha all 255) and discards every byte.
    let mk = |trns: Option<&[u8]>| {
        let mut v = SIG.to_vec();
        v.extend(ihdr(2, 1, 8, 3));
        v.extend(chunk(b"PLTE", &[255, 0, 0, 0, 255, 0]));
        if let Some(t) = trns {
            v.extend(chunk(b"tRNS", t));
        }
        v.extend(chunk(b"IDAT", &zlib_stored(&[0, 0, 1])));
        v.extend(chunk(b"IEND", &[]));
        v
    };
    let with = mk(Some(SECRET));
    let opaque = mk(Some(&[255, 255]));
    let inv = inv_with(&with, None);
    let p = &parts_of(&inv, b"tRNS")[0];
    let a = pixels(&decode_with(&with, None).unwrap());
    let b = pixels(&decode_with(&opaque, None).unwrap());
    eprintln!(
        "tRNS {:?}: pixels equal to all-opaque tRNS: {}",
        p.disposition,
        a == b
    );
    assert_eq!(p.disposition, D::ImageData);
    assert_eq!(a, b);
    assert_unconsumed(&inv, &with, SECRET);
}

#[test]
fn plte_entries_past_256_are_unreferenced() {
    let mut pal: Vec<u8> = (0..256 * 3).map(|i| i as u8).collect();
    let mk = |pal: &[u8]| {
        let mut v = SIG.to_vec();
        v.extend(ihdr(2, 1, 8, 3));
        v.extend(chunk(b"PLTE", pal));
        v.extend(chunk(b"IDAT", &zlib_stored(&[0, 0, 255])));
        v.extend(chunk(b"IEND", &[]));
        v
    };
    let base = mk(&pal);
    // 18 extra "entries" = the secret padded to a multiple of 3.
    let mut extra = SECRET.to_vec();
    while !extra.len().is_multiple_of(3) {
        extra.push(b' ');
    }
    pal.extend_from_slice(&extra);
    let with = mk(&pal);
    let inv = inv_with(&with, None);
    let p = &parts_of(&inv, b"PLTE")[0];
    let r = decode_with(&with, None);
    eprintln!("PLTE {:?} decode ok={}", p.disposition, r.is_ok());
    let a = pixels(&r.unwrap());
    let b = pixels(&decode_with(&base, None).unwrap());
    assert_eq!(p.disposition, D::ImageData);
    assert_eq!(a, b);
    assert_unconsumed(&inv, &with, &pal[768..]);
}

#[test]
fn iend_payload_is_unreferenced() {
    let mut png = SIG.to_vec();
    png.extend(ihdr(2, 1, 8, 2));
    png.extend(rgb_idat());
    png.extend(chunk(b"IEND", SECRET));
    let inv = inv_with(&png, None);
    let p = &parts_of(&inv, b"IEND")[0];
    eprintln!("IEND len {} {:?} {:?}", p.len(), p.disposition, p.detail);
    assert!(decode_with(&png, None).is_ok());
    assert_eq!(p.disposition, D::Structure);
    assert_eq!(p.len(), 12 + SECRET.len() as u64);
    assert_unconsumed(&inv, &png, SECRET);
}

#[test]
fn idat_after_zlib_end_is_skipped_and_junk_inside_the_last_idat_is_unreferenced() {
    // 64x64 RGB8 so the streaming path (not only the stored fast path) runs.
    let (w, h) = (64u32, 64u32);
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0u8);
        for x in 0..w * 3 {
            raw.push(((x * 7 + y * 13) % 251) as u8);
        }
    }
    for (name, stream) in [
        ("compressed", zlib(&raw)),
        ("stored-ish", zlib_stored(&raw[..7])),
    ] {
        let (w, h) = if name == "compressed" { (w, h) } else { (2, 1) };
        let mut base = SIG.to_vec();
        base.extend(ihdr(w, h, 8, 2));
        base.extend(chunk(b"IDAT", &stream));
        let mut with = base.clone();
        with.extend(chunk(b"IDAT", SECRET));
        base.extend(chunk(b"IEND", &[]));
        with.extend(chunk(b"IEND", &[]));
        let inv = inv_with(&with, None);
        let idats = parts_of(&inv, b"IDAT");
        assert_eq!(idats[0].disposition, D::ImageData);
        assert_eq!(
            idats[1].disposition,
            D::Skipped,
            "{name}\n{inv}\n{:?}",
            inv.parts()
                .iter()
                .map(|p| p.detail.clone())
                .collect::<Vec<_>>()
        );
        assert_unconsumed(&inv, &with, SECRET);
        // Junk glued to the end of the same IDAT, after the zlib footer.
        let mut glued = SIG.to_vec();
        glued.extend(ihdr(w, h, 8, 2));
        glued.extend(chunk(b"IDAT", &[&stream[..], SECRET].concat()));
        glued.extend(chunk(b"IEND", &[]));
        let inv_g = inv_with(&glued, None);
        assert_eq!(
            pixels(&decode_with(&glued, None).unwrap()),
            pixels(&decode_with(&base, None).unwrap())
        );
        assert_unconsumed(&inv_g, &glued, SECRET);
        for (pname, policy) in [("default", None), ("strict-crc", Some(strict_crc_only()))] {
            let a = decode_with(&with, policy);
            let b = decode_with(&base, policy).unwrap();
            eprintln!(
                "{name} {pname}: decode ok={} {:?} equal-to-base={}",
                a.is_ok(),
                a.as_ref().err(),
                a.as_ref().ok().map(pixels) == Some(pixels(&b))
            );
            if pname == "default" {
                assert_eq!(pixels(&a.unwrap()), pixels(&b));
            }
        }
    }
}

// ── policy allow_animation=false: fcTL/fdAT are never read ──────────────────

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

#[test]
fn animation_policy_skips_frame_chunks() {
    let png = apng();
    let policy = DecodePolicy::none().with_allow_animation(false);
    let inv = inv_with(&png, Some(policy));
    let fdat = &parts_of(&inv, b"fdAT")[0];
    let fctl = parts_of(&inv, b"fcTL");
    let anim = PngDecoderConfig::new()
        .job()
        .with_policy(policy)
        .animation_frame_decoder(png.as_slice().into(), &[]);
    let one = decode_with(&png, Some(policy)).unwrap();
    eprintln!(
        "fdAT {:?}, fcTL {:?}/{:?}; animation decoder ok={}; decode() sequence {:?}",
        fdat.disposition,
        fctl[0].disposition,
        fctl[1].disposition,
        anim.is_ok(),
        one.info().sequence
    );
    // Control: default policy animation decoder works.
    assert!(
        PngDecoderConfig::new()
            .job()
            .animation_frame_decoder(png.as_slice().into(), &[])
            .is_ok()
    );
    assert!(anim.is_err());
    assert_eq!(fdat.disposition, D::Skipped);
    assert_eq!(fctl[0].disposition, D::Skipped);
    assert_eq!(fctl[1].disposition, D::Skipped);
    // acTL still reaches ImageInfo through decode().
    let acl = parts_of(&inv, b"acTL");
    assert_eq!(acl[0].disposition, D::Metadata(M::Animation));
}

#[test]
fn iccp_bytes_after_the_zlib_stream_are_unreferenced() {
    let profile = b"fake icc profile bytes";
    let mut body = b"name\0\0".to_vec();
    body.extend(zlib_stored(profile));
    body.extend_from_slice(SECRET);
    let mut png = SIG.to_vec();
    png.extend(ihdr(2, 1, 8, 2));
    png.extend(chunk(b"iCCP", &body));
    png.extend(rgb_idat());
    png.extend(chunk(b"IEND", &[]));
    let inv = inv_with(&png, None);
    let p = &parts_of(&inv, b"iCCP")[0];
    let out = decode_with(&png, None).unwrap();
    let icc = out.info().metadata().icc_profile;
    eprintln!(
        "iCCP {:?}; decoded icc = {:?}",
        p.disposition,
        icc.as_deref()
            .map(|b| String::from_utf8_lossy(b).into_owned())
    );
    assert_eq!(p.disposition, D::Metadata(M::Icc));
    assert_eq!(icc.as_deref(), Some(&profile[..]));
    assert_unconsumed(&inv, &png, SECRET);
}

#[test]
fn invalid_idot_is_skipped() {
    let mut png = SIG.to_vec();
    png.extend(ihdr(2, 1, 8, 2));
    png.extend(chunk(b"iDOT", SECRET));
    png.extend(rgb_idat());
    png.extend(chunk(b"IEND", &[]));
    let inv = inv_with(&png, None);
    let p = &parts_of(&inv, b"iDOT")[0];
    eprintln!("iDOT {:?} {:?}", p.disposition, p.detail);
    assert!(decode_with(&png, None).is_ok());
    assert_eq!(p.disposition, D::Skipped);
    assert_unconsumed(&inv, &png, SECRET);
}

// ── 4. The inventory's own inflate budget changes dispositions ──────────────

#[test]
fn many_large_profiles_do_not_change_icc_or_xmp_dispositions() {
    let mut png = SIG.to_vec();
    png.extend(ihdr(2, 1, 8, 2));
    // 256 valid iCCP chunks, each inflating to exactly 1 MiB (the decoder's cap).
    for i in 0..256u32 {
        let profile = vec![(i % 251) as u8; 1 << 20];
        let mut body = format!("p{i}\0\0").into_bytes();
        body.extend(zlib(&profile));
        png.extend(chunk(b"iCCP", &body));
    }
    // A broken profile: the decoder ignores it and keeps the 256th.
    png.extend(chunk(b"iCCP", b"broken\0\0not zlib"));
    // A compressed XMP: the decoder inflates and reports it.
    png.extend(chunk(
        b"iTXt",
        &itxt("XML:com.adobe.xmp", 1, &zlib(b"<x:xmpmeta/>")),
    ));
    png.extend(rgb_idat());
    png.extend(chunk(b"IEND", &[]));
    eprintln!("file size {} bytes", png.len());

    let inv = inv_with(&png, None);
    let iccp = parts_of(&inv, b"iCCP");
    let xmp = &parts_of(&inv, b"iTXt")[0];
    let out = decode_with(&png, None).unwrap();
    let m = out.info().metadata();
    let icc = m.icc_profile.as_deref().unwrap();
    eprintln!(
        "decode: icc len {} first byte {}, xmp {:?}",
        icc.len(),
        icc[0],
        m.xmp
            .as_deref()
            .map(|b| String::from_utf8_lossy(b).into_owned())
    );
    eprintln!(
        "inventory: iCCP#255 {:?} {:?}; broken iCCP {:?} {:?}; XMP {:?} {:?}",
        iccp[255].disposition,
        iccp[255].detail,
        iccp[256].disposition,
        iccp[256].detail,
        xmp.disposition,
        xmp.detail
    );
    assert_eq!(icc[0], (255 % 251) as u8, "decoder keeps the 256th profile");
    assert!(m.xmp.is_some(), "decoder reports the XMP");
    assert_eq!(iccp[255].disposition, D::Metadata(M::Icc));
    assert_eq!(iccp[256].disposition, D::Malformed);
    assert_eq!(xmp.disposition, D::Metadata(M::Xmp));
}

// ── 5. Truncation / no IEND: decode rejects what the detail says is tolerated ─

#[test]
fn partial_tail_chunk_is_reported_as_rejected_by_decode() {
    let mut png = SIG.to_vec();
    png.extend(ihdr(2, 1, 8, 2));
    png.extend(rgb_idat());
    png.extend_from_slice(&[0, 0, 0, 20, b't', b'E']); // 6 junk bytes, no IEND
    let inv = inv_with(&png, None);
    let last = inv.parts().last().unwrap();
    let r = decode_with(&png, None);
    eprintln!(
        "last part {:?} {:?}; IDAT {:?}; decode: {:?}",
        last.disposition,
        last.detail,
        parts_of(&inv, b"IDAT")[0].disposition,
        r.as_ref().err()
    );
    assert!(r.is_err());
    assert_eq!(parts_of(&inv, b"IDAT")[0].disposition, D::ImageData);
    assert!(
        last.detail
            .as_deref()
            .unwrap()
            .contains("DecodeJob::decode rejects the file")
    );
}

#[test]
fn ok_length_past_eof_and_overflow() {
    for len in [0xFFFF_FFFFu32, 0x8000_0000, 1000] {
        let mut png = SIG.to_vec();
        png.extend(ihdr(2, 1, 8, 2));
        png.extend_from_slice(&len.to_be_bytes());
        png.extend_from_slice(b"tEXtabc");
        let inv = inv_with(&png, None);
        let last = inv.parts().last().unwrap();
        assert_eq!(last.disposition, D::Malformed, "{inv}");
    }
}

#[test]
fn ok_after_iend_is_one_trailing_gap_even_if_it_looks_like_chunks() {
    let mut png = SIG.to_vec();
    png.extend(ihdr(2, 1, 8, 2));
    png.extend(rgb_idat());
    png.extend(chunk(b"IEND", &[]));
    let iend_end = png.len() as u64;
    png.extend(chunk(b"eXIf", b"II*\0\x08\0\0\0\0\0"));
    png.extend(chunk(b"tEXt", b"Comment\0hidden"));
    let inv = inv_with(&png, None);
    let tail: Vec<_> = inv
        .parts()
        .iter()
        .filter(|p| p.range.start >= iend_end)
        .collect();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].disposition, D::Trailing);
    assert_eq!(tail[0].range.end, png.len() as u64);
}

#[test]
fn ok_duplicate_and_private_units() {
    let mut png = SIG.to_vec();
    png.extend(ihdr(2, 1, 8, 2));
    png.extend(chunk(b"cICP", &[1, 13, 0, 1]));
    png.extend(chunk(b"cICP", &[9, 16, 0, 1]));
    png.extend(chunk(b"prIv", SECRET));
    png.extend(chunk(b"PRIV", SECRET)); // unknown *critical* chunk
    png.extend(chunk(b"IHDR", &[0; 13]));
    png.extend(rgb_idat());
    png.extend(chunk(b"cICP", &[1, 1, 0, 1]));
    png.extend(chunk(b"IEND", &[]));
    let inv = inv_with(&png, None);
    eprintln!("{inv}");
    let c = parts_of(&inv, b"cICP");
    assert_eq!(c[0].disposition, D::Skipped);
    assert_eq!(c[1].disposition, D::Metadata(M::Cicp));
    assert_eq!(c[2].disposition, D::Skipped);
    let out = decode_with(&png, None).unwrap();
    assert_eq!(
        out.info().metadata().cicp,
        Some(zencodec::Cicp::new(9, 16, 0, true))
    );
    assert_eq!(parts_of(&inv, b"prIv")[0].disposition, D::Unknown);
    assert_eq!(parts_of(&inv, b"PRIV")[0].disposition, D::Unknown);
    assert_eq!(parts_of(&inv, b"IHDR")[1].disposition, D::Skipped);
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

#[test]
fn pre_idat_fdat_is_skipped() {
    let base = apng();
    // Insert an fdAT carrying junk right before the IDAT.
    let idat_at = base.windows(4).position(|w| w == b"IDAT").unwrap() - 4;
    let mut fd = 7u32.to_be_bytes().to_vec();
    fd.extend_from_slice(SECRET);
    let mut with = base[..idat_at].to_vec();
    with.extend(chunk(b"fdAT", &fd));
    with.extend_from_slice(&base[idat_at..]);
    let inv = inv_with(&with, None);
    let pre = &parts_of(&inv, b"fdAT")[0];
    eprintln!("pre-IDAT fdAT {:?} at {:?}", pre.disposition, pre.range);
    assert_eq!(frames(&with), frames(&base));
    assert_eq!(pre.disposition, D::Skipped);
    assert_unconsumed(&inv, &with, SECRET);
}

// ── extra cases found while fixing the review ───────────────────────────────

#[test]
fn xmp_bytes_after_the_zlib_stream_are_unreferenced() {
    let mut png = SIG.to_vec();
    png.extend(ihdr(2, 1, 8, 2));
    png.extend(chunk(
        b"iTXt",
        &itxt(
            "XML:com.adobe.xmp",
            1,
            &[&zlib(b"<x:xmpmeta/>")[..], SECRET].concat(),
        ),
    ));
    png.extend(rgb_idat());
    png.extend(chunk(b"IEND", &[]));
    let inv = inv_with(&png, None);
    assert_eq!(parts_of(&inv, b"iTXt")[0].disposition, D::Metadata(M::Xmp));
    let out = decode_with(&png, None).unwrap();
    assert_eq!(
        out.info().metadata().xmp.as_deref(),
        Some(&b"<x:xmpmeta/>"[..])
    );
    assert_unconsumed(&inv, &png, SECRET);
}

#[test]
fn rgb_trns_tail_and_split_idat_stream_are_placed_exactly() {
    // RGB tRNS reads 6 bytes; the zlib stream is split across two IDATs with junk
    // glued to the end of the second.
    let stream = zlib_stored(&[0, 1, 2, 3, 4, 5, 6]);
    let (a, b) = stream.split_at(7);
    let mut png = SIG.to_vec();
    png.extend(ihdr(2, 1, 8, 2));
    png.extend(chunk(b"tRNS", &[&[0, 1, 0, 2, 0, 3][..], SECRET].concat()));
    png.extend(chunk(b"IDAT", a));
    png.extend(chunk(b"IDAT", &[b, SECRET].concat()));
    png.extend(chunk(b"IEND", &[]));
    let inv = inv_with(&png, None);
    assert!(decode_with(&png, None).is_ok());
    let idats = parts_of(&inv, b"IDAT");
    assert!(idats.iter().all(|p| p.disposition == D::ImageData));
    // Both SECRET copies must be unconsumed: check each occurrence.
    let mut from = 0;
    let mut hits = 0;
    while let Some(i) = png[from..].windows(SECRET.len()).position(|w| w == SECRET) {
        let at = from + i;
        let r = at as u64..(at + SECRET.len()) as u64;
        let has_child: Vec<bool> = (0..inv.parts().len())
            .map(|k| {
                inv.parts()
                    .iter()
                    .any(|p| p.parent.is_some_and(|q| q.index() == k))
            })
            .collect();
        for (k, p) in inv.parts().iter().enumerate() {
            if !has_child[k] && p.range.start < r.end && p.range.end > r.start {
                assert!(!p.disposition.is_consumed(), "{inv}");
            }
        }
        hits += 1;
        from = at + SECRET.len();
    }
    assert_eq!(hits, 2);
}
