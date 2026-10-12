//! The encoder writes the same bytes whatever threads it may use: images
//! of two or more strips always get the strip layout (compressed strip by
//! strip, concurrently only when threads are allowed), and the searches at
//! higher efforts don't depend on thread scheduling.

use enough::Unstoppable;
use imgref::ImgVec;
use rgb::{Gray, Rgb, Rgba};
use zenpng::{Compression, EncodeConfig};

/// Smooth bands plus noise, so different strips pick different filters.
fn bytes(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed | 1;
    (0..n)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            if (i / 5000) % 3 == 0 {
                (x >> 24) as u8
            } else {
                (i / 11) as u8 ^ (x & 7) as u8
            }
        })
        .collect()
}

/// Thread settings that must all give the same bytes.
fn configs(effort: u32) -> Vec<(&'static str, EncodeConfig)> {
    let base = EncodeConfig::default().with_compression(Compression::Effort(effort));
    let threads = |t: usize| {
        let mut c = base.clone().with_parallel(true);
        c.max_threads = t;
        c
    };
    vec![
        ("serial", base.clone()),
        ("parallel, 1 thread", threads(1)),
        ("parallel, 2 threads", threads(2)),
        ("parallel, 8 threads", threads(8)),
    ]
}

fn check<F: Fn(&EncodeConfig) -> Vec<u8>>(name: &str, efforts: &[u32], encode: F) {
    for &e in efforts {
        let mut first: Option<(&str, Vec<u8>)> = None;
        for (label, cfg) in configs(e) {
            let out = encode(&cfg);
            match &first {
                None => first = Some((label, out)),
                Some((l0, o0)) => assert!(
                    &out == o0,
                    "{name} e{e}: {label} wrote {} bytes, {l0} {} bytes",
                    out.len(),
                    o0.len()
                ),
            }
        }
    }
}

const EFFORTS: &[u32] = &[1, 2, 5, 7, 10, 12, 13, 15];

#[test]
fn same_bytes_for_every_thread_setting() {
    // RGB8 1024 wide: 3073 filtered bytes a row, 2 strips from ~342 rows.
    for h in [300usize, 360, 700] {
        let px = bytes(1024 * h * 3, h as u32);
        let img = ImgVec::new(bytemuck::cast_slice::<u8, Rgb<u8>>(&px).to_vec(), 1024, h);
        check(&format!("rgb8 1024x{h}"), EFFORTS, |c| {
            zenpng::encode_rgb8(img.as_ref(), None, c, &Unstoppable, &Unstoppable).unwrap()
        });
    }
    let px = bytes(512 * 600 * 4, 7);
    let img = ImgVec::new(bytemuck::cast_slice::<u8, Rgba<u8>>(&px).to_vec(), 512, 600);
    check("rgba8 512x600", EFFORTS, |c| {
        zenpng::encode_rgba8(img.as_ref(), None, c, &Unstoppable, &Unstoppable).unwrap()
    });
    let px = bytes(2048 * 600, 9);
    let img = ImgVec::new(
        bytemuck::cast_slice::<u8, Gray<u8>>(&px).to_vec(),
        2048,
        600,
    );
    check("gray8 2048x600", EFFORTS, |c| {
        zenpng::encode_gray8(img.as_ref(), None, c, &Unstoppable, &Unstoppable).unwrap()
    });
    let px = bytes(512 * 400 * 6, 11);
    let img = ImgVec::new(bytemuck::cast_slice::<u8, Rgb<u16>>(&px).to_vec(), 512, 400);
    check("rgb16 512x400", EFFORTS, |c| {
        zenpng::encode_rgb16(img.as_ref(), None, c, &Unstoppable, &Unstoppable).unwrap()
    });
    let px = bytes(256 * 600 * 8, 13);
    let img = ImgVec::new(
        bytemuck::cast_slice::<u8, Rgba<u16>>(&px).to_vec(),
        256,
        600,
    );
    check("rgba16 256x600", EFFORTS, |c| {
        zenpng::encode_rgba16(img.as_ref(), None, c, &Unstoppable, &Unstoppable).unwrap()
    });
    let px = bytes(1024 * 600 * 2, 15);
    let img = ImgVec::new(
        bytemuck::cast_slice::<u8, Gray<u16>>(&px).to_vec(),
        1024,
        600,
    );
    check("gray16 1024x600", EFFORTS, |c| {
        zenpng::encode_gray16(img.as_ref(), None, c, &Unstoppable, &Unstoppable).unwrap()
    });
}

/// Efforts 16+ search the whole image (brute force, fork, beam), with
/// threads for screening, refinement and recompression when allowed.
#[test]
fn same_bytes_at_whole_image_efforts() {
    let px = bytes(1024 * 360 * 3, 17);
    let img = ImgVec::new(bytemuck::cast_slice::<u8, Rgb<u8>>(&px).to_vec(), 1024, 360);
    check("rgb8 1024x360", &[16, 19, 21], |c| {
        zenpng::encode_rgb8(img.as_ref(), None, c, &Unstoppable, &Unstoppable).unwrap()
    });
}
