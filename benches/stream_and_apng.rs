// Lane-attribution bench (swe-zenpng): exercises decode / decode_apng
// (IDAT + fdAT chunk-source paths) and encode on deterministic NON-smooth
// synthetic fixtures.
use rgb::FromSlice;

fn noise_px(w: usize, h: usize, ch: usize, seed: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(w * h * ch);
    let mut s: u32 = seed;
    for _ in 0..w * h * ch {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        v.push((s >> 8) as u8);
    }
    v
}

fn main() {
    let rgb_px = noise_px(256, 256, 3, 0x9e3779b9);
    let rgba_px = noise_px(128, 128, 4, 0x85ebca6b);
    let png = zenpng::encode_rgb8(
        imgref::ImgRef::new(rgb_px.as_rgb(), 256, 256),
        None,
        &zenpng::EncodeConfig::default(),
        &enough::Unstoppable,
        &enough::Unstoppable,
    )
    .unwrap();
    let frame = zenpng::ApngFrameInput::new(&rgba_px, 1, 30);
    let apng = zenpng::encode_apng(
        &[frame, frame, frame],
        128,
        128,
        &zenpng::ApngEncodeConfig::default(),
        None,
        &enough::Unstoppable,
        &enough::Unstoppable,
    )
    .unwrap();
    let result = zenbench::run_gated(zenbench::GateConfig::disabled(), |suite| {
        suite.compare("stream_apng", move |g| {
            let png = png.clone();
            g.bench("decode_png_256", move |b| {
                let png = png.clone();
                b.iter(move || {
                    let out = zenpng::decode(
                        &png,
                        &zenpng::PngDecodeConfig::default(),
                        &enough::Unstoppable,
                    )
                    .unwrap();
                    zenbench::black_box(out.info.width)
                })
            });
            let apng = apng.clone();
            g.bench("decode_apng_128x3", move |b| {
                let apng = apng.clone();
                b.iter(move || {
                    let out = zenpng::decode_apng(
                        &apng,
                        &zenpng::PngDecodeConfig::default(),
                        &enough::Unstoppable,
                    )
                    .unwrap();
                    zenbench::black_box(out.frames.len())
                })
            });
            let rgb_px = rgb_px.clone();
            g.bench("encode_rgb8_256", move |b| {
                let rgb_px = rgb_px.clone();
                b.iter(move || {
                    zenbench::black_box(
                        zenpng::encode_rgb8(
                            imgref::ImgRef::new(rgb_px.as_rgb(), 256, 256),
                            None,
                            &zenpng::EncodeConfig::default(),
                            &enough::Unstoppable,
                            &enough::Unstoppable,
                        )
                        .unwrap()
                        .len(),
                    )
                })
            });
        });
    });
    zenbench::postprocess_result(&result);
}
