//! `zenpng` — reference CLI for the zenpng codec.
//!
//! ```text
//! zenpng normalize <in.png> <out.png>          # pixels-only re-encode
//! zenpng crop <in.png> <out.png> <side>        # center crop, then pixels-only
//! zenpng compare <a.png> <b.png>               # prints EXACT / DIFFER (exit 0)
//! ```
//!
//! Decode `<in.png>` with zenpng and re-encode as a **pixels-only** PNG (no
//! iCCP / eXIf / gAMA / cHRM / text ancillary chunks), preserving the source
//! channel layout. This is the dogfood replacement for the OpenCV/cv2
//! `IMREAD_UNCHANGED -> imwrite` normalization the codec scoreboard used to
//! run: it exercises our own decoder on every reference image and produces a
//! clean input both encoders read identically (some Display-P3 / EXIF PNGs
//! crash libjxl 0.12's PNG reader until the ancillary chunks are stripped —
//! see jxl-encoder scripts/scoreboard/run_scoreboard.py). `crop` takes a
//! centered `side`x`side` square (clamped to the image) for the size-axis
//! fixed-overhead cells.
//!
//! SDR (8-bit) only: a 16-bit source is rejected loudly rather than silently
//! truncated to 8-bit (which would corrupt an RD comparison). Add a 16-bit
//! path the day a 16-bit SDR source appears.

use enough::Unstoppable;
use imgref::ImgVec;
use rgb::{Rgb, Rgba};
use zenpixels_convert::PixelBufferConvertTypedExt as _;
use zenpng::{EncodeConfig, PngDecodeConfig, decode, encode_gray8, encode_rgb8, encode_rgba8};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = || {
        eprintln!(
            "usage:\n  zenpng normalize <in.png> <out.png>\n  zenpng crop <in.png> <out.png> <side>\n  zenpng compare <a.png> <b.png>"
        );
        std::process::exit(2);
    };
    let res = match args.get(1).map(String::as_str) {
        Some("normalize") if args.len() == 4 => run(&args[2], &args[3], None),
        Some("crop") if args.len() == 5 => match args[4].parse::<u32>() {
            Ok(side) => run(&args[2], &args[3], Some(side)),
            Err(_) => usage(),
        },
        Some("compare") if args.len() == 4 => compare(&args[2], &args[3]),
        _ => usage(),
    };
    if let Err(e) = res {
        eprintln!("zenpng: {e}");
        std::process::exit(1);
    }
}

/// Decode both PNGs with zenpng and report exact pixel equality on stdout
/// (`EXACT` / `DIFFER: <reason>`), exit 0. Used as the lossless-exactness gate
/// so the scoreboard's "decoded pixels must match" check runs through our own
/// decoder consistently on both sides. 16-bit is rejected loudly so a 16->8
/// downconvert can never fake an EXACT verdict.
fn compare(a_path: &str, b_path: &str) -> Result<(), String> {
    let cfg = PngDecodeConfig::default().with_max_pixels(1_000_000_000);
    let read = |p: &str| -> Result<zenpng::PngDecodeOutput, String> {
        let bytes = std::fs::read(p).map_err(|e| format!("read {p}: {e}"))?;
        decode(&bytes, &cfg, &Unstoppable).map_err(|e| format!("decode {p}: {e:?}"))
    };
    let a = read(a_path)?;
    let b = read(b_path)?;
    if a.info.bit_depth > 8 || b.info.bit_depth > 8 {
        return Err(
            "16-bit compare unsupported (would risk a truncation-based false EXACT)".into(),
        );
    }
    let verdict = if a.info.width != b.info.width || a.info.height != b.info.height {
        format!(
            "DIFFER: dimensions {}x{} vs {}x{}",
            a.info.width, a.info.height, b.info.width, b.info.height
        )
    } else {
        // Compare in canonical RGBA8 — exact for 8-bit content, and both sides
        // go through the identical conversion so any real pixel difference shows.
        let ai = a.pixels.to_rgba8();
        let bi = b.pixels.to_rgba8();
        let ap: Vec<Rgba<u8>> = ai.as_imgref().pixels().collect();
        let bp: Vec<Rgba<u8>> = bi.as_imgref().pixels().collect();
        if ap == bp {
            "EXACT".to_string()
        } else {
            let ndiff = ap.iter().zip(&bp).filter(|(x, y)| x != y).count();
            format!("DIFFER: {ndiff} pixels")
        }
    };
    println!("{verdict}");
    Ok(())
}

/// Decode with zenpng, optionally center-crop to `side`x`side`, re-encode
/// pixels-only.
fn run(input: &str, output: &str, side: Option<u32>) -> Result<(), String> {
    let bytes = std::fs::read(input).map_err(|e| format!("read {input}: {e}"))?;
    // No pixel cap: the scoreboard feeds trusted local corpus images that can
    // exceed the 120 MP default.
    let cfg_dec = PngDecodeConfig::default().with_max_pixels(1_000_000_000);
    let out =
        decode(&bytes, &cfg_dec, &Unstoppable).map_err(|e| format!("decode {input}: {e:?}"))?;
    let enc = EncodeConfig::default();

    if out.info.bit_depth > 8 {
        return Err(format!(
            "{input}: 16-bit PNG not supported by `zenpng` (SDR path is 8-bit only)"
        ));
    }

    // Center-crop helper: clamp side to the image, offset to center.
    let crop_box = |w: usize, h: usize| -> (usize, usize, usize, usize) {
        match side {
            None => (0, 0, w, h),
            Some(s) => {
                let (cw, ch) = ((s as usize).min(w), (s as usize).min(h));
                ((w - cw) / 2, (h - ch) / 2, cw, ch)
            }
        }
    };

    let png = if out.info.has_alpha {
        let img = out.pixels.to_rgba8();
        let src = img.as_imgref();
        let (x, y, w, h) = crop_box(src.width(), src.height());
        let buf: Vec<Rgba<u8>> = src.sub_image(x, y, w, h).pixels().collect();
        encode_rgba8(
            ImgVec::new(buf, w, h).as_ref(),
            None,
            &enc,
            &Unstoppable,
            &Unstoppable,
        )
    } else if out.info.color_type == 0 {
        let img = out.pixels.to_gray8();
        let src = img.as_imgref();
        let (x, y, w, h) = crop_box(src.width(), src.height());
        let buf: Vec<_> = src.sub_image(x, y, w, h).pixels().collect();
        encode_gray8(
            ImgVec::new(buf, w, h).as_ref(),
            None,
            &enc,
            &Unstoppable,
            &Unstoppable,
        )
    } else {
        let img = out.pixels.to_rgb8();
        let src = img.as_imgref();
        let (x, y, w, h) = crop_box(src.width(), src.height());
        let buf: Vec<Rgb<u8>> = src.sub_image(x, y, w, h).pixels().collect();
        encode_rgb8(
            ImgVec::new(buf, w, h).as_ref(),
            None,
            &enc,
            &Unstoppable,
            &Unstoppable,
        )
    }
    .map_err(|e| format!("encode {output}: {e:?}"))?;

    std::fs::write(output, png).map_err(|e| format!("write {output}: {e}"))?;
    Ok(())
}
