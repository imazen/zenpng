//! Structural inventory (`DecodeJob::inventory`) conformance, fixtures and corpus runs.

use std::path::{Path, PathBuf};

use zencodec::decode::{DecodeJob, DecoderConfig};
use zencodec::inventory::{Disposition, Inventory, MetadataKind, PartKind, PartTag};
use zenpng::{PngDecoderConfig, PngEncoderConfig};

// ── Chunk builders ───────────────────────────────────────────────────

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

/// zlib stream made of one stored block.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut v = vec![0x78, 0x01, 0x01];
    v.extend_from_slice(&(data.len() as u16).to_le_bytes());
    v.extend_from_slice(&(!(data.len() as u16)).to_le_bytes());
    v.extend_from_slice(data);
    v.extend_from_slice(&zenflate::adler32(1, data).to_be_bytes());
    v
}

fn ihdr(w: u32, h: u32, depth: u8, color: u8) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend_from_slice(&w.to_be_bytes());
    d.extend_from_slice(&h.to_be_bytes());
    d.extend_from_slice(&[depth, color, 0, 0, 0]);
    chunk(b"IHDR", &d)
}

/// 4x2 indexed image rows (filter byte 0 + 4 indices).
fn indexed_idat() -> Vec<u8> {
    let raw = [0, 0, 1, 2, 0, 0, 2, 1, 0];
    // 2 rows x (1 + 4) bytes
    let raw: Vec<u8> = [&raw[..5], &[0u8, 2, 1, 0, 1][..]].concat();
    chunk(b"IDAT", &zlib_stored(&raw))
}

/// What the fixture builder expects for one top-level part.
struct Want {
    tag: String,
    kind: PartKind,
    range: std::ops::Range<u64>,
    disp: Disposition,
    label: Option<&'static str>,
}

struct Fixture {
    bytes: Vec<u8>,
    want: Vec<Want>,
}

impl Fixture {
    fn new() -> Self {
        let mut f = Fixture {
            bytes: SIG.to_vec(),
            want: Vec::new(),
        };
        f.want.push(Want {
            kind: PartKind::Header,
            tag: "-".into(),
            range: 0..8,
            disp: Disposition::Structure,
            label: Some("PNG signature"),
        });
        f
    }

    fn add(
        &mut self,
        ty: &[u8; 4],
        data: &[u8],
        disp: Disposition,
        label: Option<&'static str>,
    ) -> usize {
        let start = self.bytes.len() as u64;
        self.bytes.extend(chunk(ty, data));
        self.want.push(Want {
            kind: PartKind::Chunk,
            tag: String::from_utf8_lossy(ty).into_owned(),
            range: start..self.bytes.len() as u64,
            disp,
            label,
        });
        self.want.len() - 1
    }

    fn add_raw(
        &mut self,
        ty: &[u8; 4],
        bytes: Vec<u8>,
        disp: Disposition,
        label: Option<&'static str>,
    ) {
        let start = self.bytes.len() as u64;
        self.bytes.extend(bytes);
        self.want.push(Want {
            kind: PartKind::Chunk,
            tag: String::from_utf8_lossy(ty).into_owned(),
            range: start..self.bytes.len() as u64,
            disp,
            label,
        });
    }
}

fn itxt(keyword: &str, flag: u8, lang: &str, translated: &str, text: &[u8]) -> Vec<u8> {
    let mut d = keyword.as_bytes().to_vec();
    d.push(0);
    d.push(flag);
    d.push(0);
    d.extend_from_slice(lang.as_bytes());
    d.push(0);
    d.extend_from_slice(translated.as_bytes());
    d.push(0);
    d.extend_from_slice(text);
    d
}

use Disposition as D;
use MetadataKind as M;

/// An indexed, animated PNG containing every chunk type the decoder knows, a
/// private chunk, duplicates that lose, post-IDAT metadata, and bytes after IEND.
fn everything() -> Fixture {
    let mut f = Fixture::new();
    let i = f.bytes.len() as u64;
    f.bytes.extend(ihdr(4, 2, 8, 3));
    f.want.push(Want {
        kind: PartKind::Chunk,
        tag: "IHDR".into(),
        range: i..f.bytes.len() as u64,
        disp: D::Structure,
        label: None,
    });
    f.add(
        b"acTL",
        &[0, 0, 0, 2, 0, 0, 0, 0],
        D::Metadata(M::Animation),
        None,
    );
    let mut fctl0 = vec![0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 2];
    fctl0.extend_from_slice(&[0; 8]);
    fctl0.extend_from_slice(&[0, 1, 0, 10, 0, 0]);
    assert_eq!(fctl0.len(), 26);
    f.add(b"fcTL", &fctl0, D::Metadata(M::Animation), None);
    f.add(b"sRGB", &[0], D::Skipped, None);
    f.add(b"gAMA", &45455u32.to_be_bytes(), D::Skipped, None);
    f.add(b"cHRM", &[0; 32], D::Skipped, None);
    // First iCCP loses to the second (a later successful parse replaces it).
    f.add(
        b"iCCP",
        &[b"old\0\0".as_slice(), &zlib_stored(b"first profile")].concat(),
        D::Skipped,
        Some("old"),
    );
    f.add(
        b"iCCP",
        &[b"test\0\0".as_slice(), &zlib_stored(b"second profile")].concat(),
        D::Metadata(M::Icc),
        Some("test"),
    );
    // An iCCP whose zlib stream is garbage is dropped silently by the decoder.
    f.add(
        b"iCCP",
        b"broken\0\0not a zlib stream",
        D::Malformed,
        Some("broken"),
    );
    f.add(b"sBIT", &[8, 8, 8], D::Skipped, None);
    f.add(b"cICP", &[1, 13, 0, 1], D::Metadata(M::Cicp), None);
    f.add(b"mDCV", &[0; 24], D::Metadata(M::HdrStatic), None);
    f.add(
        b"cLLI",
        &[0, 0, 3, 232, 0, 0, 0, 100],
        D::Metadata(M::HdrStatic),
        None,
    );
    f.add(
        b"PLTE",
        &[255, 0, 0, 0, 255, 0, 0, 0, 255],
        D::ImageData,
        None,
    );
    f.add(b"tRNS", &[255, 128], D::ImageData, None);
    f.add(b"bKGD", &[1], D::Skipped, None);
    f.add(b"hIST", &[0; 6], D::Skipped, None);
    f.add(
        b"pHYs",
        &[0, 0, 0x16, 0x25, 0, 0, 0x16, 0x25, 1],
        D::Metadata(M::Resolution),
        None,
    );
    f.add(b"sPLT", b"pal\0\x08\0\0\0\0\0\0\0", D::Skipped, None);
    f.add(b"oFFs", &[0; 9], D::Skipped, None);
    f.add(b"tIME", &[0x07, 0xea, 10, 9, 12, 30, 15], D::Skipped, None);
    // First eXIf loses to the second one before IDAT.
    f.add(b"eXIf", b"MM\0*\0\0\0\x08\0\0", D::Skipped, None);
    f.add(b"eXIf", b"II*\0\x08\0\0\0\0\0", D::Metadata(M::Exif), None);
    f.add(b"tEXt", b"Title\0A title", D::Skipped, Some("Title"));
    f.add(
        b"tEXt",
        b"Software\0zenpng test",
        D::Metadata(M::Supplement),
        Some("Software"),
    );
    f.add(
        b"zTXt",
        &[b"Comment\0\0".as_slice(), &zlib_stored(b"a comment")].concat(),
        D::Skipped,
        Some("Comment"),
    );
    f.add(
        b"iTXt",
        &itxt("XML:com.adobe.xmp", 0, "", "", b"<x:xmpmeta/>"),
        D::Metadata(M::Xmp),
        Some("XML:com.adobe.xmp"),
    );
    f.add(
        b"iTXt",
        &itxt("Author", 0, "en", "Autor", b"someone"),
        D::Skipped,
        Some("Author"),
    );
    f.add(b"iDOT", &[0; 12], D::Skipped, None); // fails idot::validate
    f.add(b"prVt", b"private camera serial 12345", D::Unknown, None);
    f.add_raw(b"IDAT", indexed_idat(), D::ImageData, None);
    // Post-IDAT metadata. Earlier eXIf/XMP win; gAMA is not collected after IDAT.
    f.add(b"tEXt", b"Late\0late text", D::Skipped, Some("Late"));
    f.add(b"eXIf", b"II*\0\x08\0\0\0\0\0", D::Skipped, None);
    f.add(
        b"iTXt",
        &itxt("XML:com.adobe.xmp", 0, "", "", b"<x:xmpmeta late/>"),
        D::Skipped,
        Some("XML:com.adobe.xmp"),
    );
    f.add(b"gAMA", &45455u32.to_be_bytes(), D::Skipped, None);
    let mut fctl1 = vec![0, 0, 0, 1, 0, 0, 0, 4, 0, 0, 0, 2];
    fctl1.extend_from_slice(&[0; 8]);
    fctl1.extend_from_slice(&[0, 1, 0, 10, 0, 0]);
    f.add(b"fcTL", &fctl1, D::Metadata(M::Animation), None);
    f.add(
        b"fdAT",
        &[
            &[0, 0, 0, 2][..],
            &zlib_stored(&[0, 0, 1, 2, 0, 0, 2, 1, 0, 0])[..],
        ]
        .concat(),
        D::ImageData,
        None,
    );
    f.add(b"zzZz", b"private after IDAT", D::Unknown, None);
    f.add(b"IEND", &[], D::Structure, None);
    // Trailing bytes after IEND.
    let start = f.bytes.len() as u64;
    f.bytes
        .extend_from_slice(b"TRAILING PNG JUNK, e.g. a second payload");
    f.want.push(Want {
        kind: PartKind::Gap,
        tag: "-".into(),
        range: start..f.bytes.len() as u64,
        disp: D::Trailing,
        label: None,
    });
    f
}

fn run(bytes: &[u8]) -> Inventory {
    let inv = PngDecoderConfig::new()
        .job()
        .inventory(bytes)
        .expect("inventory errored")
        .expect("inventory is Some");
    inv.validate()
        .unwrap_or_else(|e| panic!("invalid inventory: {e}\n{inv}"));
    inv
}

type Row = (
    String,
    PartKind,
    std::ops::Range<u64>,
    Disposition,
    Option<String>,
);

fn top(inv: &Inventory) -> Vec<Row> {
    inv.children(None)
        .into_iter()
        .map(|id| {
            let p = inv.get(id).unwrap();
            (
                p.tag().to_string(),
                p.kind(),
                p.range(),
                p.disposition(),
                p.label().as_ref().map(|l| l.to_string()),
            )
        })
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────────

/// Encode a small image through the zencodec encode path.
fn encode(w: u32, h: u32, desc: zenpixels::PixelDescriptor, bpp: usize) -> Vec<u8> {
    use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
    let bytes: Vec<u8> = (0..w as usize * h as usize * bpp)
        .map(|i| (i * 7 % 251) as u8 | 1)
        .collect();
    let slice = zenpixels::PixelSlice::new(&bytes, w, h, w as usize * bpp, desc).unwrap();
    PngEncoderConfig::new()
        .job()
        .encoder()
        .unwrap()
        .encode(slice)
        .unwrap()
        .into_vec()
}

/// `check_all` can't gate zenpng yet: its pixel round-trip check feeds RGBA8 with
/// alpha 0 pixels and zenpng zeroes RGB under alpha 0 (CLAUDE.md "Known Issues"),
/// so it fails before reaching the inventory step. Run the inventory check on the
/// same kind of encoder output directly.
#[test]
fn conformance_with_encoder_output() {
    use zenpixels::PixelDescriptor as P;
    for (w, h, desc, bpp) in [
        (40, 24, P::RGBA8_SRGB, 4),
        (40, 24, P::RGB8_SRGB, 3),
        (7, 5, P::GRAY8_SRGB, 1),
        (1, 1, P::RGB8_SRGB, 3),
    ] {
        let bytes = encode(w, h, desc, bpp);
        zencodec_testkit::check_inventory(PngDecoderConfig::new(), &bytes)
            .unwrap_or_else(|e| panic!("{w}x{h} bpp {bpp}: {e}"));
    }
}

#[test]
fn capability_declared() {
    assert!(PngDecoderConfig::capabilities().inventory());
}

#[test]
fn everything_fixture_pins_the_part_list() {
    let f = everything();
    let inv = run(&f.bytes);
    let got = top(&inv);
    let want: Vec<_> = f
        .want
        .iter()
        .map(|w| {
            (
                w.tag.clone(),
                w.kind,
                w.range.clone(),
                w.disp,
                w.label.map(str::to_string),
            )
        })
        .collect();
    assert_eq!(
        got.len(),
        want.len(),
        "part count differs\n{inv}\nwant {want:#?}"
    );
    for (g, w) in got.iter().zip(&want) {
        assert_eq!(g, w, "\n{inv}");
    }
    // Every part is top-level and chunks carry their own bytes (no children).
    assert!(inv.parts().iter().all(|p| p.parent().is_none()));
    assert!(inv.parts().iter().all(|p| p.kind() != PartKind::Box));
    assert!(
        inv.parts()
            .iter()
            .any(|p| p.kind() == PartKind::Gap && p.disposition() == D::Trailing)
    );
}

#[test]
fn everything_fixture_passes_testkit_check() {
    let f = everything();
    zencodec_testkit::check_inventory(PngDecoderConfig::new(), &f.bytes)
        .unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn details_name_native_fields_and_reasons() {
    let f = everything();
    let inv = run(&f.bytes);
    let detail_of = |tag: &str, nth: usize| -> String {
        inv.parts()
            .iter()
            .filter(|p| *p.tag() == PartTag::FourCc(tag.as_bytes().try_into().unwrap()))
            .nth(nth)
            .and_then(|p| p.detail())
            .unwrap_or_default()
            .to_string()
    };
    assert!(detail_of("gAMA", 0).contains("PngInfo::gamma"));
    assert!(detail_of("sBIT", 0).contains("PngInfo::significant_bits"));
    assert!(detail_of("bKGD", 0).contains("PngInfo::background"));
    assert!(detail_of("tEXt", 0).contains("PngInfo::text_chunks"));
    assert!(detail_of("tEXt", 1).contains("creating_tool"));
    assert!(detail_of("iCCP", 0).contains("superseded"));
    assert!(detail_of("iCCP", 2).contains("inflate failed"));
    assert!(detail_of("eXIf", 0).contains("superseded"));
    assert!(detail_of("eXIf", 2).contains("earlier eXIf wins"));
    assert!(detail_of("iTXt", 1).contains("lang=en"));
    assert!(detail_of("fdAT", 0).contains("sequence numbers are not checked"));
    // The IDAT run carries one remark: the zlib footer the default decode does not verify.
    assert!(detail_of("IDAT", 0).contains("Adler-32"));
}

#[test]
fn bad_crc_is_reported_without_changing_the_disposition() {
    let mut f = everything();
    // Corrupt the CRC of the first tEXt chunk.
    let w = f.want.iter().find(|w| w.tag == "tEXt").unwrap();
    let crc_byte = (w.range.end - 1) as usize;
    f.bytes[crc_byte] ^= 0xff;
    let inv = run(&f.bytes);
    let p = inv
        .parts()
        .iter()
        .find(|p| *p.tag() == PartTag::FourCc(*b"tEXt"))
        .unwrap();
    assert_eq!(p.disposition(), D::Skipped);
    assert!(p.detail().unwrap().contains("crc mismatch"));
    // Intact chunks carry no crc remark.
    assert!(
        inv.parts()
            .iter()
            .filter(|p| p.detail().is_some_and(|d| d.contains("crc mismatch")))
            .count()
            == 1
    );
}

#[test]
fn missing_iend_ends_where_the_data_ends() {
    let f = everything();
    let iend = f.want.iter().find(|w| w.tag == "IEND").unwrap();
    let cut = &f.bytes[..iend.range.start as usize];
    let inv = run(cut);
    assert!(
        inv.unconsumed()
            .all(|(_, p)| p.disposition() != D::Trailing)
    );
    assert_eq!(inv.children(None).len(), f.want.len() - 2);
}

#[test]
fn text_beyond_the_chunk_cap_is_skipped_with_a_reason() {
    let mut f = Fixture::new();
    f.add(b"IHDR", &ihdr(4, 2, 8, 3)[8..21], D::Structure, None);
    for i in 0..258 {
        let body = format!("Key{i}\0value");
        f.add(b"tEXt", body.as_bytes(), D::Skipped, None);
    }
    f.add_raw(b"IDAT", indexed_idat(), D::ImageData, None);
    f.add(b"IEND", &[], D::Structure, None);
    let inv = run(&f.bytes);
    let capped: Vec<_> = inv
        .parts()
        .iter()
        .filter(|p| p.detail().is_some_and(|d| d.contains("text cap")))
        .collect();
    assert_eq!(capped.len(), 2);
}

#[test]
fn software_text_past_the_cap_still_feeds_creating_tool() {
    let mut f = Fixture::new();
    f.add(b"IHDR", &ihdr(4, 2, 8, 3)[8..21], D::Structure, None);
    for i in 0..256 {
        let body = format!("Key{i}\0value");
        f.add(b"tEXt", body.as_bytes(), D::Skipped, None);
    }
    f.add(
        b"tEXt",
        b"Software\0late tool",
        D::Metadata(M::Supplement),
        Some("Software"),
    );
    f.add_raw(b"IDAT", indexed_idat(), D::ImageData, None);
    f.add(b"IEND", &[], D::Structure, None);
    let inv = run(&f.bytes);
    let p = inv
        .parts()
        .iter()
        .find(|p| p.label() == Some("Software"))
        .unwrap();
    assert_eq!(p.disposition(), D::Metadata(M::Supplement));
    assert!(p.detail().unwrap().contains("text cap"));
}

#[test]
fn not_a_png_and_short_inputs_still_yield_inventories() {
    assert_eq!(run(b"").parts().len(), 0);
    let inv = run(&SIG[..5]);
    assert_eq!(inv.parts()[0].disposition(), D::Malformed);
    let inv = run(b"GIF89a plus some more bytes");
    assert_eq!(inv.parts()[0].disposition(), D::Malformed);
    assert!(
        inv.parts()[0]
            .detail()
            .unwrap()
            .contains("missing PNG signature")
    );
}

#[test]
fn keyword_labels_escape_control_bytes_and_stay_latin1() {
    let mut f = Fixture::new();
    f.add(b"IHDR", &ihdr(4, 2, 8, 3)[8..21], D::Structure, None);
    f.add(b"tEXt", b"Caf\xe9\x1b[2J\0x", D::Skipped, None);
    f.add_raw(b"IDAT", indexed_idat(), D::ImageData, None);
    f.add(b"IEND", &[], D::Structure, None);
    let inv = run(&f.bytes);
    let l = inv
        .parts()
        .iter()
        .find_map(|p| (*p.tag() == PartTag::FourCc(*b"tEXt")).then(|| p.label()))
        .flatten()
        .unwrap();
    assert_eq!(l, "Caf\u{e9}\\x1b[2J");
}

// ── Corpus runs ──────────────────────────────────────────────────────

/// Directory of one codec-corpus set: `$ZENPNG_CODEC_CORPUS/<set>` (a local
/// checkout, read-only) or the codec-corpus crate's cache of imazen/codec-corpus.
fn corpus_dir(set: &str) -> PathBuf {
    if let Some(d) = std::env::var_os("ZENPNG_CODEC_CORPUS") {
        return PathBuf::from(d).join(set);
    }
    codec_corpus::Corpus::new()
        .expect("codec-corpus cache unavailable")
        .github_repo("imazen/codec-corpus", set, "main")
        .unwrap_or_else(|e| panic!("fetching imazen/codec-corpus {set} (main) failed: {e}"))
}

const SETS: [&str; 3] = ["pngsuite", "png-conformance", "apng-conformance"];

fn corpus_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    for set in SETS {
        pngs_under(&corpus_dir(set), &mut files);
    }
    files
}

fn pngs_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            pngs_under(&p, out);
        } else if p.extension().is_some_and(|x| x == "png") {
            out.push(p);
        }
    }
}

/// Every PNG in the conformance sets: valid files pass the testkit check;
/// files the decoder rejects still produce a valid inventory.
#[test]
fn corpus_conformance_sets() {
    let files = corpus_files();
    assert!(files.len() > 150, "found only {} files", files.len());
    let (mut checked, mut validated_only) = (0, 0);
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        let decodes = zencodec::decode::Decode::decode(
            PngDecoderConfig::new()
                .job()
                .decoder(bytes.as_slice().into(), &[])
                .map_err(|e| panic!("{}: {e}", path.display()))
                .unwrap(),
        )
        .is_ok();
        if decodes {
            zencodec_testkit::check_inventory(PngDecoderConfig::new(), &bytes)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            checked += 1;
        } else {
            run(&bytes);
            validated_only += 1;
        }
    }
    eprintln!(
        "corpus: {checked} decodable files passed check_inventory, {validated_only} undecodable files validated"
    );
    assert!(checked > 100);
}

/// One chunk line of `exiftool -v3`.
#[derive(Debug)]
struct OracleChunk {
    name: String,
    /// Declared data length (`None` for IDAT runs and IEND).
    len: Option<u64>,
    /// Total payload of an IDAT run ("1 chunk, total N bytes").
    idat_total: Option<u64>,
    /// File offset of the first dumped data byte (absent for empty chunks).
    data_off: Option<u64>,
}

/// Chunk types exiftool -v3 does not print a line for (filled in from the corpus run).
const EXIFTOOL_SILENT: [&str; 0] = [];

/// `exiftool -v3` lists each PNG chunk as `PNG IHDR (13 bytes):` followed by a
/// hex dump whose first row starts at the chunk's data offset; IDAT runs are
/// merged (`PNG IDAT (2 chunks, total N bytes)`) and IEND prints
/// `PNG IEND (end of image)`.
fn parse_exiftool_chunks(text: &str) -> Vec<OracleChunk> {
    let mut out: Vec<OracleChunk> = Vec::new();
    for l in text.lines() {
        if let Some(rest) = l.strip_prefix("PNG ") {
            let (name, tail) = rest.split_once(' ').unwrap_or((rest, ""));
            let num = |s: &str| {
                s.split(|c: char| !c.is_ascii_digit())
                    .find(|t| !t.is_empty())
                    .and_then(|t| t.parse::<u64>().ok())
            };
            let mut c = OracleChunk {
                name: name.to_string(),
                len: None,
                idat_total: None,
                data_off: None,
            };
            if let Some(t) = tail.split_once("total ") {
                c.idat_total = num(t.1);
            } else if tail.starts_with('(') && tail.contains("bytes") {
                c.len = num(tail);
            }
            out.push(c);
        } else if let Some(c) = out.last_mut()
            && c.data_off.is_none()
            && l.starts_with("      ")
            && !l.starts_with("       ")
        {
            // "      0010: 00 00 ..." (six spaces, hex offset, colon)
            if let Some((hex, _)) = l.trim_start().split_once(':')
                && hex.len() >= 4
                && let Ok(v) = u64::from_str_radix(hex, 16)
            {
                c.data_off = Some(v);
            }
        }
    }
    out
}

/// Cross-check against `exiftool -v3` when `INVENTORY_ORACLE_EXIFTOOL` names the
/// binary (see `just inventory-oracle`). Every chunk exiftool lists must appear
/// in the inventory with the same name, data offset and length; IDAT runs must
/// have the same total payload.
#[test]
fn oracle_exiftool_chunk_offsets() {
    // Opt-in by environment, like the other oracle runs: `just inventory-oracle` (or CI) sets the
    // variable. Set but wrong (missing binary) is a failure, not a skip.
    let Some(exiftool) = std::env::var_os("INVENTORY_ORACLE_EXIFTOOL") else {
        eprintln!(
            "INVENTORY_ORACLE_EXIFTOOL unset: exiftool oracle not requested (just inventory-oracle)"
        );
        return;
    };
    let files = corpus_files();
    let (mut compared, mut chunks_matched, mut bad_signature) = (0usize, 0usize, 0usize);
    let mut notes = Vec::new();
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        let out = std::process::Command::new(&exiftool)
            .arg("-v3")
            .arg(path)
            .output()
            .expect("run exiftool");
        let text = String::from_utf8_lossy(&out.stdout);
        let listed = parse_exiftool_chunks(&text);
        let inv = run(&bytes);
        if listed.is_empty() {
            // exiftool refuses files whose signature is damaged (PngSuite x*n0g*); the
            // inventory must flag exactly those.
            let first = &inv.parts()[0];
            if first.kind() == PartKind::Header && first.disposition() == D::Malformed {
                bad_signature += 1;
            } else {
                notes.push(format!("{}: exiftool listed no PNG chunks", path.display()));
            }
            continue;
        }
        // Chunks of ours, up to and including IEND, IDAT runs merged.
        let mut ours: Vec<(String, u64, u64)> = Vec::new(); // (name, start, total length)
        let mut sig_ok = false;
        for id in inv.children(None) {
            let p = inv.get(id).unwrap();
            if p.kind() == PartKind::Header {
                sig_ok = true;
            }
            match &p.tag() {
                PartTag::FourCc(cc) if p.kind() == PartKind::Chunk => {
                    let name = String::from_utf8_lossy(cc).into_owned();
                    let len = p.range().end - p.range().start;
                    if name == "IDAT"
                        && let Some(last) = ours.last_mut()
                        && last.0 == "IDAT"
                        && last.1 + last.2 == p.range().start
                    {
                        last.2 += len;
                    } else {
                        ours.push((name, p.range().start, len));
                    }
                }
                _ => {}
            }
        }
        assert!(
            sig_ok || bytes.len() < 8 || bytes[..8] != SIG,
            "{}",
            path.display()
        );
        compared += 1;
        // Match each exiftool line to a distinct inventory chunk; exiftool prints IDAT
        // runs merged and IEND without an offset, so those match by name/total.
        let mut used = vec![false; ours.len()];
        for c in &listed {
            let hit = ours.iter().enumerate().find(|(i, (n, start, tl))| {
                !used[*i]
                    && n == &c.name
                    && c.data_off.is_none_or(|o| o == start + 8)
                    && c.len.is_none_or(|l| l + 12 == *tl)
                    && c.idat_total.is_none_or(|t| {
                        let n = inv
                            .children(None)
                            .into_iter()
                            .filter_map(|id| inv.get(id))
                            .filter(|p| {
                                *p.tag() == PartTag::FourCc(*b"IDAT")
                                    && p.range().start >= *start
                                    && p.range().end <= start + tl
                            })
                            .map(|p| p.range().end - p.range().start - 12)
                            .sum::<u64>();
                        n == t
                    })
            });
            if let Some((i, _)) = hit {
                used[i] = true;
                chunks_matched += 1;
            } else {
                notes.push(format!(
                    "{}: exiftool {c:?}; inventory {:?}",
                    path.file_name().unwrap().to_string_lossy(),
                    ours.iter()
                        .filter(|(n, ..)| *n == c.name)
                        .collect::<Vec<_>>()
                ));
            }
        }
        // Other direction: every inventory chunk exiftool did not list must be a type it
        // does not print (damaged streams, chunks after IEND are not in `ours`).
        for (i, (n, start, _)) in ours.iter().enumerate() {
            if !used[i] && !EXIFTOOL_SILENT.contains(&n.as_str()) {
                notes.push(format!(
                    "{}: inventory chunk {n} at {start} was not listed by exiftool",
                    path.file_name().unwrap().to_string_lossy()
                ));
            }
        }
    }
    eprintln!(
        "oracle: {compared} files compared, {chunks_matched} chunks matched, {bad_signature} bad-signature files rejected by both, {} differences",
        notes.len()
    );
    for n in &notes {
        eprintln!("  DIFF {n}");
    }
    assert!(compared >= 20, "only {compared} files compared");
    assert!(
        notes.is_empty(),
        "{} difference(s) from exiftool",
        notes.len()
    );
}

/// The dispositions are claims about the decoder. Check the Metadata ones against
/// what the zencodec decode returns for the same bytes: the winner's payload is
/// what reaches `ImageInfo`, and what the inventory marks `Skipped` does not.
#[test]
fn metadata_dispositions_match_decoded_image_info() {
    use zencodec::decode::Decode;
    let f = everything();
    let out = PngDecoderConfig::new()
        .job()
        .decoder(f.bytes.as_slice().into(), &[])
        .unwrap()
        .decode()
        .expect("fixture decodes");
    let info = out.info();
    let m = info.metadata();
    assert_eq!(
        m.icc_profile.as_deref(),
        Some(&b"second profile"[..]),
        "iCCP: second chunk wins, broken third is dropped"
    );
    assert_eq!(m.exif.as_deref(), Some(&b"II*\0\x08\0\0\0\0\0"[..]));
    assert_eq!(m.xmp.as_deref(), Some(&b"<x:xmpmeta/>"[..]));
    assert_eq!(m.cicp, Some(zencodec::Cicp::new(1, 13, 0, true)));
    assert!(m.content_light_level.is_some());
    assert!(m.mastering_display.is_some());
    assert!(info.resolution.is_some());
    assert!(info.sequence.is_animation());
}

fn run_with(bytes: &[u8], policy: zencodec::decode::DecodePolicy) -> Inventory {
    let inv = PngDecoderConfig::new()
        .job()
        .with_policy(policy)
        .inventory(bytes)
        .unwrap()
        .unwrap();
    inv.validate().unwrap();
    inv
}

/// A strict policy makes ChunkIter skip a bad-CRC ancillary chunk silently; the
/// inventory says Dropped and the decode agrees (no EXIF in `ImageInfo`), while the
/// default policy still reads the chunk.
#[test]
fn strict_policy_drops_bad_crc_ancillary_chunks() {
    use zencodec::decode::{Decode, DecodePolicy};
    let exif = chunk(b"eXIf", b"II*\0\x08\0\0\0\0\0");
    let mut bad = exif.clone();
    *bad.last_mut().unwrap() ^= 0xff;
    let mut png = SIG.to_vec();
    png.extend(ihdr(2, 1, 8, 2));
    png.extend(&bad);
    png.extend(chunk(b"IDAT", &zlib_stored(&[0, 1, 2, 3, 4, 5, 6])));
    png.extend(chunk(b"IEND", &[]));

    let exif_part = |inv: &Inventory| {
        inv.parts()
            .iter()
            .find(|p| *p.tag() == PartTag::FourCc(*b"eXIf"))
            .cloned()
            .unwrap()
    };
    let lenient = run(&png);
    assert_eq!(exif_part(&lenient).disposition(), D::Metadata(M::Exif));
    assert!(
        exif_part(&lenient)
            .detail()
            .unwrap()
            .contains("crc mismatch")
    );
    let strict = run_with(&png, DecodePolicy::none().with_strict(true));
    assert_eq!(exif_part(&strict).disposition(), D::Dropped);

    for (policy, want_exif) in [
        (None, true),
        (Some(DecodePolicy::none().with_strict(true)), false),
    ] {
        let mut job = PngDecoderConfig::new().job();
        if let Some(p) = policy {
            job = job.with_policy(p);
        }
        let out = job
            .decoder(png.as_slice().into(), &[])
            .unwrap()
            .decode()
            .unwrap();
        assert_eq!(out.info().metadata().exif.is_some(), want_exif);
    }
}

/// Critical-chunk CRCs are verified by the zencodec decode path's probe pre-check
/// even though the decode config skips them; the inventory says so.
#[test]
fn bad_critical_crc_is_flagged_and_decode_agrees() {
    use zencodec::decode::Decode;
    let mut png = SIG.to_vec();
    png.extend(ihdr(2, 1, 8, 2));
    let mut idat = chunk(b"IDAT", &zlib_stored(&[0, 1, 2, 3, 4, 5, 6]));
    *idat.last_mut().unwrap() ^= 0xff;
    png.extend(idat);
    png.extend(chunk(b"IEND", &[]));
    let inv = run(&png);
    let p = inv
        .parts()
        .iter()
        .find(|p| *p.tag() == PartTag::FourCc(*b"IDAT"))
        .unwrap();
    assert_eq!(p.disposition(), D::ImageData);
    assert!(p.detail().unwrap().contains("rejects the file"));
    assert!(
        PngDecoderConfig::new()
            .job()
            .decoder(png.as_slice().into(), &[])
            .unwrap()
            .decode()
            .is_err()
    );
}

#[test]
fn policy_suppressing_metadata_marks_it_dropped() {
    let f = everything();
    let p = zencodec::decode::DecodePolicy::none()
        .with_allow_icc(false)
        .with_allow_exif(false)
        .with_allow_xmp(false);
    let inv = run_with(&f.bytes, p);
    for (tag, label) in [
        (*b"iCCP", Some("test")),
        (*b"eXIf", None),
        (*b"iTXt", Some("XML:com.adobe.xmp")),
    ] {
        let dropped = inv
            .parts()
            .iter()
            .filter(|p| *p.tag() == PartTag::FourCc(tag) && p.label() == label)
            .filter(|p| p.disposition() == D::Dropped)
            .count();
        assert!(dropped >= 1, "{tag:?} not dropped\n{inv}");
    }
}
