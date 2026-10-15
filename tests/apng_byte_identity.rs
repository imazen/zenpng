//! Byte-identity oracle for APNG encoding.
//!
//! Encodes a fixed matrix of animations — pixel formats, efforts, downcast
//! flags, and `begin` usage — and asserts the exact output bytes (as
//! SHA-256) match goldens captured from the build this file was added on.
//! Any change to APNG output bytes fails here; optimizations must not.

#![cfg(not(target_arch = "wasm32"))]

use sha2::Digest;
use zencodec::encode::{DynAnimationFrameEncoder, EncodeJob, EncoderConfig};
use zenpixels::{PixelDescriptor, PixelSlice};
use zenpng::{Compression, DowncastFlags, PngEncoderConfig};

const RGB8: PixelDescriptor = PixelDescriptor::RGB8_SRGB;
const RGBA8: PixelDescriptor = PixelDescriptor::RGBA8_SRGB;
const BGRA8: PixelDescriptor = PixelDescriptor::BGRA8_SRGB;
const GRAY8: PixelDescriptor = PixelDescriptor::GRAY8_SRGB;
const GRAYA8: PixelDescriptor = PixelDescriptor::GRAYA8_SRGB;
const RGB16: PixelDescriptor = PixelDescriptor::RGB16_SRGB;
const RGBA16: PixelDescriptor = PixelDescriptor::RGBA16_SRGB;

const W: u32 = 96;
const H: u32 = 64;
const FRAMES: u32 = 6;

/// Pixel `(x, y)` of frame `f`: a gradient with a lit block that moves a
/// few columns per frame, plus per-frame variation.
fn pixel(x: u32, y: u32, f: u32) -> [u8; 4] {
    let lit = x >= f * 4 && x < f * 4 + 11 && y >= 8 && y < H / 2 + 8;
    [
        (x.wrapping_mul(5) as u8) ^ if lit { 0xC0 } else { 0 },
        y.wrapping_mul(7) as u8,
        (f.wrapping_mul(31) as u8).wrapping_add((x / 7) as u8),
        ((x.wrapping_add(y.wrapping_mul(3))) % 256) as u8,
    ]
}

fn frame_bytes(desc: PixelDescriptor, f: u32, alpha: impl Fn(u32, u32) -> u16) -> Vec<u8> {
    let bpp = desc.bytes_per_pixel();
    let sixteen = matches!(desc.channel_type(), zenpixels::ChannelType::U16);
    let mut out = Vec::with_capacity(W as usize * H as usize * bpp);
    for y in 0..H {
        for x in 0..W {
            let [r, g, b, _] = pixel(x, y, f);
            let a = alpha(x, y);
            let samples: Vec<u16> = match desc.layout() {
                zenpixels::ChannelLayout::Gray => vec![u16::from(r)],
                zenpixels::ChannelLayout::GrayAlpha => vec![u16::from(r), a],
                zenpixels::ChannelLayout::Rgb => vec![u16::from(r), u16::from(g), u16::from(b)],
                zenpixels::ChannelLayout::Rgba => {
                    vec![u16::from(r), u16::from(g), u16::from(b), a]
                }
                zenpixels::ChannelLayout::Bgra => {
                    vec![u16::from(b), u16::from(g), u16::from(r), a]
                }
                _ => panic!("test layout"),
            };
            for s in samples {
                if sixteen {
                    let v = s << 8 | s; // replicate into 16 bits
                    out.extend_from_slice(&v.to_le_bytes());
                } else {
                    out.push(s as u8);
                }
            }
        }
    }
    out
}

fn frames_for(desc: PixelDescriptor, alpha: impl Fn(u32, u32) -> u16) -> Vec<Vec<u8>> {
    (0..FRAMES).map(|f| frame_bytes(desc, f, &alpha)).collect()
}

/// Encode `fs` through the dyn animation API; `declare` is the frame count
/// passed to `begin` (`Some(None)` declares an unknown count).
fn encode(
    cfg: &PngEncoderConfig,
    fs: &[Vec<u8>],
    desc: PixelDescriptor,
    declare: Option<Option<u32>>,
) -> Vec<u8> {
    let mut a: Box<dyn DynAnimationFrameEncoder> = cfg
        .clone()
        .job()
        .with_loop_count(Some(0))
        .dyn_animation_frame_encoder()
        .unwrap();
    if let Some(n) = declare {
        a.begin(W, H, desc, n).unwrap();
    }
    let stride = W as usize * desc.bytes_per_pixel();
    let mut out = Vec::new();
    for (i, f) in fs.iter().enumerate() {
        a.push_frame(
            PixelSlice::new(f, W, H, stride, desc).unwrap(),
            40 + i as u32 * 10,
            None,
        )
        .unwrap();
        a.take_output(&mut out);
    }
    out.extend_from_slice(a.finish(None).unwrap().data());
    out
}

fn cfg(effort: Option<u32>, downcast: Option<DowncastFlags>) -> PngEncoderConfig {
    let mut c = PngEncoderConfig::new();
    if let Some(e) = effort {
        c = c.with_compression(Compression::Effort(e));
    }
    if let Some(d) = downcast {
        c = c.with_downcast(d);
    }
    c
}

fn sha(data: &[u8]) -> String {
    let mut h = sha2::Sha256::new();
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

struct Case {
    name: &'static str,
    desc: PixelDescriptor,
    opaque: bool,
}

const CASES: &[Case] = &[
    Case { name: "rgb8", desc: RGB8, opaque: true },
    Case { name: "rgba8_opaque", desc: RGBA8, opaque: true },
    Case { name: "rgba8_alpha", desc: RGBA8, opaque: false },
    Case { name: "bgra8_opaque", desc: BGRA8, opaque: true },
    Case { name: "gray8", desc: GRAY8, opaque: true },
    Case { name: "graya8_alpha", desc: GRAYA8, opaque: false },
    Case { name: "rgb16", desc: RGB16, opaque: true },
    Case { name: "rgba16_alpha", desc: RGBA16, opaque: false },
];

fn effort_name(e: Option<u32>) -> String {
    e.map_or("default".to_string(), |e| format!("e{e}"))
}

/// Run the whole matrix, returning `name -> sha256` in deterministic order.
fn run_matrix() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for case in CASES {
        let alpha = |x: u32, y: u32| -> u16 {
            if case.opaque {
                255
            } else {
                // Varied alpha: fully transparent block, partial ramp, opaque rest.
                let base = ((x.wrapping_mul(9).wrapping_add(y.wrapping_mul(5))) % 256) as u16;
                if x >= 10 && x < 30 && y >= 10 && y < 20 {
                    0
                } else {
                    base
                }
            }
        };
        let fs = frames_for(case.desc, alpha);
        for effort in [None, Some(0), Some(1), Some(2), Some(5), Some(7)] {
            for (dname, downcast) in [
                ("dcdefault", Some(DowncastFlags::default())),
                ("dcnone", Some(DowncastFlags::none())),
            ] {
                for (bname, declare) in [
                    ("nobegin", None),
                    ("begin_n", Some(Some(FRAMES))),
                    ("begin_unk", Some(None)),
                ] {
                    let cfg = cfg(effort, downcast);
                    let bytes = encode(&cfg, &fs, case.desc, declare);
                    out.push((
                        format!("{}/{}/{}/{}", case.name, effort_name(effort), dname, bname),
                        sha(&bytes),
                    ));
                }
            }
        }
    }
    out
}

#[test]
fn apng_output_is_byte_identical() {
    let got = run_matrix();
    // Regenerate the table: APNG_GOLDEN=/path cargo test --test apng_byte_identity -- --nocapture
    if let Ok(path) = std::env::var("APNG_GOLDEN") {
        let mut text = String::new();
        for (name, hash) in &got {
            text.push_str(&format!("(\"{name}\", \"{hash}\"),\n"));
        }
        std::fs::write(&path, &text).unwrap();
    }
    let expected: std::collections::BTreeMap<&str, &str> = EXPECTED.iter().copied().collect();
    let mut failures = 0;
    for (name, hash) in &got {
        match expected.get(name.as_str()) {
            Some(&want) => assert_eq!(hash, want, "APNG bytes changed: {name}"),
            None => {
                eprintln!("missing golden for {name}");
                failures += 1;
            }
        }
    }
    assert_eq!(failures, 0, "missing goldens");
    assert_eq!(got.len(), expected.len(), "golden table size drifted");
}

const EXPECTED: &[(&str, &str)] = &[
("rgb8/default/dcdefault/nobegin", "112ac74adc3120e553f4dd42e00349e5c2f7a3e038090ac171b53705ca30e6ae"),
("rgb8/default/dcdefault/begin_n", "112ac74adc3120e553f4dd42e00349e5c2f7a3e038090ac171b53705ca30e6ae"),
("rgb8/default/dcdefault/begin_unk", "112ac74adc3120e553f4dd42e00349e5c2f7a3e038090ac171b53705ca30e6ae"),
("rgb8/default/dcnone/nobegin", "112ac74adc3120e553f4dd42e00349e5c2f7a3e038090ac171b53705ca30e6ae"),
("rgb8/default/dcnone/begin_n", "112ac74adc3120e553f4dd42e00349e5c2f7a3e038090ac171b53705ca30e6ae"),
("rgb8/default/dcnone/begin_unk", "112ac74adc3120e553f4dd42e00349e5c2f7a3e038090ac171b53705ca30e6ae"),
("rgb8/e0/dcdefault/nobegin", "2a5b6cdaa0037161ef58011eeb93e298a27bfa14f78d3ef28baa760763210112"),
("rgb8/e0/dcdefault/begin_n", "2a5b6cdaa0037161ef58011eeb93e298a27bfa14f78d3ef28baa760763210112"),
("rgb8/e0/dcdefault/begin_unk", "2a5b6cdaa0037161ef58011eeb93e298a27bfa14f78d3ef28baa760763210112"),
("rgb8/e0/dcnone/nobegin", "2a5b6cdaa0037161ef58011eeb93e298a27bfa14f78d3ef28baa760763210112"),
("rgb8/e0/dcnone/begin_n", "2a5b6cdaa0037161ef58011eeb93e298a27bfa14f78d3ef28baa760763210112"),
("rgb8/e0/dcnone/begin_unk", "2a5b6cdaa0037161ef58011eeb93e298a27bfa14f78d3ef28baa760763210112"),
("rgb8/e1/dcdefault/nobegin", "33941cc82f41bfbbefbe2e5a2b3980913589027e7da3218c0500d3d28609df6a"),
("rgb8/e1/dcdefault/begin_n", "33941cc82f41bfbbefbe2e5a2b3980913589027e7da3218c0500d3d28609df6a"),
("rgb8/e1/dcdefault/begin_unk", "33941cc82f41bfbbefbe2e5a2b3980913589027e7da3218c0500d3d28609df6a"),
("rgb8/e1/dcnone/nobegin", "33941cc82f41bfbbefbe2e5a2b3980913589027e7da3218c0500d3d28609df6a"),
("rgb8/e1/dcnone/begin_n", "33941cc82f41bfbbefbe2e5a2b3980913589027e7da3218c0500d3d28609df6a"),
("rgb8/e1/dcnone/begin_unk", "33941cc82f41bfbbefbe2e5a2b3980913589027e7da3218c0500d3d28609df6a"),
("rgb8/e2/dcdefault/nobegin", "f8f4a8b3008110eabf48c6f30c5852199df8f70c7d6cac7b55b845cadce1f957"),
("rgb8/e2/dcdefault/begin_n", "f8f4a8b3008110eabf48c6f30c5852199df8f70c7d6cac7b55b845cadce1f957"),
("rgb8/e2/dcdefault/begin_unk", "f8f4a8b3008110eabf48c6f30c5852199df8f70c7d6cac7b55b845cadce1f957"),
("rgb8/e2/dcnone/nobegin", "f8f4a8b3008110eabf48c6f30c5852199df8f70c7d6cac7b55b845cadce1f957"),
("rgb8/e2/dcnone/begin_n", "f8f4a8b3008110eabf48c6f30c5852199df8f70c7d6cac7b55b845cadce1f957"),
("rgb8/e2/dcnone/begin_unk", "f8f4a8b3008110eabf48c6f30c5852199df8f70c7d6cac7b55b845cadce1f957"),
("rgb8/e5/dcdefault/nobegin", "62f419b307dc791486138842b5c81563433471689f81669c094550e0affa28d1"),
("rgb8/e5/dcdefault/begin_n", "62f419b307dc791486138842b5c81563433471689f81669c094550e0affa28d1"),
("rgb8/e5/dcdefault/begin_unk", "62f419b307dc791486138842b5c81563433471689f81669c094550e0affa28d1"),
("rgb8/e5/dcnone/nobegin", "62f419b307dc791486138842b5c81563433471689f81669c094550e0affa28d1"),
("rgb8/e5/dcnone/begin_n", "62f419b307dc791486138842b5c81563433471689f81669c094550e0affa28d1"),
("rgb8/e5/dcnone/begin_unk", "62f419b307dc791486138842b5c81563433471689f81669c094550e0affa28d1"),
("rgb8/e7/dcdefault/nobegin", "302ed5f3a59aa7cba65222f0cd2911917c91372121073b0654e34a3704f90e23"),
("rgb8/e7/dcdefault/begin_n", "302ed5f3a59aa7cba65222f0cd2911917c91372121073b0654e34a3704f90e23"),
("rgb8/e7/dcdefault/begin_unk", "302ed5f3a59aa7cba65222f0cd2911917c91372121073b0654e34a3704f90e23"),
("rgb8/e7/dcnone/nobegin", "302ed5f3a59aa7cba65222f0cd2911917c91372121073b0654e34a3704f90e23"),
("rgb8/e7/dcnone/begin_n", "302ed5f3a59aa7cba65222f0cd2911917c91372121073b0654e34a3704f90e23"),
("rgb8/e7/dcnone/begin_unk", "302ed5f3a59aa7cba65222f0cd2911917c91372121073b0654e34a3704f90e23"),
("rgba8_opaque/default/dcdefault/nobegin", "112ac74adc3120e553f4dd42e00349e5c2f7a3e038090ac171b53705ca30e6ae"),
("rgba8_opaque/default/dcdefault/begin_n", "112ac74adc3120e553f4dd42e00349e5c2f7a3e038090ac171b53705ca30e6ae"),
("rgba8_opaque/default/dcdefault/begin_unk", "112ac74adc3120e553f4dd42e00349e5c2f7a3e038090ac171b53705ca30e6ae"),
("rgba8_opaque/default/dcnone/nobegin", "6bb011543903a4459a430574d12e013de20068572687cbb24cf62e222a6c0082"),
("rgba8_opaque/default/dcnone/begin_n", "6bb011543903a4459a430574d12e013de20068572687cbb24cf62e222a6c0082"),
("rgba8_opaque/default/dcnone/begin_unk", "6bb011543903a4459a430574d12e013de20068572687cbb24cf62e222a6c0082"),
("rgba8_opaque/e0/dcdefault/nobegin", "2a5b6cdaa0037161ef58011eeb93e298a27bfa14f78d3ef28baa760763210112"),
("rgba8_opaque/e0/dcdefault/begin_n", "2a5b6cdaa0037161ef58011eeb93e298a27bfa14f78d3ef28baa760763210112"),
("rgba8_opaque/e0/dcdefault/begin_unk", "2a5b6cdaa0037161ef58011eeb93e298a27bfa14f78d3ef28baa760763210112"),
("rgba8_opaque/e0/dcnone/nobegin", "00ac920331e6e60fb6353cae62c10b4b4511c40b4b98fda3a2e934b0b931c6b4"),
("rgba8_opaque/e0/dcnone/begin_n", "00ac920331e6e60fb6353cae62c10b4b4511c40b4b98fda3a2e934b0b931c6b4"),
("rgba8_opaque/e0/dcnone/begin_unk", "00ac920331e6e60fb6353cae62c10b4b4511c40b4b98fda3a2e934b0b931c6b4"),
("rgba8_opaque/e1/dcdefault/nobegin", "33941cc82f41bfbbefbe2e5a2b3980913589027e7da3218c0500d3d28609df6a"),
("rgba8_opaque/e1/dcdefault/begin_n", "33941cc82f41bfbbefbe2e5a2b3980913589027e7da3218c0500d3d28609df6a"),
("rgba8_opaque/e1/dcdefault/begin_unk", "33941cc82f41bfbbefbe2e5a2b3980913589027e7da3218c0500d3d28609df6a"),
("rgba8_opaque/e1/dcnone/nobegin", "d813f8fbad12bc8eb0a13c748242587b682b87866fc186a4a307ad4addc2dcb5"),
("rgba8_opaque/e1/dcnone/begin_n", "d813f8fbad12bc8eb0a13c748242587b682b87866fc186a4a307ad4addc2dcb5"),
("rgba8_opaque/e1/dcnone/begin_unk", "d813f8fbad12bc8eb0a13c748242587b682b87866fc186a4a307ad4addc2dcb5"),
("rgba8_opaque/e2/dcdefault/nobegin", "f8f4a8b3008110eabf48c6f30c5852199df8f70c7d6cac7b55b845cadce1f957"),
("rgba8_opaque/e2/dcdefault/begin_n", "f8f4a8b3008110eabf48c6f30c5852199df8f70c7d6cac7b55b845cadce1f957"),
("rgba8_opaque/e2/dcdefault/begin_unk", "f8f4a8b3008110eabf48c6f30c5852199df8f70c7d6cac7b55b845cadce1f957"),
("rgba8_opaque/e2/dcnone/nobegin", "536927a17ac1f3df98e7f1f9a05e7044feeed77caa846f4a5365bc842879496d"),
("rgba8_opaque/e2/dcnone/begin_n", "536927a17ac1f3df98e7f1f9a05e7044feeed77caa846f4a5365bc842879496d"),
("rgba8_opaque/e2/dcnone/begin_unk", "536927a17ac1f3df98e7f1f9a05e7044feeed77caa846f4a5365bc842879496d"),
("rgba8_opaque/e5/dcdefault/nobegin", "62f419b307dc791486138842b5c81563433471689f81669c094550e0affa28d1"),
("rgba8_opaque/e5/dcdefault/begin_n", "62f419b307dc791486138842b5c81563433471689f81669c094550e0affa28d1"),
("rgba8_opaque/e5/dcdefault/begin_unk", "62f419b307dc791486138842b5c81563433471689f81669c094550e0affa28d1"),
("rgba8_opaque/e5/dcnone/nobegin", "d7cd51f53af2e8f59a76e56e8c2b3d65f7a09979bb65b2ecaec1a3c7e8e8f05b"),
("rgba8_opaque/e5/dcnone/begin_n", "d7cd51f53af2e8f59a76e56e8c2b3d65f7a09979bb65b2ecaec1a3c7e8e8f05b"),
("rgba8_opaque/e5/dcnone/begin_unk", "d7cd51f53af2e8f59a76e56e8c2b3d65f7a09979bb65b2ecaec1a3c7e8e8f05b"),
("rgba8_opaque/e7/dcdefault/nobegin", "302ed5f3a59aa7cba65222f0cd2911917c91372121073b0654e34a3704f90e23"),
("rgba8_opaque/e7/dcdefault/begin_n", "302ed5f3a59aa7cba65222f0cd2911917c91372121073b0654e34a3704f90e23"),
("rgba8_opaque/e7/dcdefault/begin_unk", "302ed5f3a59aa7cba65222f0cd2911917c91372121073b0654e34a3704f90e23"),
("rgba8_opaque/e7/dcnone/nobegin", "a247ffb1a62b0f7ac3514886b3c6f26bd78aa2216d8590b620c6e1f9ffeaeb76"),
("rgba8_opaque/e7/dcnone/begin_n", "a247ffb1a62b0f7ac3514886b3c6f26bd78aa2216d8590b620c6e1f9ffeaeb76"),
("rgba8_opaque/e7/dcnone/begin_unk", "a247ffb1a62b0f7ac3514886b3c6f26bd78aa2216d8590b620c6e1f9ffeaeb76"),
("rgba8_alpha/default/dcdefault/nobegin", "da46ca9373b576205d6745bc220c550571aaa7c59b688fc9ef13bc284e796447"),
("rgba8_alpha/default/dcdefault/begin_n", "da46ca9373b576205d6745bc220c550571aaa7c59b688fc9ef13bc284e796447"),
("rgba8_alpha/default/dcdefault/begin_unk", "da46ca9373b576205d6745bc220c550571aaa7c59b688fc9ef13bc284e796447"),
("rgba8_alpha/default/dcnone/nobegin", "da46ca9373b576205d6745bc220c550571aaa7c59b688fc9ef13bc284e796447"),
("rgba8_alpha/default/dcnone/begin_n", "da46ca9373b576205d6745bc220c550571aaa7c59b688fc9ef13bc284e796447"),
("rgba8_alpha/default/dcnone/begin_unk", "da46ca9373b576205d6745bc220c550571aaa7c59b688fc9ef13bc284e796447"),
("rgba8_alpha/e0/dcdefault/nobegin", "65d9ed5bd25c5f7f39026d6747448c8c27a1469df923a8031b6fcb17f228286d"),
("rgba8_alpha/e0/dcdefault/begin_n", "65d9ed5bd25c5f7f39026d6747448c8c27a1469df923a8031b6fcb17f228286d"),
("rgba8_alpha/e0/dcdefault/begin_unk", "65d9ed5bd25c5f7f39026d6747448c8c27a1469df923a8031b6fcb17f228286d"),
("rgba8_alpha/e0/dcnone/nobegin", "65d9ed5bd25c5f7f39026d6747448c8c27a1469df923a8031b6fcb17f228286d"),
("rgba8_alpha/e0/dcnone/begin_n", "65d9ed5bd25c5f7f39026d6747448c8c27a1469df923a8031b6fcb17f228286d"),
("rgba8_alpha/e0/dcnone/begin_unk", "65d9ed5bd25c5f7f39026d6747448c8c27a1469df923a8031b6fcb17f228286d"),
("rgba8_alpha/e1/dcdefault/nobegin", "df0e3bd86ce88accbb02adcb0dba0915cf8225f432454fda8a1c0de095940f8e"),
("rgba8_alpha/e1/dcdefault/begin_n", "df0e3bd86ce88accbb02adcb0dba0915cf8225f432454fda8a1c0de095940f8e"),
("rgba8_alpha/e1/dcdefault/begin_unk", "df0e3bd86ce88accbb02adcb0dba0915cf8225f432454fda8a1c0de095940f8e"),
("rgba8_alpha/e1/dcnone/nobegin", "df0e3bd86ce88accbb02adcb0dba0915cf8225f432454fda8a1c0de095940f8e"),
("rgba8_alpha/e1/dcnone/begin_n", "df0e3bd86ce88accbb02adcb0dba0915cf8225f432454fda8a1c0de095940f8e"),
("rgba8_alpha/e1/dcnone/begin_unk", "df0e3bd86ce88accbb02adcb0dba0915cf8225f432454fda8a1c0de095940f8e"),
("rgba8_alpha/e2/dcdefault/nobegin", "df0e3bd86ce88accbb02adcb0dba0915cf8225f432454fda8a1c0de095940f8e"),
("rgba8_alpha/e2/dcdefault/begin_n", "df0e3bd86ce88accbb02adcb0dba0915cf8225f432454fda8a1c0de095940f8e"),
("rgba8_alpha/e2/dcdefault/begin_unk", "df0e3bd86ce88accbb02adcb0dba0915cf8225f432454fda8a1c0de095940f8e"),
("rgba8_alpha/e2/dcnone/nobegin", "df0e3bd86ce88accbb02adcb0dba0915cf8225f432454fda8a1c0de095940f8e"),
("rgba8_alpha/e2/dcnone/begin_n", "df0e3bd86ce88accbb02adcb0dba0915cf8225f432454fda8a1c0de095940f8e"),
("rgba8_alpha/e2/dcnone/begin_unk", "df0e3bd86ce88accbb02adcb0dba0915cf8225f432454fda8a1c0de095940f8e"),
("rgba8_alpha/e5/dcdefault/nobegin", "98bc1ae49e37c6823f808d59804b9deb5bff3cf3cf8de5a5350496ef6e8d7daf"),
("rgba8_alpha/e5/dcdefault/begin_n", "98bc1ae49e37c6823f808d59804b9deb5bff3cf3cf8de5a5350496ef6e8d7daf"),
("rgba8_alpha/e5/dcdefault/begin_unk", "98bc1ae49e37c6823f808d59804b9deb5bff3cf3cf8de5a5350496ef6e8d7daf"),
("rgba8_alpha/e5/dcnone/nobegin", "98bc1ae49e37c6823f808d59804b9deb5bff3cf3cf8de5a5350496ef6e8d7daf"),
("rgba8_alpha/e5/dcnone/begin_n", "98bc1ae49e37c6823f808d59804b9deb5bff3cf3cf8de5a5350496ef6e8d7daf"),
("rgba8_alpha/e5/dcnone/begin_unk", "98bc1ae49e37c6823f808d59804b9deb5bff3cf3cf8de5a5350496ef6e8d7daf"),
("rgba8_alpha/e7/dcdefault/nobegin", "a67947bd9df64aa6d5de29fc22fd4d10dfab3665dd1b2088739d63c4f6f5bb53"),
("rgba8_alpha/e7/dcdefault/begin_n", "a67947bd9df64aa6d5de29fc22fd4d10dfab3665dd1b2088739d63c4f6f5bb53"),
("rgba8_alpha/e7/dcdefault/begin_unk", "a67947bd9df64aa6d5de29fc22fd4d10dfab3665dd1b2088739d63c4f6f5bb53"),
("rgba8_alpha/e7/dcnone/nobegin", "a67947bd9df64aa6d5de29fc22fd4d10dfab3665dd1b2088739d63c4f6f5bb53"),
("rgba8_alpha/e7/dcnone/begin_n", "a67947bd9df64aa6d5de29fc22fd4d10dfab3665dd1b2088739d63c4f6f5bb53"),
("rgba8_alpha/e7/dcnone/begin_unk", "a67947bd9df64aa6d5de29fc22fd4d10dfab3665dd1b2088739d63c4f6f5bb53"),
("bgra8_opaque/default/dcdefault/nobegin", "112ac74adc3120e553f4dd42e00349e5c2f7a3e038090ac171b53705ca30e6ae"),
("bgra8_opaque/default/dcdefault/begin_n", "112ac74adc3120e553f4dd42e00349e5c2f7a3e038090ac171b53705ca30e6ae"),
("bgra8_opaque/default/dcdefault/begin_unk", "112ac74adc3120e553f4dd42e00349e5c2f7a3e038090ac171b53705ca30e6ae"),
("bgra8_opaque/default/dcnone/nobegin", "6bb011543903a4459a430574d12e013de20068572687cbb24cf62e222a6c0082"),
("bgra8_opaque/default/dcnone/begin_n", "6bb011543903a4459a430574d12e013de20068572687cbb24cf62e222a6c0082"),
("bgra8_opaque/default/dcnone/begin_unk", "6bb011543903a4459a430574d12e013de20068572687cbb24cf62e222a6c0082"),
("bgra8_opaque/e0/dcdefault/nobegin", "2a5b6cdaa0037161ef58011eeb93e298a27bfa14f78d3ef28baa760763210112"),
("bgra8_opaque/e0/dcdefault/begin_n", "2a5b6cdaa0037161ef58011eeb93e298a27bfa14f78d3ef28baa760763210112"),
("bgra8_opaque/e0/dcdefault/begin_unk", "2a5b6cdaa0037161ef58011eeb93e298a27bfa14f78d3ef28baa760763210112"),
("bgra8_opaque/e0/dcnone/nobegin", "00ac920331e6e60fb6353cae62c10b4b4511c40b4b98fda3a2e934b0b931c6b4"),
("bgra8_opaque/e0/dcnone/begin_n", "00ac920331e6e60fb6353cae62c10b4b4511c40b4b98fda3a2e934b0b931c6b4"),
("bgra8_opaque/e0/dcnone/begin_unk", "00ac920331e6e60fb6353cae62c10b4b4511c40b4b98fda3a2e934b0b931c6b4"),
("bgra8_opaque/e1/dcdefault/nobegin", "33941cc82f41bfbbefbe2e5a2b3980913589027e7da3218c0500d3d28609df6a"),
("bgra8_opaque/e1/dcdefault/begin_n", "33941cc82f41bfbbefbe2e5a2b3980913589027e7da3218c0500d3d28609df6a"),
("bgra8_opaque/e1/dcdefault/begin_unk", "33941cc82f41bfbbefbe2e5a2b3980913589027e7da3218c0500d3d28609df6a"),
("bgra8_opaque/e1/dcnone/nobegin", "d813f8fbad12bc8eb0a13c748242587b682b87866fc186a4a307ad4addc2dcb5"),
("bgra8_opaque/e1/dcnone/begin_n", "d813f8fbad12bc8eb0a13c748242587b682b87866fc186a4a307ad4addc2dcb5"),
("bgra8_opaque/e1/dcnone/begin_unk", "d813f8fbad12bc8eb0a13c748242587b682b87866fc186a4a307ad4addc2dcb5"),
("bgra8_opaque/e2/dcdefault/nobegin", "f8f4a8b3008110eabf48c6f30c5852199df8f70c7d6cac7b55b845cadce1f957"),
("bgra8_opaque/e2/dcdefault/begin_n", "f8f4a8b3008110eabf48c6f30c5852199df8f70c7d6cac7b55b845cadce1f957"),
("bgra8_opaque/e2/dcdefault/begin_unk", "f8f4a8b3008110eabf48c6f30c5852199df8f70c7d6cac7b55b845cadce1f957"),
("bgra8_opaque/e2/dcnone/nobegin", "536927a17ac1f3df98e7f1f9a05e7044feeed77caa846f4a5365bc842879496d"),
("bgra8_opaque/e2/dcnone/begin_n", "536927a17ac1f3df98e7f1f9a05e7044feeed77caa846f4a5365bc842879496d"),
("bgra8_opaque/e2/dcnone/begin_unk", "536927a17ac1f3df98e7f1f9a05e7044feeed77caa846f4a5365bc842879496d"),
("bgra8_opaque/e5/dcdefault/nobegin", "62f419b307dc791486138842b5c81563433471689f81669c094550e0affa28d1"),
("bgra8_opaque/e5/dcdefault/begin_n", "62f419b307dc791486138842b5c81563433471689f81669c094550e0affa28d1"),
("bgra8_opaque/e5/dcdefault/begin_unk", "62f419b307dc791486138842b5c81563433471689f81669c094550e0affa28d1"),
("bgra8_opaque/e5/dcnone/nobegin", "d7cd51f53af2e8f59a76e56e8c2b3d65f7a09979bb65b2ecaec1a3c7e8e8f05b"),
("bgra8_opaque/e5/dcnone/begin_n", "d7cd51f53af2e8f59a76e56e8c2b3d65f7a09979bb65b2ecaec1a3c7e8e8f05b"),
("bgra8_opaque/e5/dcnone/begin_unk", "d7cd51f53af2e8f59a76e56e8c2b3d65f7a09979bb65b2ecaec1a3c7e8e8f05b"),
("bgra8_opaque/e7/dcdefault/nobegin", "302ed5f3a59aa7cba65222f0cd2911917c91372121073b0654e34a3704f90e23"),
("bgra8_opaque/e7/dcdefault/begin_n", "302ed5f3a59aa7cba65222f0cd2911917c91372121073b0654e34a3704f90e23"),
("bgra8_opaque/e7/dcdefault/begin_unk", "302ed5f3a59aa7cba65222f0cd2911917c91372121073b0654e34a3704f90e23"),
("bgra8_opaque/e7/dcnone/nobegin", "a247ffb1a62b0f7ac3514886b3c6f26bd78aa2216d8590b620c6e1f9ffeaeb76"),
("bgra8_opaque/e7/dcnone/begin_n", "a247ffb1a62b0f7ac3514886b3c6f26bd78aa2216d8590b620c6e1f9ffeaeb76"),
("bgra8_opaque/e7/dcnone/begin_unk", "a247ffb1a62b0f7ac3514886b3c6f26bd78aa2216d8590b620c6e1f9ffeaeb76"),
("gray8/default/dcdefault/nobegin", "046c634ce1821ce6218d2a2c011c7003870f44cc28ae804abc07e33810353b2f"),
("gray8/default/dcdefault/begin_n", "046c634ce1821ce6218d2a2c011c7003870f44cc28ae804abc07e33810353b2f"),
("gray8/default/dcdefault/begin_unk", "046c634ce1821ce6218d2a2c011c7003870f44cc28ae804abc07e33810353b2f"),
("gray8/default/dcnone/nobegin", "046c634ce1821ce6218d2a2c011c7003870f44cc28ae804abc07e33810353b2f"),
("gray8/default/dcnone/begin_n", "046c634ce1821ce6218d2a2c011c7003870f44cc28ae804abc07e33810353b2f"),
("gray8/default/dcnone/begin_unk", "046c634ce1821ce6218d2a2c011c7003870f44cc28ae804abc07e33810353b2f"),
("gray8/e0/dcdefault/nobegin", "ceb634a19c9101b89263c5641cd72e1a701ebfdad15da74b40fe81ea43443441"),
("gray8/e0/dcdefault/begin_n", "ceb634a19c9101b89263c5641cd72e1a701ebfdad15da74b40fe81ea43443441"),
("gray8/e0/dcdefault/begin_unk", "ceb634a19c9101b89263c5641cd72e1a701ebfdad15da74b40fe81ea43443441"),
("gray8/e0/dcnone/nobegin", "ceb634a19c9101b89263c5641cd72e1a701ebfdad15da74b40fe81ea43443441"),
("gray8/e0/dcnone/begin_n", "ceb634a19c9101b89263c5641cd72e1a701ebfdad15da74b40fe81ea43443441"),
("gray8/e0/dcnone/begin_unk", "ceb634a19c9101b89263c5641cd72e1a701ebfdad15da74b40fe81ea43443441"),
("gray8/e1/dcdefault/nobegin", "c502d59e6cdd84489afedf8a56c5614efe08a1e129c65b481abd3e89d71a0a45"),
("gray8/e1/dcdefault/begin_n", "c502d59e6cdd84489afedf8a56c5614efe08a1e129c65b481abd3e89d71a0a45"),
("gray8/e1/dcdefault/begin_unk", "c502d59e6cdd84489afedf8a56c5614efe08a1e129c65b481abd3e89d71a0a45"),
("gray8/e1/dcnone/nobegin", "c502d59e6cdd84489afedf8a56c5614efe08a1e129c65b481abd3e89d71a0a45"),
("gray8/e1/dcnone/begin_n", "c502d59e6cdd84489afedf8a56c5614efe08a1e129c65b481abd3e89d71a0a45"),
("gray8/e1/dcnone/begin_unk", "c502d59e6cdd84489afedf8a56c5614efe08a1e129c65b481abd3e89d71a0a45"),
("gray8/e2/dcdefault/nobegin", "cecddb1349c193bf45baefadf350f1fafeb585e9e7b6455c22da4b7cd8b31dfb"),
("gray8/e2/dcdefault/begin_n", "cecddb1349c193bf45baefadf350f1fafeb585e9e7b6455c22da4b7cd8b31dfb"),
("gray8/e2/dcdefault/begin_unk", "cecddb1349c193bf45baefadf350f1fafeb585e9e7b6455c22da4b7cd8b31dfb"),
("gray8/e2/dcnone/nobegin", "cecddb1349c193bf45baefadf350f1fafeb585e9e7b6455c22da4b7cd8b31dfb"),
("gray8/e2/dcnone/begin_n", "cecddb1349c193bf45baefadf350f1fafeb585e9e7b6455c22da4b7cd8b31dfb"),
("gray8/e2/dcnone/begin_unk", "cecddb1349c193bf45baefadf350f1fafeb585e9e7b6455c22da4b7cd8b31dfb"),
("gray8/e5/dcdefault/nobegin", "2eb8f376d82affa04eb5a54d56dd2b7d0db6b17c25939731f7ff3654d177db54"),
("gray8/e5/dcdefault/begin_n", "2eb8f376d82affa04eb5a54d56dd2b7d0db6b17c25939731f7ff3654d177db54"),
("gray8/e5/dcdefault/begin_unk", "2eb8f376d82affa04eb5a54d56dd2b7d0db6b17c25939731f7ff3654d177db54"),
("gray8/e5/dcnone/nobegin", "2eb8f376d82affa04eb5a54d56dd2b7d0db6b17c25939731f7ff3654d177db54"),
("gray8/e5/dcnone/begin_n", "2eb8f376d82affa04eb5a54d56dd2b7d0db6b17c25939731f7ff3654d177db54"),
("gray8/e5/dcnone/begin_unk", "2eb8f376d82affa04eb5a54d56dd2b7d0db6b17c25939731f7ff3654d177db54"),
("gray8/e7/dcdefault/nobegin", "0e23eaafffabbba5c255162a88569520a8bdb6339ad0fb549de25b8009a4fb25"),
("gray8/e7/dcdefault/begin_n", "0e23eaafffabbba5c255162a88569520a8bdb6339ad0fb549de25b8009a4fb25"),
("gray8/e7/dcdefault/begin_unk", "0e23eaafffabbba5c255162a88569520a8bdb6339ad0fb549de25b8009a4fb25"),
("gray8/e7/dcnone/nobegin", "0e23eaafffabbba5c255162a88569520a8bdb6339ad0fb549de25b8009a4fb25"),
("gray8/e7/dcnone/begin_n", "0e23eaafffabbba5c255162a88569520a8bdb6339ad0fb549de25b8009a4fb25"),
("gray8/e7/dcnone/begin_unk", "0e23eaafffabbba5c255162a88569520a8bdb6339ad0fb549de25b8009a4fb25"),
("graya8_alpha/default/dcdefault/nobegin", "632806037d2ee2f4f32fa3b1ea5b8e52260a2637b022a57c59a876d009e0ef32"),
("graya8_alpha/default/dcdefault/begin_n", "632806037d2ee2f4f32fa3b1ea5b8e52260a2637b022a57c59a876d009e0ef32"),
("graya8_alpha/default/dcdefault/begin_unk", "632806037d2ee2f4f32fa3b1ea5b8e52260a2637b022a57c59a876d009e0ef32"),
("graya8_alpha/default/dcnone/nobegin", "632806037d2ee2f4f32fa3b1ea5b8e52260a2637b022a57c59a876d009e0ef32"),
("graya8_alpha/default/dcnone/begin_n", "632806037d2ee2f4f32fa3b1ea5b8e52260a2637b022a57c59a876d009e0ef32"),
("graya8_alpha/default/dcnone/begin_unk", "632806037d2ee2f4f32fa3b1ea5b8e52260a2637b022a57c59a876d009e0ef32"),
("graya8_alpha/e0/dcdefault/nobegin", "0d4a5aeb4348d80bd08b10304840023262c837290738988ce73193453f39020c"),
("graya8_alpha/e0/dcdefault/begin_n", "0d4a5aeb4348d80bd08b10304840023262c837290738988ce73193453f39020c"),
("graya8_alpha/e0/dcdefault/begin_unk", "0d4a5aeb4348d80bd08b10304840023262c837290738988ce73193453f39020c"),
("graya8_alpha/e0/dcnone/nobegin", "0d4a5aeb4348d80bd08b10304840023262c837290738988ce73193453f39020c"),
("graya8_alpha/e0/dcnone/begin_n", "0d4a5aeb4348d80bd08b10304840023262c837290738988ce73193453f39020c"),
("graya8_alpha/e0/dcnone/begin_unk", "0d4a5aeb4348d80bd08b10304840023262c837290738988ce73193453f39020c"),
("graya8_alpha/e1/dcdefault/nobegin", "5e233eaef3daada9bfce49ceb9dc7d12e082e1773760c1370926050970be420e"),
("graya8_alpha/e1/dcdefault/begin_n", "5e233eaef3daada9bfce49ceb9dc7d12e082e1773760c1370926050970be420e"),
("graya8_alpha/e1/dcdefault/begin_unk", "5e233eaef3daada9bfce49ceb9dc7d12e082e1773760c1370926050970be420e"),
("graya8_alpha/e1/dcnone/nobegin", "5e233eaef3daada9bfce49ceb9dc7d12e082e1773760c1370926050970be420e"),
("graya8_alpha/e1/dcnone/begin_n", "5e233eaef3daada9bfce49ceb9dc7d12e082e1773760c1370926050970be420e"),
("graya8_alpha/e1/dcnone/begin_unk", "5e233eaef3daada9bfce49ceb9dc7d12e082e1773760c1370926050970be420e"),
("graya8_alpha/e2/dcdefault/nobegin", "33655878213f3b28a6d16ddc43020b27685797508f4db5d32a5d38b9940bfe62"),
("graya8_alpha/e2/dcdefault/begin_n", "33655878213f3b28a6d16ddc43020b27685797508f4db5d32a5d38b9940bfe62"),
("graya8_alpha/e2/dcdefault/begin_unk", "33655878213f3b28a6d16ddc43020b27685797508f4db5d32a5d38b9940bfe62"),
("graya8_alpha/e2/dcnone/nobegin", "33655878213f3b28a6d16ddc43020b27685797508f4db5d32a5d38b9940bfe62"),
("graya8_alpha/e2/dcnone/begin_n", "33655878213f3b28a6d16ddc43020b27685797508f4db5d32a5d38b9940bfe62"),
("graya8_alpha/e2/dcnone/begin_unk", "33655878213f3b28a6d16ddc43020b27685797508f4db5d32a5d38b9940bfe62"),
("graya8_alpha/e5/dcdefault/nobegin", "c403bf41292562f5064aec2608687c02ac12cec9742e3c5e6afd2c9c418d9e06"),
("graya8_alpha/e5/dcdefault/begin_n", "c403bf41292562f5064aec2608687c02ac12cec9742e3c5e6afd2c9c418d9e06"),
("graya8_alpha/e5/dcdefault/begin_unk", "c403bf41292562f5064aec2608687c02ac12cec9742e3c5e6afd2c9c418d9e06"),
("graya8_alpha/e5/dcnone/nobegin", "c403bf41292562f5064aec2608687c02ac12cec9742e3c5e6afd2c9c418d9e06"),
("graya8_alpha/e5/dcnone/begin_n", "c403bf41292562f5064aec2608687c02ac12cec9742e3c5e6afd2c9c418d9e06"),
("graya8_alpha/e5/dcnone/begin_unk", "c403bf41292562f5064aec2608687c02ac12cec9742e3c5e6afd2c9c418d9e06"),
("graya8_alpha/e7/dcdefault/nobegin", "188bf2bab2c22f589f2a26fe31cc220d85420ba0281721b25deb5f14df418a89"),
("graya8_alpha/e7/dcdefault/begin_n", "188bf2bab2c22f589f2a26fe31cc220d85420ba0281721b25deb5f14df418a89"),
("graya8_alpha/e7/dcdefault/begin_unk", "188bf2bab2c22f589f2a26fe31cc220d85420ba0281721b25deb5f14df418a89"),
("graya8_alpha/e7/dcnone/nobegin", "188bf2bab2c22f589f2a26fe31cc220d85420ba0281721b25deb5f14df418a89"),
("graya8_alpha/e7/dcnone/begin_n", "188bf2bab2c22f589f2a26fe31cc220d85420ba0281721b25deb5f14df418a89"),
("graya8_alpha/e7/dcnone/begin_unk", "188bf2bab2c22f589f2a26fe31cc220d85420ba0281721b25deb5f14df418a89"),
("rgb16/default/dcdefault/nobegin", "0136f729c8a88b522b32bcd91b2691a4ac87b0417332845fc855adaed1d4e0d0"),
("rgb16/default/dcdefault/begin_n", "0136f729c8a88b522b32bcd91b2691a4ac87b0417332845fc855adaed1d4e0d0"),
("rgb16/default/dcdefault/begin_unk", "0136f729c8a88b522b32bcd91b2691a4ac87b0417332845fc855adaed1d4e0d0"),
("rgb16/default/dcnone/nobegin", "0136f729c8a88b522b32bcd91b2691a4ac87b0417332845fc855adaed1d4e0d0"),
("rgb16/default/dcnone/begin_n", "0136f729c8a88b522b32bcd91b2691a4ac87b0417332845fc855adaed1d4e0d0"),
("rgb16/default/dcnone/begin_unk", "0136f729c8a88b522b32bcd91b2691a4ac87b0417332845fc855adaed1d4e0d0"),
("rgb16/e0/dcdefault/nobegin", "84cc943590f842549422cff5614951d8cf4fc114444acb08be75c9b295670a96"),
("rgb16/e0/dcdefault/begin_n", "84cc943590f842549422cff5614951d8cf4fc114444acb08be75c9b295670a96"),
("rgb16/e0/dcdefault/begin_unk", "84cc943590f842549422cff5614951d8cf4fc114444acb08be75c9b295670a96"),
("rgb16/e0/dcnone/nobegin", "84cc943590f842549422cff5614951d8cf4fc114444acb08be75c9b295670a96"),
("rgb16/e0/dcnone/begin_n", "84cc943590f842549422cff5614951d8cf4fc114444acb08be75c9b295670a96"),
("rgb16/e0/dcnone/begin_unk", "84cc943590f842549422cff5614951d8cf4fc114444acb08be75c9b295670a96"),
("rgb16/e1/dcdefault/nobegin", "8d69fd7ff6fae93d4ea6b17f7b2db7728f68ab08e44949ff306b91cae0b0bb5e"),
("rgb16/e1/dcdefault/begin_n", "8d69fd7ff6fae93d4ea6b17f7b2db7728f68ab08e44949ff306b91cae0b0bb5e"),
("rgb16/e1/dcdefault/begin_unk", "8d69fd7ff6fae93d4ea6b17f7b2db7728f68ab08e44949ff306b91cae0b0bb5e"),
("rgb16/e1/dcnone/nobegin", "8d69fd7ff6fae93d4ea6b17f7b2db7728f68ab08e44949ff306b91cae0b0bb5e"),
("rgb16/e1/dcnone/begin_n", "8d69fd7ff6fae93d4ea6b17f7b2db7728f68ab08e44949ff306b91cae0b0bb5e"),
("rgb16/e1/dcnone/begin_unk", "8d69fd7ff6fae93d4ea6b17f7b2db7728f68ab08e44949ff306b91cae0b0bb5e"),
("rgb16/e2/dcdefault/nobegin", "fee9f510b4ddf63626768cf5a888b80fedd458bd99fb606b920202898b6efbc4"),
("rgb16/e2/dcdefault/begin_n", "fee9f510b4ddf63626768cf5a888b80fedd458bd99fb606b920202898b6efbc4"),
("rgb16/e2/dcdefault/begin_unk", "fee9f510b4ddf63626768cf5a888b80fedd458bd99fb606b920202898b6efbc4"),
("rgb16/e2/dcnone/nobegin", "fee9f510b4ddf63626768cf5a888b80fedd458bd99fb606b920202898b6efbc4"),
("rgb16/e2/dcnone/begin_n", "fee9f510b4ddf63626768cf5a888b80fedd458bd99fb606b920202898b6efbc4"),
("rgb16/e2/dcnone/begin_unk", "fee9f510b4ddf63626768cf5a888b80fedd458bd99fb606b920202898b6efbc4"),
("rgb16/e5/dcdefault/nobegin", "9250f3731eade57dd2e3f0ce09d87ea2fce2f300ac0ac4246b02d0bd7634d03e"),
("rgb16/e5/dcdefault/begin_n", "9250f3731eade57dd2e3f0ce09d87ea2fce2f300ac0ac4246b02d0bd7634d03e"),
("rgb16/e5/dcdefault/begin_unk", "9250f3731eade57dd2e3f0ce09d87ea2fce2f300ac0ac4246b02d0bd7634d03e"),
("rgb16/e5/dcnone/nobegin", "9250f3731eade57dd2e3f0ce09d87ea2fce2f300ac0ac4246b02d0bd7634d03e"),
("rgb16/e5/dcnone/begin_n", "9250f3731eade57dd2e3f0ce09d87ea2fce2f300ac0ac4246b02d0bd7634d03e"),
("rgb16/e5/dcnone/begin_unk", "9250f3731eade57dd2e3f0ce09d87ea2fce2f300ac0ac4246b02d0bd7634d03e"),
("rgb16/e7/dcdefault/nobegin", "9250f3731eade57dd2e3f0ce09d87ea2fce2f300ac0ac4246b02d0bd7634d03e"),
("rgb16/e7/dcdefault/begin_n", "9250f3731eade57dd2e3f0ce09d87ea2fce2f300ac0ac4246b02d0bd7634d03e"),
("rgb16/e7/dcdefault/begin_unk", "9250f3731eade57dd2e3f0ce09d87ea2fce2f300ac0ac4246b02d0bd7634d03e"),
("rgb16/e7/dcnone/nobegin", "9250f3731eade57dd2e3f0ce09d87ea2fce2f300ac0ac4246b02d0bd7634d03e"),
("rgb16/e7/dcnone/begin_n", "9250f3731eade57dd2e3f0ce09d87ea2fce2f300ac0ac4246b02d0bd7634d03e"),
("rgb16/e7/dcnone/begin_unk", "9250f3731eade57dd2e3f0ce09d87ea2fce2f300ac0ac4246b02d0bd7634d03e"),
("rgba16_alpha/default/dcdefault/nobegin", "6991bf38444845d1ee7ba365fcd8d75423a26c87794b539513518de4973e9143"),
("rgba16_alpha/default/dcdefault/begin_n", "6991bf38444845d1ee7ba365fcd8d75423a26c87794b539513518de4973e9143"),
("rgba16_alpha/default/dcdefault/begin_unk", "6991bf38444845d1ee7ba365fcd8d75423a26c87794b539513518de4973e9143"),
("rgba16_alpha/default/dcnone/nobegin", "6991bf38444845d1ee7ba365fcd8d75423a26c87794b539513518de4973e9143"),
("rgba16_alpha/default/dcnone/begin_n", "6991bf38444845d1ee7ba365fcd8d75423a26c87794b539513518de4973e9143"),
("rgba16_alpha/default/dcnone/begin_unk", "6991bf38444845d1ee7ba365fcd8d75423a26c87794b539513518de4973e9143"),
("rgba16_alpha/e0/dcdefault/nobegin", "dc0ba317b989118574157d9237f6576cab27fc73c3e9a26222c3577934c43b3d"),
("rgba16_alpha/e0/dcdefault/begin_n", "dc0ba317b989118574157d9237f6576cab27fc73c3e9a26222c3577934c43b3d"),
("rgba16_alpha/e0/dcdefault/begin_unk", "dc0ba317b989118574157d9237f6576cab27fc73c3e9a26222c3577934c43b3d"),
("rgba16_alpha/e0/dcnone/nobegin", "dc0ba317b989118574157d9237f6576cab27fc73c3e9a26222c3577934c43b3d"),
("rgba16_alpha/e0/dcnone/begin_n", "dc0ba317b989118574157d9237f6576cab27fc73c3e9a26222c3577934c43b3d"),
("rgba16_alpha/e0/dcnone/begin_unk", "dc0ba317b989118574157d9237f6576cab27fc73c3e9a26222c3577934c43b3d"),
("rgba16_alpha/e1/dcdefault/nobegin", "eed6d53fa9534f3618704a7bcf135e69e5245134ed4360d301a2aa9873da4884"),
("rgba16_alpha/e1/dcdefault/begin_n", "eed6d53fa9534f3618704a7bcf135e69e5245134ed4360d301a2aa9873da4884"),
("rgba16_alpha/e1/dcdefault/begin_unk", "eed6d53fa9534f3618704a7bcf135e69e5245134ed4360d301a2aa9873da4884"),
("rgba16_alpha/e1/dcnone/nobegin", "eed6d53fa9534f3618704a7bcf135e69e5245134ed4360d301a2aa9873da4884"),
("rgba16_alpha/e1/dcnone/begin_n", "eed6d53fa9534f3618704a7bcf135e69e5245134ed4360d301a2aa9873da4884"),
("rgba16_alpha/e1/dcnone/begin_unk", "eed6d53fa9534f3618704a7bcf135e69e5245134ed4360d301a2aa9873da4884"),
("rgba16_alpha/e2/dcdefault/nobegin", "cf465f948d8de5405da3db055b582589a1bb79c703a7966d55a55b132ed98309"),
("rgba16_alpha/e2/dcdefault/begin_n", "cf465f948d8de5405da3db055b582589a1bb79c703a7966d55a55b132ed98309"),
("rgba16_alpha/e2/dcdefault/begin_unk", "cf465f948d8de5405da3db055b582589a1bb79c703a7966d55a55b132ed98309"),
("rgba16_alpha/e2/dcnone/nobegin", "cf465f948d8de5405da3db055b582589a1bb79c703a7966d55a55b132ed98309"),
("rgba16_alpha/e2/dcnone/begin_n", "cf465f948d8de5405da3db055b582589a1bb79c703a7966d55a55b132ed98309"),
("rgba16_alpha/e2/dcnone/begin_unk", "cf465f948d8de5405da3db055b582589a1bb79c703a7966d55a55b132ed98309"),
("rgba16_alpha/e5/dcdefault/nobegin", "1cb0394784936c82fff39aa23348730caa4e0f16caad71324074d5a7a22378d6"),
("rgba16_alpha/e5/dcdefault/begin_n", "1cb0394784936c82fff39aa23348730caa4e0f16caad71324074d5a7a22378d6"),
("rgba16_alpha/e5/dcdefault/begin_unk", "1cb0394784936c82fff39aa23348730caa4e0f16caad71324074d5a7a22378d6"),
("rgba16_alpha/e5/dcnone/nobegin", "1cb0394784936c82fff39aa23348730caa4e0f16caad71324074d5a7a22378d6"),
("rgba16_alpha/e5/dcnone/begin_n", "1cb0394784936c82fff39aa23348730caa4e0f16caad71324074d5a7a22378d6"),
("rgba16_alpha/e5/dcnone/begin_unk", "1cb0394784936c82fff39aa23348730caa4e0f16caad71324074d5a7a22378d6"),
("rgba16_alpha/e7/dcdefault/nobegin", "19a0ac28671a4061988a968da2239aa17f33cdc71df52f4a845c9e0c6a795974"),
("rgba16_alpha/e7/dcdefault/begin_n", "19a0ac28671a4061988a968da2239aa17f33cdc71df52f4a845c9e0c6a795974"),
("rgba16_alpha/e7/dcdefault/begin_unk", "19a0ac28671a4061988a968da2239aa17f33cdc71df52f4a845c9e0c6a795974"),
("rgba16_alpha/e7/dcnone/nobegin", "19a0ac28671a4061988a968da2239aa17f33cdc71df52f4a845c9e0c6a795974"),
("rgba16_alpha/e7/dcnone/begin_n", "19a0ac28671a4061988a968da2239aa17f33cdc71df52f4a845c9e0c6a795974"),
("rgba16_alpha/e7/dcnone/begin_unk", "19a0ac28671a4061988a968da2239aa17f33cdc71df52f4a845c9e0c6a795974"),
];
