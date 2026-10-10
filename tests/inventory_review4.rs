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
        if let Some(par) = p.parent {
            has_child[par.index()] = true;
        }
    }
    for (i, p) in inv.parts().iter().enumerate() {
        if has_child[i]
            || !p.disposition.is_consumed()
            || p.kind == zencodec::inventory::PartKind::Header
        {
            continue;
        }
        if p.label.as_deref() == Some("crc") {
            continue;
        }
        // The chunk that holds this leaf.
        let top = match p.parent {
            Some(par) => inv.get(par).unwrap().range.clone(),
            None => p.range.clone(),
        };
        let data = (top.start + 8).max(p.range.start)..(top.end - 4).min(p.range.end);
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
                p.tag, p.range, p.disposition, p.detail
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
        tx.disposition,
        tx.detail,
        fd.disposition,
        fd.detail
    );
    assert_eq!(fb, fw, "decoder reads the fdAT after the short chunk");
    assert_eq!(fd.disposition, D::ImageData, "{inv}");
    assert!(
        tx.detail
            .as_deref()
            .unwrap()
            .contains("contributes no frame data"),
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
        late_idat.disposition,
        late_idat.detail
    );
    assert_eq!(fb, fw, "decoder reads the IDAT-typed chunk as frame data");
    assert_eq!(late_idat.disposition, D::ImageData, "{inv}");
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
        .any(|p| p.range.start == boundary || p.range.end == boundary);
    eprintln!(
        "boundary at {boundary}: part boundary present = {has_boundary}; leaf: {}",
        leaf_over(&inv, &v, SECRET)
    );
    assert!(has_boundary, "a child marks the xpacket end\n{inv}");
    let tail = inv
        .parts()
        .iter()
        .find(|p| p.parent.is_some() && p.range.start == boundary)
        .unwrap();
    assert_eq!(tail.disposition, D::Metadata(M::Xmp));
    assert!(tail.detail.as_deref().unwrap().contains("reach the caller"));
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
        .any(|q| q.range.start == at || q.range.end == at);
    eprintln!(
        "iCCP {:?} {:?}; boundary present = {has_boundary}",
        p.disposition, p.detail
    );
    assert!(has_boundary, "{inv}");
    let tail = inv
        .parts()
        .iter()
        .find(|q| q.parent.is_some() && q.range.start == at)
        .unwrap();
    assert_eq!(tail.disposition, D::Metadata(M::Icc));
    assert!(
        tail.detail.as_deref().unwrap().contains("declares 128"),
        "{inv}"
    );
}
