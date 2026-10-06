//! Encoders reuse zenflate compressors across encodes on a thread. Output must
//! not depend on what the thread encoded before.

use imgref::ImgVec;
use rgb::Rgb;
use zenflate::Unstoppable;
use zenpng::{Compression, EncodeConfig, encode_rgb8};

fn image(w: usize, h: usize, seed: u32) -> ImgVec<Rgb<u8>> {
    let mut s = seed | 1;
    let px = (0..w * h)
        .map(|i| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            Rgb {
                r: (i % w) as u8,
                g: (s >> 28) as u8,
                b: (i / w) as u8,
            }
        })
        .collect();
    ImgVec::new(px, w, h)
}

#[test]
fn reused_compressors_give_the_same_bytes() {
    let a = image(64, 48, 1);
    let b = image(200, 150, 2);
    for effort in [1u32, 2, 3, 5, 7, 9, 13] {
        let cfg = EncodeConfig::default().with_compression(Compression::Effort(effort));
        let enc = |img: &ImgVec<Rgb<u8>>| {
            encode_rgb8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap()
        };
        let fresh = std::thread::scope(|s| s.spawn(|| enc(&a)).join().unwrap());
        let first = enc(&a);
        let _ = enc(&b);
        let again = enc(&a);
        assert_eq!(
            first, fresh,
            "effort {effort}: first encode differs from a fresh thread"
        );
        assert_eq!(
            again, fresh,
            "effort {effort}: encode after another image differs"
        );
    }
}
