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
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use enough::{Stop, Unstoppable};
use zencodec::ImageFormat;
use zencodec::inventory::{Disposition, Inventory, MetadataKind, Part, PartKind, PartTag};
use zenflate::crc32;

use crate::chunk::PNG_SIGNATURE;
use crate::chunk::ancillary::{MAX_TEXT_CHUNKS, MAX_TEXT_COMPRESSED_BYTES};
use crate::error::PngError;

/// Longest keyword / profile name PNG allows (spec: 1-79 bytes).
const MAX_KEYWORD: usize = 79;
/// Output cap the decoder gives `iCCP` inflation (`PngAncillary::parse_iccp`).
const ICC_INFLATE_CAP: usize = 1024 * 1024;
/// Output cap for `zTXt` inflation (`parse_ztxt`).
const ZTXT_INFLATE_CAP: usize = 1024 * 1024;
/// Output cap for compressed XMP `iTXt` (`try_parse_xmp`).
const XMP_INFLATE_CAP: usize = 4 * 1024 * 1024;
/// Total inflate output the inventory will produce for classification. The
/// decoder inflates every `iCCP` without a global budget; the inventory bounds
/// its own work and says so in `detail` when the budget runs out.
const INFLATE_BUDGET: u64 = 256 * 1024 * 1024;

/// One chunk's worth of classification, collected before the `Inventory` is built
/// so later chunks can demote earlier winners (`Skipped` "superseded").
struct Entry {
    start: u64,
    end: u64,
    tag: PartTag,
    disp: Disposition,
    label: Option<String>,
    detail: Vec<String>,
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
    /// `DecodePolicy::strict`: critical and ancillary CRCs are verified, and an
    /// ancillary chunk with a bad CRC is skipped before the decoder sees it.
    pub strict_crc: bool,
    /// The policy suppresses ICC / EXIF / XMP in `ImageInfo` (`apply_policy_to_info`).
    pub drop_icc: bool,
    pub drop_exif: bool,
    pub drop_xmp: bool,
}

struct Walker<'a> {
    data: &'a [u8],
    stop: &'a dyn Stop,
    opts: Options,
    entries: Vec<Entry>,
    winners: [Option<usize>; SLOTS],
    color_type: Option<u8>,
    palette_present: bool,
    text_count: usize,
    text_compressed: u64,
    inflate_budget: u64,
    scratch: Vec<u8>,
    idot_entries: Vec<usize>,
    anim_entries: Vec<(usize, bool)>,
}

/// Walk `data` and return its inventory.
pub(crate) fn walk(data: &[u8], stop: &dyn Stop, opts: Options) -> Result<Inventory, PngError> {
    let len = data.len() as u64;
    let mut inv = Inventory::new(ImageFormat::Png, len);
    let push_err = |_| PngError::Decode("PNG inventory exceeds the part limit".into());

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
        .map_err(push_err)?;
        inv.fill_gaps(None, Disposition::Malformed)
            .map_err(push_err)?;
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
    .map_err(push_err)?;

    let mut w = Walker {
        data,
        stop,
        opts,
        entries: Vec::new(),
        winners: [None; SLOTS],
        color_type: None,
        palette_present: false,
        text_count: 0,
        text_compressed: 0,
        inflate_budget: INFLATE_BUDGET,
        scratch: Vec::new(),
        idot_entries: Vec::new(),
        anim_entries: Vec::new(),
    };
    w.run()?;
    w.finish();

    for e in w.entries {
        let mut part = Part::new(PartKind::Chunk, e.tag, e.start..e.end, e.disp);
        if let Some(l) = e.label {
            part = part.with_label(Cow::Owned(l));
        }
        if !e.detail.is_empty() {
            part = part.with_detail(e.detail.join("; "));
        }
        inv.push(None, part).map_err(push_err)?;
    }
    inv.fill_gaps(None, Disposition::Trailing)
        .map_err(push_err)?;
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

impl Walker<'_> {
    fn slot_win(&mut self, slot: Slot, idx: usize, at: u64) {
        if let Some(prev) = self.winners[slot as usize].replace(idx) {
            let e = &mut self.entries[prev];
            e.disp = Disposition::Skipped;
            e.note(format!("superseded by a later chunk at offset {at}"));
        }
    }

    fn slot_taken(&self, slot: Slot) -> bool {
        self.winners[slot as usize].is_some()
    }

    fn inflate(&mut self, compressed: &[u8], cap: usize) -> Result<usize, String> {
        if self.inflate_budget < cap as u64 {
            return Err("inventory inflate budget exhausted".into());
        }
        if self.scratch.len() < cap {
            self.scratch.resize(cap, 0);
        }
        let mut d = zenflate::Decompressor::new();
        match d.zlib_decompress(compressed, &mut self.scratch[..cap], Unstoppable) {
            Ok(o) => {
                self.inflate_budget = self.inflate_budget.saturating_sub(o.output_written as u64);
                Ok(o.output_written)
            }
            Err(e) => {
                // A failed inflate may have written up to `cap`; charge a flat 64 KiB.
                self.inflate_budget = self.inflate_budget.saturating_sub(64 * 1024);
                Err(format!("{e:?}"))
            }
        }
    }

    fn run(&mut self) -> Result<(), PngError> {
        let data = self.data;
        let total = data.len();
        let mut pos = 8usize;
        let mut phase = Phase::Pre;
        let mut first = true;
        while pos < total {
            self.stop.check().map_err(PngError::from)?;
            // Bound memory before `Inventory::push` can: it enforces the same cap, but only
            // after the whole walk has been buffered here.
            if self.entries.len() >= zencodec::inventory::DEFAULT_MAX_PARTS as usize {
                return Err(PngError::Decode(
                    "PNG inventory exceeds the part limit".into(),
                ));
            }
            if pos + 12 > total {
                // Fewer bytes than the smallest chunk: the decoder's chunk readers all stop here.
                let mut e = Entry {
                    start: pos as u64,
                    end: total as u64,
                    tag: PartTag::None,
                    disp: Disposition::Malformed,
                    label: None,
                    detail: Vec::new(),
                };
                if pos + 8 <= total {
                    e.tag = PartTag::FourCc([
                        data[pos + 4],
                        data[pos + 5],
                        data[pos + 6],
                        data[pos + 7],
                    ]);
                }
                e.note(format!(
                    "truncated chunk header: {} bytes remain, a chunk needs at least 12",
                    total - pos
                ));
                self.entries.push(e);
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
                let mut e = Entry {
                    start: pos as u64,
                    end: total as u64,
                    tag: PartTag::FourCc(ty),
                    disp: Disposition::Malformed,
                    label: None,
                    detail: Vec::new(),
                };
                e.note(format!(
                    "chunk declares {length} data bytes but only {} remain",
                    total - body_start
                ));
                self.entries.push(e);
                break;
            };
            let body = &data[body_start..crc_end - 4];
            let stored = be32(&data[crc_end - 4..]);
            let mut entry = Entry {
                start: pos as u64,
                end: crc_end as u64,
                tag: PartTag::FourCc(ty),
                disp: Disposition::Skipped,
                label: None,
                detail: Vec::new(),
            };
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

            let idx = self.entries.len();
            let is_ihdr = first;
            first = false;

            // Phase transitions mirror RowDecoder::new / IdatSource / finish_metadata.
            if phase == Phase::Idat && &ty != b"IDAT" {
                phase = Phase::Late;
            }
            let mut done = false;
            if crc_bad && ancillary && self.opts.strict_crc {
                // ChunkIter drops the chunk before PngAncillary sees it.
                entry.disp = Disposition::Dropped;
                entry.note(
                    "strict policy: bad-CRC ancillary chunk skipped silently (no PngWarning)",
                );
            } else if is_ihdr {
                self.classify_first(&ty, body, &mut entry);
            } else {
                match phase {
                    Phase::Pre => {
                        if &ty == b"IDAT" {
                            entry.disp = Disposition::ImageData;
                            phase = Phase::Idat;
                        } else {
                            self.classify_pre(idx, &ty, body, &mut entry);
                        }
                    }
                    Phase::Idat => {
                        entry.disp = Disposition::ImageData;
                    }
                    Phase::Late => {
                        done = self.classify_late(idx, &ty, body, &mut entry);
                    }
                }
            }
            self.entries.push(entry);
            pos = crc_end;
            if done {
                break;
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
        e.disp = Disposition::Structure;
    }

    /// Keyword before the first NUL, only when it is a legal 1..=79 byte keyword.
    fn keyword(body: &[u8]) -> Option<(&[u8], usize)> {
        let nul = body.iter().take(MAX_KEYWORD + 1).position(|&b| b == 0)?;
        (nul >= 1).then(|| (&body[..nul], nul))
    }

    fn classify_pre(&mut self, idx: usize, ty: &[u8; 4], body: &[u8], e: &mut Entry) {
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
                    self.palette_present = true;
                    if self.color_type == Some(3) {
                        e.disp = Disposition::ImageData;
                        e.note("palette");
                    } else {
                        e.disp = Disposition::Dropped;
                        e.note("PLTE in a non-indexed image: stored but not used for pixels");
                    }
                    self.slot_win(Slot::Plte, idx, e.start);
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
                    } else {
                        e.disp = Disposition::Dropped;
                        e.note("tRNS with an alpha colour type: not used");
                    }
                    self.slot_win(Slot::Trns, idx, e.start);
                }
            }
            b"iCCP" => self.iccp(idx, body, e),
            b"gAMA" => native(e, body.len() == 4, "gAMA", "PngInfo::gamma"),
            b"sRGB" => native(e, !body.is_empty(), "sRGB", "PngInfo::srgb_intent"),
            b"cHRM" => native(e, body.len() == 32, "cHRM", "PngInfo::chrm"),
            b"cICP" => {
                if body.len() == 4 {
                    e.disp = md(MetadataKind::Cicp);
                    self.slot_win(Slot::Cicp, idx, e.start);
                } else {
                    wrong_len(e, "cICP", 4);
                }
            }
            b"cLLI" => {
                if body.len() == 8 {
                    e.disp = md(MetadataKind::HdrStatic);
                    self.slot_win(Slot::Clli, idx, e.start);
                } else {
                    wrong_len(e, "cLLI", 8);
                }
            }
            b"mDCV" => {
                if body.len() == 24 {
                    e.disp = md(MetadataKind::HdrStatic);
                    self.slot_win(Slot::Mdcv, idx, e.start);
                } else {
                    wrong_len(e, "mDCV", 24);
                }
            }
            b"eXIf" => {
                e.disp = md(MetadataKind::Exif);
                self.slot_win(Slot::Exif, idx, e.start);
            }
            b"iTXt" => self.itxt(idx, body, e, false),
            b"tEXt" => self.text(idx, body, e),
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
                        self.slot_win(Slot::Actl, idx, e.start);
                    }
                }
            }
            b"pHYs" => {
                if body.len() == 9 {
                    e.disp = md(MetadataKind::Resolution);
                    self.slot_win(Slot::Phys, idx, e.start);
                } else {
                    wrong_len(e, "pHYs", 9);
                }
            }
            b"bKGD" => {
                let ok = if self.palette_present {
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
                self.idot_entries.push(idx);
            }
            b"fcTL" => {
                self.anim_entries.push((idx, true));
            }
            b"fdAT" => {
                self.anim_entries.push((idx, false));
            }
            b"IDAT" => {}
            _ => other(e, ty, "ignored by the decoder"),
        }
    }

    /// Returns true when the chunk is `IEND`.
    fn classify_late(&mut self, idx: usize, ty: &[u8; 4], body: &[u8], e: &mut Entry) -> bool {
        match ty {
            b"IEND" => {
                e.disp = Disposition::Structure;
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
                    self.slot_win(Slot::Exif, idx, e.start);
                    e.note("after IDAT");
                }
            }
            b"iTXt" => self.itxt(idx, body, e, true),
            b"tEXt" => self.text(idx, body, e),
            b"zTXt" => self.ztxt(body, e),
            b"tIME" => {
                // collect_late keeps only an unset slot; either way it is native-only.
                native(e, body.len() == 7, "tIME", "PngInfo::last_modified");
            }
            b"fcTL" => self.anim_entries.push((idx, true)),
            b"fdAT" => self.anim_entries.push((idx, false)),
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

    fn iccp(&mut self, idx: usize, body: &[u8], e: &mut Entry) {
        let Some(nul) = body.iter().position(|&b| b == 0) else {
            e.disp = Disposition::Malformed;
            e.note("iCCP has no profile-name terminator; profile silently dropped");
            return;
        };
        if nul <= MAX_KEYWORD {
            e.label = Some(latin1(&body[..nul]));
        }
        if nul + 2 > body.len() {
            e.disp = Disposition::Malformed;
            e.note("iCCP ends after the profile name; profile silently dropped");
            return;
        }
        if body[nul + 1] != 0 {
            e.disp = Disposition::Malformed;
            e.note(format!(
                "iCCP compression method {}; profile silently dropped",
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
            Ok(_) => {
                e.disp = md(MetadataKind::Icc);
                self.slot_win(Slot::Iccp, idx, e.start);
            }
            Err(why) if why.contains("budget") => {
                e.disp = md(MetadataKind::Icc);
                e.note(why);
                self.slot_win(Slot::Iccp, idx, e.start);
            }
            Err(why) => {
                e.disp = Disposition::Malformed;
                e.note(format!(
                    "iCCP inflate failed ({why}); profile silently dropped (PngAncillary::collect ignores the error)"
                ));
            }
        }
    }

    fn text(&mut self, idx: usize, body: &[u8], e: &mut Entry) {
        let Some((kw, _)) = Self::keyword(body) else {
            e.disp = Disposition::Malformed;
            e.note("tEXt without a 1..=79 byte NUL-terminated keyword; ignored");
            return;
        };
        e.label = Some(latin1(kw));
        let is_tool = kw == b"Software" || kw == b"Creator" || kw == b"Comment";
        let tool_wins = is_tool && !self.slot_taken(Slot::CreatingTool);
        if tool_wins {
            self.slot_win(Slot::CreatingTool, idx, e.start);
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

    fn itxt(&mut self, idx: usize, body: &[u8], e: &mut Entry, late: bool) {
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
            self.itxt_xmp(idx, body, e, late);
        } else if matches!(kw, Some(b"Software") | Some(b"Creator")) {
            self.itxt_tool(idx, body, e);
        } else if kw.is_none() {
            e.disp = Disposition::Skipped;
            e.note("iTXt without a terminated keyword; the decoder reads only XMP and Software/Creator");
        } else {
            e.note("iTXt keyword is not XML:com.adobe.xmp or Software/Creator: the decoder does not read it");
        }
    }

    fn itxt_tool(&mut self, idx: usize, body: &[u8], e: &mut Entry) {
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
                self.slot_win(Slot::CreatingTool, idx, e.start);
            }
            _ => {
                e.disp = Disposition::Malformed;
                e.note("iTXt text is missing or not UTF-8; creating_tool ignores it");
            }
        }
    }

    fn itxt_xmp(&mut self, idx: usize, body: &[u8], e: &mut Entry, late: bool) {
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
                self.slot_win(Slot::Xmp, idx, e.start);
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
                    Ok(n) if n > 0 => {
                        e.disp = md(MetadataKind::Xmp);
                        self.slot_win(Slot::Xmp, idx, e.start);
                    }
                    Ok(_) => e.note("XMP inflates to nothing; ignored"),
                    Err(why) => e.note(format!("XMP inflate failed ({why}); ignored")),
                }
            }
            f => e.note(format!("XMP iTXt compression flag {f}; ignored")),
        }
    }

    /// Settle animation chunks and iDOT once the whole walk is known.
    fn finish(&mut self) {
        for (slot, drop, what) in [
            (Slot::Iccp, self.opts.drop_icc, "ICC"),
            (Slot::Exif, self.opts.drop_exif, "EXIF"),
            (Slot::Xmp, self.opts.drop_xmp, "XMP"),
        ] {
            if let (true, Some(idx)) = (drop, self.winners[slot as usize]) {
                let e = &mut self.entries[idx];
                e.disp = Disposition::Dropped;
                e.note(format!(
                    "decode policy suppresses {what} (apply_policy_to_info)"
                ));
            }
        }
        let animated = self.slot_taken(Slot::Actl);
        for &(idx, is_fctl) in &self.anim_entries {
            let e = &mut self.entries[idx];
            let len = e.end - e.start - 12;
            if !animated {
                e.disp = Disposition::Skipped;
                e.note("fcTL/fdAT without a valid acTL: not read");
            } else if is_fctl {
                if len == 26 {
                    e.disp = md(MetadataKind::Animation);
                } else {
                    e.disp = Disposition::Malformed;
                    e.note(format!("fcTL is {len} bytes, expected 26"));
                }
            } else {
                e.disp = Disposition::ImageData;
                e.note("animation frame data; sequence numbers are not checked");
            }
        }
        if self.idot_entries.len() > 1 {
            for &idx in &self.idot_entries {
                let e = &mut self.entries[idx];
                e.disp = Disposition::Skipped;
                e.note("more than one iDOT makes the table ambiguous: dropped");
            }
        }
    }
}

fn native(e: &mut Entry, ok: bool, name: &str, field: &str) {
    if ok {
        e.disp = Disposition::Skipped;
        e.note(format!("native {field} only; not in zencodec ImageInfo"));
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
