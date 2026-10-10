//! Structural inventory of a PNG file: signature, every chunk through `IEND`,
//! and whatever follows.
//!
//! The walker never decodes pixels. It classifies each chunk by replaying the
//! decisions the zencodec decode path makes (`PngAncillary::collect` before the
//! first `IDAT`, `collect_late` after the `IDAT` run, `ImageInfo` conversion in
//! `codec.rs::convert_info`), so a disposition says what *this decoder* does
//! with the bytes:
//!
//! - Data that reaches `ImageInfo` (ICC, cICP, cLLI/mDCV, eXIf, XMP, pHYs,
//!   acTL) is `Metadata(..)`.
//! - Data kept only in the native `PngInfo` (gAMA, cHRM, sRGB, sBIT, bKGD, tIME,
//!   tEXt/zTXt) is `Skipped` with a `detail` naming the native field.
//! - Duplicates that lose (`eXIf`, `iCCP`, ...) are `Skipped`.
//! - Chunks the decoder ignores that are registered PNG types are `Skipped`,
//!   unregistered types are `Unknown`, and anything after `IEND` is `Trailing`.
//!
//! CRCs are computed here even though the default decode configuration never
//! checks them; a mismatch is reported in `detail` and does not change the
//! disposition.

use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use enough::{Stop, Unstoppable};
use zencodec::ImageFormat;
use zencodec::inventory::{
    Disposition, Inventory, InventoryError, MetadataKind, Part, PartId, PartKind, PartTag,
};
use zenflate::crc32;

use crate::chunk::PNG_SIGNATURE;
use crate::chunk::ancillary::{MAX_TEXT_CHUNKS, MAX_TEXT_COMPRESSED_BYTES};
use crate::chunk::ihdr::Ihdr;
use crate::error::PngError;

/// Longest keyword / profile name PNG allows (spec: 1-79 bytes).
const MAX_KEYWORD: usize = 79;
/// Output cap the decoder gives `iCCP` inflation (`PngAncillary::parse_iccp`).
const ICC_INFLATE_CAP: usize = 1024 * 1024;
/// Output cap for `zTXt` inflation (`parse_ztxt`).
const ZTXT_INFLATE_CAP: usize = 1024 * 1024;
/// Output cap for compressed XMP `iTXt` (`try_parse_xmp`).
const XMP_INFLATE_CAP: usize = 4 * 1024 * 1024;
/// Window for the discard-inflate that finds where the IDAT zlib stream ends.
const IDAT_WINDOW: usize = 128 * 1024;
/// Output bound for that discard-inflate (the decoder has none beyond the image size).
const IDAT_MAX_OUTPUT: usize = 1 << 32;
/// Appended to a malformed iCCP: another consumer still reacts to the chunk's presence.
const ICCP_BAD: &str =
    "; zenpipe's png_srgb_transform_icc still sees the chunk and skips its sRGB transform";
/// Inflate output the prefix searches of one file may spend (the first pass over a run is
/// not counted: it mirrors the decode's own inflate). Measured in `tests/inventory_cost.rs`.
const SEARCH_WORK_BUDGET: u64 = 512 * 1024 * 1024;
/// Search for the first-row-complete position is skipped above this much compressed data.
const RUN_SEARCH_CAP: usize = 16 * 1024 * 1024;

/// One chunk's classification, built per chunk and pushed by [`Walker::commit`].
struct Entry {
    start: u64,
    end: u64,
    tag: PartTag,
    disp: Disposition,
    label: Option<String>,
    detail: Vec<String>,
    /// Byte ranges (absolute) inside the chunk's data that are split off as children while
    /// the chunk is consumed: bytes the decoder never reads (`Unreferenced`), or a tail past an
    /// internal end that still reaches the caller (`delivered`: keeps the chunk's disposition).
    holes: Vec<Hole>,
    /// Singleton slot this chunk takes (replacing, and demoting, the previous holder).
    win: Option<Slot>,
}

struct Hole {
    start: u64,
    end: u64,
    note: Cow<'static, str>,
    delivered: bool,
}

impl Entry {
    fn new(start: usize, end: usize, tag: PartTag, disp: Disposition) -> Self {
        Entry {
            start: start as u64,
            end: end as u64,
            tag,
            disp,
            label: None,
            detail: Vec::new(),
            holes: Vec::new(),
            win: None,
        }
    }

    /// The decoder reads only `used` data bytes of this chunk (data starts at `data_start`).
    fn used_prefix(&mut self, data_start: usize, used: usize, data_len: usize, note: &'static str) {
        if used < data_len {
            self.hole(data_start + used, data_start + data_len, note);
        }
    }

    /// The decoder never reads absolute bytes `start..end` of this chunk.
    fn hole(&mut self, start: usize, end: usize, note: &'static str) {
        if start < end {
            self.holes.push(Hole {
                start: start as u64,
                end: end as u64,
                note: Cow::Borrowed(note),
                delivered: false,
            });
        }
    }

    /// Absolute bytes `start..end` lie past a blob's internal end but the decoder still hands
    /// them to the caller: a child with the chunk's own disposition.
    fn delivered_tail(&mut self, start: usize, end: usize, note: String) {
        if start < end {
            self.holes.push(Hole {
                start: start as u64,
                end: end as u64,
                note: Cow::Owned(note),
                delivered: true,
            });
        }
    }
}

impl Entry {
    fn note(&mut self, s: impl Into<String>) {
        self.detail.push(s.into());
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Before the first `IDAT`: `PngAncillary::collect`.
    Pre,
    /// Inside the contiguous `IDAT` run.
    Idat,
    /// After the `IDAT` run: `PngAncillary::collect_late`.
    Late,
}

/// Singleton slots: the decoder keeps one value, a later chunk may replace it.
#[derive(Clone, Copy)]
enum Slot {
    Plte,
    Trns,
    Iccp,
    Cicp,
    Clli,
    Mdcv,
    Exif,
    Xmp,
    Actl,
    Phys,
    CreatingTool,
}
const SLOTS: usize = 11;

/// Job-level switches that change what the decode path does with a chunk.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Options {
    /// `DecodePolicy::strict`: CRCs are verified, and an ancillary chunk with a bad CRC
    /// is skipped by `ChunkIter` before `PngAncillary` sees it. That iterator only
    /// walks up to the first IDAT; `finish_metadata` reads later chunks unchecked.
    pub strict_crc: bool,
    /// The policy suppresses ICC / EXIF / XMP in `ImageInfo` (`apply_policy_to_info`).
    /// EXIF Orientation still reaches `ImageInfo` (`convert_info`).
    pub drop_icc: bool,
    pub drop_exif: bool,
    pub drop_xmp: bool,
    /// `allow_animation(false)`: `animation_frame_decoder` is rejected.
    pub no_animation: bool,
    /// `allow_progressive(false)`: `check_progressive_policy` rejects interlaced files.
    pub no_progressive: bool,
}

struct IdatChunk {
    ty: [u8; 4],
    start: usize,
    data_start: usize,
    data_end: usize,
    end: usize,
}

/// Result of inflating a prefix of a run into a discard sink.
struct Probe {
    /// The zlib stream completed.
    done: bool,
    /// Inflating stopped early because `stop_at` bytes of output were produced.
    stopped: bool,
    /// Decompressed bytes produced.
    out: u64,
    adler: u32,
    /// How far the source was read (an upper bound of the stream's end).
    pos: usize,
}

/// Filtered byte count the rows of a `w` x `h` image need (filter bytes included).
fn raw_size(ihdr: &Ihdr, w: u32, h: u32, interlaced: bool) -> Option<u64> {
    let bits = u64::from(ihdr.bit_depth) * ihdr.channels() as u64;
    let pass = |w: u64, h: u64| -> Option<u64> {
        if w == 0 || h == 0 {
            return Some(0);
        }
        h.checked_mul((w.checked_mul(bits)?.checked_add(7)? / 8).checked_add(1)?)
    };
    if !interlaced {
        return pass(u64::from(w), u64::from(h));
    }
    let mut total = 0u64;
    for (x0, y0, dx, dy) in [
        (0u64, 0u64, 8u64, 8u64),
        (4, 0, 8, 8),
        (0, 4, 4, 8),
        (2, 0, 4, 4),
        (0, 2, 2, 4),
        (1, 0, 2, 2),
        (0, 1, 1, 2),
    ] {
        let (w, h) = (u64::from(w), u64::from(h));
        let pw = if w > x0 { (w - x0).div_ceil(dx) } else { 0 };
        let ph = if h > y0 { (h - y0).div_ceil(dy) } else { 0 };
        total = total.checked_add(pass(pw, ph)?)?;
    }
    Some(total)
}

struct Walker<'a> {
    data: &'a [u8],
    stop: &'a dyn Stop,
    opts: Options,
    inv: Inventory,
    winners: [Option<PartId>; SLOTS],
    /// Child parts of chunks that were split; demoting the parent demotes these too.
    kids: BTreeMap<usize, Vec<PartId>>,
    ihdr: Option<Ihdr>,
    color_type: Option<u8>,
    palette_len: Option<usize>,
    text_count: usize,
    text_compressed: u64,
    scratch: Vec<u8>,
    idat_notes: BTreeMap<usize, String>,
    /// The contiguous IDAT run being collected; flushed when it ends.
    idat_run: Vec<IdatChunk>,
    first_idat_pos: Option<usize>,
    /// `iDOT` parts waiting for the IDAT run's position: (id, chunk start, data range).
    idot: Vec<(PartId, usize, core::ops::Range<usize>)>,
    /// Pre-IDAT fcTL / fdAT parts settled at the end: (id, is_fctl, true, data_len).
    anim: Vec<(PartId, bool, bool, u64)>,
    /// acTL `num_frames` of the winning acTL, and whether a pre-IDAT fcTL made the IDAT run frame 0.
    actl_frames: Option<u32>,
    pre_fctl: bool,
    /// Width and height of the last 26-byte pre-IDAT fcTL (frame 0 of an animation).
    pre_fctl_dims: Option<(u32, u32)>,
    /// Raw size of frame 0 per its fcTL, while the IDAT run is placed.
    frame0_alt: Option<u64>,
    /// Inflate output spent by prefix searches so far (see `SEARCH_WORK_BUDGET`).
    search_work: u64,
    /// Post-IDAT frames accepted so far, and how the fdATs after the last fcTL are treated.
    late_frames: u32,
    /// The next chunk is the first after an accepted fcTL (read as frame data whatever its type).
    fd_first: bool,
    fd_mode: FdMode,
    fd_run: Vec<IdatChunk>,
    fd_notes: BTreeMap<usize, String>,
    /// Indexed tRNS chunks, committed when the IDAT run starts, against the final palette:
    /// (entry, data start, data length, palette entries in force when the tRNS arrived).
    pending_trns: Vec<(Entry, usize, usize, Option<usize>)>,
}

#[derive(Clone, Copy)]
enum FdMode {
    /// Not directly after an accepted fcTL: never read.
    Skip(&'static str),
    /// Frame data for an accepted fcTL of this size.
    Read { w: u32, h: u32 },
}

fn limit_err(_: InventoryError) -> PngError {
    PngError::Decode("PNG inventory exceeds the part limit".into())
}

/// Walk `data` and return its inventory.
pub(crate) fn walk(data: &[u8], stop: &dyn Stop, opts: Options) -> Result<Inventory, PngError> {
    let len = data.len() as u64;
    let mut inv = Inventory::new(ImageFormat::Png, len);

    if data.is_empty() {
        return Ok(inv);
    }
    let sig_len = data.len().min(8);
    if data.len() < 8 || data[..8] != PNG_SIGNATURE {
        let why = if data.len() < 8 && data[..] == PNG_SIGNATURE[..data.len()] {
            "truncated PNG signature"
        } else {
            "missing PNG signature; the decoder rejects the file"
        };
        inv.push(
            None,
            Part::new(
                PartKind::Header,
                PartTag::None,
                0..sig_len as u64,
                Disposition::Malformed,
            )
            .with_detail(why),
        )
        .map_err(limit_err)?;
        inv.fill_gaps(None, Disposition::Malformed)
            .map_err(limit_err)?;
        return Ok(inv);
    }
    inv.push(
        None,
        Part::new(
            PartKind::Header,
            PartTag::None,
            0..8,
            Disposition::Structure,
        )
        .with_label("PNG signature"),
    )
    .map_err(limit_err)?;

    let mut w = Walker {
        data,
        stop,
        opts,
        inv,
        winners: [None; SLOTS],
        kids: BTreeMap::new(),
        ihdr: None,
        color_type: None,
        palette_len: None,
        text_count: 0,
        text_compressed: 0,
        scratch: Vec::new(),
        idat_notes: BTreeMap::new(),
        idat_run: Vec::new(),
        first_idat_pos: None,
        idot: Vec::new(),
        anim: Vec::new(),
        actl_frames: None,
        pre_fctl: false,
        pre_fctl_dims: None,
        frame0_alt: None,
        search_work: 0,
        late_frames: 0,
        fd_first: false,
        fd_mode: FdMode::Skip("fdAT not directly after an fcTL"),
        fd_run: Vec::new(),
        fd_notes: BTreeMap::new(),
        pending_trns: Vec::new(),
    };
    w.run()?;
    w.finish()?;
    let mut inv = w.inv;
    inv.fill_gaps(None, Disposition::Trailing)
        .map_err(limit_err)?;
    Ok(inv)
}

const SEARCH_LIMIT_NOTE: &str = "inflate work limit for placing the stream end reached; bytes after the end of the data the decoder reads are not distinguished";

/// Offset just past the `<?xpacket end=...?>` processing instruction, if any.
fn xpacket_end(text: &[u8]) -> Option<usize> {
    const PI: &[u8] = b"<?xpacket end=";
    let at = text.windows(PI.len()).rposition(|w| w == PI)?;
    let close = text[at..].windows(2).position(|w| w == b"?>")?;
    Some(at + close + 2)
}

fn md(kind: MetadataKind) -> Disposition {
    Disposition::Metadata(kind)
}

/// Latin-1 text from `bytes`, control characters escaped so an auditor's
/// terminal can't be driven by a file's keyword.
fn latin1(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        if b < 0x20 || (0x7f..0xa0).contains(&b) {
            s.push_str(&format!("\\x{b:02x}"));
        } else {
            s.push(b as char);
        }
    }
    s
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn is_letter(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

/// Registered PNG chunk types the decoder has no code for.
fn registered_unread(t: &[u8; 4]) -> bool {
    matches!(
        t,
        b"hIST"
            | b"sPLT"
            | b"oFFs"
            | b"pCAL"
            | b"sCAL"
            | b"sTER"
            | b"gIFg"
            | b"gIFx"
            | b"gIFt"
            | b"dSIG"
            | b"tRNS"
            | b"PLTE"
            | b"IHDR"
            | b"gAMA"
            | b"cHRM"
            | b"sRGB"
            | b"iCCP"
            | b"sBIT"
            | b"bKGD"
            | b"pHYs"
            | b"tIME"
            | b"cICP"
            | b"cLLI"
            | b"mDCV"
            | b"eXIf"
            | b"acTL"
            | b"fcTL"
            | b"fdAT"
            | b"iDOT"
            | b"tEXt"
            | b"zTXt"
            | b"iTXt"
    )
}

/// `InputSource` over the data of the contiguous IDAT run, serving at most `left` bytes
/// and tracking how far the inflater read.
struct RunSource<'a> {
    data: &'a [u8],
    chunks: &'a [IdatChunk],
    ci: usize,
    off: usize,
    left: usize,
}

impl zenflate::InputSource for RunSource<'_> {
    type Error = core::convert::Infallible;

    fn fill_buf(&mut self) -> Result<&[u8], Self::Error> {
        while let Some(c) = self.chunks.get(self.ci) {
            let len = c.data_end - c.data_start;
            if self.off < len {
                let s = &self.data[c.data_start + self.off..c.data_end];
                return Ok(&s[..s.len().min(self.left)]);
            }
            self.ci += 1;
            self.off = 0;
        }
        Ok(&[])
    }

    fn consume(&mut self, n: usize) {
        self.off += n;
        self.left -= n;
    }
}

/// Staging window of the streaming inflater (`INPUT_BUF_SIZE`) plus its bit buffer: the
/// source can be read this far past the true end of the zlib stream.
const OVERREAD_WINDOW: usize = 32 * 1024 + 16;

impl Walker<'_> {
    fn slot_win(&mut self, slot: Slot, id: PartId, at: u64) {
        if let Some(prev) = self.winners[slot as usize].replace(id) {
            self.demote(
                prev,
                Disposition::Skipped,
                &format!("superseded by a later chunk at offset {at}"),
            );
        }
    }

    fn slot_taken(&self, slot: Slot) -> bool {
        self.winners[slot as usize].is_some()
    }

    /// Change a pushed part's disposition (and its split children, which stop being
    /// consumed with it) and append a remark to its detail.
    fn demote(&mut self, id: PartId, disp: Disposition, note: &str) {
        let old = self.inv.get(id).and_then(|p| p.detail.clone());
        self.inv.set_disposition(id, disp);
        let detail = match old {
            Some(o) if !note.is_empty() => format!("{o}; {note}"),
            Some(o) => o,
            None => String::from(note),
        };
        if !detail.is_empty() {
            self.inv.set_detail(id, detail);
        }
        if let Some(kids) = self.kids.get(&id.index()).cloned() {
            for k in kids {
                if self.inv.get(k).map(|p| p.disposition) != Some(Disposition::Unreferenced) {
                    self.inv.set_disposition(k, disp);
                }
            }
        }
    }

    /// Push a classified chunk, splitting off the bytes the decoder never reads when
    /// the chunk is consumed (`Entry::holes`).
    fn commit(&mut self, mut e: Entry) -> Result<PartId, PngError> {
        let split = e.disp.is_consumed() && !e.holes.is_empty();
        let mut part = Part::new(PartKind::Chunk, e.tag.clone(), e.start..e.end, e.disp);
        if let Some(l) = e.label.take() {
            part = part.with_label(Cow::Owned(l));
        }
        if !e.detail.is_empty() {
            part = part.with_detail(e.detail.join("; "));
        }
        if split {
            part = part.with_body(e.start..e.end);
        }
        let id = self.inv.push(None, part).map_err(limit_err)?;
        if split {
            e.holes.sort_by_key(|h| (h.start, h.end));
            let data_end = e.end - 4;
            let mut kids = Vec::new();
            let mut cursor = e.start;
            let mut push = |w: &mut Self, p: Part| -> Result<(), PngError> {
                kids.push(w.inv.push(Some(id), p).map_err(limit_err)?);
                Ok(())
            };
            for h in &e.holes {
                let (hs, he) = (h.start.max(cursor), h.end.min(data_end));
                if hs >= he {
                    continue;
                }
                if cursor < hs {
                    let head = Part::new(PartKind::Field, PartTag::None, cursor..hs, e.disp)
                        .with_detail("bytes the decoder reads");
                    push(self, head)?;
                }
                let hole = if h.delivered {
                    Part::new(PartKind::Field, PartTag::None, hs..he, e.disp)
                } else {
                    Part::new(
                        PartKind::Gap,
                        PartTag::None,
                        hs..he,
                        Disposition::Unreferenced,
                    )
                }
                .with_detail(h.note.clone());
                push(self, hole)?;
                cursor = he;
            }
            if cursor < data_end {
                let head = Part::new(PartKind::Field, PartTag::None, cursor..data_end, e.disp)
                    .with_detail("bytes the decoder reads");
                push(self, head)?;
            }
            let crc = Part::new(
                PartKind::Field,
                PartTag::None,
                data_end..e.end,
                Disposition::Structure,
            )
            .with_label("crc");
            push(self, crc)?;
            self.kids.insert(id.index(), kids);
        }
        if let Some(slot) = e.win {
            self.slot_win(slot, id, e.start);
        }
        Ok(id)
    }

    /// Inflate `compressed` into the scratch buffer (capped like the decoder's own
    /// buffer, so a profile the decoder rejects for size is rejected here too).
    /// Returns (bytes written, bytes of input consumed).
    fn inflate(&mut self, compressed: &[u8], cap: usize) -> Result<(usize, usize), String> {
        if self.scratch.len() < cap {
            self.scratch.resize(cap, 0);
        }
        let mut d = zenflate::Decompressor::new();
        match d.zlib_decompress(compressed, &mut self.scratch[..cap], Unstoppable) {
            Ok(o) => Ok((o.output_written, o.input_consumed)),
            Err(e) => Err(format!("{e:?}")),
        }
    }

    fn truncated(&mut self, e: Entry) -> Result<(), PngError> {
        self.end_idat_run()?;
        self.commit(e)?;
        Ok(())
    }

    fn run(&mut self) -> Result<(), PngError> {
        let data = self.data;
        let total = data.len();
        let mut pos = 8usize;
        let mut phase = Phase::Pre;
        let mut first = true;
        while pos < total {
            self.stop.check().map_err(PngError::from)?;
            if pos + 12 > total {
                // Fewer bytes than the smallest chunk. The decoder's own chunk readers stop
                // here, but the probe pre-check in DecodeJob::decode errors on it.
                let mut e = Entry::new(pos, total, PartTag::None, Disposition::Malformed);
                if pos + 8 <= total {
                    e.tag = PartTag::FourCc([
                        data[pos + 4],
                        data[pos + 5],
                        data[pos + 6],
                        data[pos + 7],
                    ]);
                }
                e.note(format!(
                    "truncated chunk header: {} bytes remain, a chunk needs at least 12; \
                     DecodeJob::decode rejects the file (its probe pre-check fails on it)",
                    total - pos
                ));
                self.truncated(e)?;
                break;
            }
            let length = be32(&data[pos..]) as usize;
            let ty: [u8; 4] = [data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]];
            let body_start = pos + 8;
            let crc_end = body_start
                .checked_add(length)
                .and_then(|v| v.checked_add(4))
                .filter(|&v| v <= total);
            let Some(crc_end) = crc_end else {
                let mut e = Entry::new(pos, total, PartTag::FourCc(ty), Disposition::Malformed);
                e.note(format!(
                    "chunk declares {length} data bytes but only {} remain; \
                     DecodeJob::decode rejects the file (its probe pre-check fails on it)",
                    total - body_start
                ));
                self.truncated(e)?;
                break;
            };
            let body = &data[body_start..crc_end - 4];
            let stored = be32(&data[crc_end - 4..]);
            let mut entry = Entry::new(pos, crc_end, PartTag::FourCc(ty), Disposition::Skipped);
            let crc_bad = crc32(crc32(0, &ty), body) != stored;
            // Ancillary = bit 5 of the first type byte (ChunkIter).
            let ancillary = ty[0] & 0x20 != 0;
            let after_idat = phase == Phase::Late || (phase == Phase::Idat && &ty != b"IDAT");
            if crc_bad {
                entry.note(if ancillary && after_idat {
                    "crc mismatch (no decode path checks it: finish_metadata reads chunks after the IDAT run unchecked)"
                } else if ancillary {
                    "crc mismatch (the default decode does not check it; a strict policy skips the chunk)"
                } else {
                    "crc mismatch in a critical chunk: DecodeJob::decode rejects the file \
                     (its probe pre-check verifies critical CRCs even when decode skips them)"
                });
            }
            if !ty.iter().all(|&b| is_letter(b)) {
                entry.note("chunk type is not four ASCII letters");
            }

            // Phase transitions mirror RowDecoder::new / IdatSource / finish_metadata.
            if phase == Phase::Idat && &ty != b"IDAT" {
                self.end_idat_run()?;
                phase = Phase::Late;
            }
            // Only ChunkIter (up to the first IDAT) skips bad-CRC ancillary chunks;
            // finish_metadata reads the later ones without checking.
            let strict_skip = crc_bad && ancillary && self.opts.strict_crc && phase == Phase::Pre;
            let mut done = false;
            if strict_skip {
                entry.disp = Disposition::Dropped;
                entry.note(
                    "strict policy: bad-CRC ancillary chunk skipped silently (no PngWarning)",
                );
                self.commit(entry)?;
            } else if first {
                first = false;
                self.classify_first(&ty, body, &mut entry);
                self.commit(entry)?;
            } else {
                match phase {
                    Phase::Pre if &ty == b"IDAT" => {
                        self.flush_trns()?;
                        phase = Phase::Idat;
                        self.first_idat_pos = Some(pos);
                        self.idat_run.push(IdatChunk {
                            ty,
                            start: pos,
                            data_start: body_start,
                            data_end: crc_end - 4,
                            end: crc_end,
                        });
                        self.note_crc(pos, &entry);
                    }
                    Phase::Pre => {
                        self.classify_pre(&ty, body, body_start, &mut entry);
                        self.commit_pre(&ty, body_start, body.len(), entry)?;
                    }
                    Phase::Idat => {
                        self.idat_run.push(IdatChunk {
                            ty,
                            start: pos,
                            data_start: body_start,
                            data_end: crc_end - 4,
                            end: crc_end,
                        });
                        self.note_crc(pos, &entry);
                    }
                    Phase::Late => {
                        // `FdatSource::new` never checks the type of the chunk after an fcTL:
                        // whatever it is, its data is the frame's zlib stream.
                        let first_frame_chunk = self.fd_first
                            && matches!(self.fd_mode, FdMode::Read { .. })
                            && !matches!(&ty, b"fdAT" | b"fcTL" | b"IEND");
                        let as_frame = first_frame_chunk && body.len() >= 4;
                        if &ty != b"fdAT" && !first_frame_chunk {
                            self.end_fd_run()?;
                        }
                        if first_frame_chunk && !as_frame {
                            // Shorter than a sequence number: `FdatSource::new` takes it as the
                            // frame's first chunk, gets no deflate data from it, and goes on
                            // into the fdATs that follow, so the frame stays readable.
                            self.fd_first = false;
                            done = self.classify_late(&ty, body, body_start, &mut entry);
                            entry.note(
                                "taken as the frame's first fdAT by the animation decoder (FdatSource::new does not check the chunk type); shorter than a sequence number, it contributes no frame data",
                            );
                            self.commit(entry)?;
                        } else if as_frame {
                            self.fd_first = false;
                            let c = IdatChunk {
                                ty,
                                start: pos,
                                data_start: body_start + 4,
                                data_end: crc_end - 4,
                                end: crc_end,
                            };
                            let mut note = entry.detail.join("; ");
                            if !note.is_empty() {
                                note.push_str("; ");
                            }
                            note.push_str(
                                "read as the frame's zlib data although it is not an fdAT: FdatSource::new does not check the chunk type",
                            );
                            self.fd_notes.insert(c.start, note);
                            self.fd_run.push(c);
                        } else {
                            done = self.classify_late(&ty, body, body_start, &mut entry);
                            self.commit_late(&ty, body, body_start, entry)?;
                        }
                    }
                }
            }
            pos = crc_end;
            if done {
                break;
            }
        }
        self.flush_trns()?;
        self.end_idat_run()?;
        self.end_fd_run()?;
        Ok(())
    }

    /// Per-chunk remarks of IDATs held back until the run's end: keep them with the chunk.
    fn note_crc(&mut self, _pos: usize, entry: &Entry) {
        if let Some(c) = self.idat_run.last()
            && !entry.detail.is_empty()
        {
            self.idat_notes.insert(c.start, entry.detail.join("; "));
        }
    }

    /// fcTL / fdAT / iDOT need facts only known at the end of the walk; register them.
    /// An indexed tRNS before PLTE waits for the palette size.
    fn commit_pre(
        &mut self,
        ty: &[u8; 4],
        body_start: usize,
        data_len: usize,
        entry: Entry,
    ) -> Result<(), PngError> {
        let (start, end) = (entry.start as usize, entry.end as usize);
        if ty == b"tRNS" && entry.disp == Disposition::ImageData && self.color_type == Some(3) {
            self.pending_trns
                .push((entry, body_start, data_len, self.palette_len));
            return Ok(());
        }
        let id = self.commit(entry)?;
        match ty {
            b"fcTL" => {
                self.pre_fctl = true;
                if data_len == 26 {
                    let b = &self.data[body_start..body_start + 26];
                    self.pre_fctl_dims = Some((be32(&b[4..]), be32(&b[8..])));
                }
                self.anim.push((id, true, true, data_len as u64));
            }
            b"fdAT" => self.anim.push((id, false, true, data_len as u64)),
            b"iDOT" => self.idot.push((id, start, body_start..end - 4)),
            _ => {}
        }
        Ok(())
    }

    /// Commit the held indexed tRNS chunks against the final palette. `collect` keeps a tRNS
    /// whole unless it is longer than the palette present when it arrives (then only its
    /// presence is kept); the expander reads at most one alpha value per final palette entry,
    /// and an index has `bit_depth` bits.
    fn flush_trns(&mut self) -> Result<(), PngError> {
        let reach = 1usize << self.ihdr.map_or(8, |i| i.bit_depth.min(8));
        for (mut e, body_start, len, arrival) in core::mem::take(&mut self.pending_trns) {
            let (used, why) = match arrival {
                Some(n) if len > n => (
                    0,
                    "tRNS longer than the palette present when it arrives: only its presence is kept, the bytes are discarded",
                ),
                _ => (
                    len.min(self.palette_len.unwrap_or(len)).min(reach),
                    "tRNS entries beyond the final palette size or 2^bit_depth: never read",
                ),
            };
            e.used_prefix(body_start, used, len, why);
            self.commit(e)?;
        }
        Ok(())
    }

    /// Post-IDAT chunks. fcTL decides at once how its frame's fdATs are read; fdAT chunks of
    /// a read frame are held until the group ends so the end of the frame's zlib stream can
    /// be placed (as for the IDAT run).
    fn commit_late(
        &mut self,
        ty: &[u8; 4],
        body: &[u8],
        body_start: usize,
        mut entry: Entry,
    ) -> Result<(), PngError> {
        self.fd_first = false;
        match ty {
            b"fcTL" => {
                self.fd_mode = FdMode::Skip("fdAT after an fcTL that is not read");
                let allowed = self
                    .actl_frames
                    .map(|n| n.saturating_sub(u32::from(self.pre_fctl)));
                if !self.slot_taken(Slot::Actl) {
                    entry.note("fcTL without a valid acTL: not read");
                } else if self.opts.no_animation {
                    entry.note(
                        "decode policy forbids animation (animation_frame_decoder is rejected)",
                    );
                } else if body.len() != 26 {
                    entry.disp = Disposition::Malformed;
                    entry.note(format!("fcTL is {} bytes, expected 26", body.len()));
                } else if allowed.is_some_and(|n| self.late_frames >= n) {
                    entry.note(
                        "frame beyond acTL num_frames: ApngDecoder::next_frame stops before it",
                    );
                } else {
                    entry.disp = md(MetadataKind::Animation);
                    self.late_frames += 1;
                    self.fd_mode = FdMode::Read {
                        w: be32(&body[4..]),
                        h: be32(&body[8..]),
                    };
                    self.fd_first = true;
                }
                self.commit(entry)?;
            }
            b"fdAT" if body.len() < 4 => {
                self.end_fd_run()?;
                entry.disp = Disposition::Malformed;
                entry.note("fdAT shorter than its sequence number");
                self.commit(entry)?;
            }
            b"fdAT" => match self.fd_mode {
                FdMode::Read { .. } => {
                    let c = IdatChunk {
                        ty: *ty,
                        start: entry.start as usize,
                        data_start: body_start + 4,
                        data_end: entry.end as usize - 4,
                        end: entry.end as usize,
                    };
                    if !entry.detail.is_empty() {
                        self.fd_notes.insert(c.start, entry.detail.join("; "));
                    }
                    self.fd_run.push(c);
                }
                FdMode::Skip(why) => {
                    entry.note(why);
                    self.commit(entry)?;
                }
            },
            _ => {
                self.fd_mode = FdMode::Skip("fdAT not directly after an fcTL");
                self.commit(entry)?;
            }
        }
        Ok(())
    }

    /// Push the held fdAT group of the current frame.
    fn end_fd_run(&mut self) -> Result<(), PngError> {
        if self.fd_run.is_empty() {
            return Ok(());
        }
        let run = core::mem::take(&mut self.fd_run);
        let notes = core::mem::take(&mut self.fd_notes);
        let needed = match (self.fd_mode, self.ihdr) {
            (FdMode::Read { w, h }, Some(i)) => raw_size(&i, w, h, false),
            _ => None,
        };
        self.push_run(run, notes, *b"fdAT", needed)
    }

    /// End of the contiguous IDAT run: place the end of its zlib stream and push the chunks.
    fn end_idat_run(&mut self) -> Result<(), PngError> {
        if self.idat_run.is_empty() {
            return Ok(());
        }
        let run = core::mem::take(&mut self.idat_run);
        let notes = core::mem::take(&mut self.idat_notes);
        let needed = self
            .ihdr
            .and_then(|i| raw_size(&i, i.width, i.height, i.interlace == 1));
        // A valid pre-IDAT fcTL makes the IDAT run frame 0 for the animation decoder, which reads
        // only the fcTL-sized rows. When the stream yields fewer bytes than the IHDR rows,
        // `decode()` rejects the file and that path is what reads it.
        let alt = match (self.pre_fctl_dims, self.ihdr) {
            (Some((w, h)), Some(i)) if self.slot_taken(Slot::Actl) && !self.opts.no_animation => {
                raw_size(&i, w, h, false)
            }
            _ => None,
        };
        self.frame0_alt = alt;
        self.push_run(run, notes, *b"IDAT", needed)?;
        self.frame0_alt = None;
        self.settle_idot()
    }

    /// Push the chunks of a zlib-carrying run. The end of the consumed data is found by
    /// inflating the run into a discard sink (what `drain_stream` does after the last row):
    /// the chunk holding it keeps the bytes after it as `Unreferenced`, and chunks entirely
    /// after it are never read. `needed` is the filtered size the image's rows require.
    fn push_run(
        &mut self,
        run: Vec<IdatChunk>,
        notes: BTreeMap<usize, String>,
        tag: [u8; 4],
        needed: Option<u64>,
    ) -> Result<(), PngError> {
        let (end_abs, extra) = self.place_run(&run, needed)?;
        // Remarks about the whole run go on the chunk holding the cut (else the first chunk).
        let carrier = end_abs
            .and_then(|end| {
                run.iter()
                    .position(|c| c.data_start <= end && end <= c.data_end)
            })
            .unwrap_or(0);
        for (i, c) in run.iter().enumerate() {
            let mut e = Entry::new(
                c.start,
                c.end,
                PartTag::FourCc(c.ty),
                Disposition::ImageData,
            );
            if let Some(n) = notes.get(&c.start) {
                e.note(n.clone());
            }
            if tag == *b"fdAT" {
                e.note("animation frame data; sequence numbers are not checked");
            }
            if i == carrier {
                for n in &extra {
                    e.note(n.clone());
                }
            }
            if let Some(end) = end_abs {
                if c.data_start >= end && c.data_end > c.data_start {
                    e.disp = Disposition::Skipped;
                    e.note("after the end of the data the decoder reads from the zlib stream: never read");
                } else if c.data_end > end && end >= c.data_start {
                    e.used_prefix(
                        c.data_start,
                        end - c.data_start,
                        c.data_end - c.data_start,
                        "bytes after the data the decoder reads from the zlib stream: never read",
                    );
                }
            }
            self.commit(e)?;
        }
        Ok(())
    }

    /// Inflate the first `limit` bytes of the run into a discard sink, as the default
    /// decode does (checksum skipped).
    fn inflate_run(
        &mut self,
        run: &[IdatChunk],
        limit: usize,
        stop_at: Option<u64>,
    ) -> Result<Probe, PngError> {
        let src = RunSource {
            data: self.data,
            chunks: run,
            ci: 0,
            off: 0,
            left: limit,
        };
        let mut d = zenflate::StreamDecompressor::zlib(src, IDAT_WINDOW)
            .with_skip_checksum(true)
            .with_max_output_size(Some(IDAT_MAX_OUTPUT));
        let mut sum = zenflate::Adler32Hasher::new();
        let (mut out, mut stalled, mut done, mut stopped) = (0u64, 0, false, false);
        loop {
            self.stop.check().map_err(PngError::from)?;
            if d.is_done() {
                done = true;
                break;
            }
            match d.fill() {
                Ok(o) => {
                    let n = o.len();
                    sum.write(o);
                    out += n as u64;
                    d.advance(n);
                    if stop_at.is_some_and(|s| out >= s) {
                        stopped = true;
                        break;
                    }
                    stalled = if n == 0 { stalled + 1 } else { 0 };
                    if stalled > 2 && !d.is_done() {
                        break;
                    }
                }
                Err(_) => {
                    // Output decoded before the error is still pending in the window.
                    let pending = d.peek();
                    sum.write(pending);
                    out += pending.len() as u64;
                    break;
                }
            }
        }
        let src = d.source_ref();
        let mut pos = 0;
        for c in run.iter().take(src.ci) {
            pos += c.data_end - c.data_start;
        }
        Ok(Probe {
            done,
            stopped,
            out,
            adler: sum.finish(),
            pos: pos + src.off,
        })
    }

    /// File offset where the data the decoder reads from the run's zlib stream ends, plus
    /// remarks. `None` when it can't be placed.
    ///
    /// - If the stream inflates to more than `needed` bytes (the rows), or breaks after the
    ///   rows, the end is the shortest input prefix that yields `needed` bytes.
    /// - Otherwise it is the end of the stream: the shortest prefix that still completes
    ///   (the inflater stages up to 32 KiB of input, so the source position only bounds it;
    ///   candidates are the positions whose preceding four bytes equal the Adler-32 of the
    ///   output, with a binary search over the staging window when the file's checksum is
    ///   wrong, which the default decode accepts).
    fn place_run(
        &mut self,
        run: &[IdatChunk],
        needed: Option<u64>,
    ) -> Result<(Option<usize>, Vec<String>), PngError> {
        let total: usize = run.iter().map(|c| c.data_end - c.data_start).sum();
        let stop = needed.map(|n| n.saturating_add(1));
        let p = self.inflate_run(run, total, stop)?;
        // Concatenated offset -> file offset of the first byte not read.
        let to_file = |l: usize| -> usize {
            let mut left = l;
            for c in run {
                let len = c.data_end - c.data_start;
                if left <= len {
                    return c.data_start + left;
                }
                left -= len;
            }
            run.last().map_or(0, |c| c.data_end)
        };
        // The rows to cut at: the IHDR rows, or frame 0's smaller fcTL rows when the stream
        // does not even yield the IHDR rows.
        let needed = match (needed, self.frame0_alt) {
            (Some(n), Some(a)) if p.out < n && a < n => Some(a),
            (n, _) => n,
        };
        if let Some(n) = needed
            && ((p.done && p.out > n) || (!p.done && p.out >= n))
        {
            if total > RUN_SEARCH_CAP {
                return Ok((
                    None,
                    vec![String::from(
                        "stream holds more than the image rows (or breaks after them); too large to place the cut, bytes after the rows are not distinguished",
                    )],
                ));
            }
            let (mut a, mut b) = (0usize, total);
            while a < b {
                if self.search_work > SEARCH_WORK_BUDGET {
                    return Ok((None, vec![String::from(SEARCH_LIMIT_NOTE)]));
                }
                let mid = a + (b - a) / 2;
                let q = self.inflate_run(run, mid, Some(n))?;
                self.search_work += q.out;
                if q.out >= n {
                    b = mid;
                } else {
                    a = mid + 1;
                }
            }
            let why = if p.done {
                format!(
                    "{} decompressed bytes after the last row are discarded; the compressed bytes after the rows (including the zlib footer) are never used",
                    p.out - n
                )
            } else if p.stopped {
                String::from(
                    "more decompressed data follows the last row and is discarded; the compressed bytes after the rows (including the zlib footer) are never used",
                )
            } else {
                String::from(
                    "the zlib stream breaks after the last row; bytes after the rows are never used",
                )
            };
            return Ok((Some(to_file(b)), vec![why]));
        }
        if !p.done {
            return Ok((
                None,
                vec![String::from(
                    "zlib stream does not complete; bytes after its end are not distinguished",
                )],
            ));
        }
        // Cumulative start (in the concatenated run data) of each chunk, for byte lookups.
        let mut cum = Vec::with_capacity(run.len());
        let mut acc = 0usize;
        for c in run {
            cum.push(acc);
            acc += c.data_end - c.data_start;
        }
        let byte = |i: usize| -> u8 {
            let k = cum.partition_point(|&c| c <= i).saturating_sub(1);
            run.get(k)
                .and_then(|c| self.data.get(c.data_start + (i - cum[k])))
                .copied()
                .unwrap_or(0)
        };
        let hi = p.pos;
        let lo = hi.saturating_sub(OVERREAD_WINDOW).max(2);
        let mut found = None;
        for l in lo..=hi {
            if l >= 4
                && u32::from_be_bytes([byte(l - 4), byte(l - 3), byte(l - 2), byte(l - 1)])
                    == p.adler
                && {
                    let q = self.inflate_run(run, l, None)?;
                    self.search_work += q.out;
                    q.done
                }
            {
                found = Some(l);
                break;
            }
        }
        if found.is_none() {
            let (mut a, mut b) = (lo, hi);
            while a < b {
                if self.search_work > SEARCH_WORK_BUDGET {
                    return Ok((None, vec![String::from(SEARCH_LIMIT_NOTE)]));
                }
                let mid = a + (b - a) / 2;
                let q = self.inflate_run(run, mid, None)?;
                self.search_work += q.out;
                if q.done {
                    b = mid;
                } else {
                    a = mid + 1;
                }
            }
            found = Some(b);
        }
        Ok((Some(to_file(found.unwrap_or(hi))), Vec::new()))
    }

    /// `iDOT` is read only when `idot::validate` accepts it (non-interlaced, one table).
    fn settle_idot(&mut self) -> Result<(), PngError> {
        let ids = core::mem::take(&mut self.idot);
        if ids.is_empty() {
            return Ok(());
        }
        let (Some(ihdr), Some(first)) = (self.ihdr, self.first_idat_pos) else {
            return Ok(());
        };
        let multiple = ids.len() > 1;
        for (id, start, body) in ids {
            let ok = !multiple
                && crate::decoder::idot::validate(self.data, start, &self.data[body], &ihdr, first)
                    .is_some();
            if !ok {
                let why = if multiple {
                    "more than one iDOT makes the table ambiguous: ignored"
                } else {
                    "iDOT rejected by idot::validate (interlaced, malformed, or not tiling the IDAT run): ignored"
                };
                self.demote(id, Disposition::Skipped, why);
            }
        }
        Ok(())
    }

    fn classify_first(&mut self, ty: &[u8; 4], body: &[u8], e: &mut Entry) {
        if ty != b"IHDR" {
            e.disp = Disposition::Malformed;
            e.note("first chunk is not IHDR; the decoder rejects the file");
            return;
        }
        if body.len() != 13 {
            e.disp = Disposition::Malformed;
            e.note(format!("IHDR is {} bytes, expected 13", body.len()));
            return;
        }
        self.color_type = Some(body[9]);
        self.ihdr = Ihdr::parse_fields(body).ok();
        if self.opts.no_progressive && body[12] == 1 {
            e.note("interlaced image: DecodeJob::decode rejects it under this policy (allow_progressive = false)");
        }
        e.disp = Disposition::Structure;
    }

    /// Keyword before the first NUL, only when it is a legal 1..=79 byte keyword.
    fn keyword(body: &[u8]) -> Option<(&[u8], usize)> {
        let nul = body.iter().take(MAX_KEYWORD + 1).position(|&b| b == 0)?;
        (nul >= 1).then(|| (&body[..nul], nul))
    }

    fn classify_pre(&mut self, ty: &[u8; 4], body: &[u8], body_start: usize, e: &mut Entry) {
        match ty {
            b"IHDR" => e.note("duplicate IHDR ignored (PngAncillary::collect has no IHDR arm)"),
            b"IEND" => e.note(
                "IEND before the first IDAT is ignored by RowDecoder::new, which keeps scanning",
            ),
            b"PLTE" => {
                if !body.len().is_multiple_of(3) || body.is_empty() {
                    e.disp = Disposition::Malformed;
                    e.note("PLTE length is zero or not a multiple of 3; the decode fails");
                } else {
                    self.palette_len = Some(body.len() / 3);
                    if self.color_type == Some(3) {
                        e.disp = Disposition::ImageData;
                        e.note("palette");
                        // An index has bit_depth bits, so entries past 2^bit_depth are unreachable.
                        let reach = 1usize << self.ihdr.map_or(8, |i| i.bit_depth.min(8));
                        e.used_prefix(
                            body_start,
                            3 * (body.len() / 3).min(reach),
                            body.len(),
                            "palette entries beyond 2^bit_depth: unreachable by any index, never used",
                        );
                    } else {
                        e.disp = Disposition::Dropped;
                        e.note("PLTE in a non-indexed image: stored but not used for pixels");
                    }
                    e.win = Some(Slot::Plte);
                }
            }
            b"tRNS" => {
                if body.is_empty() {
                    e.disp = Disposition::Dropped;
                    e.note("empty tRNS ignored");
                } else {
                    if matches!(self.color_type, Some(0 | 2 | 3)) {
                        e.disp = Disposition::ImageData;
                        e.note("transparency");
                        // RowExpander reads 2 (gray) / 6 (RGB) bytes; an indexed tRNS longer than
                        // the palette is reduced to its presence (`PngAncillary::collect`).
                        // Indexed: settled in `flush_trns` against the final palette.
                        match self.color_type {
                            Some(0) => e.used_prefix(
                                body_start,
                                2,
                                body.len(),
                                "tRNS bytes past the gray sample: never read",
                            ),
                            Some(2) => e.used_prefix(
                                body_start,
                                6,
                                body.len(),
                                "tRNS bytes past the RGB sample: never read",
                            ),
                            _ => {}
                        }
                    } else {
                        e.disp = Disposition::Dropped;
                        e.note("tRNS with an alpha colour type: not used");
                    }
                    e.win = Some(Slot::Trns);
                }
            }
            b"iCCP" => self.iccp(body, body_start, e),
            b"gAMA" => native(e, body.len() == 4, "gAMA", "PngInfo::gamma"),
            b"sRGB" => native(e, !body.is_empty(), "sRGB", "PngInfo::srgb_intent"),
            b"cHRM" => native(e, body.len() == 32, "cHRM", "PngInfo::chrm"),
            b"cICP" => {
                if body.len() == 4 {
                    e.disp = md(MetadataKind::Cicp);
                    e.win = Some(Slot::Cicp);
                } else {
                    wrong_len(e, "cICP", 4);
                }
            }
            b"cLLI" => {
                if body.len() == 8 {
                    e.disp = md(MetadataKind::HdrStatic);
                    e.win = Some(Slot::Clli);
                } else {
                    wrong_len(e, "cLLI", 8);
                }
            }
            b"mDCV" => {
                if body.len() == 24 {
                    e.disp = md(MetadataKind::HdrStatic);
                    e.win = Some(Slot::Mdcv);
                } else {
                    wrong_len(e, "mDCV", 24);
                }
            }
            b"eXIf" => {
                e.disp = md(MetadataKind::Exif);
                e.win = Some(Slot::Exif);
            }
            b"iTXt" => self.itxt(body, body_start, e, false),
            b"tEXt" => self.text(body, e),
            b"zTXt" => self.ztxt(body, e),
            b"acTL" => {
                if body.len() != 8 {
                    wrong_len(e, "acTL", 8);
                } else {
                    let frames = be32(body);
                    if frames == 0 || frames > 65536 {
                        e.disp = Disposition::Malformed;
                        e.note("acTL num_frames is 0 or above 65536; the decode fails");
                    } else {
                        e.disp = md(MetadataKind::Animation);
                        e.win = Some(Slot::Actl);
                        self.actl_frames = Some(frames);
                    }
                }
            }
            b"pHYs" => {
                if body.len() == 9 {
                    e.disp = md(MetadataKind::Resolution);
                    e.win = Some(Slot::Phys);
                } else {
                    wrong_len(e, "pHYs", 9);
                }
            }
            b"bKGD" => {
                let ok = if self.palette_len.is_some() {
                    body.len() == 1
                } else {
                    body.len() == 2 || body.len() == 6
                };
                native(e, ok, "bKGD", "PngInfo::background");
            }
            b"tIME" => native(e, body.len() == 7, "tIME", "PngInfo::last_modified"),
            b"sBIT" => native(
                e,
                (1..=4).contains(&body.len()),
                "sBIT",
                "PngInfo::significant_bits",
            ),
            b"iDOT" => {
                e.disp = Disposition::Structure;
                e.note("Apple iDOT segment table: read only for parallel decode (RowDecoder::new)");
            }
            // Settled in `finish` (they depend on a valid acTL and the animation policy).
            b"fcTL" | b"fdAT" => {}
            b"IDAT" => {}
            _ => other(e, ty, "ignored by the decoder"),
        }
    }

    /// Returns true when the chunk is `IEND`.
    fn classify_late(
        &mut self,
        ty: &[u8; 4],
        body: &[u8],
        body_start: usize,
        e: &mut Entry,
    ) -> bool {
        match ty {
            b"IEND" => {
                e.disp = Disposition::Structure;
                // finish_metadata breaks on the type and never reads an IEND payload.
                e.used_prefix(
                    body_start,
                    0,
                    body.len(),
                    "IEND payload: never read (finish_metadata stops at the chunk type)",
                );
                return true;
            }
            b"IDAT" => e.note(
                "IDAT after a non-IDAT chunk: the decoded stream ends at the first gap (IdatSource)",
            ),
            b"eXIf" => {
                if self.slot_taken(Slot::Exif) {
                    e.note("an earlier eXIf wins (collect_late only fills an empty slot)");
                } else {
                    e.disp = md(MetadataKind::Exif);
                    e.win = Some(Slot::Exif);
                    e.note("after IDAT");
                }
            }
            b"iTXt" => self.itxt(body, body_start, e, true),
            b"tEXt" => self.text(body, e),
            b"zTXt" => self.ztxt(body, e),
            b"tIME" => {
                // collect_late keeps only an unset slot; either way it is native-only.
                native(e, body.len() == 7, "tIME", "PngInfo::last_modified");
            }
            b"fcTL" | b"fdAT" => {}
            _ => {
                if registered_unread(ty) {
                    e.note("after IDAT: collect_late does not read this chunk type");
                } else {
                    other(e, ty, "ignored by the decoder");
                }
            }
        }
        false
    }

    fn iccp(&mut self, body: &[u8], body_start: usize, e: &mut Entry) {
        let Some(nul) = body.iter().position(|&b| b == 0) else {
            e.disp = Disposition::Malformed;
            e.note(format!(
                "iCCP has no profile-name terminator; profile silently dropped{ICCP_BAD}"
            ));
            return;
        };
        if nul <= MAX_KEYWORD {
            e.label = Some(latin1(&body[..nul]));
        } else {
            e.hole(
                body_start,
                body_start + nul,
                "profile name longer than 79 bytes: parse_iccp discards the name",
            );
        }
        if nul + 2 > body.len() {
            e.disp = Disposition::Malformed;
            e.note(format!(
                "iCCP ends after the profile name; profile silently dropped{ICCP_BAD}"
            ));
            return;
        }
        if body[nul + 1] != 0 {
            e.disp = Disposition::Malformed;
            e.note(format!(
                "iCCP compression method {}; profile silently dropped{ICCP_BAD}",
                body[nul + 1]
            ));
            return;
        }
        let compressed = &body[nul + 2..];
        if compressed.is_empty() {
            e.disp = Disposition::Dropped;
            e.note("iCCP has no profile data");
            return;
        }
        match self.inflate(compressed, ICC_INFLATE_CAP) {
            Ok((n, consumed)) => {
                e.disp = md(MetadataKind::Icc);
                e.win = Some(Slot::Iccp);
                // The decoder hands over the whole inflated stream; compare it with the size the
                // profile header declares (bytes 0..4).
                if n >= 4 {
                    let declared = be32(&self.scratch[..4]) as usize;
                    // One final stored block: inflated offsets map 1:1 onto the chunk.
                    let stored_len = (compressed.len() >= 7 && compressed[2] & 0x07 == 0x01)
                        .then(|| usize::from(u16::from_le_bytes([compressed[3], compressed[4]])));
                    if declared < n {
                        let why = format!(
                            "inflated profile is {n} bytes, header declares {declared}; the tail past the declared size reaches the caller in ImageInfo ICC"
                        );
                        if stored_len == Some(n) {
                            let data_at = body_start + nul + 2 + 7;
                            e.delivered_tail(data_at + declared, data_at + n, why);
                        } else {
                            e.note(why);
                        }
                    } else if declared > n {
                        e.note(format!(
                            "inflated profile is {n} bytes, header declares {declared}"
                        ));
                    }
                }
                let used = nul + 2 + consumed;
                e.used_prefix(
                    body_start,
                    used,
                    body.len(),
                    "bytes after the end of the profile's zlib stream: never read",
                );
            }
            Err(why) => {
                e.disp = Disposition::Malformed;
                e.note(format!(
                    "iCCP inflate failed ({why}); profile silently dropped (PngAncillary::collect ignores the error){ICCP_BAD}"
                ));
            }
        }
    }

    fn text(&mut self, body: &[u8], e: &mut Entry) {
        let Some((kw, _)) = Self::keyword(body) else {
            e.disp = Disposition::Malformed;
            e.note("tEXt without a 1..=79 byte NUL-terminated keyword; ignored");
            return;
        };
        e.label = Some(latin1(kw));
        let is_tool = kw == b"Software" || kw == b"Creator" || kw == b"Comment";
        let tool_wins = is_tool && !self.slot_taken(Slot::CreatingTool);
        if tool_wins {
            e.win = Some(Slot::CreatingTool);
        }
        let stored = self.text_count < MAX_TEXT_CHUNKS;
        if stored {
            self.text_count += 1;
        }
        e.disp = Disposition::Skipped;
        match (tool_wins, stored) {
            (true, true) => {
                e.disp = md(MetadataKind::Supplement);
                e.note("first Software/Creator/Comment: PngProbe::creating_tool via DecodeOutput::source_encoding_details; also native PngInfo::text_chunks");
            }
            (true, false) => {
                e.disp = md(MetadataKind::Supplement);
                e.note(format!("beyond the {MAX_TEXT_CHUNKS}-chunk text cap, but first Software/Creator/Comment: kept as PngProbe::creating_tool only"));
            }
            (false, true) => e.note("native PngInfo::text_chunks only"),
            (false, false) => e.note(format!(
                "beyond the {MAX_TEXT_CHUNKS}-chunk text cap: not stored"
            )),
        }
    }

    fn ztxt(&mut self, body: &[u8], e: &mut Entry) {
        let Some((kw, nul)) = Self::keyword(body) else {
            e.disp = Disposition::Malformed;
            e.note("zTXt without a 1..=79 byte NUL-terminated keyword; ignored");
            return;
        };
        e.label = Some(latin1(kw));
        if nul + 1 >= body.len() {
            e.disp = Disposition::Malformed;
            e.note("zTXt ends after the keyword; ignored");
            return;
        }
        if body[nul + 1] != 0 {
            e.disp = Disposition::Malformed;
            e.note(format!(
                "zTXt compression method {}; ignored",
                body[nul + 1]
            ));
            return;
        }
        let compressed = &body[nul + 2..];
        if compressed.is_empty() {
            e.disp = Disposition::Malformed;
            e.note("zTXt has no compressed text; ignored");
            return;
        }
        if self.text_count >= MAX_TEXT_CHUNKS {
            e.note(format!(
                "beyond the {MAX_TEXT_CHUNKS}-chunk text cap: not inflated, not stored"
            ));
            return;
        }
        let consumed = self.text_compressed.saturating_add(compressed.len() as u64);
        if consumed > MAX_TEXT_COMPRESSED_BYTES {
            e.note("beyond the 4 MiB compressed-text cap: not inflated, not stored");
            return;
        }
        self.text_compressed = consumed;
        match self.inflate(compressed, ZTXT_INFLATE_CAP) {
            Ok(_) => {
                self.text_count += 1;
                e.note("native PngInfo::text_chunks only");
            }
            Err(why) => {
                e.disp = Disposition::Malformed;
                e.note(format!("zTXt inflate failed ({why}); ignored"));
            }
        }
    }

    fn itxt(&mut self, body: &[u8], body_start: usize, e: &mut Entry, late: bool) {
        // Keyword (Latin-1, NUL-terminated).
        let nul = body.iter().take(MAX_KEYWORD + 1).position(|&b| b == 0);
        let kw = nul.map(|n| &body[..n]);
        if let Some(kw) = kw {
            e.label = Some(latin1(kw));
        }
        // Language tag, for the detail line.
        if let Some(n) = nul
            && body.len() >= n + 3
        {
            let rest = &body[n + 3..];
            if let Some(l) = rest.iter().position(|&b| b == 0) {
                e.note(format!("lang={}", latin1(&rest[..l.min(MAX_KEYWORD)])));
            }
        }

        const XMP: &[u8] = b"XML:com.adobe.xmp";
        // The creating-tool extractor splits at the first NUL anywhere in the chunk
        // and needs valid UTF-8 for both the keyword and the text.
        if kw == Some(XMP) {
            self.itxt_xmp(body, body_start, e, late);
        } else if matches!(kw, Some(b"Software") | Some(b"Creator")) {
            self.itxt_tool(body, body_start, e);
        } else if kw.is_none() {
            e.disp = Disposition::Skipped;
            e.note("iTXt without a terminated keyword; the decoder reads only XMP and Software/Creator");
        } else {
            e.note("iTXt keyword is not XML:com.adobe.xmp or Software/Creator: the decoder does not read it");
        }
    }

    fn itxt_tool(&mut self, body: &[u8], body_start: usize, e: &mut Entry) {
        // try_extract_creating_tool_itxt: first match wins, text must be UTF-8.
        e.note("native-only unless it is the first creating tool");
        if self.slot_taken(Slot::CreatingTool) {
            e.note("an earlier text chunk already supplied creating_tool");
            return;
        }
        let nul = body.iter().position(|&b| b == 0).unwrap_or(0);
        let rest = &body[nul + 1..];
        if rest.len() < 2 {
            e.disp = Disposition::Malformed;
            e.note("iTXt too short; ignored");
            return;
        }
        let after = &rest[2..];
        let text = after.iter().position(|&b| b == 0).and_then(|p1| {
            let after_lang = &after[p1 + 1..];
            after_lang
                .iter()
                .position(|&b| b == 0)
                .map(|p2| &after_lang[p2 + 1..])
        });
        match text {
            Some(t) if core::str::from_utf8(t).is_ok() => {
                let text_off = body.len() - t.len();
                if text_off > nul + 5 {
                    e.hole(
                        body_start + nul + 3,
                        body_start + text_off,
                        "iTXt language tag and translated keyword: not read",
                    );
                }
                e.disp = md(MetadataKind::Supplement);
                e.note("PngProbe::creating_tool via DecodeOutput::source_encoding_details");
                e.win = Some(Slot::CreatingTool);
            }
            _ => {
                e.disp = Disposition::Malformed;
                e.note("iTXt text is missing or not UTF-8; creating_tool ignores it");
            }
        }
    }

    fn itxt_xmp(&mut self, body: &[u8], body_start: usize, e: &mut Entry, late: bool) {
        const KW: usize = 17;
        e.disp = Disposition::Malformed;
        if late && self.slot_taken(Slot::Xmp) {
            e.disp = Disposition::Skipped;
            e.note("an earlier XMP wins (collect_late only fills an empty slot)");
            return;
        }
        if body.len() <= KW + 1 {
            e.note("XMP iTXt has no payload; ignored");
            return;
        }
        let rest = &body[KW + 1..];
        if rest.len() < 2 {
            e.note("XMP iTXt ends after the keyword; ignored");
            return;
        }
        let flag = rest[0];
        let rest = &rest[2..];
        let Some(l) = rest.iter().position(|&b| b == 0) else {
            e.note("XMP iTXt language tag is not terminated; ignored");
            return;
        };
        let rest = &rest[l + 1..];
        let Some(t) = rest.iter().position(|&b| b == 0) else {
            e.note("XMP iTXt translated keyword is not terminated; ignored");
            return;
        };
        let text = &rest[t + 1..];
        // Language tag and translated keyword (between the method byte and the text) are
        // skipped by `try_parse_xmp`; they can be any length.
        let text_off = body.len() - text.len();
        if text_off > KW + 5 {
            e.hole(
                body_start + KW + 3,
                body_start + text_off,
                "iTXt language tag and translated keyword: not read",
            );
        }
        match flag {
            0 if !text.is_empty() => {
                e.disp = md(MetadataKind::Xmp);
                e.win = Some(Slot::Xmp);
                if let Some(end) = xpacket_end(text)
                    && end < text.len()
                {
                    e.delivered_tail(
                        body_start + text_off + end,
                        body_start + body.len(),
                        format!(
                            "{} bytes after the <?xpacket end?> trailer: past the packet's end, but they reach the caller in ImageInfo XMP",
                            text.len() - end
                        ),
                    );
                }
            }
            0 => e.note("XMP iTXt text is empty; ignored"),
            1 => {
                let consumed = self.text_compressed.saturating_add(text.len() as u64);
                if consumed > MAX_TEXT_COMPRESSED_BYTES {
                    e.disp = Disposition::Skipped;
                    e.note("beyond the 4 MiB compressed-text cap: XMP not inflated");
                    return;
                }
                self.text_compressed = consumed;
                match self.inflate(text, XMP_INFLATE_CAP) {
                    Ok((n, consumed)) if n > 0 => {
                        e.disp = md(MetadataKind::Xmp);
                        e.win = Some(Slot::Xmp);
                        if let Some(end) = xpacket_end(&self.scratch[..n])
                            && end < n
                        {
                            e.note(format!(
                                "inflated XMP has {} bytes after the <?xpacket end?> trailer; past the packet's end, but they reach the caller (compressed, so not split)",
                                n - end
                            ));
                        }
                        e.used_prefix(
                            body_start,
                            body.len() - text.len() + consumed,
                            body.len(),
                            "bytes after the end of the XMP zlib stream: never read",
                        );
                    }
                    Ok(_) => e.note("XMP inflates to nothing; ignored"),
                    Err(why) => e.note(format!("XMP inflate failed ({why}); ignored")),
                }
            }
            f => e.note(format!("XMP iTXt compression flag {f}; ignored")),
        }
    }

    /// Settle what only the whole walk decides: animation chunks, then policy effects.
    fn finish(&mut self) -> Result<(), PngError> {
        let animated = self.slot_taken(Slot::Actl);
        let anim = core::mem::take(&mut self.anim);
        // `ApngDecoder::new` keeps the last pre-IDAT fcTL as frame 0's control chunk.
        let last_pre_fctl = anim
            .iter()
            .rposition(|&(_, is_fctl, pre, _)| is_fctl && pre);
        for (i, &(id, is_fctl, pre, len)) in anim.iter().enumerate() {
            if !animated {
                self.demote(
                    id,
                    Disposition::Skipped,
                    "fcTL/fdAT without a valid acTL: not read",
                );
            } else if self.opts.no_animation {
                self.demote(
                    id,
                    Disposition::Skipped,
                    "decode policy forbids animation (animation_frame_decoder is rejected)",
                );
            } else if is_fctl {
                if len != 26 {
                    self.demote(
                        id,
                        Disposition::Malformed,
                        &format!("fcTL is {len} bytes, expected 26"),
                    );
                } else if pre && Some(i) != last_pre_fctl {
                    self.demote(
                        id,
                        Disposition::Skipped,
                        "superseded by a later pre-IDAT fcTL (the last one describes frame 0)",
                    );
                } else {
                    self.demote(id, md(MetadataKind::Animation), "");
                }
            } else if pre {
                self.demote(
                    id,
                    Disposition::Skipped,
                    "fdAT before the first IDAT: ApngDecoder::new passes it to collect, which ignores it",
                );
            } else {
                self.demote(
                    id,
                    Disposition::ImageData,
                    "animation frame data; sequence numbers are not checked",
                );
            }
        }
        for (slot, drop, what) in [
            (Slot::Iccp, self.opts.drop_icc, "ICC"),
            (Slot::Xmp, self.opts.drop_xmp, "XMP"),
        ] {
            if let (true, Some(id)) = (drop, self.winners[slot as usize]) {
                self.demote(
                    id,
                    Disposition::Dropped,
                    &format!("decode policy suppresses {what} (apply_policy_to_info)"),
                );
            }
        }
        if let (true, Some(id)) = (self.opts.drop_exif, self.winners[Slot::Exif as usize]) {
            // apply_policy_to_info clears the EXIF blob, but convert_info has already set the
            // Orientation from it and nothing clears that.
            let range = self.inv.get(id).map(|p| p.range.clone());
            let orientation = range.and_then(|r| {
                let body = self.data.get(r.start as usize + 8..r.end as usize - 4)?;
                zencodec::helpers::parse_exif_orientation(body)
            });
            match orientation {
                Some(o) if o != zencodec::Orientation::Identity => self.demote(
                    id,
                    md(MetadataKind::Orientation),
                    "decode policy suppresses the EXIF blob, but its Orientation still reaches ImageInfo",
                ),
                _ => self.demote(
                    id,
                    Disposition::Dropped,
                    "decode policy suppresses EXIF (apply_policy_to_info)",
                ),
            }
        }
        Ok(())
    }
}

fn native(e: &mut Entry, ok: bool, name: &str, field: &str) {
    if ok {
        e.disp = Disposition::Skipped;
        e.note(if matches!(name, "gAMA" | "cHRM" | "sRGB") {
            format!(
                "native {field} only; not read by the zencodec decode, but \
                 zencodecs::cms::png_srgb_transform_icc (zenpipe) re-parses it for colour conversion"
            )
        } else {
            format!("native {field} only; not in zencodec ImageInfo")
        });
    } else {
        e.disp = Disposition::Malformed;
        e.note(format!("{name} has an unexpected length; ignored"));
    }
}

fn wrong_len(e: &mut Entry, name: &str, want: usize) {
    e.disp = Disposition::Malformed;
    e.note(format!("{name} must be {want} bytes; ignored"));
}

fn other(e: &mut Entry, ty: &[u8; 4], why: &str) {
    if registered_unread(ty) {
        e.disp = Disposition::Skipped;
        e.note(format!("recognised PNG chunk type, {why}"));
    } else {
        e.disp = Disposition::Unknown;
        e.note(why.to_string());
    }
}
