//! PNG metadata chunk writing.

use alloc::vec::Vec;

use zencodec::{Cicp, ContentLightLevel, MasteringDisplay, Metadata};
use zenflate::{CompressionLevel, Compressor, Unstoppable};

use crate::chunk::write::write_chunk;
use crate::decode::PngChromaticities;
use crate::error::PngError;

/// All metadata to embed when writing a PNG file.
///
/// Aggregates both codec-generic metadata (`Metadata`) and PNG-specific
/// color chunks (gAMA, sRGB, cHRM). Constructed by the encode functions.
pub(crate) struct PngWriteMetadata<'a> {
    /// ICC profile, EXIF, XMP from Metadata.
    pub generic: Option<&'a Metadata>,
    /// gAMA chunk value (scaled by 100000, e.g. 45455 = 1/2.2).
    pub source_gamma: Option<u32>,
    /// sRGB rendering intent (0-3).
    pub srgb_intent: Option<u8>,
    /// cHRM chromaticity values.
    pub chromaticities: Option<PngChromaticities>,
    /// cICP color description.
    pub cicp: Option<Cicp>,
    /// Content Light Level (HDR).
    pub content_light_level: Option<ContentLightLevel>,
    /// Mastering Display Color Volume (HDR).
    pub mastering_display: Option<MasteringDisplay>,
}

impl<'a> PngWriteMetadata<'a> {
    /// Build from Metadata, inheriting cICP/cLLi/mDCV from it.
    pub fn from_metadata(meta: Option<&'a Metadata>) -> Self {
        let (cicp, content_light_level, mastering_display) = meta
            .map(|m| (m.cicp, m.content_light_level, m.mastering_display))
            .unwrap_or((None, None, None));
        Self {
            generic: meta,
            source_gamma: None,
            srgb_intent: None,
            chromaticities: None,
            cicp,
            content_light_level,
            mastering_display,
        }
    }
}

/// Write all metadata chunks in correct PNG order with PNGv3 precedence.
///
/// PNGv3 precedence for color chunks (highest priority first):
///   cICP > iCCP > sRGB > gAMA/cHRM
///
/// When a higher-priority chunk is present, lower-priority chunks are
/// suppressed in the output to avoid conflicting color signals. Exception:
/// iCCP is kept alongside cICP as a fallback since cICP decoder support
/// is still limited.
///
/// HDR metadata (mDCV, cLLi) always written — they complement cICP.
/// eXIf and XMP always written — they are not color chunks.
///
/// Per PNG spec: sRGB/gAMA/cHRM must come before PLTE and IDAT.
/// iCCP must come before PLTE. cICP/mDCV/cLLi must come before IDAT.
pub(crate) fn write_all_metadata(
    out: &mut Vec<u8>,
    meta: &PngWriteMetadata<'_>,
) -> Result<(), PngError> {
    let has_cicp = meta.cicp.is_some();
    let has_iccp = meta.generic.map_or(false, |g| g.icc_profile.is_some());
    let has_srgb = meta.srgb_intent.is_some();

    // PNGv3 precedence: cICP > iCCP > sRGB > gAMA/cHRM
    //
    // When cICP present: write cICP + iCCP (fallback), suppress sRGB/gAMA/cHRM
    // When iCCP present (no cICP): write iCCP, suppress sRGB/gAMA/cHRM
    // When sRGB present (no cICP/iCCP): write sRGB, suppress gAMA/cHRM
    // Otherwise: write gAMA and/or cHRM if present
    let write_srgb = has_srgb && !has_cicp && !has_iccp;
    let write_gama_chrm = !has_cicp && !has_iccp && !has_srgb;

    // sRGB rendering intent
    if write_srgb && let Some(intent) = meta.srgb_intent {
        write_srgb_chunk(out, intent);
    }

    // gAMA (source gamma) — only when no higher-priority chunk present
    if write_gama_chrm && let Some(gamma) = meta.source_gamma {
        write_gama_chunk(out, gamma);
    }

    // cHRM (chromaticities) — only when no higher-priority chunk present
    if write_gama_chrm && let Some(chrm) = &meta.chromaticities {
        write_chrm_chunk(out, chrm);
    }

    // iCCP (ICC profile) — written when present, even alongside cICP (as fallback)
    if let Some(generic) = meta.generic
        && let Some(icc) = &generic.icc_profile
    {
        write_iccp_chunk(out, icc)?;
    }

    // cICP (coding-independent code points)
    if let Some(cicp) = &meta.cicp {
        write_cicp_chunk(out, cicp);
    }

    // mDCV (mastering display color volume) — always written, complements cICP
    if let Some(mdcv) = &meta.mastering_display {
        write_mdcv_chunk(out, mdcv);
    }

    // cLLi (content light level info) — always written, complements cICP
    if let Some(clli) = &meta.content_light_level {
        write_clli_chunk(out, clli);
    }

    // eXIf — always written
    if let Some(generic) = meta.generic
        && let Some(exif) = &generic.exif
    {
        write_exif_chunk(out, exif);
    }

    // iTXt for XMP — always written
    if let Some(generic) = meta.generic
        && let Some(xmp) = &generic.xmp
    {
        let xmp_str = core::str::from_utf8(xmp).unwrap_or_default();
        if !xmp_str.is_empty() {
            write_itxt_chunk(out, "XML:com.adobe.xmp", xmp_str);
        }
    }

    Ok(())
}

// ---- Individual chunk writers ----

fn write_srgb_chunk(out: &mut Vec<u8>, intent: u8) {
    write_chunk(out, b"sRGB", &[intent]);
}

fn write_gama_chunk(out: &mut Vec<u8>, gamma: u32) {
    write_chunk(out, b"gAMA", &gamma.to_be_bytes());
}

fn write_chrm_chunk(out: &mut Vec<u8>, chrm: &PngChromaticities) {
    // cHRM: 8 i32 values in order: white_x, white_y, red_x, red_y, green_x, green_y, blue_x, blue_y
    let mut data = [0u8; 32];
    data[0..4].copy_from_slice(&chrm.white_x.to_be_bytes());
    data[4..8].copy_from_slice(&chrm.white_y.to_be_bytes());
    data[8..12].copy_from_slice(&chrm.red_x.to_be_bytes());
    data[12..16].copy_from_slice(&chrm.red_y.to_be_bytes());
    data[16..20].copy_from_slice(&chrm.green_x.to_be_bytes());
    data[20..24].copy_from_slice(&chrm.green_y.to_be_bytes());
    data[24..28].copy_from_slice(&chrm.blue_x.to_be_bytes());
    data[28..32].copy_from_slice(&chrm.blue_y.to_be_bytes());
    write_chunk(out, b"cHRM", &data);
}

fn write_cicp_chunk(out: &mut Vec<u8>, cicp: &Cicp) {
    // cICP: 4 bytes — color_primaries, transfer_function, matrix_coefficients, full_range
    let data = [
        cicp.color_primaries,
        cicp.transfer_characteristics,
        cicp.matrix_coefficients,
        if cicp.full_range { 1 } else { 0 },
    ];
    write_chunk(out, b"cICP", &data);
}

fn write_mdcv_chunk(out: &mut Vec<u8>, mdcv: &MasteringDisplay) {
    // mDCV: 6×u16 chromaticities (R, G, B primaries as xy pairs) + 2×u16 white point
    //       + u32 max_luminance + u32 min_luminance = 24 bytes
    // PNG mDCV uses u16 in units of 0.00002; luminance u32 in units of 0.0001 cd/m²
    let mut data = [0u8; 24];
    let to_u16 = |v: f32| (v / 0.00002).round() as u16;
    let to_u32 = |v: f32| (v / 0.0001).round() as u32;
    // Chromaticities: Rx, Ry, Gx, Gy, Bx, By (6 u16 values)
    for (i, &[x, y]) in mdcv.primaries_xy.iter().enumerate() {
        data[i * 4..i * 4 + 2].copy_from_slice(&to_u16(x).to_be_bytes());
        data[i * 4 + 2..i * 4 + 4].copy_from_slice(&to_u16(y).to_be_bytes());
    }
    // White point: Wx, Wy
    data[12..14].copy_from_slice(&to_u16(mdcv.white_point_xy[0]).to_be_bytes());
    data[14..16].copy_from_slice(&to_u16(mdcv.white_point_xy[1]).to_be_bytes());
    // Luminances (u32, 0.0001 cd/m²)
    data[16..20].copy_from_slice(&to_u32(mdcv.max_luminance).to_be_bytes());
    data[20..24].copy_from_slice(&to_u32(mdcv.min_luminance).to_be_bytes());
    write_chunk(out, b"mDCV", &data);
}

fn write_clli_chunk(out: &mut Vec<u8>, clli: &ContentLightLevel) {
    // cLLi: u32 max_content_light_level + u32 max_frame_average_light_level
    // PNG cLLi uses 0.0001 cd/m² units; zencodec ContentLightLevel uses cd/m² (u16)
    let max_cll = clli.max_content_light_level as u32 * 10000;
    let max_fall = clli.max_frame_average_light_level as u32 * 10000;
    let mut data = [0u8; 8];
    data[0..4].copy_from_slice(&max_cll.to_be_bytes());
    data[4..8].copy_from_slice(&max_fall.to_be_bytes());
    write_chunk(out, b"cLLI", &data);
}

fn write_iccp_chunk(out: &mut Vec<u8>, icc_profile: &[u8]) -> Result<(), PngError> {
    // iCCP: keyword "ICC Profile" + null + compression_method(0) + zlib-compressed profile
    let keyword = b"ICC Profile\0";
    let compression_method = [0u8]; // zlib

    // Compress the ICC profile with zenflate level 9
    let level = CompressionLevel::new(9);
    let mut compressor = Compressor::new(level);
    let bound = Compressor::zlib_compress_bound(icc_profile.len());
    let mut compressed = vec![0u8; bound];
    let compressed_len = compressor
        .zlib_compress(icc_profile, &mut compressed, Unstoppable)
        .map_err(|e| PngError::InvalidInput(alloc::format!("ICC compression failed: {e}")))?;

    let mut chunk_data = Vec::with_capacity(keyword.len() + 1 + compressed_len);
    chunk_data.extend_from_slice(keyword);
    chunk_data.extend_from_slice(&compression_method);
    chunk_data.extend_from_slice(&compressed[..compressed_len]);

    write_chunk(out, b"iCCP", &chunk_data);
    Ok(())
}

fn write_exif_chunk(out: &mut Vec<u8>, exif: &[u8]) {
    write_chunk(out, b"eXIf", exif);
}

fn write_itxt_chunk(out: &mut Vec<u8>, keyword: &str, text: &str) {
    // iTXt: keyword + NUL + compression_flag(0) + compression_method(0)
    //       + language_tag("") + NUL + translated_keyword("") + NUL + text
    let mut chunk_data = Vec::with_capacity(keyword.len() + 5 + text.len());
    chunk_data.extend_from_slice(keyword.as_bytes());
    chunk_data.push(0); // null separator
    chunk_data.push(0); // compression flag: uncompressed
    chunk_data.push(0); // compression method
    chunk_data.push(0); // empty language tag + null
    chunk_data.push(0); // empty translated keyword + null
    chunk_data.extend_from_slice(text.as_bytes());

    write_chunk(out, b"iTXt", &chunk_data);
}

pub(crate) fn metadata_size_estimate(meta: &PngWriteMetadata<'_>) -> usize {
    let mut size = 0;
    let has_cicp = meta.cicp.is_some();
    let has_iccp = meta.generic.map_or(false, |g| g.icc_profile.is_some());
    let has_srgb = meta.srgb_intent.is_some();

    if let Some(generic) = meta.generic {
        if let Some(ref icc) = generic.icc_profile {
            size += 12 + 13 + icc.len() / 2;
        }
        if let Some(ref exif) = generic.exif {
            size += 12 + exif.len();
        }
        if let Some(ref xmp) = generic.xmp {
            size += 12 + 25 + xmp.len();
        }
    }
    // PNGv3 precedence: cICP > iCCP > sRGB > gAMA/cHRM
    if has_srgb && !has_cicp && !has_iccp {
        size += 13; // sRGB(1) + 12 overhead
    }
    if !has_cicp && !has_iccp && !has_srgb {
        if meta.source_gamma.is_some() {
            size += 16; // gAMA(4) + 12
        }
        if meta.chromaticities.is_some() {
            size += 44; // cHRM(32) + 12
        }
    }
    if has_cicp {
        size += 16; // cICP(4) + 12
    }
    if meta.mastering_display.is_some() {
        size += 36;
    }
    if meta.content_light_level.is_some() {
        size += 20;
    }
    size
}
