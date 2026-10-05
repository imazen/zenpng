//! Post-processing: raw unfiltered rows → output pixels.

use alloc::vec::Vec;

use crate::chunk::ancillary::PngAncillary;
use crate::chunk::ihdr::Ihdr;
use crate::error::PngError;
#[allow(unused_imports)]
use whereat::at;

use imgref::ImgVec;
use rgb::{Gray, Rgb, Rgba};
use zenpixels::{GrayAlpha16, Pixel, PixelBuffer};

/// Reinterpret `Vec<u8>` as `Vec<T>` without copying when possible.
/// Falls back to per-element construction only if alignment prevents zero-copy.
pub(crate) fn bytes_to_rgba16_vec(bytes: &[u8]) -> Vec<Rgba<u16>> {
    bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|c| Rgba {
            r: u16::from_ne_bytes([c[0], c[1]]),
            g: u16::from_ne_bytes([c[2], c[3]]),
            b: u16::from_ne_bytes([c[4], c[5]]),
            a: u16::from_ne_bytes([c[6], c[7]]),
        })
        .collect()
}

fn try_cast_vec_or<T: bytemuck::AnyBitPattern + bytemuck::NoUninit>(
    pixels: Vec<u8>,
    fallback: fn(&[u8]) -> Vec<T>,
) -> Vec<T> {
    match bytemuck::try_cast_vec(pixels) {
        Ok(v) => v,
        Err((bytemuck::PodCastError::AlignmentMismatch, bytes)) => fallback(&bytes),
        Err((e, _)) => panic!("unexpected cast error: {e:?}"),
    }
}

// ── Post-processing ─────────────────────────────────────────────────

/// Compute output bytes per pixel after post-processing (for limits checks).
pub(crate) fn output_bytes_per_pixel(ihdr: &Ihdr, ancillary: &PngAncillary) -> usize {
    match ihdr.color_type {
        0 => {
            // Grayscale
            if ancillary.trns.is_some() {
                // Gray + tRNS → RGBA8 (for 8-bit) or GrayAlpha16 (4 bytes either way)
                4
            } else if ihdr.bit_depth == 16 {
                2
            } else {
                1
            }
        }
        2 => {
            // RGB
            if ancillary.trns.is_some() {
                if ihdr.bit_depth == 16 { 8 } else { 4 }
            } else if ihdr.bit_depth == 16 {
                6
            } else {
                3
            }
        }
        3 => {
            // Indexed → RGB8 or RGBA8
            if ancillary.trns.is_some() { 4 } else { 3 }
        }
        4 => {
            // GrayAlpha: GA8 → RGBA8 (4 bytes), GA16 → GrayAlpha16 (4 bytes)
            4
        }
        // RGBA
        6 if ihdr.bit_depth == 16 => 8,
        6 => 4,
        _ => 4,
    }
}

/// Scale sub-8-bit gray value to 8-bit.
pub(crate) fn scale_to_8bit(value: u8, bit_depth: u8) -> u8 {
    match bit_depth {
        1 => {
            if value != 0 {
                255
            } else {
                0
            }
        }
        2 => value * 85, // 0→0, 1→85, 2→170, 3→255
        4 => value * 17, // 0→0, 1→17, ..., 15→255
        _ => value,
    }
}

/// Post-process a raw unfiltered row into output pixels.
/// Returns the output pixel data for this row.
///
/// # Invariant
///
/// `raw` must contain at least `ceil(ihdr.width × ihdr.channels × ihdr.bit_depth / 8)`
/// bytes — the unfiltered scanline length the row decoder produces. The
/// in-tree callers (`RowDecoder` and the interlace pass loop) always supply
/// a row sized this way. Sub-byte unpacking guards against shorter input
/// with an early break, but the truecolor / palette-byte branches index
/// `raw` directly and will panic on a too-short row. Treat that as an
/// internal-API contract: violating it is a refactor bug, not an attacker
/// surface, since untrusted input flows in via `decode_png` which sizes
/// `raw` from the row decoder.
#[cfg(test)]
pub(crate) fn post_process_row(
    raw: &[u8],
    ihdr: &Ihdr,
    ancillary: &PngAncillary,
    out: &mut Vec<u8>,
) {
    // One-off convenience for callers that process a single row; hot loops
    // build a `RowExpander` once and reuse it.
    let Ok(e) = RowExpander::new(ihdr, ancillary) else {
        out.clear();
        return;
    };
    out.clear();
    out.resize(e.out_row_bytes(), 0);
    e.expand(raw, out);
}

// ── RowExpander: one raw row → one output row, written in place ──────

/// How one raw (unfiltered) scanline maps to one output row.
///
/// Built once per image from the IHDR and ancillary chunks, then applied to
/// every row with [`RowExpander::expand`], which writes straight into the
/// caller's output slice. Output formats match [`post_process_row`] exactly:
/// palette → RGB8/RGBA8, sub-byte gray → Gray8 (RGBA8 with tRNS), 16-bit →
/// native-endian, GrayAlpha8 → RGBA8, Gray8/RGB8/RGB16 + tRNS → +alpha.
pub(crate) struct RowExpander {
    kind: Expand,
    width: usize,
    out_row_bytes: usize,
}

enum Expand {
    /// Output bytes == raw bytes (Gray8, RGB8, RGBA8 without tRNS).
    Copy,
    /// Big-endian 16-bit samples → native endian, same channel count.
    Swap16,
    /// Sub-byte gray without tRNS → Gray8, one table lookup per input byte.
    PackedGray {
        bits: u8,
        table: alloc::boxed::Box<[[u8; 8]; 256]>,
    },
    /// Sub-byte gray with tRNS → RGBA8 (rare; per sample).
    SubByteGrayTrns {
        bits: u8,
        trns: u8,
    },
    Gray8Trns(u8),
    Gray16Trns(u16),
    Rgb8Trns([u8; 3]),
    Rgb16Trns([u16; 3]),
    GrayAlpha8,
    /// Palette indices (1/2/4/8-bit) through a 256-entry RGBA table;
    /// `alpha` selects RGBA8 output (tRNS present) over RGB8. Sub-byte
    /// indices are unpacked one input byte at a time through `unpack`.
    Palette {
        bits: u8,
        alpha: bool,
        lut: alloc::boxed::Box<[[u8; 4]; 256]>,
        unpack: Option<alloc::boxed::Box<[[u8; 8]; 256]>>,
    },
}

fn trns_u16(trns: &[u8], i: usize) -> u16 {
    match trns.get(i * 2..i * 2 + 2) {
        Some(b) => u16::from_be_bytes([b[0], b[1]]),
        None => 0,
    }
}

impl RowExpander {
    /// Errors only when an output row would not fit the platform's address
    /// space (wide images on 32-bit).
    pub(crate) fn new(ihdr: &Ihdr, ancillary: &PngAncillary) -> crate::error::Result<Self> {
        let width = ihdr.width as usize;
        let trns = ancillary.trns.as_deref();
        // tRNS shorter than its color type needs reads as 0 (matching the
        // per-pixel code this replaced).
        let kind = match (ihdr.color_type, ihdr.bit_depth, trns) {
            (0, 16, None) | (2, 16, None) | (4, 16, _) | (6, 16, _) => Expand::Swap16,
            (0, 16, Some(t)) => Expand::Gray16Trns(if t.len() >= 2 { trns_u16(t, 0) } else { 0 }),
            (2, 16, Some(t)) => Expand::Rgb16Trns(if t.len() >= 6 {
                [trns_u16(t, 0), trns_u16(t, 1), trns_u16(t, 2)]
            } else {
                [0; 3]
            }),
            (0, 8, Some(t)) => Expand::Gray8Trns(if t.len() >= 2 {
                trns_u16(t, 0) as u8
            } else {
                0
            }),
            (0, 8, None) | (2, 8, None) | (6, 8, _) => Expand::Copy,
            (0, bits, None) => Expand::PackedGray {
                bits,
                table: unpack_table(bits, scale_to_8bit(1, bits)),
            },
            (0, bits, Some(t)) => Expand::SubByteGrayTrns {
                bits,
                // A tRNS value outside the bit depth never matches; u8::MAX
                // can't be a 1/2/4-bit sample either.
                trns: {
                    let v = if t.len() >= 2 { trns_u16(t, 0) } else { 0 };
                    u8::try_from(v).unwrap_or(u8::MAX)
                },
            },
            (2, _, Some(t)) => Expand::Rgb8Trns(if t.len() >= 6 {
                [
                    trns_u16(t, 0) as u8,
                    trns_u16(t, 1) as u8,
                    trns_u16(t, 2) as u8,
                ]
            } else {
                [0; 3]
            }),
            (4, _, _) => Expand::GrayAlpha8,
            (3, bits, t) => {
                let palette = ancillary.palette.as_deref().unwrap_or(&[]);
                let mut lut = alloc::boxed::Box::new([[0u8, 0, 0, 255]; 256]);
                for (i, e) in lut.iter_mut().enumerate() {
                    if let Some(rgb) = palette.get(i * 3..i * 3 + 3) {
                        e[..3].copy_from_slice(rgb);
                    }
                    if let Some(&a) = t.and_then(|t| t.get(i)) {
                        e[3] = a;
                    }
                }
                Expand::Palette {
                    bits,
                    alpha: t.is_some(),
                    lut,
                    unpack: (bits < 8).then(|| unpack_table(bits, 1)),
                }
            }
            _ => unreachable!("validated in IHDR parsing"),
        };
        let out_row_bytes = width
            .checked_mul(output_bytes_per_pixel(ihdr, ancillary))
            .ok_or_else(|| {
                at!(PngError::OutOfMemory(
                    "image too large for this platform".into()
                ))
            })?;
        Ok(Self {
            kind,
            width,
            out_row_bytes,
        })
    }

    /// Bytes in one output row.
    pub(crate) fn out_row_bytes(&self) -> usize {
        self.out_row_bytes
    }

    /// True when output rows are byte-identical to raw rows.
    pub(crate) fn is_copy(&self) -> bool {
        matches!(self.kind, Expand::Copy)
    }

    /// Expand one raw row into `out` (`out_row_bytes()` bytes). `raw` holds
    /// the unfiltered scanline; if it is shorter than the image width needs
    /// (never the case for in-tree callers), the missing pixels are left as
    /// they were in `out`.
    pub(crate) fn expand(&self, raw: &[u8], out: &mut [u8]) {
        let out = &mut out[..self.out_row_bytes];
        match &self.kind {
            Expand::Copy => {
                let n = out.len().min(raw.len());
                out[..n].copy_from_slice(&raw[..n]);
            }
            Expand::Swap16 => {
                for (o, i) in out
                    .as_chunks_mut::<2>()
                    .0
                    .iter_mut()
                    .zip(raw.as_chunks::<2>().0)
                {
                    *o = u16::from_be_bytes(*i).to_ne_bytes();
                }
            }
            &Expand::Gray8Trns(t) => {
                for (o, &g) in out.as_chunks_mut::<4>().0.iter_mut().zip(raw) {
                    *o = [g, g, g, if g == t { 0 } else { 255 }];
                }
            }
            &Expand::Gray16Trns(t) => {
                for (o, i) in out
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(raw.as_chunks::<2>().0)
                {
                    let v = u16::from_be_bytes(*i);
                    let a: u16 = if v == t { 0 } else { u16::MAX };
                    let (v, a) = (v.to_ne_bytes(), a.to_ne_bytes());
                    *o = [v[0], v[1], a[0], a[1]];
                }
            }
            &Expand::Rgb8Trns(t) => {
                for (o, i) in out
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(raw.as_chunks::<3>().0)
                {
                    *o = [i[0], i[1], i[2], if *i == t { 0 } else { 255 }];
                }
            }
            &Expand::Rgb16Trns(t) => {
                for (o, i) in out
                    .as_chunks_mut::<8>()
                    .0
                    .iter_mut()
                    .zip(raw.as_chunks::<6>().0)
                {
                    let c = [
                        u16::from_be_bytes([i[0], i[1]]),
                        u16::from_be_bytes([i[2], i[3]]),
                        u16::from_be_bytes([i[4], i[5]]),
                    ];
                    let a: u16 = if c == t { 0 } else { u16::MAX };
                    let (r, g, b, a) = (
                        c[0].to_ne_bytes(),
                        c[1].to_ne_bytes(),
                        c[2].to_ne_bytes(),
                        a.to_ne_bytes(),
                    );
                    *o = [r[0], r[1], g[0], g[1], b[0], b[1], a[0], a[1]];
                }
            }
            Expand::GrayAlpha8 => {
                for (o, i) in out
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(raw.as_chunks::<2>().0)
                {
                    *o = [i[0], i[0], i[0], i[1]];
                }
            }
            Expand::PackedGray { bits, table } => match bits {
                1 => packed_gray::<8>(raw, out, self.width, table),
                2 => packed_gray::<4>(raw, out, self.width, table),
                _ => packed_gray::<2>(raw, out, self.width, table),
            },
            &Expand::SubByteGrayTrns { bits, trns } => {
                let scale = scale_to_8bit(1, bits);
                for (x, o) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let Some(v) = sub_byte(raw, x, bits) else {
                        break;
                    };
                    let g = v * scale;
                    *o = [g, g, g, if v == trns { 0 } else { 255 }];
                }
            }
            Expand::Palette {
                bits,
                alpha,
                lut,
                unpack,
            } => {
                let w = self.width;
                match (unpack, *alpha) {
                    (None, true) => {
                        for (o, &i) in out.as_chunks_mut::<4>().0.iter_mut().zip(raw) {
                            *o = lut[i as usize];
                        }
                    }
                    (None, false) => {
                        for (o, &i) in out.as_chunks_mut::<3>().0.iter_mut().zip(raw) {
                            let e = lut[i as usize];
                            *o = [e[0], e[1], e[2]];
                        }
                    }
                    (Some(t), true) => match bits {
                        1 => packed_palette::<8, 4>(raw, out, w, t, lut),
                        2 => packed_palette::<4, 4>(raw, out, w, t, lut),
                        _ => packed_palette::<2, 4>(raw, out, w, t, lut),
                    },
                    (Some(t), false) => match bits {
                        1 => packed_palette::<8, 3>(raw, out, w, t, lut),
                        2 => packed_palette::<4, 3>(raw, out, w, t, lut),
                        _ => packed_palette::<2, 3>(raw, out, w, t, lut),
                    },
                }
            }
        }
    }
}

/// For every byte value, its samples at `bits` per sample (MSB first, up to
/// 8 of them), each multiplied by `scale`.
fn unpack_table(bits: u8, scale: u8) -> alloc::boxed::Box<[[u8; 8]; 256]> {
    let per = 8 / bits as usize;
    let mask = (1u8 << bits) - 1;
    let mut t = alloc::boxed::Box::new([[0u8; 8]; 256]);
    for (b, e) in t.iter_mut().enumerate() {
        for (k, v) in e.iter_mut().take(per).enumerate() {
            *v = ((b as u8 >> (8 - bits as usize * (k + 1))) & mask) * scale;
        }
    }
    t
}

/// Packed gray row (`P` samples per byte) → one output byte per sample.
fn packed_gray<const P: usize>(raw: &[u8], out: &mut [u8], width: usize, table: &[[u8; 8]; 256]) {
    let full = (width / P).min(raw.len());
    for (o, &b) in out[..full * P].as_chunks_mut::<P>().0.iter_mut().zip(raw) {
        o.copy_from_slice(&table[b as usize][..P]);
    }
    let rem = width - full * P;
    if rem > 0
        && rem < P
        && let Some(&b) = raw.get(full)
    {
        out[full * P..full * P + rem].copy_from_slice(&table[b as usize][..rem]);
    }
}

/// Packed palette row (`P` indices per byte) → `C`-byte pixels (RGB or RGBA).
// `as_chunks_mut::<{ P * C }>` would need generic const expressions.
#[allow(clippy::chunks_exact_to_as_chunks)]
fn packed_palette<const P: usize, const C: usize>(
    raw: &[u8],
    out: &mut [u8],
    width: usize,
    unpack: &[[u8; 8]; 256],
    lut: &[[u8; 4]; 256],
) {
    let full = (width / P).min(raw.len());
    for (o, &b) in out[..full * P * C].chunks_exact_mut(P * C).zip(raw) {
        let idx = &unpack[b as usize];
        for k in 0..P {
            o[k * C..k * C + C].copy_from_slice(&lut[idx[k] as usize][..C]);
        }
    }
    let rem = width - full * P;
    if rem > 0
        && rem < P
        && let Some(&b) = raw.get(full)
    {
        let idx = &unpack[b as usize];
        for k in 0..rem {
            let at = (full * P + k) * C;
            out[at..at + C].copy_from_slice(&lut[idx[k] as usize][..C]);
        }
    }
}

/// Sample `x` of a row packed at `bits` (1, 2 or 4) per sample, MSB first.
#[inline(always)]
fn sub_byte(raw: &[u8], x: usize, bits: u8) -> Option<u8> {
    let per_byte = 8 / bits as usize;
    let byte = *raw.get(x / per_byte)?;
    let shift = (per_byte - 1 - x % per_byte) * bits as usize;
    Some((byte >> shift) & ((1u8 << bits) - 1))
}

/// A decoded image buffer, typed by sample width so 16-bit output can be
/// handed to `PixelBuffer` without the realignment copy a `Vec<u8>` needs.
pub(crate) enum OutBuf {
    U8(Vec<u8>),
    U16(Vec<u16>),
}

impl OutBuf {
    /// Zeroed buffer of `bytes` bytes; 16-bit samples when `sixteen`.
    pub(crate) fn alloc(
        pref: zencodec::AllocPreference,
        bytes: usize,
        sixteen: bool,
    ) -> crate::error::Result<Self> {
        Ok(if sixteen {
            OutBuf::U16(crate::alloc_util::alloc_zeroed_typed(
                pref,
                true,
                bytes / 2,
            )?)
        } else {
            OutBuf::U8(crate::alloc_util::alloc_zeroed(pref, true, bytes)?)
        })
    }

    pub(crate) fn bytes_mut(&mut self) -> &mut [u8] {
        match self {
            OutBuf::U8(v) => v,
            OutBuf::U16(v) => bytemuck::cast_slice_mut(v),
        }
    }
}

/// Wrap a decoded buffer (rows from [`RowExpander`]) as a `PixelBuffer`,
/// without copying.
pub(crate) fn build_pixel_buffer(
    ihdr: &Ihdr,
    ancillary: &PngAncillary,
    buf: OutBuf,
    w: usize,
    h: usize,
) -> crate::error::Result<PixelBuffer> {
    let erased = |r: Result<PixelBuffer, whereat::At<zenpixels::BufferError>>| -> crate::error::Result<PixelBuffer> {
        r.map_err(|e: whereat::At<zenpixels::BufferError>| at!(PngError::Decode(alloc::format!("{e}"))))
    };
    let (w32, h32) = (w as u32, h as u32);
    let trns = ancillary.trns.is_some();
    match buf {
        OutBuf::U16(v) => match (ihdr.color_type, trns) {
            (0, false) => erased(PixelBuffer::from_pixels_erased(
                bytemuck::cast_vec::<u16, Gray<u16>>(v),
                w32,
                h32,
            )),
            (0, true) | (4, _) => erased(PixelBuffer::from_pixels_erased(
                bytemuck::cast_vec::<u16, GrayAlpha16>(v),
                w32,
                h32,
            )),
            (2, false) => erased(PixelBuffer::from_pixels_erased(
                bytemuck::cast_vec::<u16, Rgb<u16>>(v),
                w32,
                h32,
            )),
            (2, true) | (6, _) => erased(PixelBuffer::from_pixels_erased(
                bytemuck::cast_vec::<u16, Rgba<u16>>(v),
                w32,
                h32,
            )),
            _ => build_pixel_data(ihdr, ancillary, bytemuck::cast_slice(&v).to_vec(), w, h),
        },
        OutBuf::U8(v) => match (ihdr.color_type, trns) {
            (0, false) => erased(PixelBuffer::from_pixels_erased(
                bytemuck::cast_vec::<u8, Gray<u8>>(v),
                w32,
                h32,
            )),
            _ => build_pixel_data(ihdr, ancillary, v, w, h),
        },
    }
}

/// Determine the output PixelData variant info for `PngInfo` construction.
pub(crate) struct OutputFormat {
    pub channels: usize,
    pub bytes_per_channel: usize,
}

impl OutputFormat {
    pub fn from_ihdr(ihdr: &Ihdr, ancillary: &PngAncillary) -> crate::error::Result<Self> {
        Ok(match ihdr.color_type {
            0 => {
                if ancillary.trns.is_some() {
                    // Gray + tRNS → RGBA
                    if ihdr.bit_depth == 16 {
                        Self {
                            channels: 2,
                            bytes_per_channel: 2,
                        }
                    } else {
                        Self {
                            channels: 4,
                            bytes_per_channel: 1,
                        }
                    }
                } else if ihdr.bit_depth == 16 {
                    Self {
                        channels: 1,
                        bytes_per_channel: 2,
                    }
                } else {
                    Self {
                        channels: 1,
                        bytes_per_channel: 1,
                    }
                }
            }
            2 => {
                if ancillary.trns.is_some() {
                    if ihdr.bit_depth == 16 {
                        Self {
                            channels: 4,
                            bytes_per_channel: 2,
                        }
                    } else {
                        Self {
                            channels: 4,
                            bytes_per_channel: 1,
                        }
                    }
                } else if ihdr.bit_depth == 16 {
                    Self {
                        channels: 3,
                        bytes_per_channel: 2,
                    }
                } else {
                    Self {
                        channels: 3,
                        bytes_per_channel: 1,
                    }
                }
            }
            3 => {
                if ancillary.trns.is_some() {
                    Self {
                        channels: 4,
                        bytes_per_channel: 1,
                    }
                } else {
                    Self {
                        channels: 3,
                        bytes_per_channel: 1,
                    }
                }
            }
            4 => {
                if ihdr.bit_depth == 16 {
                    Self {
                        channels: 2,
                        bytes_per_channel: 2,
                    }
                } else {
                    // GA8 → RGBA8
                    Self {
                        channels: 4,
                        bytes_per_channel: 1,
                    }
                }
            }
            6 => {
                if ihdr.bit_depth == 16 {
                    Self {
                        channels: 4,
                        bytes_per_channel: 2,
                    }
                } else {
                    Self {
                        channels: 4,
                        bytes_per_channel: 1,
                    }
                }
            }
            _ => {
                return Err(at!(PngError::UnsupportedFeature(alloc::format!(
                    "unsupported color_type {} in IHDR",
                    ihdr.color_type
                ))));
            }
        })
    }
}

/// Build PixelBuffer from the fully assembled pixel bytes.
pub(crate) fn build_pixel_data(
    ihdr: &Ihdr,
    ancillary: &PngAncillary,
    pixels: Vec<u8>,
    w: usize,
    h: usize,
) -> crate::error::Result<PixelBuffer> {
    let w32 = w as u32;
    let h32 = h as u32;
    match (ihdr.color_type, ihdr.bit_depth, ancillary.trns.is_some()) {
        // Grayscale
        (0, 16, false) => {
            let gray = try_cast_vec_or(pixels, |b| {
                b.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| Gray(u16::from_ne_bytes([c[0], c[1]])))
                    .collect()
            });
            Ok(PixelBuffer::from_imgvec(ImgVec::new(gray, w, h)).into())
        }
        (0, 16, true) => {
            // Gray16 + tRNS → GrayAlpha16 (already processed to native u16 pairs)
            // GrayAlpha16 now impls Pod; construct via raw bytes
            let ga_bytes: Vec<u8> = pixels;
            PixelBuffer::from_vec(ga_bytes, w as u32, h as u32, GrayAlpha16::DESCRIPTOR)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))
        }
        (0, _, false) if ihdr.bit_depth <= 8 => {
            let gray: Vec<Gray<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_imgvec(ImgVec::new(gray, w, h)).into())
        }
        (0, _, true) if ihdr.bit_depth <= 8 => {
            // Gray + tRNS → RGBA8
            let rgba: Vec<Rgba<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgba, w32, h32)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))?)
        }
        // RGB
        (2, 16, false) => {
            let rgb = try_cast_vec_or(pixels, |b| {
                b.as_chunks::<6>()
                    .0
                    .iter()
                    .map(|c| Rgb {
                        r: u16::from_ne_bytes([c[0], c[1]]),
                        g: u16::from_ne_bytes([c[2], c[3]]),
                        b: u16::from_ne_bytes([c[4], c[5]]),
                    })
                    .collect()
            });
            Ok(PixelBuffer::from_imgvec(ImgVec::new(rgb, w, h)).into())
        }
        (2, 16, true) => {
            let rgba = try_cast_vec_or(pixels, bytes_to_rgba16_vec);
            Ok(PixelBuffer::from_imgvec(ImgVec::new(rgba, w, h)).into())
        }
        (2, 8, false) => {
            let rgb: Vec<Rgb<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgb, w32, h32)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))?)
        }
        (2, 8, true) => {
            let rgba: Vec<Rgba<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgba, w32, h32)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))?)
        }
        // Indexed
        (3, _, true) => {
            let rgba: Vec<Rgba<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgba, w32, h32)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))?)
        }
        (3, _, false) => {
            let rgb: Vec<Rgb<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgb, w32, h32)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))?)
        }
        // GrayAlpha
        (4, 16, _) => {
            // GrayAlpha16 now impls Pod; construct via raw bytes
            // pixels are already native-endian u16 pairs (v, a) = same layout as GrayAlpha16
            PixelBuffer::from_vec(pixels, w as u32, h as u32, GrayAlpha16::DESCRIPTOR)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))
        }
        (4, 8, _) => {
            // GA8 already expanded to RGBA8
            let rgba: Vec<Rgba<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgba, w32, h32)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))?)
        }
        // RGBA
        (6, 16, _) => {
            let rgba = try_cast_vec_or(pixels, bytes_to_rgba16_vec);
            Ok(PixelBuffer::from_imgvec(ImgVec::new(rgba, w, h)).into())
        }
        (6, 8, _) => {
            let rgba: Vec<Rgba<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgba, w32, h32)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))?)
        }
        _ => Err(at!(PngError::UnsupportedFeature(alloc::format!(
            "unsupported color_type={} bit_depth={}",
            ihdr.color_type,
            ihdr.bit_depth
        )))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ihdr(color_type: u8, bit_depth: u8) -> Ihdr {
        Ihdr {
            width: 4,
            height: 1,
            bit_depth,
            color_type,
            interlace: 0,
        }
    }

    fn empty_anc() -> PngAncillary {
        PngAncillary::default()
    }

    fn anc_with_trns(trns: Vec<u8>) -> PngAncillary {
        PngAncillary {
            trns: Some(trns),
            ..Default::default()
        }
    }

    // ── scale_to_8bit ──

    #[test]
    fn scale_to_8bit_identity() {
        // bit_depth=8 hits the `_ => value` fallback
        assert_eq!(scale_to_8bit(42, 8), 42);
        assert_eq!(scale_to_8bit(0, 8), 0);
        assert_eq!(scale_to_8bit(255, 8), 255);
    }

    #[test]
    fn scale_to_8bit_1bit() {
        assert_eq!(scale_to_8bit(0, 1), 0);
        assert_eq!(scale_to_8bit(1, 1), 255);
    }

    #[test]
    fn scale_to_8bit_2bit() {
        assert_eq!(scale_to_8bit(0, 2), 0);
        assert_eq!(scale_to_8bit(1, 2), 85);
        assert_eq!(scale_to_8bit(2, 2), 170);
        assert_eq!(scale_to_8bit(3, 2), 255);
    }

    #[test]
    fn scale_to_8bit_4bit() {
        assert_eq!(scale_to_8bit(0, 4), 0);
        assert_eq!(scale_to_8bit(15, 4), 255);
    }

    // ── output_bytes_per_pixel ──

    #[test]
    fn output_bpp_gray_variants() {
        let anc = empty_anc();
        // Gray8 no tRNS → 1 byte
        assert_eq!(output_bytes_per_pixel(&make_ihdr(0, 8), &anc), 1);
        // Gray16 no tRNS → 2 bytes
        assert_eq!(output_bytes_per_pixel(&make_ihdr(0, 16), &anc), 2);
        // Gray + tRNS → 4 bytes (RGBA8 or GA16)
        let anc_trns = anc_with_trns(vec![0, 0]);
        assert_eq!(output_bytes_per_pixel(&make_ihdr(0, 8), &anc_trns), 4);
        assert_eq!(output_bytes_per_pixel(&make_ihdr(0, 16), &anc_trns), 4);
    }

    #[test]
    fn output_bpp_rgb_variants() {
        let anc = empty_anc();
        assert_eq!(output_bytes_per_pixel(&make_ihdr(2, 8), &anc), 3);
        assert_eq!(output_bytes_per_pixel(&make_ihdr(2, 16), &anc), 6);
        let anc_trns = anc_with_trns(vec![0; 6]);
        assert_eq!(output_bytes_per_pixel(&make_ihdr(2, 8), &anc_trns), 4);
        assert_eq!(output_bytes_per_pixel(&make_ihdr(2, 16), &anc_trns), 8);
    }

    #[test]
    fn output_bpp_indexed() {
        let anc = empty_anc();
        assert_eq!(output_bytes_per_pixel(&make_ihdr(3, 8), &anc), 3);
        let anc_trns = anc_with_trns(vec![255]);
        assert_eq!(output_bytes_per_pixel(&make_ihdr(3, 8), &anc_trns), 4);
    }

    #[test]
    fn output_bpp_gray_alpha() {
        let anc = empty_anc();
        assert_eq!(output_bytes_per_pixel(&make_ihdr(4, 8), &anc), 4);
        assert_eq!(output_bytes_per_pixel(&make_ihdr(4, 16), &anc), 4);
    }

    #[test]
    fn output_bpp_rgba() {
        let anc = empty_anc();
        assert_eq!(output_bytes_per_pixel(&make_ihdr(6, 8), &anc), 4);
        assert_eq!(output_bytes_per_pixel(&make_ihdr(6, 16), &anc), 8);
    }

    // ── post_process_row: Gray8 + tRNS ──

    #[test]
    fn gray8_trns_expansion() {
        let ihdr = make_ihdr(0, 8);
        // tRNS gray value = 100 (big-endian u16: [0, 100])
        let anc = anc_with_trns(vec![0, 100]);
        // Raw row: 4 gray pixels, second one matches tRNS
        let raw = vec![50, 100, 200, 100];
        let mut out = Vec::new();
        post_process_row(&raw, &ihdr, &anc, &mut out);
        // Each pixel → RGBA8: [g, g, g, alpha]
        assert_eq!(out.len(), 16);
        // pixel 0: g=50, alpha=255
        assert_eq!(&out[0..4], &[50, 50, 50, 255]);
        // pixel 1: g=100 matches tRNS → alpha=0
        assert_eq!(&out[4..8], &[100, 100, 100, 0]);
        // pixel 2: g=200, alpha=255
        assert_eq!(&out[8..12], &[200, 200, 200, 255]);
        // pixel 3: g=100 matches tRNS → alpha=0
        assert_eq!(&out[12..16], &[100, 100, 100, 0]);
    }

    #[test]
    fn gray8_trns_short_data() {
        // tRNS data shorter than 2 bytes → trns_val defaults to 0
        let ihdr = make_ihdr(0, 8);
        let anc = anc_with_trns(vec![]);
        let raw = vec![0, 1, 0, 255];
        let mut out = Vec::new();
        post_process_row(&raw, &ihdr, &anc, &mut out);
        // pixel 0: g=0 matches tRNS(0) → alpha=0
        assert_eq!(&out[0..4], &[0, 0, 0, 0]);
        // pixel 1: g=1, alpha=255
        assert_eq!(&out[4..8], &[1, 1, 1, 255]);
    }

    // ── post_process_row: sub-byte Gray + tRNS (short data fallback) ──

    #[test]
    fn gray_subbyte_trns_short_data() {
        // 1-bit gray, width=4, tRNS with short data → trns_val=0
        let ihdr = Ihdr {
            width: 8,
            height: 1,
            bit_depth: 1,
            color_type: 0,
            interlace: 0,
        };
        // Short tRNS → defaults to 0 → val 0 is transparent
        let anc = anc_with_trns(vec![]);
        // 8 pixels packed in 1 byte: 0b10101010 = pixels [1,0,1,0,1,0,1,0]
        let raw = vec![0b10101010];
        let mut out = Vec::new();
        post_process_row(&raw, &ihdr, &anc, &mut out);
        assert_eq!(out.len(), 32); // 8 pixels × 4 bytes
        // pixel 0: val=1 → g=255, alpha=255
        assert_eq!(&out[0..4], &[255, 255, 255, 255]);
        // pixel 1: val=0 → g=0, alpha=0 (transparent)
        assert_eq!(&out[4..8], &[0, 0, 0, 0]);
    }

    // ── post_process_row: Gray16 + tRNS (short data fallback) ──

    #[test]
    fn gray16_trns_short_data() {
        let ihdr = make_ihdr(0, 16);
        // Short tRNS → trns_val=0
        let anc = anc_with_trns(vec![5]); // only 1 byte, needs 2
        // 4 pixels: big-endian gray16
        let raw: Vec<u8> = [0u16, 100, 200, 0]
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect();
        let mut out = Vec::new();
        post_process_row(&raw, &ihdr, &anc, &mut out);
        // Each pixel → GrayAlpha16 (4 bytes: val_ne + alpha_ne)
        assert_eq!(out.len(), 16);
        // pixel 0: val=0 matches tRNS → alpha=0
        let v0 = u16::from_ne_bytes([out[0], out[1]]);
        let a0 = u16::from_ne_bytes([out[2], out[3]]);
        assert_eq!(v0, 0);
        assert_eq!(a0, 0);
        // pixel 1: val=100, alpha=65535
        let a1 = u16::from_ne_bytes([out[6], out[7]]);
        assert_eq!(a1, 65535);
    }

    // ── post_process_row: RGB16 + tRNS (short data fallback) ──

    #[test]
    fn rgb16_trns_short_data() {
        // Short tRNS (less than 6 bytes) → defaults to (0,0,0)
        let anc = anc_with_trns(vec![0, 1]);
        // 1 pixel RGB16 = [0, 0, 0] (matches default tRNS)
        let raw: Vec<u8> = [0u16, 0, 0].iter().flat_map(|v| v.to_be_bytes()).collect();
        let mut ihdr1 = make_ihdr(2, 16);
        ihdr1.width = 1;
        let mut out = Vec::new();
        post_process_row(&raw, &ihdr1, &anc, &mut out);
        // 1 pixel → RGBA16 = 8 bytes
        assert_eq!(out.len(), 8);
        // alpha should be 0 (transparent)
        let a = u16::from_ne_bytes([out[6], out[7]]);
        assert_eq!(a, 0);
    }

    // ── post_process_row: RGB8 + tRNS (short data fallback) ──

    #[test]
    fn rgb8_trns_short_data() {
        let mut ihdr = make_ihdr(2, 8);
        ihdr.width = 2;
        // Short tRNS → defaults to (0,0,0)
        let anc = anc_with_trns(vec![0, 1, 0]);
        // 2 pixels: [0,0,0] and [1,2,3]
        let raw = vec![0, 0, 0, 1, 2, 3];
        let mut out = Vec::new();
        post_process_row(&raw, &ihdr, &anc, &mut out);
        assert_eq!(out.len(), 8); // 2 × RGBA8
        // pixel 0: matches (0,0,0) → alpha=0
        assert_eq!(&out[0..4], &[0, 0, 0, 0]);
        // pixel 1: no match → alpha=255
        assert_eq!(&out[4..8], &[1, 2, 3, 255]);
    }

    // ── OutputFormat ──

    #[test]
    fn output_format_gray_trns_16() {
        let ihdr = make_ihdr(0, 16);
        let anc = anc_with_trns(vec![0, 0]);
        let fmt = OutputFormat::from_ihdr(&ihdr, &anc).unwrap();
        assert_eq!(fmt.channels, 2);
        assert_eq!(fmt.bytes_per_channel, 2);
    }

    #[test]
    fn output_format_gray_trns_8() {
        let ihdr = make_ihdr(0, 8);
        let anc = anc_with_trns(vec![0, 0]);
        let fmt = OutputFormat::from_ihdr(&ihdr, &anc).unwrap();
        assert_eq!(fmt.channels, 4);
        assert_eq!(fmt.bytes_per_channel, 1);
    }

    #[test]
    fn output_format_all_types() {
        let anc = empty_anc();
        // Gray8
        let f = OutputFormat::from_ihdr(&make_ihdr(0, 8), &anc).unwrap();
        assert_eq!((f.channels, f.bytes_per_channel), (1, 1));
        // Gray16
        let f = OutputFormat::from_ihdr(&make_ihdr(0, 16), &anc).unwrap();
        assert_eq!((f.channels, f.bytes_per_channel), (1, 2));
        // RGB8
        let f = OutputFormat::from_ihdr(&make_ihdr(2, 8), &anc).unwrap();
        assert_eq!((f.channels, f.bytes_per_channel), (3, 1));
        // RGB16
        let f = OutputFormat::from_ihdr(&make_ihdr(2, 16), &anc).unwrap();
        assert_eq!((f.channels, f.bytes_per_channel), (3, 2));
        // RGB8 + tRNS
        let at = anc_with_trns(vec![0; 6]);
        let f = OutputFormat::from_ihdr(&make_ihdr(2, 8), &at).unwrap();
        assert_eq!((f.channels, f.bytes_per_channel), (4, 1));
        // RGB16 + tRNS
        let f = OutputFormat::from_ihdr(&make_ihdr(2, 16), &at).unwrap();
        assert_eq!((f.channels, f.bytes_per_channel), (4, 2));
        // Indexed
        let f = OutputFormat::from_ihdr(&make_ihdr(3, 8), &anc).unwrap();
        assert_eq!((f.channels, f.bytes_per_channel), (3, 1));
        let f = OutputFormat::from_ihdr(&make_ihdr(3, 8), &at).unwrap();
        assert_eq!((f.channels, f.bytes_per_channel), (4, 1));
        // GrayAlpha8
        let f = OutputFormat::from_ihdr(&make_ihdr(4, 8), &anc).unwrap();
        assert_eq!((f.channels, f.bytes_per_channel), (4, 1));
        // GrayAlpha16
        let f = OutputFormat::from_ihdr(&make_ihdr(4, 16), &anc).unwrap();
        assert_eq!((f.channels, f.bytes_per_channel), (2, 2));
        // RGBA8
        let f = OutputFormat::from_ihdr(&make_ihdr(6, 8), &anc).unwrap();
        assert_eq!((f.channels, f.bytes_per_channel), (4, 1));
        // RGBA16
        let f = OutputFormat::from_ihdr(&make_ihdr(6, 16), &anc).unwrap();
        assert_eq!((f.channels, f.bytes_per_channel), (4, 2));
    }

    #[test]
    fn output_format_invalid_color_type_returns_error() {
        let anc = empty_anc();
        // color_type=5 is invalid per PNG spec
        let ihdr = make_ihdr(5, 8);
        assert!(OutputFormat::from_ihdr(&ihdr, &anc).is_err());
        // color_type=7 is also invalid
        let ihdr = make_ihdr(7, 8);
        assert!(OutputFormat::from_ihdr(&ihdr, &anc).is_err());
    }

    // ── build_pixel_data ──

    #[test]
    fn build_pixel_data_gray8() {
        let ihdr = make_ihdr(0, 8);
        let anc = empty_anc();
        let pixels = vec![10, 20, 30, 40]; // 4 pixels
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_gray8_trns() {
        let ihdr = make_ihdr(0, 8);
        let anc = anc_with_trns(vec![0, 10]);
        // 4 RGBA8 pixels
        let pixels = vec![
            10, 10, 10, 0, 20, 20, 20, 255, 10, 10, 10, 0, 30, 30, 30, 255,
        ];
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_gray16() {
        let ihdr = make_ihdr(0, 16);
        let anc = empty_anc();
        // 4 Gray16 pixels in native endian
        let pixels: Vec<u8> = [100u16, 200, 300, 400]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_gray16_trns() {
        let anc = anc_with_trns(vec![0, 0]);
        // 2 GrayAlpha16 pixels (4 bytes each, native endian u16 pairs)
        let mut ihdr2 = make_ihdr(0, 16);
        ihdr2.width = 2;
        let pixels: Vec<u8> = [100u16, 65535, 0, 0]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let result = build_pixel_data(&ihdr2, &anc, pixels, 2, 1).unwrap();
        assert_eq!(result.width(), 2);
    }

    #[test]
    fn build_pixel_data_rgb8() {
        let ihdr = make_ihdr(2, 8);
        let anc = empty_anc();
        let pixels = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 128, 128, 128];
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_rgb8_trns() {
        let ihdr = make_ihdr(2, 8);
        let anc = anc_with_trns(vec![0; 6]);
        let pixels = vec![
            0, 0, 0, 0, 255, 0, 0, 255, 0, 0, 255, 255, 128, 128, 128, 255,
        ];
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_rgb16() {
        let anc = empty_anc();
        let mut ihdr1 = make_ihdr(2, 16);
        ihdr1.width = 1;
        let pixels: Vec<u8> = [100u16, 200, 300]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let result = build_pixel_data(&ihdr1, &anc, pixels, 1, 1).unwrap();
        assert_eq!(result.width(), 1);
    }

    #[test]
    fn build_pixel_data_rgb16_trns() {
        let anc = anc_with_trns(vec![0; 6]);
        let mut ihdr1 = make_ihdr(2, 16);
        ihdr1.width = 1;
        let pixels: Vec<u8> = [100u16, 200, 300, 65535]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let result = build_pixel_data(&ihdr1, &anc, pixels, 1, 1).unwrap();
        assert_eq!(result.width(), 1);
    }

    #[test]
    fn build_pixel_data_indexed_with_trns() {
        let ihdr = make_ihdr(3, 8);
        let anc = anc_with_trns(vec![255, 0]);
        let pixels = vec![
            255, 0, 0, 255, 0, 255, 0, 0, 0, 0, 255, 128, 128, 128, 128, 255,
        ];
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_indexed_no_trns() {
        let ihdr = make_ihdr(3, 8);
        let anc = empty_anc();
        let pixels = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 128, 128, 128];
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_gray_alpha_8() {
        let ihdr = make_ihdr(4, 8);
        let anc = empty_anc();
        // GA8 expanded to RGBA8: 4 pixels × 4 bytes
        let pixels = vec![
            100, 100, 100, 255, 200, 200, 200, 128, 50, 50, 50, 0, 0, 0, 0, 255,
        ];
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_gray_alpha_16() {
        let mut ihdr = make_ihdr(4, 16);
        ihdr.width = 2;
        let anc = empty_anc();
        let pixels: Vec<u8> = [100u16, 65535, 200, 0]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let result = build_pixel_data(&ihdr, &anc, pixels, 2, 1).unwrap();
        assert_eq!(result.width(), 2);
    }

    #[test]
    fn build_pixel_data_rgba8() {
        let ihdr = make_ihdr(6, 8);
        let anc = empty_anc();
        let pixels = vec![
            255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 128, 128, 128, 255,
        ];
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_rgba16() {
        let mut ihdr = make_ihdr(6, 16);
        ihdr.width = 1;
        let anc = empty_anc();
        let pixels: Vec<u8> = [100u16, 200, 300, 65535]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let result = build_pixel_data(&ihdr, &anc, pixels, 1, 1).unwrap();
        assert_eq!(result.width(), 1);
    }

    // ── sub-byte sample extraction ──

    #[test]
    fn sub_byte_1bit_and_4bit() {
        // 0b11001100 = samples [1,1,0,0,1,1,0,0]
        let raw = [0b11001100];
        let got: Vec<u8> = (0..8).map(|x| sub_byte(&raw, x, 1).unwrap()).collect();
        assert_eq!(got, [1, 1, 0, 0, 1, 1, 0, 0]);
        // 0xA3 = high nibble 10, low nibble 3; past the end is None
        let raw = [0xA3];
        assert_eq!(
            (sub_byte(&raw, 0, 4), sub_byte(&raw, 1, 4)),
            (Some(10), Some(3))
        );
        assert_eq!(sub_byte(&raw, 2, 4), None);
    }

    #[test]
    fn sub_byte_gray_expands_scaled() {
        let ihdr = Ihdr {
            width: 8,
            height: 1,
            bit_depth: 1,
            color_type: 0,
            interlace: 0,
        };
        let mut out = Vec::new();
        post_process_row(&[0b11001100], &ihdr, &empty_anc(), &mut out);
        assert_eq!(out, [255, 255, 0, 0, 255, 255, 0, 0]);
    }
}
