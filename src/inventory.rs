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
/// Palette bytes the decoder can use (256 entries).
const MAX_PLTE_BYTES: usize = 768;

/// One chunk's classification, built per chunk and pushed by [`Walker::commit`].
struct Entry {
    start: u64,
    end: u64,
    tag: PartTag,
    disp: Disposition,
    label: Option<String>,
    detail: Vec<String>,
    /// Absolute offset where the decoder stops reading the chunk's data; bytes from
    /// here to the CRC are split off as `Unreferenced` while the chunk is consumed.
    tail: Option<u64>,
    tail_note: &'static str,
    /// Singleton slot this chunk takes (replacing, and demoting, the previous holder).
    win: Option<Slot>,
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
            tail: None,
            tail_note: "",
            win: None,
        }
    }

    /// The decoder reads only `used` data bytes of this chunk (data starts at `data_start`).
    fn used_prefix(&mut self, data_start: usize, used: usize, data_len: usize, note: &'static str) {
        if used < data_len {
            self.tail = Some((data_start + used) as u64);
            self.tail_note = note;
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
    start: usize,
    data_start: usize,
    data_end: usize,
    end: usize,
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
    /// fcTL / fdAT parts settled at the end: (id, is_fctl, before_first_idat, data_len).
    anim: Vec<(PartId, bool, bool, u64)>,
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
    };
    w.run()?;
    w.finish()?;
    let mut inv = w.inv;
    inv.fill_gaps(None, Disposition::Trailing)
        .map_err(limit_err)?;
    Ok(inv)
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
    /// the chunk is consumed (`Entry::tail`).
    fn commit(&mut self, e: Entry) -> Result<PartId, PngError> {
        let split = e.disp.is_consumed().then_some(e.tail).flatten();
        let mut part = Part::new(PartKind::Chunk, e.tag.clone(), e.start..e.end, e.disp);
        if let Some(l) = e.label {
            part = part.with_label(Cow::Owned(l));
        }
        if !e.detail.is_empty() {
            part = part.with_detail(e.detail.join("; "));
        }
        if split.is_some() {
            part = part.with_body(e.start..e.end);
        }
        let id = self.inv.push(None, part).map_err(limit_err)?;
        if let Some(tail) = split {
            let data_end = e.end - 4;
            let head = Part::new(PartKind::Field, PartTag::None, e.start..tail, e.disp)
                .with_detail("bytes the decoder reads");
            let hole = Part::new(
                PartKind::Gap,
                PartTag::None,
                tail..data_end,
                Disposition::Unreferenced,
            )
            .with_detail(e.tail_note);
            let crc = Part::new(
                PartKind::Field,
                PartTag::None,
                data_end..e.end,
                Disposition::Structure,
            )
            .with_label("crc");
            let mut kids = Vec::new();
            for k in [head, hole, crc] {
                kids.push(self.inv.push(Some(id), k).map_err(limit_err)?);
            }
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
            if crc_bad {
                entry.note(if ancillary {
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
                        phase = Phase::Idat;
                        self.first_idat_pos = Some(pos);
                        self.idat_run.push(IdatChunk {
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
                            start: pos,
                            data_start: body_start,
                            data_end: crc_end - 4,
                            end: crc_end,
                        });
                        self.note_crc(pos, &entry);
                    }
                    Phase::Late => {
                        done = self.classify_late(&ty, body, body_start, &mut entry);
                        self.commit_late(&ty, entry)?;
                    }
                }
            }
            pos = crc_end;
            if done {
                break;
            }
        }
        self.end_idat_run()?;
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
    fn commit_pre(
        &mut self,
        ty: &[u8; 4],
        body_start: usize,
        data_len: usize,
        entry: Entry,
    ) -> Result<(), PngError> {
        let (start, end) = (entry.start as usize, entry.end as usize);
        let id = self.commit(entry)?;
        match ty {
            b"fcTL" => self.anim.push((id, true, true, data_len as u64)),
            b"fdAT" => self.anim.push((id, false, true, data_len as u64)),
            b"iDOT" => self.idot.push((id, start, body_start..end - 4)),
            _ => {}
        }
        Ok(())
    }

    fn commit_late(&mut self, ty: &[u8; 4], entry: Entry) -> Result<(), PngError> {
        let data_len = (entry.end - entry.start).saturating_sub(12);
        let id = self.commit(entry)?;
        match ty {
            b"fcTL" => self.anim.push((id, true, false, data_len)),
            b"fdAT" => self.anim.push((id, false, false, data_len)),
            _ => {}
        }
        Ok(())
    }

    /// Place the end of the IDAT zlib stream by inflating the run into a discard sink
    /// (what `drain_stream` does after the last row), then push the run's chunks:
    /// the one holding the stream end keeps the bytes after it as `Unreferenced`, and
    /// chunks entirely after it are never read.
    fn end_idat_run(&mut self) -> Result<(), PngError> {
        if self.idat_run.is_empty() {
            return Ok(());
        }
        let run = core::mem::take(&mut self.idat_run);
        let (end_abs, note) = self.zlib_end(&run)?;
        let notes = core::mem::take(&mut self.idat_notes);
        for c in &run {
            let mut e = Entry::new(
                c.start,
                c.end,
                PartTag::FourCc(*b"IDAT"),
                Disposition::ImageData,
            );
            if let Some(n) = notes.get(&c.start) {
                e.note(n.clone());
            }
            if let Some(end) = end_abs {
                if c.data_start >= end && c.data_end > c.data_start {
                    e.disp = Disposition::Skipped;
                    e.note("IDAT after the end of the zlib stream: never read (drain_stream stops at the stream end)");
                } else if c.data_end > end && end >= c.data_start {
                    e.used_prefix(
                        c.data_start,
                        end - c.data_start,
                        c.data_end - c.data_start,
                        "bytes after the end of the zlib stream: never read",
                    );
                }
            } else if let Some(n) = &note {
                e.note(n.clone());
            }
            self.commit(e)?;
        }
        self.settle_idot(&run)?;
        Ok(())
    }

    /// Inflate the first `limit` bytes of the run into a discard sink. `Some((position,
    /// adler32 of the output))` when the stream completes, `position` being how far the
    /// source was read (an upper bound for the stream's end: the inflater stages input).
    fn inflate_run(
        &mut self,
        run: &[IdatChunk],
        limit: usize,
    ) -> Result<Option<(usize, u32)>, PngError> {
        let src = RunSource {
            data: self.data,
            chunks: run,
            ci: 0,
            off: 0,
            left: limit,
        };
        let mut d = zenflate::StreamDecompressor::zlib(src, IDAT_WINDOW)
            .with_max_output_size(Some(IDAT_MAX_OUTPUT));
        let mut sum = zenflate::Adler32Hasher::new();
        let mut stalled = 0;
        loop {
            self.stop.check().map_err(PngError::from)?;
            if d.is_done() {
                break;
            }
            match d.fill() {
                Ok(out) => {
                    let n = out.len();
                    sum.write(out);
                    d.advance(n);
                    stalled = if n == 0 { stalled + 1 } else { 0 };
                    if stalled > 2 && !d.is_done() {
                        return Ok(None);
                    }
                }
                Err(_) => return Ok(None),
            }
        }
        let src = d.source_ref();
        let mut pos = 0;
        for c in run.iter().take(src.ci) {
            pos += c.data_end - c.data_start;
        }
        Ok(Some((pos + src.off, sum.finish())))
    }

    /// File offset just past the zlib stream's last byte, or `None` (with a remark)
    /// when the stream does not complete.
    ///
    /// The streaming inflater stages up to 32 KiB of input beyond what it uses, so a
    /// first pass only bounds the end. The end is then the shortest prefix of the run
    /// that still inflates: candidates are the positions in the last staging window
    /// whose preceding four bytes equal the Adler-32 of the output (one test, usually),
    /// with a binary search over the window when the file's checksum is wrong.
    fn zlib_end(&mut self, run: &[IdatChunk]) -> Result<(Option<usize>, Option<String>), PngError> {
        let total: usize = run.iter().map(|c| c.data_end - c.data_start).sum();
        let Some((hi, adler)) = self.inflate_run(run, total)? else {
            return Ok((
                None,
                Some(String::from(
                    "zlib stream does not complete; bytes after its end are not distinguished",
                )),
            ));
        };
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
        let lo = hi.saturating_sub(OVERREAD_WINDOW).max(2);
        let mut found = None;
        for l in lo..=hi {
            if l >= 4
                && u32::from_be_bytes([byte(l - 4), byte(l - 3), byte(l - 2), byte(l - 1)]) == adler
                && self.inflate_run(run, l)?.is_some()
            {
                found = Some(l);
                break;
            }
        }
        if found.is_none() {
            // Checksum mismatch in the file: binary search the shortest prefix that completes.
            let (mut a, mut b) = (lo, hi);
            while a < b {
                let mid = a + (b - a) / 2;
                if self.inflate_run(run, mid)?.is_some() {
                    b = mid;
                } else {
                    a = mid + 1;
                }
            }
            found = Some(b);
        }
        let l = found.unwrap_or(hi);
        // Concatenated offset -> file offset of the first byte not read.
        let mut left = l;
        for c in run {
            let len = c.data_end - c.data_start;
            if left <= len {
                return Ok((Some(c.data_start + left), None));
            }
            left -= len;
        }
        Ok((Some(run.last().map_or(0, |c| c.data_end)), None))
    }

    /// `iDOT` is read only when `idot::validate` accepts it (non-interlaced, one table).
    fn settle_idot(&mut self, _run: &[IdatChunk]) -> Result<(), PngError> {
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
                        e.used_prefix(
                            body_start,
                            MAX_PLTE_BYTES,
                            body.len(),
                            "palette entries past 256: unreachable by any index, never used",
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
                        let (used, why) = match (self.color_type, self.palette_len) {
                            (Some(0), _) => (2, "tRNS bytes past the gray sample: never read"),
                            (Some(2), _) => (6, "tRNS bytes past the RGB sample: never read"),
                            (_, Some(n)) if body.len() > n => (
                                0,
                                "tRNS longer than the palette: only its presence is kept, the bytes are discarded",
                            ),
                            _ => (body.len(), ""),
                        };
                        e.used_prefix(body_start, used, body.len(), why);
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
            Ok((_, consumed)) => {
                e.disp = md(MetadataKind::Icc);
                e.win = Some(Slot::Iccp);
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
            self.itxt_tool(body, e);
        } else if kw.is_none() {
            e.disp = Disposition::Skipped;
            e.note("iTXt without a terminated keyword; the decoder reads only XMP and Software/Creator");
        } else {
            e.note("iTXt keyword is not XML:com.adobe.xmp or Software/Creator: the decoder does not read it");
        }
    }

    fn itxt_tool(&mut self, body: &[u8], e: &mut Entry) {
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
        match flag {
            0 if !text.is_empty() => {
                e.disp = md(MetadataKind::Xmp);
                e.win = Some(Slot::Xmp);
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
