//! Round-4 review probes for the PNG inventory (adopted, assertions flipped to the fixed
//! outcome), plus the two round-3 edge cases. The mutation sweep and the corpus-file report are
//! opt-in (`INVENTORY_MUTATION_SWEEP`, `ZENPNG_CODEC_CORPUS`; `just inventory-sweep`).

#![allow(dead_code, unused_imports)] // helpers shared by the probes

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
        .filter(|p| *p.tag() == PartTag::FourCc(*ty))
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
        if let Some(par) = p.parent() {
            has_child[par.index()] = true;
        }
    }
    for (i, p) in inv.parts().iter().enumerate() {
        if !has_child[i] && p.range().start < range.end && p.range().end > range.start {
            assert!(
                !p.disposition().is_consumed(),
                "hidden bytes in a consumed part {:?} {:?}\n{inv}",
                p.range(),
                p.disposition()
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
        if let Some(par) = p.parent() {
            has_child[par.index()] = true;
        }
    }
    inv.parts().iter().enumerate().any(|(i, p)| {
        !has_child[i]
            && p.range().start < r.end
            && p.range().end > r.start
            && p.disposition().is_consumed()
    })
}

fn leaf_over(inv: &Inventory, file: &[u8], needle: &[u8]) -> String {
    let at = file
        .windows(needle.len())
        .position(|w| w == needle)
        .unwrap() as u64;
    inv.parts()
        .iter()
        .filter(|p| p.range().start <= at && p.range().end > at)
        .map(|p| {
            format!(
                "{:?} {:?} {:?} {:?}",
                p.kind(),
                p.range(),
                p.disposition(),
                p.detail()
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

// ── Round 4 probes ───────────────────────────────────────────────────────────

fn frames_or_err(png: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    use zencodec::decode::AnimationFrameDecoder;
    let mut d = PngDecoderConfig::new()
        .job()
        .animation_frame_decoder(png.into(), &[])
        .map_err(|e| e.to_string())?;
    let mut out =
        vec![format!("loops={:?} count={:?}", d.loop_count(), d.frame_count()).into_bytes()];
    while let Some(f) = d.render_next_frame(None).map_err(|e| e.to_string())? {
        let s = f.pixels();
        let mut v = f.duration_ms().to_le_bytes().to_vec();
        for y in 0..s.rows() {
            v.extend_from_slice(s.row(y));
        }
        out.push(v);
    }
    Ok(out)
}

/// Everything a caller receives from the zencodec paths, for before/after comparison.
fn observe(png: &[u8]) -> String {
    let still = match decode_with(png, None) {
        Ok(o) => format!(
            "px={:?} info={:?} res={:?} src={:?}",
            pixels(&o),
            o.info(),
            o.info().resolution,
            o.source_encoding_details()
                .and_then(|d| d.codec_details::<zenpng::detect::PngProbe>())
                .map(|p| format!(
                    "{:?} {:?} {}",
                    p.creating_tool, p.palette_size, p.compressed_data_size
                ))
        ),
        Err(e) => format!("err {e}"),
    };
    let anim = frames_or_err(png)
        .map(|f| format!("{f:?}"))
        .unwrap_or_else(|e| e);
    format!("{still} | {anim}")
}

fn fix_crc(file: &mut [u8], chunk_start: usize) {
    let len = u32::from_be_bytes(file[chunk_start..chunk_start + 4].try_into().unwrap()) as usize;
    let ty: [u8; 4] = file[chunk_start + 4..chunk_start + 8].try_into().unwrap();
    let data = &file[chunk_start + 8..chunk_start + 8 + len];
    let crc = zenflate::crc32(zenflate::crc32(0, &ty), data);
    file[chunk_start + 8 + len..chunk_start + 12 + len].copy_from_slice(&crc.to_be_bytes());
}

/// Both-direction check (Pitfalls: "Overwriting a consumed leaf must change one of them, or
/// its detail must say why not"): flip one data byte in each consumed leaf, fix the CRC,
/// and report leaves where nothing a caller receives changes.
fn sweep(name: &str, file: &[u8], out: &mut Vec<String>) {
    let inv = inv_with(file, None);
    let base = observe(file);
    let mut has_child = vec![false; inv.parts().len()];
    for p in inv.parts() {
        if let Some(par) = p.parent() {
            has_child[par.index()] = true;
        }
    }
    for (i, p) in inv.parts().iter().enumerate() {
        if has_child[i]
            || !p.disposition().is_consumed()
            || p.kind() == zencodec::inventory::PartKind::Header
        {
            continue;
        }
        if p.label() == Some("crc") {
            continue;
        }
        // The chunk that holds this leaf.
        let top = match p.parent() {
            Some(par) => inv.get(par).unwrap().range(),
            None => p.range(),
        };
        let data = (top.start + 8).max(p.range().start)..(top.end - 4).min(p.range().end);
        if data.start >= data.end {
            continue;
        }
        let at = (data.start + (data.end - data.start) / 2) as usize;
        let mut m = file.to_vec();
        m[at] ^= 0x01;
        fix_crc(&mut m, top.start as usize);
        if observe(&m) == base {
            out.push(format!(
                "{name}: {} {:?} {:?} byte {at} unchanged; detail {:?}",
                p.tag(),
                p.range(),
                p.disposition(),
                p.detail()
            ));
        }
    }
}

fn apng_with_after_fctl(between: Vec<u8>, frame_ty: &[u8; 4]) -> Vec<u8> {
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
    v.extend(between);
    let mut fd = 2u32.to_be_bytes().to_vec();
    fd.extend(zlib_stored(&[0, 200, 100, 50, 25, 12, 6]));
    v.extend(chunk(frame_ty, &fd));
    v.extend(chunk(b"IEND", &[]));
    v
}

/// R4: a short (<4 byte) untyped chunk right after the fcTL; FdatSource yields no data
/// from it and then reads the following fdAT.
#[test]
fn r4_short_untyped_chunk_keeps_the_frame_fdat_read() {
    let base = apng_with_after_fctl(Vec::new(), b"fdAT");
    let with = apng_with_after_fctl(chunk(b"tEXt", b"a\0"), b"fdAT");
    let fb = frames(&base);
    let fw = frames(&with);
    let inv = inv_with(&with, None);
    let fd = &parts_of(&inv, b"fdAT")[0];
    let tx = &parts_of(&inv, b"tEXt")[0];
    eprintln!(
        "frames equal={} ; tEXt {:?} {:?} ; fdAT {:?} {:?}",
        fb == fw,
        tx.disposition(),
        tx.detail(),
        fd.disposition(),
        fd.detail()
    );
    assert_eq!(fb, fw, "decoder reads the fdAT after the short chunk");
    assert_eq!(fd.disposition(), D::ImageData, "{inv}");
    assert!(
        tx.detail().unwrap().contains("contributes no frame data"),
        "{inv}"
    );
}

/// R4: an IDAT-typed chunk right after a post-IDAT fcTL.
#[test]
fn r4_idat_typed_frame_chunk() {
    let base = apng_with_after_fctl(Vec::new(), b"fdAT");
    let with = apng_with_after_fctl(Vec::new(), b"IDAT");
    let fb = frames(&base);
    let fw = frames(&with);
    let inv = inv_with(&with, None);
    let late_idat = parts_of(&inv, b"IDAT").last().cloned().unwrap();
    eprintln!(
        "frames equal={} ; late IDAT {:?} {:?}",
        fb == fw,
        late_idat.disposition(),
        late_idat.detail()
    );
    assert_eq!(fb, fw, "decoder reads the IDAT-typed chunk as frame data");
    assert_eq!(late_idat.disposition(), D::ImageData, "{inv}");
}

/// Pitfalls "Tails the caller still receives": uncompressed XMP with bytes after the xpacket
/// end trailer. The decoder hands them over; the doc asks for a child at the boundary with the
/// parent's Metadata disposition and a detail.
#[test]
fn r4_xmp_trailer_tail_is_a_delivered_child() {
    let packet =
        b"<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?><x:xmpmeta/><?xpacket end=\"w\"?>";
    let text = [&packet[..], SECRET].concat();
    let mut v = SIG.to_vec();
    v.extend(ihdr(2, 1, 8, 2));
    v.extend(chunk(b"iTXt", &itxt("XML:com.adobe.xmp", 0, &text)));
    v.extend(rgb_idat());
    v.extend(chunk(b"IEND", &[]));
    let inv = inv_with(&v, None);
    let out = decode_with(&v, None).unwrap();
    let xmp = out.info().metadata().xmp.unwrap();
    assert!(xmp.ends_with(SECRET), "the decoder delivers the tail");
    let boundary = (v
        .windows(packet.len())
        .position(|w| w == &packet[..])
        .unwrap()
        + packet.len()) as u64;
    let has_boundary = inv
        .parts()
        .iter()
        .any(|p| p.range().start == boundary || p.range().end == boundary);
    eprintln!(
        "boundary at {boundary}: part boundary present = {has_boundary}; leaf: {}",
        leaf_over(&inv, &v, SECRET)
    );
    assert!(has_boundary, "a child marks the xpacket end\n{inv}");
    let tail = inv
        .parts()
        .iter()
        .find(|p| p.parent().is_some() && p.range().start == boundary)
        .unwrap();
    assert_eq!(tail.disposition(), D::Metadata(M::Xmp));
    assert!(tail.detail().unwrap().contains("reach the caller"));
}

/// Same for an ICC profile whose header declares a smaller size than the inflated data.
#[test]
fn r4_icc_tail_past_declared_size_is_a_delivered_child() {
    let mut profile = [0u8; 128];
    profile[..4].copy_from_slice(&128u32.to_be_bytes());
    profile[36..40].copy_from_slice(b"acsp");
    let full = [&profile[..], SECRET].concat();
    let mut body = b"icc\0\0".to_vec();
    body.extend(zlib_stored(&full));
    let mut v = SIG.to_vec();
    v.extend(ihdr(2, 1, 8, 2));
    v.extend(chunk(b"iCCP", &body));
    v.extend(rgb_idat());
    v.extend(chunk(b"IEND", &[]));
    let inv = inv_with(&v, None);
    let out = decode_with(&v, None).unwrap();
    let icc = out.info().metadata().icc_profile.unwrap();
    assert_eq!(
        icc.len(),
        128 + SECRET.len(),
        "the decoder delivers the bytes past the declared size"
    );
    let p = &parts_of(&inv, b"iCCP")[0];
    let at = v.windows(SECRET.len()).position(|w| w == SECRET).unwrap() as u64;
    let has_boundary = inv
        .parts()
        .iter()
        .any(|q| q.range().start == at || q.range().end == at);
    eprintln!(
        "iCCP {:?} {:?}; boundary present = {has_boundary}",
        p.disposition(),
        p.detail()
    );
    assert!(has_boundary, "{inv}");
    let tail = inv
        .parts()
        .iter()
        .find(|q| q.parent().is_some() && q.range().start == at)
        .unwrap();
    assert_eq!(tail.disposition(), D::Metadata(M::Icc));
    assert!(tail.detail().unwrap().contains("declares 128"), "{inv}");
}

/// Pitfalls "Settings and limits change the answer": the default 120 MP limit rejects the
/// file; the inventory still reports consumed parts without saying so.
#[test]
fn r4_default_pixel_limit_rejection_is_reported() {
    let mut v = SIG.to_vec();
    v.extend(ihdr(12_000, 12_000, 8, 0)); // 144 MP > 120 MP default
    v.extend(chunk(b"IDAT", &zlib(&[0u8; 12_001])));
    v.extend(chunk(b"IEND", &[]));
    let r = decode_with(&v, None);
    let inv = inv_with(&v, None);
    let notes: Vec<_> = inv.parts().iter().filter_map(|p| p.detail()).collect();
    eprintln!(
        "decode: {:?}\nconsumed: {:?}\nnotes: {notes:?}",
        r.as_ref().err(),
        inv.parts()
            .iter()
            .filter(|p| p.disposition().is_consumed())
            .map(|p| (p.tag().to_string(), p.disposition()))
            .collect::<Vec<_>>()
    );
    assert!(r.is_err());
    assert!(
        notes
            .iter()
            .any(|n| n.contains("DecodeJob::decode rejects the file") && n.contains("limit")),
        "{notes:?}"
    );
}

/// Pitfalls "Bounded work": honour max_input_bytes.
#[test]
fn r4_max_input_bytes_rejection_is_reported() {
    let v = png_rgb(&[rgb_idat()]);
    let limits = zencodec::ResourceLimits::none().with_max_input_bytes(16);
    let job = PngDecoderConfig::new().job().with_limits(limits);
    let inv = job.inventory(&v);
    let dec = PngDecoderConfig::new()
        .job()
        .with_limits(limits)
        .decoder(v.as_slice().into(), &[])
        .unwrap()
        .decode();
    eprintln!(
        "inventory ok={} ({:?}); decode err={:?}",
        inv.is_ok(),
        inv.as_ref()
            .ok()
            .map(|i| i.as_ref().map(|i| i.parts().len())),
        dec.as_ref().err().map(|e| e.to_string())
    );
    assert!(dec.is_err());
    let inv = inv.unwrap().unwrap();
    inv.validate().unwrap();
    assert!(
        inv.parts()
            .iter()
            .any(|p| p.detail().is_some_and(|d| d.contains("input size"))),
        "{inv}"
    );
}

/// Pitfalls "More than one reader": a bad-CRC pre-IDAT eXIf reaches decode() but not probe()
/// or the animation decoder's info; the detail does not name those readers.
#[test]
fn r4_bad_crc_exif_names_the_other_readers() {
    let exif = b"II*\0\x08\0\0\0\0\0";
    let mut v = SIG.to_vec();
    v.extend(ihdr(2, 1, 8, 2));
    v.extend(bad_crc(chunk(b"eXIf", exif)));
    v.extend(rgb_idat());
    v.extend(chunk(b"IEND", &[]));
    let d = decode_with(&v, None).unwrap();
    let probe = PngDecoderConfig::new().job().probe(&v).unwrap();
    let inv = inv_with(&v, None);
    let p = &parts_of(&inv, b"eXIf")[0];
    eprintln!(
        "decode exif={} probe exif={} detail={:?}",
        d.info().metadata().exif.is_some(),
        probe.metadata().exif.is_some(),
        p.detail()
    );
    assert!(d.info().metadata().exif.is_some());
    assert!(probe.metadata().exif.is_none());
    assert!(
        p.detail()
            .unwrap_or("")
            .contains("probe() and the animation decoder skip it")
    );
}

/// Directory of one codec-corpus set: `$ZENPNG_CODEC_CORPUS/<set>` (a local checkout,
/// read-only) or the codec-corpus crate's cache of imazen/codec-corpus.
fn corpus_dir(set: &str) -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("ZENPNG_CODEC_CORPUS") {
        return std::path::PathBuf::from(d).join(set);
    }
    codec_corpus::Corpus::new()
        .expect("codec-corpus cache unavailable")
        .github_repo("imazen/codec-corpus", set, "main")
        .unwrap_or_else(|e| panic!("fetching imazen/codec-corpus {set} (main) failed: {e}"))
}

/// R4-5: the three apng-conformance files whose animation path fails. The still decode
/// succeeds; the inventory must say where the animation decoder fails and stop claiming the
/// frames after that point.
#[test]
fn r4_failing_animation_frames_are_reported() {
    let dir = corpus_dir("apng-conformance").join("invalid");
    let top = |inv: &Inventory, tag: &str| -> Vec<zencodec::inventory::Part> {
        inv.parts()
            .iter()
            .filter(|p| p.parent().is_none() && p.tag().to_string() == tag)
            .cloned()
            .collect()
    };
    // frame_out_of_bounds: frame 1's fcTL exceeds the canvas.
    let b = std::fs::read(dir.join("frame_out_of_bounds.png")).unwrap();
    let err = frames_or_err(&b).unwrap_err();
    assert!(err.contains("exceeds canvas width"), "{err}");
    let inv = inv_with(&b, None);
    let fctl = top(&inv, "fcTL");
    assert_eq!(fctl[1].disposition(), D::Malformed, "{inv}");
    assert!(fctl[1].detail().unwrap().contains("exceeds canvas width"));
    assert_eq!(top(&inv, "fdAT")[0].disposition(), D::Skipped, "{inv}");
    assert!(
        top(&inv, "acTL")[0]
            .detail()
            .unwrap()
            .contains("animation_frame_decoder fails")
    );
    // no_fdat: acTL promises a frame the file doesn't hold.
    let b = std::fs::read(dir.join("no_fdat.png")).unwrap();
    let err = frames_or_err(&b).unwrap_err();
    assert!(
        err.contains("reached IEND before finding expected fcTL"),
        "{err}"
    );
    let inv = inv_with(&b, None);
    assert!(
        top(&inv, "acTL")[0]
            .detail()
            .unwrap()
            .contains("acTL declares 3 frames, the file holds 1"),
        "{inv}"
    );
    // truncated_fdat: frame 1's zlib data ends before its rows.
    let b = std::fs::read(dir.join("truncated_fdat.png")).unwrap();
    let err = frames_or_err(&b).unwrap_err();
    assert!(err.contains("decompression error"), "{err}");
    let inv = inv_with(&b, None);
    let fd = &top(&inv, "fdAT")[0];
    assert_eq!(fd.disposition(), D::Skipped, "{inv}");
    assert!(fd.detail().unwrap().contains("fails on this frame"));
}

/// R4-5 without the corpus: frame 1's fcTL lies outside the canvas, and a later frame is never
/// reached.
#[test]
fn r4_frame_after_a_failing_frame_is_not_claimed() {
    let fctl = |seq: u32, x: u32| {
        let mut d = seq.to_be_bytes().to_vec();
        d.extend_from_slice(&2u32.to_be_bytes());
        d.extend_from_slice(&1u32.to_be_bytes());
        d.extend_from_slice(&x.to_be_bytes());
        d.extend_from_slice(&0u32.to_be_bytes());
        d.extend_from_slice(&[0, 1, 0, 10, 0, 0]);
        d
    };
    let fdat = |seq: u32| {
        let mut d = seq.to_be_bytes().to_vec();
        d.extend(zlib_stored(&[0, 9, 9, 9, 8, 8, 8]));
        chunk(b"fdAT", &d)
    };
    let mut v = SIG.to_vec();
    v.extend(ihdr(2, 1, 8, 2));
    v.extend(chunk(b"acTL", &[0, 0, 0, 3, 0, 0, 0, 0]));
    v.extend(chunk(b"fcTL", &fctl(0, 0)));
    v.extend(rgb_idat());
    v.extend(chunk(b"fcTL", &fctl(1, 5))); // x_offset 5 + width 2 > canvas 2
    v.extend(fdat(2));
    v.extend(chunk(b"fcTL", &fctl(3, 0)));
    v.extend(fdat(4));
    v.extend(chunk(b"IEND", &[]));
    assert!(frames_or_err(&v).is_err());
    let inv = inv_with(&v, None);
    let tags: Vec<_> = inv
        .parts()
        .iter()
        .filter(|p| {
            p.parent().is_none() && ["fcTL", "fdAT"].contains(&p.tag().to_string().as_str())
        })
        .map(|p| (p.tag().to_string(), p.disposition()))
        .collect();
    assert_eq!(
        tags,
        vec![
            ("fcTL".into(), D::Metadata(M::Animation)),
            ("fcTL".into(), D::Malformed),
            ("fdAT".into(), D::Skipped),
            ("fcTL".into(), D::Skipped),
            ("fdAT".into(), D::Skipped),
        ],
        "{inv}"
    );
}

/// fdAT / fcTL sequence numbers and the IDAT zlib footer: consumed, but overwriting them
/// changes nothing a caller receives (default policy).
#[test]
fn r4_sequence_numbers_and_footer_carry_details() {
    let base = apng();
    let base_obs = observe(&base);
    let inv = inv_with(&base, None);
    let mut rows = Vec::new();
    for p in inv.parts().iter().filter(|p| p.parent().is_none()) {
        let tag = p.tag().to_string();
        let off = match tag.as_str() {
            "fdAT" | "fcTL" => Some(p.range().start as usize + 8 + 3), // last byte of the sequence number
            "IDAT" => Some(p.range().end as usize - 4 - 1),            // last Adler-32 byte
            _ => None,
        };
        if let Some(at) = off {
            let mut m = base.clone();
            m[at] ^= 0x01;
            fix_crc(&mut m, p.range().start as usize);
            let leaf = inv
                .parts()
                .iter()
                .rfind(|q| q.range().start <= at as u64 && (at as u64) < q.range().end)
                .unwrap();
            rows.push((tag, at, observe(&m) == base_obs, leaf.disposition()));
        }
    }
    eprintln!("{rows:?}");
    // Every unused-but-consumed byte here carries a detail saying why.
    for p in inv.parts().iter().filter(|p| p.parent().is_none()) {
        match p.tag().to_string().as_str() {
            "fcTL" | "fdAT" => assert!(
                p.detail().unwrap_or("").contains("sequence number"),
                "{inv}"
            ),
            "IDAT" => assert!(p.detail().unwrap_or("").contains("Adler-32"), "{inv}"),
            _ => {}
        }
    }
}

/// R4-6: the first pass inflates past the rows (within the search budget), so a stream that
/// breaks after the rows is told apart from one that ends cleanly with excess data.
#[test]
fn r4_stream_break_after_rows_is_named_in_the_detail() {
    let raw = [0u8, 1, 2, 3, 4, 5, 6];
    let mut s = vec![0x78, 0x01];
    // Enough excess that the break lies in a later inflate window than the last row, as in
    // the reviewer's round-3 probe; otherwise the decoder reports the break itself.
    let zeros = vec![0u8; 65535];
    let mut blocks: Vec<&[u8]> = vec![&raw[..]];
    blocks.extend(std::iter::repeat_n(&zeros[..], 16));
    for b in blocks {
        s.push(0x00); // BFINAL=0, stored
        s.extend_from_slice(&(b.len() as u16).to_le_bytes());
        s.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
        s.extend_from_slice(b);
    }
    s.push(0xFF); // invalid block type
    s.extend_from_slice(b"tail after the break");
    let v = png_rgb(&[chunk(b"IDAT", &s)]);
    assert!(decode_with(&v, None).is_ok());
    let inv = inv_with(&v, None);
    let d = parts_of(&inv, b"IDAT")[0].detail().unwrap().to_string();
    assert!(d.contains("breaks after the last row"), "{d}");
    // A clean stream with the same excess says how many bytes are discarded instead.
    let v = png_rgb(&[chunk(
        b"IDAT",
        &zlib_stored(&[&raw[..], &[7u8; 100][..]].concat()),
    )]);
    let d = parts_of(&inv_with(&v, None), b"IDAT")[0]
        .detail()
        .unwrap()
        .to_string();
    assert!(
        d.contains("100 decompressed bytes after the last row are discarded"),
        "{d}"
    );
}

/// Mutation survey (opt-in): see the module docs.
#[test]
fn r4_overwrite_consumed_leaves_sweep() {
    if std::env::var_os("INVENTORY_MUTATION_SWEEP").is_none() {
        eprintln!(
            "INVENTORY_MUTATION_SWEEP unset: mutation sweep not requested (just inventory-sweep)"
        );
        return;
    }
    let mut out = Vec::new();
    let root = std::path::PathBuf::from(
        std::env::var("ZENPNG_CODEC_CORPUS").expect("set ZENPNG_CODEC_CORPUS"),
    );
    let mut files = Vec::new();
    for set in ["pngsuite", "png-conformance", "apng-conformance"] {
        let mut stack = vec![root.join(set)];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "png") {
                    files.push(p);
                }
            }
        }
    }
    files.sort();
    let mut n = 0;
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        if decode_with(&bytes, None).is_err() && frames_or_err(&bytes).is_err() {
            continue;
        }
        n += 1;
        sweep(&f.file_name().unwrap().to_string_lossy(), &bytes, &mut out);
    }
    eprintln!(
        "swept {n} decodable files; {} consumed leaves unchanged by a flipped byte:",
        out.len()
    );
    for l in &out {
        eprintln!("  {l}");
    }
}

/// Cut edges placed by `zenflate::zlib_scan`, both directions: overwriting the bytes the
/// inventory reports unconsumed (an IDAT/fdAT run's `Unreferenced` tail and the chunks
/// after it) changes nothing a caller receives, except that a tail after the last row may
/// turn the decode into an error, as its detail says (the decoder inflates past the rows);
/// at a cut after the last row, flipping the last byte claimed read changes the result.
/// Returns the number of mutations checked.
fn cut_edges(name: &str, file: &[u8], bad: &mut Vec<String>) -> usize {
    let inv = inv_with(file, None);
    let base = observe(file);
    let mut checked = 0;
    for id in inv.children(None) {
        let p = inv.get(id).unwrap();
        if !matches!(p.tag(), PartTag::FourCc(t) if t == b"IDAT" || t == b"fdAT") {
            continue;
        }
        let top = p.range();
        let data = top.start + 8..top.end - 4;
        let tail =
            |d: Option<&str>| d.map(|d| (d.contains("never read"), d.contains("decoded ahead")));
        let unread: Vec<_> =
            if p.disposition() == D::Skipped && tail(p.detail()).is_some_and(|(n, a)| n || a) {
                vec![(data.clone(), tail(p.detail()).unwrap().1)]
            } else {
                inv.children(Some(id))
                    .into_iter()
                    .map(|k| inv.get(k).unwrap())
                    .filter(|k| k.disposition() == D::Unreferenced)
                    .map(|k| (k.range(), tail(k.detail()).is_some_and(|t| t.1)))
                    .collect()
            };
        let mutate = |at: std::ops::Range<u64>, x: u8| {
            let mut m = file.to_vec();
            for b in &mut m[at.start as usize..at.end as usize] {
                *b ^= x;
            }
            fix_crc(&mut m, top.start as usize);
            observe(&m)
        };
        for (r, ahead) in unread.iter().filter(|r| !r.0.is_empty()) {
            checked += 1;
            let got = mutate(r.clone(), 0xA5);
            if got != base && !(*ahead && got.starts_with("err ")) {
                bad.push(format!(
                    "{name}: overwriting unconsumed {r:?} changed the result"
                ));
            }
        }
        if p.disposition() != D::Skipped
            && p.detail().is_some_and(|d| d.contains("after the last row"))
        {
            let cut = unread.first().map_or(data.end, |r| r.0.start);
            if cut > data.start {
                checked += 1;
                if mutate(cut - 1..cut, 0xFF) == base {
                    bad.push(format!(
                        "{name}: last byte read ({}) changes nothing",
                        cut - 1
                    ));
                }
            }
        }
    }
    checked
}

#[test]
fn scan_cut_edges_hold_under_mutation() {
    let mut bad = Vec::new();
    let mut synthetic = 0;
    // Rows plus excess in one stream, stored and at three zenflate levels, whole and split
    // over three IDATs; and clean streams followed by junk.
    let rows: Vec<u8> = (0..16 * 49)
        .map(|i| if i % 49 == 0 { 0 } else { (i * 7 % 13) as u8 })
        .collect();
    let excess: Vec<u8> = (0..1500).map(|i| (i * 31 % 251) as u8).collect();
    let level = |l: zenflate::CompressionLevel, d: &[u8]| {
        let mut c = zenflate::Compressor::new(l);
        let mut out = vec![0u8; zenflate::Compressor::zlib_compress_bound(d.len())];
        let n = c.zlib_compress(d, &mut out, zenflate::Unstoppable).unwrap();
        out.truncate(n);
        out
    };
    let with_excess = [&rows[..], &excess[..]].concat();
    let mut streams = vec![zlib_stored(&with_excess)];
    for l in [
        zenflate::CompressionLevel::fastest(),
        zenflate::CompressionLevel::balanced(),
        zenflate::CompressionLevel::best(),
    ] {
        streams.push(level(l, &with_excess));
        streams.push([level(l, &rows), b"junk after the stream end".to_vec()].concat());
    }
    for (i, z) in streams.iter().enumerate() {
        for parts in [1, 3] {
            let mut v = SIG.to_vec();
            v.extend(ihdr(16, 16, 8, 2));
            for c in z.chunks(z.len().div_ceil(parts)) {
                v.extend(chunk(b"IDAT", c));
            }
            v.extend(chunk(b"IEND", &[]));
            let n = cut_edges(&format!("synthetic {i}/{parts}"), &v, &mut bad);
            assert!(n > 0, "synthetic {i}/{parts}: no cut found");
            synthetic += n;
        }
    }
    // (The conformance corpora hold no bytes the decoder skips: no cut edges there.)
    eprintln!("cut-edge mutations checked: {synthetic}");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
