//! Convert zenpng's decoded pixels into Apple ImageIO's decoded-buffer layout
//! (as dumped by `tests/fixtures/idot/mac/imageio_tool.swift`), so the two can
//! be compared byte for byte. Shared by `tests/idot_imageio.rs` and
//! `examples/idot_imageio_compare.rs`.

#![allow(dead_code)]

use zenpixels::{ChannelLayout, ChannelType};

/// ImageIO keeps 1/2-bit palette images indexed (one index per byte in the
/// buffer). Expand such a buffer to RGBA8 through the file's PLTE/tRNS.
pub fn expand_indexed(png: &[u8], indices: &[u8]) -> Option<Vec<u8>> {
    let (mut plte, mut trns) = (None, None);
    let mut p = 8;
    while p + 12 <= png.len() {
        let n = u32::from_be_bytes(png[p..p + 4].try_into().unwrap()) as usize;
        let d = png.get(p + 8..p + 8 + n)?;
        match &png[p + 4..p + 8] {
            b"PLTE" => plte = Some(d),
            b"tRNS" => trns = Some(d),
            _ => {}
        }
        p += 12 + n;
    }
    let (plte, trns) = (plte?, trns.unwrap_or(&[]));
    let mut out = Vec::with_capacity(indices.len() * 4);
    for &i in indices {
        let i = i as usize;
        out.extend_from_slice(plte.get(i * 3..i * 3 + 3)?);
        out.push(*trns.get(i).unwrap_or(&255));
    }
    Some(out)
}

/// zenpng pixels -> ImageIO layout, plus a mask of bytes to compare.
pub fn to_imageio(
    px: &zenpixels::PixelBuffer,
    bpc: usize,
    bpp: usize,
    alpha_info: u32,
) -> Result<(Vec<u8>, Vec<bool>), String> {
    let desc = px.descriptor();
    let src_ch = desc.layout().channels();
    let src_bpc = match desc.channel_type() {
        ChannelType::U8 => 8,
        ChannelType::U16 => 16,
        t => return Err(format!("zenpng channel type {t:?}")),
    };
    if src_bpc != bpc {
        return Err(format!("depth: zenpng {src_bpc}-bit, ImageIO {bpc}-bit"));
    }
    let bytes = px.copy_to_contiguous_bytes();
    let cb = bpc / 8;
    let dst_ch = bpp / bpc;
    let n = bytes.len() / (src_ch * cb);
    let max = vec![0xFFu8; cb];
    let mut out = Vec::with_capacity(n * dst_ch * cb);
    let mut mask = Vec::with_capacity(n * dst_ch * cb);
    let (src_gray, src_alpha) = match desc.layout() {
        ChannelLayout::Gray => (true, false),
        ChannelLayout::GrayAlpha => (true, true),
        ChannelLayout::Rgb => (false, false),
        ChannelLayout::Rgba => (false, true),
        l => return Err(format!("zenpng layout {l:?}")),
    };
    let dst_alpha = matches!(alpha_info, 1..=4);
    let dst_skip = matches!(alpha_info, 5 | 6);
    let dst_color = if dst_alpha || dst_skip {
        dst_ch - 1
    } else {
        dst_ch
    };
    for i in 0..n {
        let p = &bytes[i * src_ch * cb..(i + 1) * src_ch * cb];
        // A value guaranteed to differ from what ImageIO has is not knowable
        // here; use the bitwise complement of the red channel as a marker.
        let inverted: Vec<u8> = p[..cb].iter().map(|b| !b).collect();
        let c = |k: usize| &p[k * cb..(k + 1) * cb];
        let color: Vec<&[u8]> = match (src_gray, dst_color) {
            (true, 1) => vec![c(0)],
            (true, 3) => vec![c(0), c(0), c(0)],
            (false, 3) => vec![c(0), c(1), c(2)],
            // zenpng may widen gray to RGB; ImageIO kept gray. Only valid
            // when the channels are equal; otherwise count it as a mismatch.
            (false, 1) if c(0) == c(1) && c(1) == c(2) => vec![c(0)],
            (false, 1) => vec![&inverted[..]],
            _ => {
                return Err(format!(
                    "{src_ch}ch -> {dst_ch}ch (alpha_info {alpha_info})"
                ));
            }
        };
        for v in color {
            out.extend_from_slice(v);
            mask.extend(std::iter::repeat_n(true, cb));
        }
        if dst_alpha {
            let a = if src_alpha { c(src_ch - 1) } else { &max[..] };
            out.extend_from_slice(a);
            mask.extend(std::iter::repeat_n(true, cb));
        } else if dst_skip {
            // ImageIO fills RGBX padding with 0xFF (observed on macOS 27).
            out.extend_from_slice(&max);
            mask.extend(std::iter::repeat_n(true, cb));
        }
    }
    Ok((out, mask))
}
