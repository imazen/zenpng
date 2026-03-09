/// Test color metadata roundtrip across a PNG corpus.
///
/// Usage: cargo run --release --example metadata_roundtrip [-- /path/to/png/dir]
///
/// Decodes each PNG, extracts gAMA/sRGB/cHRM/iCCP/cICP/mDCV/cLLI metadata,
/// re-encodes with the same metadata, decodes again, and verifies exact match.
use std::path::{Path, PathBuf};

use enough::Unstoppable;
use zc::MetadataView;
use zenpixels::PixelBuffer;
use zenpixels::descriptor::{ChannelLayout, ChannelType};
use zenpixels_convert::PixelBufferConvertExt;
use zenpng::{EncodeConfig, PngInfo};

fn main() {
    let dirs = resolve_corpus_dirs();
    let mut paths = Vec::new();
    for dir in &dirs {
        collect_pngs(Path::new(dir), &mut paths);
    }
    paths.sort();

    let total = paths.len();
    println!("Corpus: {} dirs ({total} PNGs)\n", dirs.len());
    for dir in &dirs {
        println!("  {dir}");
    }
    println!();

    let mut tested = 0u32;
    let mut skipped_format = 0u32;
    let mut with_gama = 0u32;
    let mut with_srgb = 0u32;
    let mut with_chrm = 0u32;
    let mut with_icc = 0u32;
    let mut failures = Vec::new();

    for path in &paths {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("SKIP {name}: {e}");
                continue;
            }
        };

        let orig = match zenpng::decode(&data, &zenpng::PngDecodeConfig::none(), &Unstoppable) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("SKIP {name}: decode error: {e}");
                continue;
            }
        };

        let info = &orig.info;

        // Skip if no color metadata to test
        if info.source_gamma.is_none()
            && info.srgb_intent.is_none()
            && info.chromaticities.is_none()
            && info.icc_profile.is_none()
        {
            continue;
        }

        tested += 1;
        if info.source_gamma.is_some() {
            with_gama += 1;
        }
        if info.srgb_intent.is_some() {
            with_srgb += 1;
        }
        if info.chromaticities.is_some() {
            with_chrm += 1;
        }
        if info.icc_profile.is_some() {
            with_icc += 1;
        }

        // Re-encode with same color metadata
        let config = EncodeConfig::default()
            .with_source_gamma(info.source_gamma)
            .with_srgb_intent(info.srgb_intent)
            .with_chromaticities(info.chromaticities);

        let meta = build_metadata(info);
        let encoded = match reencode(&orig.pixels, meta.as_ref(), &config) {
            Ok(e) => e,
            Err(e) if e.to_string().contains("not supported") => {
                skipped_format += 1;
                continue;
            }
            Err(e) => {
                failures.push(format!("{name}: re-encode failed: {e}"));
                continue;
            }
        };

        // Decode re-encoded
        let rt = match zenpng::decode(&encoded, &zenpng::PngDecodeConfig::none(), &Unstoppable) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{name}: re-decode failed: {e}"));
                continue;
            }
        };

        // Verify metadata roundtrip
        if let Err(e) = verify_metadata(info, &rt.info) {
            failures.push(format!("{name}: {e}"));
        }
    }

    println!(
        "Tested: {tested} / {total} PNGs with color metadata (skipped {skipped_format} unsupported formats)"
    );
    println!("  gAMA: {with_gama}");
    println!("  sRGB: {with_srgb}");
    println!("  cHRM: {with_chrm}");
    println!("  iCCP: {with_icc}");
    println!();

    if failures.is_empty() {
        println!("ALL PASSED");
    } else {
        println!("{} FAILURES:", failures.len());
        for f in &failures {
            println!("  {f}");
        }
        std::process::exit(1);
    }
}

fn build_metadata(info: &PngInfo) -> Option<MetadataView<'_>> {
    let has_anything = info.icc_profile.is_some()
        || info.cicp.is_some()
        || info.content_light_level.is_some()
        || info.mastering_display.is_some();
    if !has_anything {
        return None;
    }

    let mut meta = MetadataView::none();
    if let Some(ref icc) = info.icc_profile {
        meta = meta.with_icc(icc);
    }
    if let Some(cicp) = info.cicp {
        meta = meta.with_cicp(cicp);
    }
    if let Some(clli) = info.content_light_level {
        meta = meta.with_content_light_level(clli);
    }
    if let Some(mdcv) = info.mastering_display {
        meta = meta.with_mastering_display(mdcv);
    }
    Some(meta)
}

fn reencode(
    pixels: &PixelBuffer,
    meta: Option<&MetadataView<'_>>,
    config: &EncodeConfig,
) -> Result<Vec<u8>, whereat::At<zenpng::PngError>> {
    let u = &enough::Unstoppable;
    let desc = pixels.descriptor();
    match (desc.layout(), desc.channel_type()) {
        (ChannelLayout::Rgb, ChannelType::U8) => {
            let buf = pixels.to_rgb8();
            zenpng::encode_rgb8(buf.as_imgref(), meta, config, u, u)
        }
        (ChannelLayout::Rgba, ChannelType::U8) => {
            let buf = pixels.to_rgba8();
            zenpng::encode_rgba8(buf.as_imgref(), meta, config, u, u)
        }
        (ChannelLayout::Gray, ChannelType::U8) => {
            let buf = pixels.to_gray8();
            zenpng::encode_gray8(buf.as_imgref(), meta, config, u, u)
        }
        (ChannelLayout::Rgb, ChannelType::U16) => {
            let imgref = pixels.try_as_imgref::<rgb::Rgb<u16>>().unwrap();
            zenpng::encode_rgb16(imgref, meta, config, u, u)
        }
        (ChannelLayout::Rgba, ChannelType::U16) => {
            let imgref = pixels.try_as_imgref::<rgb::Rgba<u16>>().unwrap();
            zenpng::encode_rgba16(imgref, meta, config, u, u)
        }
        (ChannelLayout::Gray, ChannelType::U16) => {
            let imgref = pixels.try_as_imgref::<rgb::Gray<u16>>().unwrap();
            zenpng::encode_gray16(imgref, meta, config, u, u)
        }
        (ChannelLayout::GrayAlpha, _) => {
            Err(zenpng::PngError::InvalidInput("GrayAlpha not supported".into()).into())
        }
        _ => Err(zenpng::PngError::InvalidInput(format!(
            "unsupported pixel format for re-encode: {:?}",
            desc
        ))
        .into()),
    }
}

fn verify_metadata(orig: &PngInfo, rt: &PngInfo) -> Result<(), String> {
    if orig.source_gamma != rt.source_gamma {
        return Err(format!(
            "gAMA mismatch: {:?} vs {:?}",
            orig.source_gamma, rt.source_gamma
        ));
    }
    if orig.srgb_intent != rt.srgb_intent {
        return Err(format!(
            "sRGB mismatch: {:?} vs {:?}",
            orig.srgb_intent, rt.srgb_intent
        ));
    }
    if orig.chromaticities != rt.chromaticities {
        return Err(format!(
            "cHRM mismatch: {:?} vs {:?}",
            orig.chromaticities, rt.chromaticities
        ));
    }
    if orig.icc_profile != rt.icc_profile {
        let orig_len = orig.icc_profile.as_ref().map_or(0, |p| p.len());
        let rt_len = rt.icc_profile.as_ref().map_or(0, |p| p.len());
        return Err(format!(
            "iCCP mismatch: orig {orig_len} bytes vs rt {rt_len} bytes"
        ));
    }
    if orig.cicp != rt.cicp {
        return Err(format!("cICP mismatch: {:?} vs {:?}", orig.cicp, rt.cicp));
    }
    if orig.content_light_level != rt.content_light_level {
        return Err(format!(
            "cLLI mismatch: {:?} vs {:?}",
            orig.content_light_level, rt.content_light_level
        ));
    }
    if orig.mastering_display != rt.mastering_display {
        return Err(format!(
            "mDCV mismatch: {:?} vs {:?}",
            orig.mastering_display, rt.mastering_display
        ));
    }
    Ok(())
}

fn resolve_corpus_dirs() -> Vec<String> {
    if let Some(dir) = std::env::args().nth(1) {
        return vec![dir];
    }
    let base = std::env::var("CODEC_CORPUS_DIR")
        .unwrap_or_else(|_| "/home/lilith/work/codec-corpus".to_string());
    let dirs = [
        format!("{base}/pngsuite"),
        format!("{base}/CID22/CID22-512/training"),
        format!("{base}/clic2025-1024"),
        format!("{base}/imageflow/test_inputs"),
        format!("{base}/image-rs/test-images/png/16bpc"),
    ];
    let found: Vec<String> = dirs
        .iter()
        .filter(|d| Path::new(d.as_str()).is_dir())
        .map(|d| d.to_string())
        .collect();
    if found.is_empty() {
        eprintln!("No corpus directory found");
        std::process::exit(1);
    }
    found
}

fn collect_pngs(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_pngs(&path, out);
        } else if path.extension().is_some_and(|e| e == "png") {
            out.push(path);
        }
    }
}
