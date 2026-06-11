//! Empirical validation of the curated sweep axes (`zenpng::sweep`) —
//! playbook patterns 6 + 14 + 15 (`zenjpeg/docs/VARIANT_GENERATION.md`).
//!
//! PNG is lossless, so no perceptual metric is involved: the gates are
//! per-cell decodability (pattern 14), EXACT pixel roundtrip
//! (zero-tolerance as a hard gate), and step liveness (every curated
//! tier must change output bytes vs the default somewhere). The corpus
//! is synthetic (noise / checkerboard / palette-ish bands / tiny) plus
//! an odd 509×381 leg — PNG has no block grid, but odd widths exercise
//! the per-scanline filter/bpp edge paths (pattern 15's spirit).

use imgref::ImgRef;
use rgb::Rgb;
use zenpng::sweep::{SweepAxes, plan};
use zenpng::{DowncastFlags, PngDecodeConfig, decode, encode_rgb8};

struct Image {
    name: &'static str,
    w: usize,
    h: usize,
    rgb: Vec<u8>,
}

fn xorshift_noise(w: usize, h: usize, mut state: u32) -> Vec<u8> {
    (0..w * h * 3)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 24) as u8
        })
        .collect()
}

fn checkerboard(w: usize, h: usize, block: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let c = if ((x / block) + (y / block)).is_multiple_of(2) {
                255
            } else {
                0
            };
            v.extend_from_slice(&[c, c, c]);
        }
    }
    v
}

/// Banded low-color content (palette-friendly — the regime where the
/// strong tiers' filter/strategy search actually has choices).
fn bands(w: usize, h: usize) -> Vec<u8> {
    let palette: [[u8; 3]; 6] = [
        [220, 50, 47],
        [38, 139, 210],
        [133, 153, 0],
        [181, 137, 0],
        [42, 161, 152],
        [253, 246, 227],
    ];
    let mut v = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let p = palette[((x / 17) + (y / 23)) % palette.len()];
            v.extend_from_slice(&p);
        }
    }
    v
}

fn corpus() -> Vec<Image> {
    vec![
        Image {
            name: "noise256",
            w: 256,
            h: 256,
            rgb: xorshift_noise(256, 256, 0x9e37_79b9),
        },
        Image {
            name: "checker256",
            w: 256,
            h: 256,
            rgb: checkerboard(256, 256, 8),
        },
        Image {
            name: "bands256",
            w: 256,
            h: 256,
            rgb: bands(256, 256),
        },
        Image {
            name: "odd509x381",
            w: 509,
            h: 381,
            rgb: bands(509, 381),
        },
        Image {
            name: "tiny48",
            w: 48,
            h: 48,
            rgb: xorshift_noise(48, 48, 0x1234_5678),
        },
    ]
}

#[test]
fn sweep_cells_decode_exactly_and_steps_are_live() {
    let p = plan(&SweepAxes::modes_full());
    assert_eq!(p.cells[0].id, "png-balanced");
    let images = corpus();
    let mut failures: Vec<String> = Vec::new();
    // bytes[cell][image]
    let mut bytes: Vec<Vec<usize>> = Vec::new();

    for cell in &p.cells {
        // Downcasting (RGB -> gray/palette when representable) changes the
        // decoded buffer FORMAT, not the pixels; disable it so the exact-
        // roundtrip comparison stays byte-shaped. (The sweep variant's
        // own identity is unaffected — downcast is content-dependent
        // output negotiation, not a curated axis.)
        let cfg = cell.variant.build().with_downcast(DowncastFlags::none());
        let mut row = Vec::new();
        for img in &images {
            let pixels: &[Rgb<u8>] = bytemuck::cast_slice(&img.rgb);
            let imgr = ImgRef::new(pixels, img.w, img.h);
            let png = encode_rgb8(imgr, None, &cfg, &enough::Unstoppable, &enough::Unstoppable)
                .unwrap_or_else(|e| panic!("encode {} on {}: {e:?}", cell.id, img.name));
            // Pattern 14: every cell must decode…
            let out = decode(&png, &PngDecodeConfig::default(), &enough::Unstoppable)
                .unwrap_or_else(|e| panic!("UNDECODABLE {} on {}: {e:?}", cell.id, img.name));
            // …and roundtrip EXACTLY (zero-tolerance rule as a gate).
            let decoded = out.pixels.copy_to_contiguous_bytes();
            if decoded != img.rgb {
                failures.push(format!(
                    "LOSSLESS ROUNDTRIP MISMATCH: {} on {}",
                    cell.id, img.name
                ));
            }
            row.push(png.len());
        }
        bytes.push(row);
    }

    // Step liveness: every non-default tier must change bytes vs the
    // default somewhere in the corpus.
    for (ci, cell) in p.cells.iter().enumerate().skip(1) {
        if bytes[ci] == bytes[0] {
            failures.push(format!(
                "INERT STEP: {} byte-matched png-balanced on every image",
                cell.id
            ));
        }
    }
    // Soft sanity, hard-checked at the extremes: None is the largest
    // tier everywhere; Intense never loses to Fastest on compressible
    // content.
    let idx = |id: &str| p.cells.iter().position(|c| c.id == id).unwrap();
    let (none_i, fastest_i, intense_i) = (idx("png-none"), idx("png-fastest"), idx("png-intense"));
    for (ii, img) in images.iter().enumerate() {
        if bytes[none_i][ii] < bytes[fastest_i][ii] {
            failures.push(format!("uncompressed smaller than fastest on {}", img.name));
        }
    }
    let bands_i = images.iter().position(|i| i.name == "bands256").unwrap();
    if bytes[intense_i][bands_i] > bytes[fastest_i][bands_i] {
        failures.push("intense lost to fastest on palette-friendly content".into());
    }

    assert!(
        failures.is_empty(),
        "{} hard failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
