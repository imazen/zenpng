//! APNG frames streamed through `AnimationFrameEncoder::begin`. With the
//! frame count declared, the cross-frame optimizer off and a colour type
//! that cannot depend on later frames, each frame is written as it is
//! pushed: lag 0, byte-identical to the same encode without `begin`.

#![cfg(not(target_arch = "wasm32"))]

use zencodec::encode::{AnimationFrameEncoder, AnimationOutputTiming, EncodeJob, EncoderConfig};
use zenpixels::{PixelDescriptor, PixelSlice};
use zenpng::{Compression, DowncastFlags, PngDecoderConfig, PngEncoderConfig};

const RGBA8: PixelDescriptor = PixelDescriptor::RGBA8_SRGB;
const RGB8: PixelDescriptor = PixelDescriptor::RGB8_SRGB;

/// `n` frames of a moving block over a gradient; RGBA with varying alpha,
/// or RGB.
fn frames(n: usize, w: u32, h: u32, desc: PixelDescriptor) -> Vec<Vec<u8>> {
    let bpp = desc.bytes_per_pixel();
    (0..n)
        .map(|f| {
            (0..w * h)
                .flat_map(|i| {
                    let (x, y) = (i % w, i / w);
                    let lit = x as usize >= f * 3 && (x as usize) < f * 3 + 9 && y < h / 2;
                    let px = [
                        (x * 5) as u8 ^ if lit { 0xC0 } else { 0 },
                        (y * 7) as u8,
                        (f * 30) as u8,
                        ((x + y * 3) % 256) as u8,
                    ];
                    px.into_iter().take(bpp)
                })
                .collect()
        })
        .collect()
}

fn cfg(effort: u32, downcast: Option<DowncastFlags>) -> PngEncoderConfig {
    let c = PngEncoderConfig::new().with_compression(Compression::Effort(effort));
    match downcast {
        Some(d) => c.with_downcast(d),
        None => c,
    }
}

/// Encode `frames`. With `declare`, call `begin` first (with that frame
/// count) and drain after `begin` and after every frame. Returns the
/// timing, the file, and the bytes available after `begin` and each frame.
fn encode(
    cfg: &PngEncoderConfig,
    frames: &[Vec<u8>],
    (w, h, desc): (u32, u32, PixelDescriptor),
    declare: Option<Option<u32>>,
) -> (Option<AnimationOutputTiming>, Vec<u8>, Vec<usize>) {
    let mut a = cfg
        .clone()
        .job()
        .with_loop_count(Some(0))
        .animation_frame_encoder()
        .unwrap();
    let timing = declare.map(|n| a.begin(w, h, desc, n).unwrap());
    let mut out = Vec::new();
    let mut seen = vec![a.pending_output().len()];
    a.take_output(&mut out);
    for (i, f) in frames.iter().enumerate() {
        let s = PixelSlice::new(f, w, h, w as usize * desc.bytes_per_pixel(), desc).unwrap();
        a.push_frame(s, 40 + i as u32 * 10, None).unwrap();
        seen.push(out.len() + a.pending_output().len());
        let mut chunk = Vec::new();
        a.take_output(&mut chunk);
        out.extend_from_slice(&chunk);
    }
    out.extend_from_slice(a.finish(None).unwrap().data());
    (timing, out, seen)
}

#[test]
fn low_effort_streams_each_frame_byte_identical_to_one_shot() {
    let (w, h) = (40, 24);
    for effort in [0, 1, 2] {
        // Transparent RGBA: streams once the RGBA-to-RGB reduction is off.
        let c = cfg(effort, Some(DowncastFlags::none()));
        let fr = frames(5, w, h, RGBA8);
        let (timing, streamed, seen) = encode(&c, &fr, (w, h, RGBA8), Some(Some(5)));
        assert_eq!(
            timing,
            Some(AnimationOutputTiming::streaming(0)),
            "effort {effort}"
        );
        let (_, whole, _) = encode(&c, &fr, (w, h, RGBA8), None);
        assert_eq!(streamed, whole, "effort {effort}");
        // The header waits for the first frame; then every frame adds bytes.
        assert_eq!(seen[0], 0);
        assert!(seen.windows(2).all(|p| p[1] > p[0]), "{seen:?}");
    }
}

#[test]
fn rgb_input_streams_with_the_default_reductions() {
    let (w, h) = (40, 24);
    let c = cfg(1, None);
    let fr = frames(4, w, h, RGB8);
    let (timing, streamed, _) = encode(&c, &fr, (w, h, RGB8), Some(Some(4)));
    assert_eq!(timing, Some(AnimationOutputTiming::streaming(0)));
    let (_, whole, _) = encode(&c, &fr, (w, h, RGB8), None);
    assert_eq!(streamed, whole);
    assert_eq!(streamed[25], 2, "opaque frames are written as RGB");
}

#[test]
fn configurations_that_need_every_frame_report_at_finish() {
    let (w, h) = (40, 24);
    let begin = |c: PngEncoderConfig, desc, n| {
        c.job()
            .animation_frame_encoder()
            .unwrap()
            .begin(w, h, desc, n)
            .unwrap()
    };
    let none = Some(DowncastFlags::none());
    // No frame count: acTL comes first.
    assert_eq!(
        begin(cfg(1, none), RGBA8, None),
        AnimationOutputTiming::AT_FINISH
    );
    // The cross-frame optimizer.
    assert_eq!(
        begin(cfg(7, none), RGBA8, Some(3)),
        AnimationOutputTiming::AT_FINISH
    );
    // RGBA that may turn out opaque, with the RGB reduction on.
    assert_eq!(
        begin(cfg(1, None), RGBA8, Some(3)),
        AnimationOutputTiming::AT_FINISH
    );
    // The default effort.
    assert_eq!(
        begin(PngEncoderConfig::new(), RGB8, Some(3)),
        AnimationOutputTiming::AT_FINISH
    );
}

#[test]
fn the_declared_frame_count_binds() {
    let (w, h) = (16, 8);
    let fr = frames(3, w, h, RGBA8);
    for c in [cfg(1, Some(DowncastFlags::none())), cfg(7, None)] {
        let mut a = c.clone().job().animation_frame_encoder().unwrap();
        a.begin(w, h, RGBA8, Some(2)).unwrap();
        for f in &fr[..2] {
            let s = PixelSlice::new(f, w, h, w as usize * 4, RGBA8).unwrap();
            a.push_frame(s, 40, None).unwrap();
        }
        let s = PixelSlice::new(&fr[2], w, h, w as usize * 4, RGBA8).unwrap();
        assert!(a.push_frame(s, 40, None).is_err(), "a third of two frames");
        let mut a = c.clone().job().animation_frame_encoder().unwrap();
        a.begin(w, h, RGBA8, Some(3)).unwrap();
        let s = PixelSlice::new(&fr[0], w, h, w as usize * 4, RGBA8).unwrap();
        a.push_frame(s, 40, None).unwrap();
        assert!(a.finish(None).is_err(), "one of three frames");
        let mut a = c.job().animation_frame_encoder().unwrap();
        a.begin(w, h, RGBA8, Some(3)).unwrap();
        assert!(a.begin(w, h, RGBA8, Some(3)).is_err(), "begin twice");
    }
}

#[test]
fn the_rgb_reduction_flag_is_honored_without_begin() {
    // Opaque RGBA frames: RGB with the reduction on, RGBA with it off.
    let (w, h) = (16, 8);
    let fr: Vec<Vec<u8>> = frames(2, w, h, RGB8)
        .into_iter()
        .map(|f| {
            f.as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect()
        })
        .collect();
    let (_, on, _) = encode(&cfg(1, None), &fr, (w, h, RGBA8), None);
    let (_, off, _) = encode(
        &cfg(1, Some(DowncastFlags::none())),
        &fr,
        (w, h, RGBA8),
        None,
    );
    assert_eq!(on[25], 2);
    assert_eq!(off[25], 6);
}

#[test]
fn testkit_animation_streaming_output_honesty() {
    let fr: Vec<zencodec_testkit::TestImage> = (0..6)
        .map(|i| zencodec_testkit::TestImage::rgba8_gradient_seeded(24, 16, i * 40))
        .collect();
    for c in [
        cfg(1, Some(DowncastFlags::none())),
        cfg(0, Some(DowncastFlags::none())),
        PngEncoderConfig::new(),
    ] {
        zencodec_testkit::check_animation_streaming_output_honesty(c, PngDecoderConfig::new(), &fr)
            .unwrap();
    }
}
