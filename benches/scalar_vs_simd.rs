//! Runtime-dispatched PNG scan predicates versus their scalar references.
//! Four sizes exercise full scans; scalar loops may auto-vectorize on ARM.
//! Run: cargo bench --bench scalar_vs_simd --features _dev

use zenbench::prelude::*;

use zenpng::__bench_scan as scan;

fn rgba8_all_pass(w: usize, h: usize) -> Vec<u8> {
    let n = w * h;
    let mut v = Vec::with_capacity(n * 4);
    for i in 0..n {
        let g = (i * 7 + 3) as u8;
        v.extend_from_slice(&[g, g, g, if i & 3 == 0 { 0 } else { 255 }]);
    }
    v
}

fn rgb8_all_gray(w: usize, h: usize) -> Vec<u8> {
    let n = w * h;
    let mut v = Vec::with_capacity(n * 3);
    for i in 0..n {
        let g = (i * 7 + 3) as u8;
        v.extend_from_slice(&[g, g, g]);
    }
    v
}

fn be16_all_replicated(w: usize, h: usize) -> Vec<u8> {
    let n = w * h;
    let mut v = Vec::with_capacity(n * 8);
    for i in 0..n {
        let r = (i * 3 + 1) as u8;
        let g = (i * 5 + 7) as u8;
        let b = (i * 7 + 11) as u8;
        let a = (i * 11 + 13) as u8;
        v.extend_from_slice(&[r, r, g, g, b, b, a, a]);
    }
    v
}

fn build_size_group(suite: &mut Suite, w: usize, h: usize, label: &'static str) {
    let pixels = (w * h) as u64;
    let rgba_input = rgba8_all_pass(w, h);
    let mut opaque_input = rgba_input.clone();
    for pixel in opaque_input.as_chunks_mut::<4>().0 {
        pixel[3] = 255;
    }
    assert!(scan::scalar_is_opaque_rgba8(&opaque_input));
    assert!(scan::is_opaque_rgba8(&opaque_input));
    assert_eq!(
        scan::scalar_is_grayscale_rgba8(&rgba_input),
        scan::is_grayscale_rgba8(&rgba_input)
    );
    assert_eq!(
        scan::scalar_alpha_is_binary_rgba8(&rgba_input),
        scan::alpha_is_binary_rgba8(&rgba_input)
    );
    let rgb_input = rgb8_all_gray(w, h);
    let be16_input = be16_all_replicated(w, h);
    assert_eq!(
        scan::scalar_is_grayscale_rgb8(&rgb_input),
        scan::is_grayscale_rgb8(&rgb_input)
    );
    assert_eq!(
        scan::scalar_bit_replication_lossless_be16(&be16_input),
        scan::bit_replication_lossless_be16(&be16_input)
    );
    let req = scan::FusedRequest::all();
    let expected = scan::scalar_fused_predicates_rgba8(&rgba_input, req);
    assert_eq!(expected, scan::fused_predicates_rgba8(&rgba_input, req));
    assert_eq!(expected, scan::fused_predicates_rgba8_cg(&rgba_input, req));

    suite.compare(format!("{label}/is_opaque_rgba8"), |g| {
        g.throughput(Throughput::Elements(pixels));
        g.throughput_unit("px");
        let s = opaque_input.clone();
        g.bench("scalar", move |b| {
            b.iter(|| zenbench::black_box(scan::scalar_is_opaque_rgba8(&s)))
        });
        let v = opaque_input;
        g.bench("runtime_simd", move |b| {
            b.iter(|| zenbench::black_box(scan::is_opaque_rgba8(&v)))
        });
    });

    suite.compare(format!("{label}/is_grayscale_rgba8"), |g| {
        g.throughput(Throughput::Elements(pixels));
        g.throughput_unit("px");
        let s = rgba_input.clone();
        g.bench("scalar", move |b| {
            b.iter(|| zenbench::black_box(scan::scalar_is_grayscale_rgba8(&s)))
        });
        let v = rgba_input.clone();
        g.bench("runtime_simd", move |b| {
            b.iter(|| zenbench::black_box(scan::is_grayscale_rgba8(&v)))
        });
    });

    suite.compare(format!("{label}/alpha_is_binary_rgba8"), |g| {
        g.throughput(Throughput::Elements(pixels));
        g.throughput_unit("px");
        let s = rgba_input.clone();
        g.bench("scalar", move |b| {
            b.iter(|| zenbench::black_box(scan::scalar_alpha_is_binary_rgba8(&s)))
        });
        let v = rgba_input.clone();
        g.bench("runtime_simd", move |b| {
            b.iter(|| zenbench::black_box(scan::alpha_is_binary_rgba8(&v)))
        });
    });

    suite.compare(format!("{label}/is_grayscale_rgb8"), |g| {
        g.throughput(Throughput::Elements(pixels));
        g.throughput_unit("px");
        let s = rgb_input.clone();
        g.bench("scalar", move |b| {
            b.iter(|| zenbench::black_box(scan::scalar_is_grayscale_rgb8(&s)))
        });
        let v = rgb_input;
        g.bench("runtime_simd", move |b| {
            b.iter(|| zenbench::black_box(scan::is_grayscale_rgb8(&v)))
        });
    });

    suite.compare(format!("{label}/bit_replication_be16"), |g| {
        g.throughput(Throughput::Elements(pixels));
        g.throughput_unit("px");
        let s = be16_input.clone();
        g.bench("scalar", move |b| {
            b.iter(|| zenbench::black_box(scan::scalar_bit_replication_lossless_be16(&s)))
        });
        let v = be16_input;
        g.bench("runtime_simd", move |b| {
            b.iter(|| zenbench::black_box(scan::bit_replication_lossless_be16(&v)))
        });
    });

    suite.compare(format!("{label}/fused_three_checks"), |g| {
        g.throughput(Throughput::Elements(pixels));
        g.throughput_unit("px");
        let req = scan::FusedRequest {
            check_opaque: true,
            check_grayscale: true,
            check_binary_alpha: true,
        };
        let s = rgba_input.clone();
        g.bench("scalar", move |b| {
            b.iter(|| zenbench::black_box(scan::scalar_fused_predicates_rgba8(&s, req)))
        });
        let v = rgba_input.clone();
        g.bench("simd_runtime", move |b| {
            b.iter(|| zenbench::black_box(scan::fused_predicates_rgba8(&v, req)))
        });
        let v = rgba_input;
        g.bench("simd_const_generic", move |b| {
            b.iter(|| zenbench::black_box(scan::fused_predicates_rgba8_cg(&v, req)))
        });
    });
}

fn bench_scalar_vs_simd(suite: &mut Suite) {
    build_size_group(suite, 64, 64, "tiny_64x64_4Kpx");
    build_size_group(suite, 256, 256, "small_256x256_64Kpx");
    build_size_group(suite, 1024, 1024, "medium_1024x1024_1MP");
    build_size_group(suite, 4096, 4096, "large_4096x4096_16MP");
}

zenbench::main!(bench_scalar_vs_simd);
