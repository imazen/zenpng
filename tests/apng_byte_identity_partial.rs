//! Byte-identity oracle for APNG frames that change only part of the canvas.
//!
//! `apng_byte_identity.rs` changes every pixel of every frame, so its
//! encodes never take a BLEND_OP_OVER frame or a background dispose. This
//! matrix animates sprites over static and transparent backgrounds, with
//! noisy RGB under alpha 0, identical consecutive frames and padded rows,
//! and asserts the output against goldens captured from `479a656` (before the
//! 2026-10-15 APNG performance work). It also asserts the matrix reaches the
//! OVER blend and a non-NONE dispose, so it cannot silently go vacuous.

#![cfg(not(target_arch = "wasm32"))]

use sha2::Digest;
use zencodec::encode::{DynAnimationFrameEncoder, EncodeJob, EncoderConfig};
use zenpixels::{PixelDescriptor, PixelSlice};
use zenpng::{Compression, DowncastFlags, PngEncoderConfig};

/// A small deterministic hash, for noise that does not depend on any crate.
fn noise(x: u32, y: u32, salt: u32) -> u8 {
    let mut h =
        x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ salt.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h as u8
}

/// One animation: canvas size, row padding in pixels, and the RGBA8 value of
/// pixel `(x, y)` in frame `f`.
struct Anim {
    name: &'static str,
    w: u32,
    h: u32,
    pad: u32,
    frames: u32,
    px: fn(u32, u32, u32) -> [u8; 4],
}

/// An opaque 12×10 sprite moving 5 columns and 2 rows per frame.
fn in_sprite(x: u32, y: u32, f: u32) -> bool {
    let (sx, sy) = (3 + f * 5, 4 + f * 2);
    (sx..sx + 12).contains(&x) && (sy..sy + 10).contains(&y)
}

fn sprite_color(x: u32, y: u32) -> [u8; 4] {
    [200, (x * 9) as u8, (y * 13) as u8, 255]
}

const ANIMS: &[Anim] = &[
    // Opaque sprite over a static opaque gradient.
    Anim {
        name: "sprite_on_gradient",
        w: 64,
        h: 48,
        pad: 0,
        frames: 6,
        px: |x, y, f| {
            if in_sprite(x, y, f) {
                sprite_color(x, y)
            } else {
                [(x * 4) as u8, (y * 5) as u8, 90, 255]
            }
        },
    },
    // Opaque sprite over a fully transparent canvas whose alpha-0 pixels
    // carry noisy RGB that changes every frame.
    Anim {
        name: "sprite_on_noisy_transparent",
        w: 48,
        h: 32,
        pad: 0,
        frames: 5,
        px: |x, y, f| {
            if in_sprite(x, y, f) {
                sprite_color(x, y)
            } else {
                [noise(x, y, f), noise(y, x, f + 7), noise(x ^ y, f, 3), 0]
            }
        },
    },
    // The same with constant (smooth) RGB under alpha 0.
    Anim {
        name: "sprite_on_smooth_transparent",
        w: 48,
        h: 32,
        pad: 0,
        frames: 5,
        px: |x, y, f| {
            if in_sprite(x, y, f) {
                sprite_color(x, y)
            } else {
                [(x * 3) as u8, (y * 4) as u8, 77, 0]
            }
        },
    },
    // A half-transparent sprite on a transparent canvas, and a fading
    // alpha-0 trail where it was.
    Anim {
        name: "semi_sprite_trail",
        w: 40,
        h: 40,
        pad: 0,
        frames: 6,
        px: |x, y, f| {
            if in_sprite(x, y, f) {
                [40, 220, (x * 5) as u8, 128]
            } else if f > 0 && in_sprite(x, y, f - 1) {
                [noise(x, y, f), 9, 9, 0]
            } else {
                [0, 0, 0, 0]
            }
        },
    },
    // A sprite that blinks on a transparent canvas: every other frame is
    // empty, so disposing the sprite's region is the natural encoding.
    Anim {
        name: "blinking_sprite",
        w: 48,
        h: 40,
        pad: 0,
        frames: 7,
        px: |x, y, f| {
            if f % 2 == 0 && in_sprite(x, y, f / 2) {
                sprite_color(x, y)
            } else {
                [0, 0, 0, 0]
            }
        },
    },
    // Identical consecutive frames, with one change in the middle.
    Anim {
        name: "mostly_identical",
        w: 32,
        h: 24,
        pad: 0,
        frames: 5,
        px: |x, y, f| {
            let lit = f == 2 && (5..9).contains(&x) && (5..9).contains(&y);
            [
                if lit { 255 } else { (x * 7) as u8 },
                (y * 9) as u8,
                30,
                if x % 5 == 0 { 0 } else { 255 },
            ]
        },
    },
    // Odd size and padded rows.
    Anim {
        name: "odd_padded",
        w: 37,
        h: 23,
        pad: 3,
        frames: 4,
        px: |x, y, f| {
            if in_sprite(x, y, f) {
                sprite_color(x, y)
            } else {
                [
                    noise(x, y, 1),
                    (y * 11) as u8,
                    50,
                    if (x + y) % 7 == 0 { 0 } else { 255 },
                ]
            }
        },
    },
];

/// The frames of `a` in `desc` (RGB8, RGBA8 or BGRA8), rows padded by
/// `a.pad` pixels of junk.
fn frames(a: &Anim, desc: PixelDescriptor) -> (Vec<Vec<u8>>, usize) {
    let bpp = desc.bytes_per_pixel();
    let stride = (a.w + a.pad) as usize * bpp;
    let fs = (0..a.frames)
        .map(|f| {
            let mut out = vec![0xA5u8; stride * a.h as usize];
            for y in 0..a.h {
                for x in 0..a.w {
                    let [r, g, b, al] = (a.px)(x, y, f);
                    let o = y as usize * stride + x as usize * bpp;
                    let px: &[u8] = match desc.layout() {
                        zenpixels::ChannelLayout::Rgb => &[r, g, b],
                        zenpixels::ChannelLayout::Rgba => &[r, g, b, al],
                        zenpixels::ChannelLayout::Bgra => &[b, g, r, al],
                        _ => panic!("test layout"),
                    };
                    out[o..o + bpp].copy_from_slice(px);
                }
            }
            out
        })
        .collect();
    (fs, stride)
}

fn encode(cfg: &PngEncoderConfig, a: &Anim, desc: PixelDescriptor, begin: bool) -> Vec<u8> {
    let (fs, stride) = frames(a, desc);
    let mut enc: Box<dyn DynAnimationFrameEncoder> = cfg
        .clone()
        .job()
        .with_loop_count(Some(0))
        .dyn_animation_frame_encoder()
        .unwrap();
    if begin {
        enc.begin(a.w, a.h, desc, Some(a.frames)).unwrap();
    }
    let mut out = Vec::new();
    for (i, f) in fs.iter().enumerate() {
        enc.push_frame(
            PixelSlice::new(f, a.w, a.h, stride, desc).unwrap(),
            50 + i as u32,
            None,
        )
        .unwrap();
        enc.take_output(&mut out);
    }
    out.extend_from_slice(enc.finish(None).unwrap().data());
    out
}

/// `(dispose_op, blend_op)` of every `fcTL` in a PNG stream.
fn frame_ops(png: &[u8]) -> Vec<(u8, u8)> {
    let mut ops = Vec::new();
    let mut i = 8;
    while i + 8 <= png.len() {
        let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
        if &png[i + 4..i + 8] == b"fcTL" {
            let d = &png[i + 8..i + 8 + len];
            ops.push((d[24], d[25]));
        }
        i += 12 + len;
    }
    ops
}

fn sha(data: &[u8]) -> String {
    let mut h = sha2::Sha256::new();
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// `(case name, sha256)` per encode, and every frame's `(dispose, blend)`.
type MatrixRun = (Vec<(String, String)>, Vec<(u8, u8)>);

fn run_matrix() -> MatrixRun {
    let mut out = Vec::new();
    let mut ops = Vec::new();
    for a in ANIMS {
        for (dname, desc) in [
            ("rgb8", PixelDescriptor::RGB8_SRGB),
            ("rgba8", PixelDescriptor::RGBA8_SRGB),
            ("bgra8", PixelDescriptor::BGRA8_SRGB),
        ] {
            for effort in [None, Some(2), Some(3), Some(5), Some(7), Some(10)] {
                for (cname, downcast) in [
                    ("dcdefault", DowncastFlags::default()),
                    ("dcnone", DowncastFlags::none()),
                ] {
                    for begin in [false, true] {
                        let mut c = PngEncoderConfig::new().with_downcast(downcast);
                        if let Some(e) = effort {
                            c = c.with_compression(Compression::Effort(e));
                        }
                        let bytes = encode(&c, a, desc, begin);
                        ops.extend(frame_ops(&bytes));
                        let e = effort.map_or("default".to_string(), |e| format!("e{e}"));
                        let b = if begin { "begin" } else { "nobegin" };
                        out.push((format!("{}/{dname}/{e}/{cname}/{b}", a.name), sha(&bytes)));
                    }
                }
            }
        }
    }
    (out, ops)
}

#[test]
fn partial_change_apng_output_is_byte_identical() {
    let (got, ops) = run_matrix();
    // Regenerate: APNG_GOLDEN=/path cargo test --test apng_byte_identity_partial -- --nocapture
    if let Ok(path) = std::env::var("APNG_GOLDEN") {
        let mut text = String::new();
        for (name, hash) in &got {
            text.push_str(&format!("(\"{name}\", \"{hash}\"),\n"));
        }
        std::fs::write(&path, &text).unwrap();
    }
    // The matrix must reach the paths it exists to cover: OVER frames, and
    // frames disposed to something other than NONE. (The optimizer prefers
    // DISPOSE_OP_PREVIOUS here; none of these animations draws a
    // DISPOSE_OP_BACKGROUND frame from it, measured at 479a656.)
    const DISPOSE_NONE: u8 = 0;
    const BLEND_OVER: u8 = 1;
    let over = ops.iter().filter(|o| o.1 == BLEND_OVER).count();
    let disposed = ops.iter().filter(|o| o.0 != DISPOSE_NONE).count();
    assert!(over > 0, "no BLEND_OP_OVER frame in the matrix");
    assert!(
        disposed > 0,
        "no frame disposed to BACKGROUND or PREVIOUS in the matrix"
    );

    let expected: std::collections::BTreeMap<&str, &str> = EXPECTED.iter().copied().collect();
    let mut changed = Vec::new();
    for (name, hash) in &got {
        match expected.get(name.as_str()) {
            Some(&want) if want == hash => {}
            Some(_) => changed.push(name.clone()),
            None => changed.push(format!("{name} (no golden)")),
        }
    }
    assert!(
        changed.is_empty(),
        "APNG bytes changed in {} of {} cases: {changed:#?}",
        changed.len(),
        got.len()
    );
    assert_eq!(got.len(), expected.len(), "golden table size drifted");
}

const EXPECTED: &[(&str, &str)] = &[
    (
        "sprite_on_gradient/rgb8/default/dcdefault/nobegin",
        "e63d3a1d079712d64d0f4068fa10a793d9d424c6de6eb272eb369d1de960986e",
    ),
    (
        "sprite_on_gradient/rgb8/default/dcdefault/begin",
        "e63d3a1d079712d64d0f4068fa10a793d9d424c6de6eb272eb369d1de960986e",
    ),
    (
        "sprite_on_gradient/rgb8/default/dcnone/nobegin",
        "e63d3a1d079712d64d0f4068fa10a793d9d424c6de6eb272eb369d1de960986e",
    ),
    (
        "sprite_on_gradient/rgb8/default/dcnone/begin",
        "e63d3a1d079712d64d0f4068fa10a793d9d424c6de6eb272eb369d1de960986e",
    ),
    (
        "sprite_on_gradient/rgb8/e2/dcdefault/nobegin",
        "c98b46b76045c19971e38fd1f43a606e2aab613bbc163cbf08fb690b7549b4f4",
    ),
    (
        "sprite_on_gradient/rgb8/e2/dcdefault/begin",
        "c98b46b76045c19971e38fd1f43a606e2aab613bbc163cbf08fb690b7549b4f4",
    ),
    (
        "sprite_on_gradient/rgb8/e2/dcnone/nobegin",
        "c98b46b76045c19971e38fd1f43a606e2aab613bbc163cbf08fb690b7549b4f4",
    ),
    (
        "sprite_on_gradient/rgb8/e2/dcnone/begin",
        "c98b46b76045c19971e38fd1f43a606e2aab613bbc163cbf08fb690b7549b4f4",
    ),
    (
        "sprite_on_gradient/rgb8/e3/dcdefault/nobegin",
        "fa8b1e3825943f4d2211caa5241eec32527cce06c12119e1c76a8c2d2db7e1f3",
    ),
    (
        "sprite_on_gradient/rgb8/e3/dcdefault/begin",
        "fa8b1e3825943f4d2211caa5241eec32527cce06c12119e1c76a8c2d2db7e1f3",
    ),
    (
        "sprite_on_gradient/rgb8/e3/dcnone/nobegin",
        "fa8b1e3825943f4d2211caa5241eec32527cce06c12119e1c76a8c2d2db7e1f3",
    ),
    (
        "sprite_on_gradient/rgb8/e3/dcnone/begin",
        "fa8b1e3825943f4d2211caa5241eec32527cce06c12119e1c76a8c2d2db7e1f3",
    ),
    (
        "sprite_on_gradient/rgb8/e5/dcdefault/nobegin",
        "536920e8dabff4c2a8cd622699881e43b3e1e826b112dd9504c7b5bec7b93734",
    ),
    (
        "sprite_on_gradient/rgb8/e5/dcdefault/begin",
        "536920e8dabff4c2a8cd622699881e43b3e1e826b112dd9504c7b5bec7b93734",
    ),
    (
        "sprite_on_gradient/rgb8/e5/dcnone/nobegin",
        "536920e8dabff4c2a8cd622699881e43b3e1e826b112dd9504c7b5bec7b93734",
    ),
    (
        "sprite_on_gradient/rgb8/e5/dcnone/begin",
        "536920e8dabff4c2a8cd622699881e43b3e1e826b112dd9504c7b5bec7b93734",
    ),
    (
        "sprite_on_gradient/rgb8/e7/dcdefault/nobegin",
        "4ee81de00a7c5517230a73cc549afadb58494640517c7909a67e43e8c6cd6835",
    ),
    (
        "sprite_on_gradient/rgb8/e7/dcdefault/begin",
        "4ee81de00a7c5517230a73cc549afadb58494640517c7909a67e43e8c6cd6835",
    ),
    (
        "sprite_on_gradient/rgb8/e7/dcnone/nobegin",
        "4ee81de00a7c5517230a73cc549afadb58494640517c7909a67e43e8c6cd6835",
    ),
    (
        "sprite_on_gradient/rgb8/e7/dcnone/begin",
        "4ee81de00a7c5517230a73cc549afadb58494640517c7909a67e43e8c6cd6835",
    ),
    (
        "sprite_on_gradient/rgb8/e10/dcdefault/nobegin",
        "d4a3d4062554177234ca2836139b8cb4b50988ba2d9adbba6614b836b20c9be4",
    ),
    (
        "sprite_on_gradient/rgb8/e10/dcdefault/begin",
        "d4a3d4062554177234ca2836139b8cb4b50988ba2d9adbba6614b836b20c9be4",
    ),
    (
        "sprite_on_gradient/rgb8/e10/dcnone/nobegin",
        "d4a3d4062554177234ca2836139b8cb4b50988ba2d9adbba6614b836b20c9be4",
    ),
    (
        "sprite_on_gradient/rgb8/e10/dcnone/begin",
        "d4a3d4062554177234ca2836139b8cb4b50988ba2d9adbba6614b836b20c9be4",
    ),
    (
        "sprite_on_gradient/rgba8/default/dcdefault/nobegin",
        "e63d3a1d079712d64d0f4068fa10a793d9d424c6de6eb272eb369d1de960986e",
    ),
    (
        "sprite_on_gradient/rgba8/default/dcdefault/begin",
        "e63d3a1d079712d64d0f4068fa10a793d9d424c6de6eb272eb369d1de960986e",
    ),
    (
        "sprite_on_gradient/rgba8/default/dcnone/nobegin",
        "70244705d4a0f0130871fff56149e60231c921b01cee3f4f46de8ecc378fb08d",
    ),
    (
        "sprite_on_gradient/rgba8/default/dcnone/begin",
        "70244705d4a0f0130871fff56149e60231c921b01cee3f4f46de8ecc378fb08d",
    ),
    (
        "sprite_on_gradient/rgba8/e2/dcdefault/nobegin",
        "c98b46b76045c19971e38fd1f43a606e2aab613bbc163cbf08fb690b7549b4f4",
    ),
    (
        "sprite_on_gradient/rgba8/e2/dcdefault/begin",
        "c98b46b76045c19971e38fd1f43a606e2aab613bbc163cbf08fb690b7549b4f4",
    ),
    (
        "sprite_on_gradient/rgba8/e2/dcnone/nobegin",
        "159b6c3c9a5df756937012ead9b802f94418eb576569a1b0ef913d8682db91c1",
    ),
    (
        "sprite_on_gradient/rgba8/e2/dcnone/begin",
        "159b6c3c9a5df756937012ead9b802f94418eb576569a1b0ef913d8682db91c1",
    ),
    (
        "sprite_on_gradient/rgba8/e3/dcdefault/nobegin",
        "fa8b1e3825943f4d2211caa5241eec32527cce06c12119e1c76a8c2d2db7e1f3",
    ),
    (
        "sprite_on_gradient/rgba8/e3/dcdefault/begin",
        "fa8b1e3825943f4d2211caa5241eec32527cce06c12119e1c76a8c2d2db7e1f3",
    ),
    (
        "sprite_on_gradient/rgba8/e3/dcnone/nobegin",
        "65b7fc03671c1bb00229df437f3d2c1bd5ec3c93fdef0b90074f442fdd8b1dad",
    ),
    (
        "sprite_on_gradient/rgba8/e3/dcnone/begin",
        "65b7fc03671c1bb00229df437f3d2c1bd5ec3c93fdef0b90074f442fdd8b1dad",
    ),
    (
        "sprite_on_gradient/rgba8/e5/dcdefault/nobegin",
        "536920e8dabff4c2a8cd622699881e43b3e1e826b112dd9504c7b5bec7b93734",
    ),
    (
        "sprite_on_gradient/rgba8/e5/dcdefault/begin",
        "536920e8dabff4c2a8cd622699881e43b3e1e826b112dd9504c7b5bec7b93734",
    ),
    (
        "sprite_on_gradient/rgba8/e5/dcnone/nobegin",
        "25da092e2751eedda05d05fad668133755ef8cc454a47632d0e53ed0eed11c05",
    ),
    (
        "sprite_on_gradient/rgba8/e5/dcnone/begin",
        "25da092e2751eedda05d05fad668133755ef8cc454a47632d0e53ed0eed11c05",
    ),
    (
        "sprite_on_gradient/rgba8/e7/dcdefault/nobegin",
        "4ee81de00a7c5517230a73cc549afadb58494640517c7909a67e43e8c6cd6835",
    ),
    (
        "sprite_on_gradient/rgba8/e7/dcdefault/begin",
        "4ee81de00a7c5517230a73cc549afadb58494640517c7909a67e43e8c6cd6835",
    ),
    (
        "sprite_on_gradient/rgba8/e7/dcnone/nobegin",
        "7e0d60969b154e6ea8ea1e748cf8b1a1250baaee944f6ff7c2d1ef1c52809b28",
    ),
    (
        "sprite_on_gradient/rgba8/e7/dcnone/begin",
        "7e0d60969b154e6ea8ea1e748cf8b1a1250baaee944f6ff7c2d1ef1c52809b28",
    ),
    (
        "sprite_on_gradient/rgba8/e10/dcdefault/nobegin",
        "d4a3d4062554177234ca2836139b8cb4b50988ba2d9adbba6614b836b20c9be4",
    ),
    (
        "sprite_on_gradient/rgba8/e10/dcdefault/begin",
        "d4a3d4062554177234ca2836139b8cb4b50988ba2d9adbba6614b836b20c9be4",
    ),
    (
        "sprite_on_gradient/rgba8/e10/dcnone/nobegin",
        "af824130bc218a4af8666716a8521f84e7273517fb0758ab871aa9e9c88b3dd0",
    ),
    (
        "sprite_on_gradient/rgba8/e10/dcnone/begin",
        "af824130bc218a4af8666716a8521f84e7273517fb0758ab871aa9e9c88b3dd0",
    ),
    (
        "sprite_on_gradient/bgra8/default/dcdefault/nobegin",
        "e63d3a1d079712d64d0f4068fa10a793d9d424c6de6eb272eb369d1de960986e",
    ),
    (
        "sprite_on_gradient/bgra8/default/dcdefault/begin",
        "e63d3a1d079712d64d0f4068fa10a793d9d424c6de6eb272eb369d1de960986e",
    ),
    (
        "sprite_on_gradient/bgra8/default/dcnone/nobegin",
        "70244705d4a0f0130871fff56149e60231c921b01cee3f4f46de8ecc378fb08d",
    ),
    (
        "sprite_on_gradient/bgra8/default/dcnone/begin",
        "70244705d4a0f0130871fff56149e60231c921b01cee3f4f46de8ecc378fb08d",
    ),
    (
        "sprite_on_gradient/bgra8/e2/dcdefault/nobegin",
        "c98b46b76045c19971e38fd1f43a606e2aab613bbc163cbf08fb690b7549b4f4",
    ),
    (
        "sprite_on_gradient/bgra8/e2/dcdefault/begin",
        "c98b46b76045c19971e38fd1f43a606e2aab613bbc163cbf08fb690b7549b4f4",
    ),
    (
        "sprite_on_gradient/bgra8/e2/dcnone/nobegin",
        "159b6c3c9a5df756937012ead9b802f94418eb576569a1b0ef913d8682db91c1",
    ),
    (
        "sprite_on_gradient/bgra8/e2/dcnone/begin",
        "159b6c3c9a5df756937012ead9b802f94418eb576569a1b0ef913d8682db91c1",
    ),
    (
        "sprite_on_gradient/bgra8/e3/dcdefault/nobegin",
        "fa8b1e3825943f4d2211caa5241eec32527cce06c12119e1c76a8c2d2db7e1f3",
    ),
    (
        "sprite_on_gradient/bgra8/e3/dcdefault/begin",
        "fa8b1e3825943f4d2211caa5241eec32527cce06c12119e1c76a8c2d2db7e1f3",
    ),
    (
        "sprite_on_gradient/bgra8/e3/dcnone/nobegin",
        "65b7fc03671c1bb00229df437f3d2c1bd5ec3c93fdef0b90074f442fdd8b1dad",
    ),
    (
        "sprite_on_gradient/bgra8/e3/dcnone/begin",
        "65b7fc03671c1bb00229df437f3d2c1bd5ec3c93fdef0b90074f442fdd8b1dad",
    ),
    (
        "sprite_on_gradient/bgra8/e5/dcdefault/nobegin",
        "536920e8dabff4c2a8cd622699881e43b3e1e826b112dd9504c7b5bec7b93734",
    ),
    (
        "sprite_on_gradient/bgra8/e5/dcdefault/begin",
        "536920e8dabff4c2a8cd622699881e43b3e1e826b112dd9504c7b5bec7b93734",
    ),
    (
        "sprite_on_gradient/bgra8/e5/dcnone/nobegin",
        "25da092e2751eedda05d05fad668133755ef8cc454a47632d0e53ed0eed11c05",
    ),
    (
        "sprite_on_gradient/bgra8/e5/dcnone/begin",
        "25da092e2751eedda05d05fad668133755ef8cc454a47632d0e53ed0eed11c05",
    ),
    (
        "sprite_on_gradient/bgra8/e7/dcdefault/nobegin",
        "4ee81de00a7c5517230a73cc549afadb58494640517c7909a67e43e8c6cd6835",
    ),
    (
        "sprite_on_gradient/bgra8/e7/dcdefault/begin",
        "4ee81de00a7c5517230a73cc549afadb58494640517c7909a67e43e8c6cd6835",
    ),
    (
        "sprite_on_gradient/bgra8/e7/dcnone/nobegin",
        "7e0d60969b154e6ea8ea1e748cf8b1a1250baaee944f6ff7c2d1ef1c52809b28",
    ),
    (
        "sprite_on_gradient/bgra8/e7/dcnone/begin",
        "7e0d60969b154e6ea8ea1e748cf8b1a1250baaee944f6ff7c2d1ef1c52809b28",
    ),
    (
        "sprite_on_gradient/bgra8/e10/dcdefault/nobegin",
        "d4a3d4062554177234ca2836139b8cb4b50988ba2d9adbba6614b836b20c9be4",
    ),
    (
        "sprite_on_gradient/bgra8/e10/dcdefault/begin",
        "d4a3d4062554177234ca2836139b8cb4b50988ba2d9adbba6614b836b20c9be4",
    ),
    (
        "sprite_on_gradient/bgra8/e10/dcnone/nobegin",
        "af824130bc218a4af8666716a8521f84e7273517fb0758ab871aa9e9c88b3dd0",
    ),
    (
        "sprite_on_gradient/bgra8/e10/dcnone/begin",
        "af824130bc218a4af8666716a8521f84e7273517fb0758ab871aa9e9c88b3dd0",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/default/dcdefault/nobegin",
        "cdcf385b7f4645c32ae194c0a2045e5d751524ac660e7c13b7cb95a0a049c587",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/default/dcdefault/begin",
        "cdcf385b7f4645c32ae194c0a2045e5d751524ac660e7c13b7cb95a0a049c587",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/default/dcnone/nobegin",
        "cdcf385b7f4645c32ae194c0a2045e5d751524ac660e7c13b7cb95a0a049c587",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/default/dcnone/begin",
        "cdcf385b7f4645c32ae194c0a2045e5d751524ac660e7c13b7cb95a0a049c587",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e2/dcdefault/nobegin",
        "50ce9fadce97f909f3102582d8942a2e4d7639de4a07d1f6f24faeaa7b0a6dfe",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e2/dcdefault/begin",
        "50ce9fadce97f909f3102582d8942a2e4d7639de4a07d1f6f24faeaa7b0a6dfe",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e2/dcnone/nobegin",
        "50ce9fadce97f909f3102582d8942a2e4d7639de4a07d1f6f24faeaa7b0a6dfe",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e2/dcnone/begin",
        "50ce9fadce97f909f3102582d8942a2e4d7639de4a07d1f6f24faeaa7b0a6dfe",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e3/dcdefault/nobegin",
        "50ce9fadce97f909f3102582d8942a2e4d7639de4a07d1f6f24faeaa7b0a6dfe",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e3/dcdefault/begin",
        "50ce9fadce97f909f3102582d8942a2e4d7639de4a07d1f6f24faeaa7b0a6dfe",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e3/dcnone/nobegin",
        "50ce9fadce97f909f3102582d8942a2e4d7639de4a07d1f6f24faeaa7b0a6dfe",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e3/dcnone/begin",
        "50ce9fadce97f909f3102582d8942a2e4d7639de4a07d1f6f24faeaa7b0a6dfe",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e5/dcdefault/nobegin",
        "4466de15ce04b4f0c77ace8bcc6b5eff6d479c2c8f3118056bf587ea9e49fd63",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e5/dcdefault/begin",
        "4466de15ce04b4f0c77ace8bcc6b5eff6d479c2c8f3118056bf587ea9e49fd63",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e5/dcnone/nobegin",
        "4466de15ce04b4f0c77ace8bcc6b5eff6d479c2c8f3118056bf587ea9e49fd63",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e5/dcnone/begin",
        "4466de15ce04b4f0c77ace8bcc6b5eff6d479c2c8f3118056bf587ea9e49fd63",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e7/dcdefault/nobegin",
        "db93c6372774408c70eb6da054a75b5d055beeb1242a4779a2518ab400779b8e",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e7/dcdefault/begin",
        "db93c6372774408c70eb6da054a75b5d055beeb1242a4779a2518ab400779b8e",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e7/dcnone/nobegin",
        "db93c6372774408c70eb6da054a75b5d055beeb1242a4779a2518ab400779b8e",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e7/dcnone/begin",
        "db93c6372774408c70eb6da054a75b5d055beeb1242a4779a2518ab400779b8e",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e10/dcdefault/nobegin",
        "4466de15ce04b4f0c77ace8bcc6b5eff6d479c2c8f3118056bf587ea9e49fd63",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e10/dcdefault/begin",
        "4466de15ce04b4f0c77ace8bcc6b5eff6d479c2c8f3118056bf587ea9e49fd63",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e10/dcnone/nobegin",
        "4466de15ce04b4f0c77ace8bcc6b5eff6d479c2c8f3118056bf587ea9e49fd63",
    ),
    (
        "sprite_on_noisy_transparent/rgb8/e10/dcnone/begin",
        "4466de15ce04b4f0c77ace8bcc6b5eff6d479c2c8f3118056bf587ea9e49fd63",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/default/dcdefault/nobegin",
        "ce858158fb37982a774bf4b499eeebab67ea6302b44ae632c553abcac1fa1269",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/default/dcdefault/begin",
        "ce858158fb37982a774bf4b499eeebab67ea6302b44ae632c553abcac1fa1269",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/default/dcnone/nobegin",
        "ce858158fb37982a774bf4b499eeebab67ea6302b44ae632c553abcac1fa1269",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/default/dcnone/begin",
        "ce858158fb37982a774bf4b499eeebab67ea6302b44ae632c553abcac1fa1269",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e2/dcdefault/nobegin",
        "e612aefe305c502c735df05d7d7edc018c6344a53d5e0013c2001fdd42aa40a6",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e2/dcdefault/begin",
        "e612aefe305c502c735df05d7d7edc018c6344a53d5e0013c2001fdd42aa40a6",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e2/dcnone/nobegin",
        "e612aefe305c502c735df05d7d7edc018c6344a53d5e0013c2001fdd42aa40a6",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e2/dcnone/begin",
        "e612aefe305c502c735df05d7d7edc018c6344a53d5e0013c2001fdd42aa40a6",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e3/dcdefault/nobegin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e3/dcdefault/begin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e3/dcnone/nobegin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e3/dcnone/begin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e5/dcdefault/nobegin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e5/dcdefault/begin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e5/dcnone/nobegin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e5/dcnone/begin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e7/dcdefault/nobegin",
        "1d893d2239eed933f34510b47e2488c537aede8aa820adfa6cf5b1583eace86d",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e7/dcdefault/begin",
        "1d893d2239eed933f34510b47e2488c537aede8aa820adfa6cf5b1583eace86d",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e7/dcnone/nobegin",
        "1d893d2239eed933f34510b47e2488c537aede8aa820adfa6cf5b1583eace86d",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e7/dcnone/begin",
        "1d893d2239eed933f34510b47e2488c537aede8aa820adfa6cf5b1583eace86d",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e10/dcdefault/nobegin",
        "69e6807cba94877fd17c60beb6a3effa5cbb5c579ff0afde5c1fc9812564a607",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e10/dcdefault/begin",
        "69e6807cba94877fd17c60beb6a3effa5cbb5c579ff0afde5c1fc9812564a607",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e10/dcnone/nobegin",
        "69e6807cba94877fd17c60beb6a3effa5cbb5c579ff0afde5c1fc9812564a607",
    ),
    (
        "sprite_on_noisy_transparent/rgba8/e10/dcnone/begin",
        "69e6807cba94877fd17c60beb6a3effa5cbb5c579ff0afde5c1fc9812564a607",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/default/dcdefault/nobegin",
        "ce858158fb37982a774bf4b499eeebab67ea6302b44ae632c553abcac1fa1269",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/default/dcdefault/begin",
        "ce858158fb37982a774bf4b499eeebab67ea6302b44ae632c553abcac1fa1269",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/default/dcnone/nobegin",
        "ce858158fb37982a774bf4b499eeebab67ea6302b44ae632c553abcac1fa1269",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/default/dcnone/begin",
        "ce858158fb37982a774bf4b499eeebab67ea6302b44ae632c553abcac1fa1269",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e2/dcdefault/nobegin",
        "e612aefe305c502c735df05d7d7edc018c6344a53d5e0013c2001fdd42aa40a6",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e2/dcdefault/begin",
        "e612aefe305c502c735df05d7d7edc018c6344a53d5e0013c2001fdd42aa40a6",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e2/dcnone/nobegin",
        "e612aefe305c502c735df05d7d7edc018c6344a53d5e0013c2001fdd42aa40a6",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e2/dcnone/begin",
        "e612aefe305c502c735df05d7d7edc018c6344a53d5e0013c2001fdd42aa40a6",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e3/dcdefault/nobegin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e3/dcdefault/begin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e3/dcnone/nobegin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e3/dcnone/begin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e5/dcdefault/nobegin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e5/dcdefault/begin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e5/dcnone/nobegin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e5/dcnone/begin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e7/dcdefault/nobegin",
        "1d893d2239eed933f34510b47e2488c537aede8aa820adfa6cf5b1583eace86d",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e7/dcdefault/begin",
        "1d893d2239eed933f34510b47e2488c537aede8aa820adfa6cf5b1583eace86d",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e7/dcnone/nobegin",
        "1d893d2239eed933f34510b47e2488c537aede8aa820adfa6cf5b1583eace86d",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e7/dcnone/begin",
        "1d893d2239eed933f34510b47e2488c537aede8aa820adfa6cf5b1583eace86d",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e10/dcdefault/nobegin",
        "69e6807cba94877fd17c60beb6a3effa5cbb5c579ff0afde5c1fc9812564a607",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e10/dcdefault/begin",
        "69e6807cba94877fd17c60beb6a3effa5cbb5c579ff0afde5c1fc9812564a607",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e10/dcnone/nobegin",
        "69e6807cba94877fd17c60beb6a3effa5cbb5c579ff0afde5c1fc9812564a607",
    ),
    (
        "sprite_on_noisy_transparent/bgra8/e10/dcnone/begin",
        "69e6807cba94877fd17c60beb6a3effa5cbb5c579ff0afde5c1fc9812564a607",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/default/dcdefault/nobegin",
        "2046b2674754b273dd67d8fe9ca900ce8c5a52b48599c85e4fedf18696b5b9f8",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/default/dcdefault/begin",
        "2046b2674754b273dd67d8fe9ca900ce8c5a52b48599c85e4fedf18696b5b9f8",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/default/dcnone/nobegin",
        "2046b2674754b273dd67d8fe9ca900ce8c5a52b48599c85e4fedf18696b5b9f8",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/default/dcnone/begin",
        "2046b2674754b273dd67d8fe9ca900ce8c5a52b48599c85e4fedf18696b5b9f8",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e2/dcdefault/nobegin",
        "fdbdf6f4a4cf4ffa4c30717ea8ed40e1b7be4f2eb113416f77aa7d9082d45148",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e2/dcdefault/begin",
        "fdbdf6f4a4cf4ffa4c30717ea8ed40e1b7be4f2eb113416f77aa7d9082d45148",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e2/dcnone/nobegin",
        "fdbdf6f4a4cf4ffa4c30717ea8ed40e1b7be4f2eb113416f77aa7d9082d45148",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e2/dcnone/begin",
        "fdbdf6f4a4cf4ffa4c30717ea8ed40e1b7be4f2eb113416f77aa7d9082d45148",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e3/dcdefault/nobegin",
        "5267f2a83625179670ada7a4df0cb686c54af10a90e9de3701fa858d52b50f28",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e3/dcdefault/begin",
        "5267f2a83625179670ada7a4df0cb686c54af10a90e9de3701fa858d52b50f28",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e3/dcnone/nobegin",
        "5267f2a83625179670ada7a4df0cb686c54af10a90e9de3701fa858d52b50f28",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e3/dcnone/begin",
        "5267f2a83625179670ada7a4df0cb686c54af10a90e9de3701fa858d52b50f28",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e5/dcdefault/nobegin",
        "68dfbc490915f16528d42680b02fd57098f055087b1ab457b09b716d005257bd",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e5/dcdefault/begin",
        "68dfbc490915f16528d42680b02fd57098f055087b1ab457b09b716d005257bd",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e5/dcnone/nobegin",
        "68dfbc490915f16528d42680b02fd57098f055087b1ab457b09b716d005257bd",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e5/dcnone/begin",
        "68dfbc490915f16528d42680b02fd57098f055087b1ab457b09b716d005257bd",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e7/dcdefault/nobegin",
        "e168fb099e2a4050aee0da94cf45be887256c6a79d3e0f9885d0a842435d9b59",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e7/dcdefault/begin",
        "e168fb099e2a4050aee0da94cf45be887256c6a79d3e0f9885d0a842435d9b59",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e7/dcnone/nobegin",
        "e168fb099e2a4050aee0da94cf45be887256c6a79d3e0f9885d0a842435d9b59",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e7/dcnone/begin",
        "e168fb099e2a4050aee0da94cf45be887256c6a79d3e0f9885d0a842435d9b59",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e10/dcdefault/nobegin",
        "32334e1e3f20baf9065049555b725a1570ee153f2d50c1bbf2ebbcedc8edecdb",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e10/dcdefault/begin",
        "32334e1e3f20baf9065049555b725a1570ee153f2d50c1bbf2ebbcedc8edecdb",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e10/dcnone/nobegin",
        "32334e1e3f20baf9065049555b725a1570ee153f2d50c1bbf2ebbcedc8edecdb",
    ),
    (
        "sprite_on_smooth_transparent/rgb8/e10/dcnone/begin",
        "32334e1e3f20baf9065049555b725a1570ee153f2d50c1bbf2ebbcedc8edecdb",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/default/dcdefault/nobegin",
        "ce858158fb37982a774bf4b499eeebab67ea6302b44ae632c553abcac1fa1269",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/default/dcdefault/begin",
        "ce858158fb37982a774bf4b499eeebab67ea6302b44ae632c553abcac1fa1269",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/default/dcnone/nobegin",
        "ce858158fb37982a774bf4b499eeebab67ea6302b44ae632c553abcac1fa1269",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/default/dcnone/begin",
        "ce858158fb37982a774bf4b499eeebab67ea6302b44ae632c553abcac1fa1269",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e2/dcdefault/nobegin",
        "3d63f659c4ee513231ca3b0b022bb3e97a5fd277658ef80447d5300474b3bf7c",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e2/dcdefault/begin",
        "3d63f659c4ee513231ca3b0b022bb3e97a5fd277658ef80447d5300474b3bf7c",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e2/dcnone/nobegin",
        "3d63f659c4ee513231ca3b0b022bb3e97a5fd277658ef80447d5300474b3bf7c",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e2/dcnone/begin",
        "3d63f659c4ee513231ca3b0b022bb3e97a5fd277658ef80447d5300474b3bf7c",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e3/dcdefault/nobegin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e3/dcdefault/begin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e3/dcnone/nobegin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e3/dcnone/begin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e5/dcdefault/nobegin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e5/dcdefault/begin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e5/dcnone/nobegin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e5/dcnone/begin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e7/dcdefault/nobegin",
        "1d893d2239eed933f34510b47e2488c537aede8aa820adfa6cf5b1583eace86d",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e7/dcdefault/begin",
        "1d893d2239eed933f34510b47e2488c537aede8aa820adfa6cf5b1583eace86d",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e7/dcnone/nobegin",
        "1d893d2239eed933f34510b47e2488c537aede8aa820adfa6cf5b1583eace86d",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e7/dcnone/begin",
        "1d893d2239eed933f34510b47e2488c537aede8aa820adfa6cf5b1583eace86d",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e10/dcdefault/nobegin",
        "69e6807cba94877fd17c60beb6a3effa5cbb5c579ff0afde5c1fc9812564a607",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e10/dcdefault/begin",
        "69e6807cba94877fd17c60beb6a3effa5cbb5c579ff0afde5c1fc9812564a607",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e10/dcnone/nobegin",
        "69e6807cba94877fd17c60beb6a3effa5cbb5c579ff0afde5c1fc9812564a607",
    ),
    (
        "sprite_on_smooth_transparent/rgba8/e10/dcnone/begin",
        "69e6807cba94877fd17c60beb6a3effa5cbb5c579ff0afde5c1fc9812564a607",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/default/dcdefault/nobegin",
        "ce858158fb37982a774bf4b499eeebab67ea6302b44ae632c553abcac1fa1269",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/default/dcdefault/begin",
        "ce858158fb37982a774bf4b499eeebab67ea6302b44ae632c553abcac1fa1269",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/default/dcnone/nobegin",
        "ce858158fb37982a774bf4b499eeebab67ea6302b44ae632c553abcac1fa1269",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/default/dcnone/begin",
        "ce858158fb37982a774bf4b499eeebab67ea6302b44ae632c553abcac1fa1269",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e2/dcdefault/nobegin",
        "3d63f659c4ee513231ca3b0b022bb3e97a5fd277658ef80447d5300474b3bf7c",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e2/dcdefault/begin",
        "3d63f659c4ee513231ca3b0b022bb3e97a5fd277658ef80447d5300474b3bf7c",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e2/dcnone/nobegin",
        "3d63f659c4ee513231ca3b0b022bb3e97a5fd277658ef80447d5300474b3bf7c",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e2/dcnone/begin",
        "3d63f659c4ee513231ca3b0b022bb3e97a5fd277658ef80447d5300474b3bf7c",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e3/dcdefault/nobegin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e3/dcdefault/begin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e3/dcnone/nobegin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e3/dcnone/begin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e5/dcdefault/nobegin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e5/dcdefault/begin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e5/dcnone/nobegin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e5/dcnone/begin",
        "105717502109aea09e090c224b5f74b56ebcd8834f2518998bdb7d2030144d0b",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e7/dcdefault/nobegin",
        "1d893d2239eed933f34510b47e2488c537aede8aa820adfa6cf5b1583eace86d",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e7/dcdefault/begin",
        "1d893d2239eed933f34510b47e2488c537aede8aa820adfa6cf5b1583eace86d",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e7/dcnone/nobegin",
        "1d893d2239eed933f34510b47e2488c537aede8aa820adfa6cf5b1583eace86d",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e7/dcnone/begin",
        "1d893d2239eed933f34510b47e2488c537aede8aa820adfa6cf5b1583eace86d",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e10/dcdefault/nobegin",
        "69e6807cba94877fd17c60beb6a3effa5cbb5c579ff0afde5c1fc9812564a607",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e10/dcdefault/begin",
        "69e6807cba94877fd17c60beb6a3effa5cbb5c579ff0afde5c1fc9812564a607",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e10/dcnone/nobegin",
        "69e6807cba94877fd17c60beb6a3effa5cbb5c579ff0afde5c1fc9812564a607",
    ),
    (
        "sprite_on_smooth_transparent/bgra8/e10/dcnone/begin",
        "69e6807cba94877fd17c60beb6a3effa5cbb5c579ff0afde5c1fc9812564a607",
    ),
    (
        "semi_sprite_trail/rgb8/default/dcdefault/nobegin",
        "7960145216e02216a1dba4eb1fc5e5735ec409fe28e95a8e6edf3b6222cae15e",
    ),
    (
        "semi_sprite_trail/rgb8/default/dcdefault/begin",
        "7960145216e02216a1dba4eb1fc5e5735ec409fe28e95a8e6edf3b6222cae15e",
    ),
    (
        "semi_sprite_trail/rgb8/default/dcnone/nobegin",
        "7960145216e02216a1dba4eb1fc5e5735ec409fe28e95a8e6edf3b6222cae15e",
    ),
    (
        "semi_sprite_trail/rgb8/default/dcnone/begin",
        "7960145216e02216a1dba4eb1fc5e5735ec409fe28e95a8e6edf3b6222cae15e",
    ),
    (
        "semi_sprite_trail/rgb8/e2/dcdefault/nobegin",
        "2cbe6c4fdcae81ce0a3e071d53b582fba7d467fcf2cda571b2cb518c37469ae9",
    ),
    (
        "semi_sprite_trail/rgb8/e2/dcdefault/begin",
        "2cbe6c4fdcae81ce0a3e071d53b582fba7d467fcf2cda571b2cb518c37469ae9",
    ),
    (
        "semi_sprite_trail/rgb8/e2/dcnone/nobegin",
        "2cbe6c4fdcae81ce0a3e071d53b582fba7d467fcf2cda571b2cb518c37469ae9",
    ),
    (
        "semi_sprite_trail/rgb8/e2/dcnone/begin",
        "2cbe6c4fdcae81ce0a3e071d53b582fba7d467fcf2cda571b2cb518c37469ae9",
    ),
    (
        "semi_sprite_trail/rgb8/e3/dcdefault/nobegin",
        "ef885d8c7523bd76b4ce5070b6bb368758f2448a26bc8434abeb10d498349822",
    ),
    (
        "semi_sprite_trail/rgb8/e3/dcdefault/begin",
        "ef885d8c7523bd76b4ce5070b6bb368758f2448a26bc8434abeb10d498349822",
    ),
    (
        "semi_sprite_trail/rgb8/e3/dcnone/nobegin",
        "ef885d8c7523bd76b4ce5070b6bb368758f2448a26bc8434abeb10d498349822",
    ),
    (
        "semi_sprite_trail/rgb8/e3/dcnone/begin",
        "ef885d8c7523bd76b4ce5070b6bb368758f2448a26bc8434abeb10d498349822",
    ),
    (
        "semi_sprite_trail/rgb8/e5/dcdefault/nobegin",
        "ef885d8c7523bd76b4ce5070b6bb368758f2448a26bc8434abeb10d498349822",
    ),
    (
        "semi_sprite_trail/rgb8/e5/dcdefault/begin",
        "ef885d8c7523bd76b4ce5070b6bb368758f2448a26bc8434abeb10d498349822",
    ),
    (
        "semi_sprite_trail/rgb8/e5/dcnone/nobegin",
        "ef885d8c7523bd76b4ce5070b6bb368758f2448a26bc8434abeb10d498349822",
    ),
    (
        "semi_sprite_trail/rgb8/e5/dcnone/begin",
        "ef885d8c7523bd76b4ce5070b6bb368758f2448a26bc8434abeb10d498349822",
    ),
    (
        "semi_sprite_trail/rgb8/e7/dcdefault/nobegin",
        "287d213acc4952fa98eeea771c19ab498d7f560d258f87d8442ced274a8e0f16",
    ),
    (
        "semi_sprite_trail/rgb8/e7/dcdefault/begin",
        "287d213acc4952fa98eeea771c19ab498d7f560d258f87d8442ced274a8e0f16",
    ),
    (
        "semi_sprite_trail/rgb8/e7/dcnone/nobegin",
        "287d213acc4952fa98eeea771c19ab498d7f560d258f87d8442ced274a8e0f16",
    ),
    (
        "semi_sprite_trail/rgb8/e7/dcnone/begin",
        "287d213acc4952fa98eeea771c19ab498d7f560d258f87d8442ced274a8e0f16",
    ),
    (
        "semi_sprite_trail/rgb8/e10/dcdefault/nobegin",
        "bda9996b1cca24e9c0c7397616a4aaf83175f775f459cca65ea5dc88835e22ec",
    ),
    (
        "semi_sprite_trail/rgb8/e10/dcdefault/begin",
        "bda9996b1cca24e9c0c7397616a4aaf83175f775f459cca65ea5dc88835e22ec",
    ),
    (
        "semi_sprite_trail/rgb8/e10/dcnone/nobegin",
        "bda9996b1cca24e9c0c7397616a4aaf83175f775f459cca65ea5dc88835e22ec",
    ),
    (
        "semi_sprite_trail/rgb8/e10/dcnone/begin",
        "bda9996b1cca24e9c0c7397616a4aaf83175f775f459cca65ea5dc88835e22ec",
    ),
    (
        "semi_sprite_trail/rgba8/default/dcdefault/nobegin",
        "5223c241e7077879c5dfbfdc84e34e507e0ab849381685bf77551dc47e04245d",
    ),
    (
        "semi_sprite_trail/rgba8/default/dcdefault/begin",
        "5223c241e7077879c5dfbfdc84e34e507e0ab849381685bf77551dc47e04245d",
    ),
    (
        "semi_sprite_trail/rgba8/default/dcnone/nobegin",
        "5223c241e7077879c5dfbfdc84e34e507e0ab849381685bf77551dc47e04245d",
    ),
    (
        "semi_sprite_trail/rgba8/default/dcnone/begin",
        "5223c241e7077879c5dfbfdc84e34e507e0ab849381685bf77551dc47e04245d",
    ),
    (
        "semi_sprite_trail/rgba8/e2/dcdefault/nobegin",
        "865a47aac6eec00e0812b71b97ecb9a729f9f8d6c451d986702f9d83dbd2c4c5",
    ),
    (
        "semi_sprite_trail/rgba8/e2/dcdefault/begin",
        "865a47aac6eec00e0812b71b97ecb9a729f9f8d6c451d986702f9d83dbd2c4c5",
    ),
    (
        "semi_sprite_trail/rgba8/e2/dcnone/nobegin",
        "865a47aac6eec00e0812b71b97ecb9a729f9f8d6c451d986702f9d83dbd2c4c5",
    ),
    (
        "semi_sprite_trail/rgba8/e2/dcnone/begin",
        "865a47aac6eec00e0812b71b97ecb9a729f9f8d6c451d986702f9d83dbd2c4c5",
    ),
    (
        "semi_sprite_trail/rgba8/e3/dcdefault/nobegin",
        "4ab77becbf46392e5b5610d1d9a2682a5619292372ddc66ea0ed9be0e01b833a",
    ),
    (
        "semi_sprite_trail/rgba8/e3/dcdefault/begin",
        "4ab77becbf46392e5b5610d1d9a2682a5619292372ddc66ea0ed9be0e01b833a",
    ),
    (
        "semi_sprite_trail/rgba8/e3/dcnone/nobegin",
        "4ab77becbf46392e5b5610d1d9a2682a5619292372ddc66ea0ed9be0e01b833a",
    ),
    (
        "semi_sprite_trail/rgba8/e3/dcnone/begin",
        "4ab77becbf46392e5b5610d1d9a2682a5619292372ddc66ea0ed9be0e01b833a",
    ),
    (
        "semi_sprite_trail/rgba8/e5/dcdefault/nobegin",
        "4ab77becbf46392e5b5610d1d9a2682a5619292372ddc66ea0ed9be0e01b833a",
    ),
    (
        "semi_sprite_trail/rgba8/e5/dcdefault/begin",
        "4ab77becbf46392e5b5610d1d9a2682a5619292372ddc66ea0ed9be0e01b833a",
    ),
    (
        "semi_sprite_trail/rgba8/e5/dcnone/nobegin",
        "4ab77becbf46392e5b5610d1d9a2682a5619292372ddc66ea0ed9be0e01b833a",
    ),
    (
        "semi_sprite_trail/rgba8/e5/dcnone/begin",
        "4ab77becbf46392e5b5610d1d9a2682a5619292372ddc66ea0ed9be0e01b833a",
    ),
    (
        "semi_sprite_trail/rgba8/e7/dcdefault/nobegin",
        "b0e6cb540b42ca6ddc09d7b700e60ccf8e414c67bf9af633b956c07ec25d55ea",
    ),
    (
        "semi_sprite_trail/rgba8/e7/dcdefault/begin",
        "b0e6cb540b42ca6ddc09d7b700e60ccf8e414c67bf9af633b956c07ec25d55ea",
    ),
    (
        "semi_sprite_trail/rgba8/e7/dcnone/nobegin",
        "b0e6cb540b42ca6ddc09d7b700e60ccf8e414c67bf9af633b956c07ec25d55ea",
    ),
    (
        "semi_sprite_trail/rgba8/e7/dcnone/begin",
        "b0e6cb540b42ca6ddc09d7b700e60ccf8e414c67bf9af633b956c07ec25d55ea",
    ),
    (
        "semi_sprite_trail/rgba8/e10/dcdefault/nobegin",
        "9621071563c7b606b2e28b8bfa9fe64e79603033538c77338cccfe27935e812b",
    ),
    (
        "semi_sprite_trail/rgba8/e10/dcdefault/begin",
        "9621071563c7b606b2e28b8bfa9fe64e79603033538c77338cccfe27935e812b",
    ),
    (
        "semi_sprite_trail/rgba8/e10/dcnone/nobegin",
        "9621071563c7b606b2e28b8bfa9fe64e79603033538c77338cccfe27935e812b",
    ),
    (
        "semi_sprite_trail/rgba8/e10/dcnone/begin",
        "9621071563c7b606b2e28b8bfa9fe64e79603033538c77338cccfe27935e812b",
    ),
    (
        "semi_sprite_trail/bgra8/default/dcdefault/nobegin",
        "5223c241e7077879c5dfbfdc84e34e507e0ab849381685bf77551dc47e04245d",
    ),
    (
        "semi_sprite_trail/bgra8/default/dcdefault/begin",
        "5223c241e7077879c5dfbfdc84e34e507e0ab849381685bf77551dc47e04245d",
    ),
    (
        "semi_sprite_trail/bgra8/default/dcnone/nobegin",
        "5223c241e7077879c5dfbfdc84e34e507e0ab849381685bf77551dc47e04245d",
    ),
    (
        "semi_sprite_trail/bgra8/default/dcnone/begin",
        "5223c241e7077879c5dfbfdc84e34e507e0ab849381685bf77551dc47e04245d",
    ),
    (
        "semi_sprite_trail/bgra8/e2/dcdefault/nobegin",
        "865a47aac6eec00e0812b71b97ecb9a729f9f8d6c451d986702f9d83dbd2c4c5",
    ),
    (
        "semi_sprite_trail/bgra8/e2/dcdefault/begin",
        "865a47aac6eec00e0812b71b97ecb9a729f9f8d6c451d986702f9d83dbd2c4c5",
    ),
    (
        "semi_sprite_trail/bgra8/e2/dcnone/nobegin",
        "865a47aac6eec00e0812b71b97ecb9a729f9f8d6c451d986702f9d83dbd2c4c5",
    ),
    (
        "semi_sprite_trail/bgra8/e2/dcnone/begin",
        "865a47aac6eec00e0812b71b97ecb9a729f9f8d6c451d986702f9d83dbd2c4c5",
    ),
    (
        "semi_sprite_trail/bgra8/e3/dcdefault/nobegin",
        "4ab77becbf46392e5b5610d1d9a2682a5619292372ddc66ea0ed9be0e01b833a",
    ),
    (
        "semi_sprite_trail/bgra8/e3/dcdefault/begin",
        "4ab77becbf46392e5b5610d1d9a2682a5619292372ddc66ea0ed9be0e01b833a",
    ),
    (
        "semi_sprite_trail/bgra8/e3/dcnone/nobegin",
        "4ab77becbf46392e5b5610d1d9a2682a5619292372ddc66ea0ed9be0e01b833a",
    ),
    (
        "semi_sprite_trail/bgra8/e3/dcnone/begin",
        "4ab77becbf46392e5b5610d1d9a2682a5619292372ddc66ea0ed9be0e01b833a",
    ),
    (
        "semi_sprite_trail/bgra8/e5/dcdefault/nobegin",
        "4ab77becbf46392e5b5610d1d9a2682a5619292372ddc66ea0ed9be0e01b833a",
    ),
    (
        "semi_sprite_trail/bgra8/e5/dcdefault/begin",
        "4ab77becbf46392e5b5610d1d9a2682a5619292372ddc66ea0ed9be0e01b833a",
    ),
    (
        "semi_sprite_trail/bgra8/e5/dcnone/nobegin",
        "4ab77becbf46392e5b5610d1d9a2682a5619292372ddc66ea0ed9be0e01b833a",
    ),
    (
        "semi_sprite_trail/bgra8/e5/dcnone/begin",
        "4ab77becbf46392e5b5610d1d9a2682a5619292372ddc66ea0ed9be0e01b833a",
    ),
    (
        "semi_sprite_trail/bgra8/e7/dcdefault/nobegin",
        "b0e6cb540b42ca6ddc09d7b700e60ccf8e414c67bf9af633b956c07ec25d55ea",
    ),
    (
        "semi_sprite_trail/bgra8/e7/dcdefault/begin",
        "b0e6cb540b42ca6ddc09d7b700e60ccf8e414c67bf9af633b956c07ec25d55ea",
    ),
    (
        "semi_sprite_trail/bgra8/e7/dcnone/nobegin",
        "b0e6cb540b42ca6ddc09d7b700e60ccf8e414c67bf9af633b956c07ec25d55ea",
    ),
    (
        "semi_sprite_trail/bgra8/e7/dcnone/begin",
        "b0e6cb540b42ca6ddc09d7b700e60ccf8e414c67bf9af633b956c07ec25d55ea",
    ),
    (
        "semi_sprite_trail/bgra8/e10/dcdefault/nobegin",
        "9621071563c7b606b2e28b8bfa9fe64e79603033538c77338cccfe27935e812b",
    ),
    (
        "semi_sprite_trail/bgra8/e10/dcdefault/begin",
        "9621071563c7b606b2e28b8bfa9fe64e79603033538c77338cccfe27935e812b",
    ),
    (
        "semi_sprite_trail/bgra8/e10/dcnone/nobegin",
        "9621071563c7b606b2e28b8bfa9fe64e79603033538c77338cccfe27935e812b",
    ),
    (
        "semi_sprite_trail/bgra8/e10/dcnone/begin",
        "9621071563c7b606b2e28b8bfa9fe64e79603033538c77338cccfe27935e812b",
    ),
    (
        "blinking_sprite/rgb8/default/dcdefault/nobegin",
        "b53d54fe77c77d0b9664a22695955156a3ad0fb4339d914a12830411de6df46c",
    ),
    (
        "blinking_sprite/rgb8/default/dcdefault/begin",
        "b53d54fe77c77d0b9664a22695955156a3ad0fb4339d914a12830411de6df46c",
    ),
    (
        "blinking_sprite/rgb8/default/dcnone/nobegin",
        "b53d54fe77c77d0b9664a22695955156a3ad0fb4339d914a12830411de6df46c",
    ),
    (
        "blinking_sprite/rgb8/default/dcnone/begin",
        "b53d54fe77c77d0b9664a22695955156a3ad0fb4339d914a12830411de6df46c",
    ),
    (
        "blinking_sprite/rgb8/e2/dcdefault/nobegin",
        "972ba822a432a31de9cae3595d84aafc430ada52e8b3c995dc1b85a15a9f91e9",
    ),
    (
        "blinking_sprite/rgb8/e2/dcdefault/begin",
        "972ba822a432a31de9cae3595d84aafc430ada52e8b3c995dc1b85a15a9f91e9",
    ),
    (
        "blinking_sprite/rgb8/e2/dcnone/nobegin",
        "972ba822a432a31de9cae3595d84aafc430ada52e8b3c995dc1b85a15a9f91e9",
    ),
    (
        "blinking_sprite/rgb8/e2/dcnone/begin",
        "972ba822a432a31de9cae3595d84aafc430ada52e8b3c995dc1b85a15a9f91e9",
    ),
    (
        "blinking_sprite/rgb8/e3/dcdefault/nobegin",
        "5285fd8a5abf243e2dbc297d3a4c9b6c510fe88b6334c8edf62bd039f5be1b63",
    ),
    (
        "blinking_sprite/rgb8/e3/dcdefault/begin",
        "5285fd8a5abf243e2dbc297d3a4c9b6c510fe88b6334c8edf62bd039f5be1b63",
    ),
    (
        "blinking_sprite/rgb8/e3/dcnone/nobegin",
        "5285fd8a5abf243e2dbc297d3a4c9b6c510fe88b6334c8edf62bd039f5be1b63",
    ),
    (
        "blinking_sprite/rgb8/e3/dcnone/begin",
        "5285fd8a5abf243e2dbc297d3a4c9b6c510fe88b6334c8edf62bd039f5be1b63",
    ),
    (
        "blinking_sprite/rgb8/e5/dcdefault/nobegin",
        "5285fd8a5abf243e2dbc297d3a4c9b6c510fe88b6334c8edf62bd039f5be1b63",
    ),
    (
        "blinking_sprite/rgb8/e5/dcdefault/begin",
        "5285fd8a5abf243e2dbc297d3a4c9b6c510fe88b6334c8edf62bd039f5be1b63",
    ),
    (
        "blinking_sprite/rgb8/e5/dcnone/nobegin",
        "5285fd8a5abf243e2dbc297d3a4c9b6c510fe88b6334c8edf62bd039f5be1b63",
    ),
    (
        "blinking_sprite/rgb8/e5/dcnone/begin",
        "5285fd8a5abf243e2dbc297d3a4c9b6c510fe88b6334c8edf62bd039f5be1b63",
    ),
    (
        "blinking_sprite/rgb8/e7/dcdefault/nobegin",
        "a8fc2993f788f8f41e0c034a195a732dd30b7f330a88c093f365d7a33c941048",
    ),
    (
        "blinking_sprite/rgb8/e7/dcdefault/begin",
        "a8fc2993f788f8f41e0c034a195a732dd30b7f330a88c093f365d7a33c941048",
    ),
    (
        "blinking_sprite/rgb8/e7/dcnone/nobegin",
        "a8fc2993f788f8f41e0c034a195a732dd30b7f330a88c093f365d7a33c941048",
    ),
    (
        "blinking_sprite/rgb8/e7/dcnone/begin",
        "a8fc2993f788f8f41e0c034a195a732dd30b7f330a88c093f365d7a33c941048",
    ),
    (
        "blinking_sprite/rgb8/e10/dcdefault/nobegin",
        "5285fd8a5abf243e2dbc297d3a4c9b6c510fe88b6334c8edf62bd039f5be1b63",
    ),
    (
        "blinking_sprite/rgb8/e10/dcdefault/begin",
        "5285fd8a5abf243e2dbc297d3a4c9b6c510fe88b6334c8edf62bd039f5be1b63",
    ),
    (
        "blinking_sprite/rgb8/e10/dcnone/nobegin",
        "5285fd8a5abf243e2dbc297d3a4c9b6c510fe88b6334c8edf62bd039f5be1b63",
    ),
    (
        "blinking_sprite/rgb8/e10/dcnone/begin",
        "5285fd8a5abf243e2dbc297d3a4c9b6c510fe88b6334c8edf62bd039f5be1b63",
    ),
    (
        "blinking_sprite/rgba8/default/dcdefault/nobegin",
        "192f97ad32498511aa1f776a8fc86caf4c90f2d5414f174ced71790cdf6f2086",
    ),
    (
        "blinking_sprite/rgba8/default/dcdefault/begin",
        "192f97ad32498511aa1f776a8fc86caf4c90f2d5414f174ced71790cdf6f2086",
    ),
    (
        "blinking_sprite/rgba8/default/dcnone/nobegin",
        "192f97ad32498511aa1f776a8fc86caf4c90f2d5414f174ced71790cdf6f2086",
    ),
    (
        "blinking_sprite/rgba8/default/dcnone/begin",
        "192f97ad32498511aa1f776a8fc86caf4c90f2d5414f174ced71790cdf6f2086",
    ),
    (
        "blinking_sprite/rgba8/e2/dcdefault/nobegin",
        "c45959786db2c6508bdabf6e059b3c491e3a68a134859e5f1879c26b725e8e63",
    ),
    (
        "blinking_sprite/rgba8/e2/dcdefault/begin",
        "c45959786db2c6508bdabf6e059b3c491e3a68a134859e5f1879c26b725e8e63",
    ),
    (
        "blinking_sprite/rgba8/e2/dcnone/nobegin",
        "c45959786db2c6508bdabf6e059b3c491e3a68a134859e5f1879c26b725e8e63",
    ),
    (
        "blinking_sprite/rgba8/e2/dcnone/begin",
        "c45959786db2c6508bdabf6e059b3c491e3a68a134859e5f1879c26b725e8e63",
    ),
    (
        "blinking_sprite/rgba8/e3/dcdefault/nobegin",
        "b17e09554a438f05897249e7cc9123f107a681d9cdc89177c21b23359973fa8f",
    ),
    (
        "blinking_sprite/rgba8/e3/dcdefault/begin",
        "b17e09554a438f05897249e7cc9123f107a681d9cdc89177c21b23359973fa8f",
    ),
    (
        "blinking_sprite/rgba8/e3/dcnone/nobegin",
        "b17e09554a438f05897249e7cc9123f107a681d9cdc89177c21b23359973fa8f",
    ),
    (
        "blinking_sprite/rgba8/e3/dcnone/begin",
        "b17e09554a438f05897249e7cc9123f107a681d9cdc89177c21b23359973fa8f",
    ),
    (
        "blinking_sprite/rgba8/e5/dcdefault/nobegin",
        "b17e09554a438f05897249e7cc9123f107a681d9cdc89177c21b23359973fa8f",
    ),
    (
        "blinking_sprite/rgba8/e5/dcdefault/begin",
        "b17e09554a438f05897249e7cc9123f107a681d9cdc89177c21b23359973fa8f",
    ),
    (
        "blinking_sprite/rgba8/e5/dcnone/nobegin",
        "b17e09554a438f05897249e7cc9123f107a681d9cdc89177c21b23359973fa8f",
    ),
    (
        "blinking_sprite/rgba8/e5/dcnone/begin",
        "b17e09554a438f05897249e7cc9123f107a681d9cdc89177c21b23359973fa8f",
    ),
    (
        "blinking_sprite/rgba8/e7/dcdefault/nobegin",
        "404fb0ae030abac1ab3714faa13e1635bb9f384455f6bdd6285d5f7a3d43e479",
    ),
    (
        "blinking_sprite/rgba8/e7/dcdefault/begin",
        "404fb0ae030abac1ab3714faa13e1635bb9f384455f6bdd6285d5f7a3d43e479",
    ),
    (
        "blinking_sprite/rgba8/e7/dcnone/nobegin",
        "404fb0ae030abac1ab3714faa13e1635bb9f384455f6bdd6285d5f7a3d43e479",
    ),
    (
        "blinking_sprite/rgba8/e7/dcnone/begin",
        "404fb0ae030abac1ab3714faa13e1635bb9f384455f6bdd6285d5f7a3d43e479",
    ),
    (
        "blinking_sprite/rgba8/e10/dcdefault/nobegin",
        "eff011a832281671a307e54070a706c1e6ce736ade94a69511443b5cb6272d74",
    ),
    (
        "blinking_sprite/rgba8/e10/dcdefault/begin",
        "eff011a832281671a307e54070a706c1e6ce736ade94a69511443b5cb6272d74",
    ),
    (
        "blinking_sprite/rgba8/e10/dcnone/nobegin",
        "eff011a832281671a307e54070a706c1e6ce736ade94a69511443b5cb6272d74",
    ),
    (
        "blinking_sprite/rgba8/e10/dcnone/begin",
        "eff011a832281671a307e54070a706c1e6ce736ade94a69511443b5cb6272d74",
    ),
    (
        "blinking_sprite/bgra8/default/dcdefault/nobegin",
        "192f97ad32498511aa1f776a8fc86caf4c90f2d5414f174ced71790cdf6f2086",
    ),
    (
        "blinking_sprite/bgra8/default/dcdefault/begin",
        "192f97ad32498511aa1f776a8fc86caf4c90f2d5414f174ced71790cdf6f2086",
    ),
    (
        "blinking_sprite/bgra8/default/dcnone/nobegin",
        "192f97ad32498511aa1f776a8fc86caf4c90f2d5414f174ced71790cdf6f2086",
    ),
    (
        "blinking_sprite/bgra8/default/dcnone/begin",
        "192f97ad32498511aa1f776a8fc86caf4c90f2d5414f174ced71790cdf6f2086",
    ),
    (
        "blinking_sprite/bgra8/e2/dcdefault/nobegin",
        "c45959786db2c6508bdabf6e059b3c491e3a68a134859e5f1879c26b725e8e63",
    ),
    (
        "blinking_sprite/bgra8/e2/dcdefault/begin",
        "c45959786db2c6508bdabf6e059b3c491e3a68a134859e5f1879c26b725e8e63",
    ),
    (
        "blinking_sprite/bgra8/e2/dcnone/nobegin",
        "c45959786db2c6508bdabf6e059b3c491e3a68a134859e5f1879c26b725e8e63",
    ),
    (
        "blinking_sprite/bgra8/e2/dcnone/begin",
        "c45959786db2c6508bdabf6e059b3c491e3a68a134859e5f1879c26b725e8e63",
    ),
    (
        "blinking_sprite/bgra8/e3/dcdefault/nobegin",
        "b17e09554a438f05897249e7cc9123f107a681d9cdc89177c21b23359973fa8f",
    ),
    (
        "blinking_sprite/bgra8/e3/dcdefault/begin",
        "b17e09554a438f05897249e7cc9123f107a681d9cdc89177c21b23359973fa8f",
    ),
    (
        "blinking_sprite/bgra8/e3/dcnone/nobegin",
        "b17e09554a438f05897249e7cc9123f107a681d9cdc89177c21b23359973fa8f",
    ),
    (
        "blinking_sprite/bgra8/e3/dcnone/begin",
        "b17e09554a438f05897249e7cc9123f107a681d9cdc89177c21b23359973fa8f",
    ),
    (
        "blinking_sprite/bgra8/e5/dcdefault/nobegin",
        "b17e09554a438f05897249e7cc9123f107a681d9cdc89177c21b23359973fa8f",
    ),
    (
        "blinking_sprite/bgra8/e5/dcdefault/begin",
        "b17e09554a438f05897249e7cc9123f107a681d9cdc89177c21b23359973fa8f",
    ),
    (
        "blinking_sprite/bgra8/e5/dcnone/nobegin",
        "b17e09554a438f05897249e7cc9123f107a681d9cdc89177c21b23359973fa8f",
    ),
    (
        "blinking_sprite/bgra8/e5/dcnone/begin",
        "b17e09554a438f05897249e7cc9123f107a681d9cdc89177c21b23359973fa8f",
    ),
    (
        "blinking_sprite/bgra8/e7/dcdefault/nobegin",
        "404fb0ae030abac1ab3714faa13e1635bb9f384455f6bdd6285d5f7a3d43e479",
    ),
    (
        "blinking_sprite/bgra8/e7/dcdefault/begin",
        "404fb0ae030abac1ab3714faa13e1635bb9f384455f6bdd6285d5f7a3d43e479",
    ),
    (
        "blinking_sprite/bgra8/e7/dcnone/nobegin",
        "404fb0ae030abac1ab3714faa13e1635bb9f384455f6bdd6285d5f7a3d43e479",
    ),
    (
        "blinking_sprite/bgra8/e7/dcnone/begin",
        "404fb0ae030abac1ab3714faa13e1635bb9f384455f6bdd6285d5f7a3d43e479",
    ),
    (
        "blinking_sprite/bgra8/e10/dcdefault/nobegin",
        "eff011a832281671a307e54070a706c1e6ce736ade94a69511443b5cb6272d74",
    ),
    (
        "blinking_sprite/bgra8/e10/dcdefault/begin",
        "eff011a832281671a307e54070a706c1e6ce736ade94a69511443b5cb6272d74",
    ),
    (
        "blinking_sprite/bgra8/e10/dcnone/nobegin",
        "eff011a832281671a307e54070a706c1e6ce736ade94a69511443b5cb6272d74",
    ),
    (
        "blinking_sprite/bgra8/e10/dcnone/begin",
        "eff011a832281671a307e54070a706c1e6ce736ade94a69511443b5cb6272d74",
    ),
    (
        "mostly_identical/rgb8/default/dcdefault/nobegin",
        "1faf45193fbe67cefa56dd5115562cf13c75be9406f2b3eb8cfe4d4579ee08fe",
    ),
    (
        "mostly_identical/rgb8/default/dcdefault/begin",
        "1faf45193fbe67cefa56dd5115562cf13c75be9406f2b3eb8cfe4d4579ee08fe",
    ),
    (
        "mostly_identical/rgb8/default/dcnone/nobegin",
        "1faf45193fbe67cefa56dd5115562cf13c75be9406f2b3eb8cfe4d4579ee08fe",
    ),
    (
        "mostly_identical/rgb8/default/dcnone/begin",
        "1faf45193fbe67cefa56dd5115562cf13c75be9406f2b3eb8cfe4d4579ee08fe",
    ),
    (
        "mostly_identical/rgb8/e2/dcdefault/nobegin",
        "ff6ceab014209f59d395288908b635b4784314ec2a442c5b4ca68a2214be581b",
    ),
    (
        "mostly_identical/rgb8/e2/dcdefault/begin",
        "ff6ceab014209f59d395288908b635b4784314ec2a442c5b4ca68a2214be581b",
    ),
    (
        "mostly_identical/rgb8/e2/dcnone/nobegin",
        "ff6ceab014209f59d395288908b635b4784314ec2a442c5b4ca68a2214be581b",
    ),
    (
        "mostly_identical/rgb8/e2/dcnone/begin",
        "ff6ceab014209f59d395288908b635b4784314ec2a442c5b4ca68a2214be581b",
    ),
    (
        "mostly_identical/rgb8/e3/dcdefault/nobegin",
        "bc9e6d4ce7448c40e77a6f511b8060121dba035b710bb876931caac9108be37f",
    ),
    (
        "mostly_identical/rgb8/e3/dcdefault/begin",
        "bc9e6d4ce7448c40e77a6f511b8060121dba035b710bb876931caac9108be37f",
    ),
    (
        "mostly_identical/rgb8/e3/dcnone/nobegin",
        "bc9e6d4ce7448c40e77a6f511b8060121dba035b710bb876931caac9108be37f",
    ),
    (
        "mostly_identical/rgb8/e3/dcnone/begin",
        "bc9e6d4ce7448c40e77a6f511b8060121dba035b710bb876931caac9108be37f",
    ),
    (
        "mostly_identical/rgb8/e5/dcdefault/nobegin",
        "bc9e6d4ce7448c40e77a6f511b8060121dba035b710bb876931caac9108be37f",
    ),
    (
        "mostly_identical/rgb8/e5/dcdefault/begin",
        "bc9e6d4ce7448c40e77a6f511b8060121dba035b710bb876931caac9108be37f",
    ),
    (
        "mostly_identical/rgb8/e5/dcnone/nobegin",
        "bc9e6d4ce7448c40e77a6f511b8060121dba035b710bb876931caac9108be37f",
    ),
    (
        "mostly_identical/rgb8/e5/dcnone/begin",
        "bc9e6d4ce7448c40e77a6f511b8060121dba035b710bb876931caac9108be37f",
    ),
    (
        "mostly_identical/rgb8/e7/dcdefault/nobegin",
        "b73c246636727b9e453fd2b5c3edbd640b42eeab0ff921991a988e414cc9ee07",
    ),
    (
        "mostly_identical/rgb8/e7/dcdefault/begin",
        "b73c246636727b9e453fd2b5c3edbd640b42eeab0ff921991a988e414cc9ee07",
    ),
    (
        "mostly_identical/rgb8/e7/dcnone/nobegin",
        "b73c246636727b9e453fd2b5c3edbd640b42eeab0ff921991a988e414cc9ee07",
    ),
    (
        "mostly_identical/rgb8/e7/dcnone/begin",
        "b73c246636727b9e453fd2b5c3edbd640b42eeab0ff921991a988e414cc9ee07",
    ),
    (
        "mostly_identical/rgb8/e10/dcdefault/nobegin",
        "4118c06d10ce291d4c1e71b132710ef00ec0d840113b755490fbb182af05bbb6",
    ),
    (
        "mostly_identical/rgb8/e10/dcdefault/begin",
        "4118c06d10ce291d4c1e71b132710ef00ec0d840113b755490fbb182af05bbb6",
    ),
    (
        "mostly_identical/rgb8/e10/dcnone/nobegin",
        "4118c06d10ce291d4c1e71b132710ef00ec0d840113b755490fbb182af05bbb6",
    ),
    (
        "mostly_identical/rgb8/e10/dcnone/begin",
        "4118c06d10ce291d4c1e71b132710ef00ec0d840113b755490fbb182af05bbb6",
    ),
    (
        "mostly_identical/rgba8/default/dcdefault/nobegin",
        "32f6440cdbf4bc53c80ef323d9aa5951c778a4a5e22f34e4dcb44db9a442e2a2",
    ),
    (
        "mostly_identical/rgba8/default/dcdefault/begin",
        "32f6440cdbf4bc53c80ef323d9aa5951c778a4a5e22f34e4dcb44db9a442e2a2",
    ),
    (
        "mostly_identical/rgba8/default/dcnone/nobegin",
        "32f6440cdbf4bc53c80ef323d9aa5951c778a4a5e22f34e4dcb44db9a442e2a2",
    ),
    (
        "mostly_identical/rgba8/default/dcnone/begin",
        "32f6440cdbf4bc53c80ef323d9aa5951c778a4a5e22f34e4dcb44db9a442e2a2",
    ),
    (
        "mostly_identical/rgba8/e2/dcdefault/nobegin",
        "154dde6bb0e33634ede128319f8403b939bee38e5f370d12d15ae7ce3c316ceb",
    ),
    (
        "mostly_identical/rgba8/e2/dcdefault/begin",
        "154dde6bb0e33634ede128319f8403b939bee38e5f370d12d15ae7ce3c316ceb",
    ),
    (
        "mostly_identical/rgba8/e2/dcnone/nobegin",
        "154dde6bb0e33634ede128319f8403b939bee38e5f370d12d15ae7ce3c316ceb",
    ),
    (
        "mostly_identical/rgba8/e2/dcnone/begin",
        "154dde6bb0e33634ede128319f8403b939bee38e5f370d12d15ae7ce3c316ceb",
    ),
    (
        "mostly_identical/rgba8/e3/dcdefault/nobegin",
        "102db9a4a0fadea66f752dfe5bd3c96a05dcd57f2d417b423f45b84f6c8a898b",
    ),
    (
        "mostly_identical/rgba8/e3/dcdefault/begin",
        "102db9a4a0fadea66f752dfe5bd3c96a05dcd57f2d417b423f45b84f6c8a898b",
    ),
    (
        "mostly_identical/rgba8/e3/dcnone/nobegin",
        "102db9a4a0fadea66f752dfe5bd3c96a05dcd57f2d417b423f45b84f6c8a898b",
    ),
    (
        "mostly_identical/rgba8/e3/dcnone/begin",
        "102db9a4a0fadea66f752dfe5bd3c96a05dcd57f2d417b423f45b84f6c8a898b",
    ),
    (
        "mostly_identical/rgba8/e5/dcdefault/nobegin",
        "102db9a4a0fadea66f752dfe5bd3c96a05dcd57f2d417b423f45b84f6c8a898b",
    ),
    (
        "mostly_identical/rgba8/e5/dcdefault/begin",
        "102db9a4a0fadea66f752dfe5bd3c96a05dcd57f2d417b423f45b84f6c8a898b",
    ),
    (
        "mostly_identical/rgba8/e5/dcnone/nobegin",
        "102db9a4a0fadea66f752dfe5bd3c96a05dcd57f2d417b423f45b84f6c8a898b",
    ),
    (
        "mostly_identical/rgba8/e5/dcnone/begin",
        "102db9a4a0fadea66f752dfe5bd3c96a05dcd57f2d417b423f45b84f6c8a898b",
    ),
    (
        "mostly_identical/rgba8/e7/dcdefault/nobegin",
        "4d63eda435bffaf8b0b01320cbdae0b320840a939ed3bb8cd011c10c0fc745c7",
    ),
    (
        "mostly_identical/rgba8/e7/dcdefault/begin",
        "4d63eda435bffaf8b0b01320cbdae0b320840a939ed3bb8cd011c10c0fc745c7",
    ),
    (
        "mostly_identical/rgba8/e7/dcnone/nobegin",
        "4d63eda435bffaf8b0b01320cbdae0b320840a939ed3bb8cd011c10c0fc745c7",
    ),
    (
        "mostly_identical/rgba8/e7/dcnone/begin",
        "4d63eda435bffaf8b0b01320cbdae0b320840a939ed3bb8cd011c10c0fc745c7",
    ),
    (
        "mostly_identical/rgba8/e10/dcdefault/nobegin",
        "16f460a84bc3c699e17cdf1c072408bb738bdd0735eb3cb27d6f38447be9ef45",
    ),
    (
        "mostly_identical/rgba8/e10/dcdefault/begin",
        "16f460a84bc3c699e17cdf1c072408bb738bdd0735eb3cb27d6f38447be9ef45",
    ),
    (
        "mostly_identical/rgba8/e10/dcnone/nobegin",
        "16f460a84bc3c699e17cdf1c072408bb738bdd0735eb3cb27d6f38447be9ef45",
    ),
    (
        "mostly_identical/rgba8/e10/dcnone/begin",
        "16f460a84bc3c699e17cdf1c072408bb738bdd0735eb3cb27d6f38447be9ef45",
    ),
    (
        "mostly_identical/bgra8/default/dcdefault/nobegin",
        "32f6440cdbf4bc53c80ef323d9aa5951c778a4a5e22f34e4dcb44db9a442e2a2",
    ),
    (
        "mostly_identical/bgra8/default/dcdefault/begin",
        "32f6440cdbf4bc53c80ef323d9aa5951c778a4a5e22f34e4dcb44db9a442e2a2",
    ),
    (
        "mostly_identical/bgra8/default/dcnone/nobegin",
        "32f6440cdbf4bc53c80ef323d9aa5951c778a4a5e22f34e4dcb44db9a442e2a2",
    ),
    (
        "mostly_identical/bgra8/default/dcnone/begin",
        "32f6440cdbf4bc53c80ef323d9aa5951c778a4a5e22f34e4dcb44db9a442e2a2",
    ),
    (
        "mostly_identical/bgra8/e2/dcdefault/nobegin",
        "154dde6bb0e33634ede128319f8403b939bee38e5f370d12d15ae7ce3c316ceb",
    ),
    (
        "mostly_identical/bgra8/e2/dcdefault/begin",
        "154dde6bb0e33634ede128319f8403b939bee38e5f370d12d15ae7ce3c316ceb",
    ),
    (
        "mostly_identical/bgra8/e2/dcnone/nobegin",
        "154dde6bb0e33634ede128319f8403b939bee38e5f370d12d15ae7ce3c316ceb",
    ),
    (
        "mostly_identical/bgra8/e2/dcnone/begin",
        "154dde6bb0e33634ede128319f8403b939bee38e5f370d12d15ae7ce3c316ceb",
    ),
    (
        "mostly_identical/bgra8/e3/dcdefault/nobegin",
        "102db9a4a0fadea66f752dfe5bd3c96a05dcd57f2d417b423f45b84f6c8a898b",
    ),
    (
        "mostly_identical/bgra8/e3/dcdefault/begin",
        "102db9a4a0fadea66f752dfe5bd3c96a05dcd57f2d417b423f45b84f6c8a898b",
    ),
    (
        "mostly_identical/bgra8/e3/dcnone/nobegin",
        "102db9a4a0fadea66f752dfe5bd3c96a05dcd57f2d417b423f45b84f6c8a898b",
    ),
    (
        "mostly_identical/bgra8/e3/dcnone/begin",
        "102db9a4a0fadea66f752dfe5bd3c96a05dcd57f2d417b423f45b84f6c8a898b",
    ),
    (
        "mostly_identical/bgra8/e5/dcdefault/nobegin",
        "102db9a4a0fadea66f752dfe5bd3c96a05dcd57f2d417b423f45b84f6c8a898b",
    ),
    (
        "mostly_identical/bgra8/e5/dcdefault/begin",
        "102db9a4a0fadea66f752dfe5bd3c96a05dcd57f2d417b423f45b84f6c8a898b",
    ),
    (
        "mostly_identical/bgra8/e5/dcnone/nobegin",
        "102db9a4a0fadea66f752dfe5bd3c96a05dcd57f2d417b423f45b84f6c8a898b",
    ),
    (
        "mostly_identical/bgra8/e5/dcnone/begin",
        "102db9a4a0fadea66f752dfe5bd3c96a05dcd57f2d417b423f45b84f6c8a898b",
    ),
    (
        "mostly_identical/bgra8/e7/dcdefault/nobegin",
        "4d63eda435bffaf8b0b01320cbdae0b320840a939ed3bb8cd011c10c0fc745c7",
    ),
    (
        "mostly_identical/bgra8/e7/dcdefault/begin",
        "4d63eda435bffaf8b0b01320cbdae0b320840a939ed3bb8cd011c10c0fc745c7",
    ),
    (
        "mostly_identical/bgra8/e7/dcnone/nobegin",
        "4d63eda435bffaf8b0b01320cbdae0b320840a939ed3bb8cd011c10c0fc745c7",
    ),
    (
        "mostly_identical/bgra8/e7/dcnone/begin",
        "4d63eda435bffaf8b0b01320cbdae0b320840a939ed3bb8cd011c10c0fc745c7",
    ),
    (
        "mostly_identical/bgra8/e10/dcdefault/nobegin",
        "16f460a84bc3c699e17cdf1c072408bb738bdd0735eb3cb27d6f38447be9ef45",
    ),
    (
        "mostly_identical/bgra8/e10/dcdefault/begin",
        "16f460a84bc3c699e17cdf1c072408bb738bdd0735eb3cb27d6f38447be9ef45",
    ),
    (
        "mostly_identical/bgra8/e10/dcnone/nobegin",
        "16f460a84bc3c699e17cdf1c072408bb738bdd0735eb3cb27d6f38447be9ef45",
    ),
    (
        "mostly_identical/bgra8/e10/dcnone/begin",
        "16f460a84bc3c699e17cdf1c072408bb738bdd0735eb3cb27d6f38447be9ef45",
    ),
    (
        "odd_padded/rgb8/default/dcdefault/nobegin",
        "cf8cf500a7507d44529accdf46cd0c5e92f07ce24721008cde032bf677a644df",
    ),
    (
        "odd_padded/rgb8/default/dcdefault/begin",
        "cf8cf500a7507d44529accdf46cd0c5e92f07ce24721008cde032bf677a644df",
    ),
    (
        "odd_padded/rgb8/default/dcnone/nobegin",
        "cf8cf500a7507d44529accdf46cd0c5e92f07ce24721008cde032bf677a644df",
    ),
    (
        "odd_padded/rgb8/default/dcnone/begin",
        "cf8cf500a7507d44529accdf46cd0c5e92f07ce24721008cde032bf677a644df",
    ),
    (
        "odd_padded/rgb8/e2/dcdefault/nobegin",
        "8d6071ad00dad5e6c6b215914b97a52a2ca4128ff68cf64d9ad42c6b04f5ae31",
    ),
    (
        "odd_padded/rgb8/e2/dcdefault/begin",
        "8d6071ad00dad5e6c6b215914b97a52a2ca4128ff68cf64d9ad42c6b04f5ae31",
    ),
    (
        "odd_padded/rgb8/e2/dcnone/nobegin",
        "8d6071ad00dad5e6c6b215914b97a52a2ca4128ff68cf64d9ad42c6b04f5ae31",
    ),
    (
        "odd_padded/rgb8/e2/dcnone/begin",
        "8d6071ad00dad5e6c6b215914b97a52a2ca4128ff68cf64d9ad42c6b04f5ae31",
    ),
    (
        "odd_padded/rgb8/e3/dcdefault/nobegin",
        "ec356afd24b3ceaed0a5759a2f5070a0dcd0204c7143b04a6de5e91c19fcc711",
    ),
    (
        "odd_padded/rgb8/e3/dcdefault/begin",
        "ec356afd24b3ceaed0a5759a2f5070a0dcd0204c7143b04a6de5e91c19fcc711",
    ),
    (
        "odd_padded/rgb8/e3/dcnone/nobegin",
        "ec356afd24b3ceaed0a5759a2f5070a0dcd0204c7143b04a6de5e91c19fcc711",
    ),
    (
        "odd_padded/rgb8/e3/dcnone/begin",
        "ec356afd24b3ceaed0a5759a2f5070a0dcd0204c7143b04a6de5e91c19fcc711",
    ),
    (
        "odd_padded/rgb8/e5/dcdefault/nobegin",
        "7d0521af9414076200d2237e7efa59cb40651e077fafacd623fa6ed0d995ff05",
    ),
    (
        "odd_padded/rgb8/e5/dcdefault/begin",
        "7d0521af9414076200d2237e7efa59cb40651e077fafacd623fa6ed0d995ff05",
    ),
    (
        "odd_padded/rgb8/e5/dcnone/nobegin",
        "7d0521af9414076200d2237e7efa59cb40651e077fafacd623fa6ed0d995ff05",
    ),
    (
        "odd_padded/rgb8/e5/dcnone/begin",
        "7d0521af9414076200d2237e7efa59cb40651e077fafacd623fa6ed0d995ff05",
    ),
    (
        "odd_padded/rgb8/e7/dcdefault/nobegin",
        "e1528842e916e6f6178532ea51babde0c5d6122eeaf1ab82091be4752a1e0854",
    ),
    (
        "odd_padded/rgb8/e7/dcdefault/begin",
        "e1528842e916e6f6178532ea51babde0c5d6122eeaf1ab82091be4752a1e0854",
    ),
    (
        "odd_padded/rgb8/e7/dcnone/nobegin",
        "e1528842e916e6f6178532ea51babde0c5d6122eeaf1ab82091be4752a1e0854",
    ),
    (
        "odd_padded/rgb8/e7/dcnone/begin",
        "e1528842e916e6f6178532ea51babde0c5d6122eeaf1ab82091be4752a1e0854",
    ),
    (
        "odd_padded/rgb8/e10/dcdefault/nobegin",
        "69358df10a4938c5134a7ff2b72bca0a306b72fadf7ce085a9586846fa547c65",
    ),
    (
        "odd_padded/rgb8/e10/dcdefault/begin",
        "69358df10a4938c5134a7ff2b72bca0a306b72fadf7ce085a9586846fa547c65",
    ),
    (
        "odd_padded/rgb8/e10/dcnone/nobegin",
        "69358df10a4938c5134a7ff2b72bca0a306b72fadf7ce085a9586846fa547c65",
    ),
    (
        "odd_padded/rgb8/e10/dcnone/begin",
        "69358df10a4938c5134a7ff2b72bca0a306b72fadf7ce085a9586846fa547c65",
    ),
    (
        "odd_padded/rgba8/default/dcdefault/nobegin",
        "f520fe31b37b01e0611ca93917b7d26db7131eadeec8543f8c60454e3d4e684f",
    ),
    (
        "odd_padded/rgba8/default/dcdefault/begin",
        "f520fe31b37b01e0611ca93917b7d26db7131eadeec8543f8c60454e3d4e684f",
    ),
    (
        "odd_padded/rgba8/default/dcnone/nobegin",
        "f520fe31b37b01e0611ca93917b7d26db7131eadeec8543f8c60454e3d4e684f",
    ),
    (
        "odd_padded/rgba8/default/dcnone/begin",
        "f520fe31b37b01e0611ca93917b7d26db7131eadeec8543f8c60454e3d4e684f",
    ),
    (
        "odd_padded/rgba8/e2/dcdefault/nobegin",
        "30ab7603a9b782291c5e8d76289e1d9e0565ecee34fbfff05cc828253c0c521c",
    ),
    (
        "odd_padded/rgba8/e2/dcdefault/begin",
        "30ab7603a9b782291c5e8d76289e1d9e0565ecee34fbfff05cc828253c0c521c",
    ),
    (
        "odd_padded/rgba8/e2/dcnone/nobegin",
        "30ab7603a9b782291c5e8d76289e1d9e0565ecee34fbfff05cc828253c0c521c",
    ),
    (
        "odd_padded/rgba8/e2/dcnone/begin",
        "30ab7603a9b782291c5e8d76289e1d9e0565ecee34fbfff05cc828253c0c521c",
    ),
    (
        "odd_padded/rgba8/e3/dcdefault/nobegin",
        "2179019f1e41c47b8ec66da467d98693e7a42fb970de394da51a7489f16351c5",
    ),
    (
        "odd_padded/rgba8/e3/dcdefault/begin",
        "2179019f1e41c47b8ec66da467d98693e7a42fb970de394da51a7489f16351c5",
    ),
    (
        "odd_padded/rgba8/e3/dcnone/nobegin",
        "2179019f1e41c47b8ec66da467d98693e7a42fb970de394da51a7489f16351c5",
    ),
    (
        "odd_padded/rgba8/e3/dcnone/begin",
        "2179019f1e41c47b8ec66da467d98693e7a42fb970de394da51a7489f16351c5",
    ),
    (
        "odd_padded/rgba8/e5/dcdefault/nobegin",
        "2179019f1e41c47b8ec66da467d98693e7a42fb970de394da51a7489f16351c5",
    ),
    (
        "odd_padded/rgba8/e5/dcdefault/begin",
        "2179019f1e41c47b8ec66da467d98693e7a42fb970de394da51a7489f16351c5",
    ),
    (
        "odd_padded/rgba8/e5/dcnone/nobegin",
        "2179019f1e41c47b8ec66da467d98693e7a42fb970de394da51a7489f16351c5",
    ),
    (
        "odd_padded/rgba8/e5/dcnone/begin",
        "2179019f1e41c47b8ec66da467d98693e7a42fb970de394da51a7489f16351c5",
    ),
    (
        "odd_padded/rgba8/e7/dcdefault/nobegin",
        "e41c1839dd2ab30d1e760b0d8975a291484c12e374f78b128d98844ef130f24d",
    ),
    (
        "odd_padded/rgba8/e7/dcdefault/begin",
        "e41c1839dd2ab30d1e760b0d8975a291484c12e374f78b128d98844ef130f24d",
    ),
    (
        "odd_padded/rgba8/e7/dcnone/nobegin",
        "e41c1839dd2ab30d1e760b0d8975a291484c12e374f78b128d98844ef130f24d",
    ),
    (
        "odd_padded/rgba8/e7/dcnone/begin",
        "e41c1839dd2ab30d1e760b0d8975a291484c12e374f78b128d98844ef130f24d",
    ),
    (
        "odd_padded/rgba8/e10/dcdefault/nobegin",
        "e41c1839dd2ab30d1e760b0d8975a291484c12e374f78b128d98844ef130f24d",
    ),
    (
        "odd_padded/rgba8/e10/dcdefault/begin",
        "e41c1839dd2ab30d1e760b0d8975a291484c12e374f78b128d98844ef130f24d",
    ),
    (
        "odd_padded/rgba8/e10/dcnone/nobegin",
        "e41c1839dd2ab30d1e760b0d8975a291484c12e374f78b128d98844ef130f24d",
    ),
    (
        "odd_padded/rgba8/e10/dcnone/begin",
        "e41c1839dd2ab30d1e760b0d8975a291484c12e374f78b128d98844ef130f24d",
    ),
    (
        "odd_padded/bgra8/default/dcdefault/nobegin",
        "f520fe31b37b01e0611ca93917b7d26db7131eadeec8543f8c60454e3d4e684f",
    ),
    (
        "odd_padded/bgra8/default/dcdefault/begin",
        "f520fe31b37b01e0611ca93917b7d26db7131eadeec8543f8c60454e3d4e684f",
    ),
    (
        "odd_padded/bgra8/default/dcnone/nobegin",
        "f520fe31b37b01e0611ca93917b7d26db7131eadeec8543f8c60454e3d4e684f",
    ),
    (
        "odd_padded/bgra8/default/dcnone/begin",
        "f520fe31b37b01e0611ca93917b7d26db7131eadeec8543f8c60454e3d4e684f",
    ),
    (
        "odd_padded/bgra8/e2/dcdefault/nobegin",
        "30ab7603a9b782291c5e8d76289e1d9e0565ecee34fbfff05cc828253c0c521c",
    ),
    (
        "odd_padded/bgra8/e2/dcdefault/begin",
        "30ab7603a9b782291c5e8d76289e1d9e0565ecee34fbfff05cc828253c0c521c",
    ),
    (
        "odd_padded/bgra8/e2/dcnone/nobegin",
        "30ab7603a9b782291c5e8d76289e1d9e0565ecee34fbfff05cc828253c0c521c",
    ),
    (
        "odd_padded/bgra8/e2/dcnone/begin",
        "30ab7603a9b782291c5e8d76289e1d9e0565ecee34fbfff05cc828253c0c521c",
    ),
    (
        "odd_padded/bgra8/e3/dcdefault/nobegin",
        "2179019f1e41c47b8ec66da467d98693e7a42fb970de394da51a7489f16351c5",
    ),
    (
        "odd_padded/bgra8/e3/dcdefault/begin",
        "2179019f1e41c47b8ec66da467d98693e7a42fb970de394da51a7489f16351c5",
    ),
    (
        "odd_padded/bgra8/e3/dcnone/nobegin",
        "2179019f1e41c47b8ec66da467d98693e7a42fb970de394da51a7489f16351c5",
    ),
    (
        "odd_padded/bgra8/e3/dcnone/begin",
        "2179019f1e41c47b8ec66da467d98693e7a42fb970de394da51a7489f16351c5",
    ),
    (
        "odd_padded/bgra8/e5/dcdefault/nobegin",
        "2179019f1e41c47b8ec66da467d98693e7a42fb970de394da51a7489f16351c5",
    ),
    (
        "odd_padded/bgra8/e5/dcdefault/begin",
        "2179019f1e41c47b8ec66da467d98693e7a42fb970de394da51a7489f16351c5",
    ),
    (
        "odd_padded/bgra8/e5/dcnone/nobegin",
        "2179019f1e41c47b8ec66da467d98693e7a42fb970de394da51a7489f16351c5",
    ),
    (
        "odd_padded/bgra8/e5/dcnone/begin",
        "2179019f1e41c47b8ec66da467d98693e7a42fb970de394da51a7489f16351c5",
    ),
    (
        "odd_padded/bgra8/e7/dcdefault/nobegin",
        "e41c1839dd2ab30d1e760b0d8975a291484c12e374f78b128d98844ef130f24d",
    ),
    (
        "odd_padded/bgra8/e7/dcdefault/begin",
        "e41c1839dd2ab30d1e760b0d8975a291484c12e374f78b128d98844ef130f24d",
    ),
    (
        "odd_padded/bgra8/e7/dcnone/nobegin",
        "e41c1839dd2ab30d1e760b0d8975a291484c12e374f78b128d98844ef130f24d",
    ),
    (
        "odd_padded/bgra8/e7/dcnone/begin",
        "e41c1839dd2ab30d1e760b0d8975a291484c12e374f78b128d98844ef130f24d",
    ),
    (
        "odd_padded/bgra8/e10/dcdefault/nobegin",
        "e41c1839dd2ab30d1e760b0d8975a291484c12e374f78b128d98844ef130f24d",
    ),
    (
        "odd_padded/bgra8/e10/dcdefault/begin",
        "e41c1839dd2ab30d1e760b0d8975a291484c12e374f78b128d98844ef130f24d",
    ),
    (
        "odd_padded/bgra8/e10/dcnone/nobegin",
        "e41c1839dd2ab30d1e760b0d8975a291484c12e374f78b128d98844ef130f24d",
    ),
    (
        "odd_padded/bgra8/e10/dcnone/begin",
        "e41c1839dd2ab30d1e760b0d8975a291484c12e374f78b128d98844ef130f24d",
    ),
];
