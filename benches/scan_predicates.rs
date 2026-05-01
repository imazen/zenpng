//! Benchmarks for SIMD-accelerated downcast predicates.
//!
//! Reports per-predicate throughput in pixels/s. To convert to ms/MP:
//! `ms_per_MP = 1e9 / pixels_per_sec`. zenbench prints "items/s" — items
//! are pixels here, so divide by 1e6 to get MP/s.
//!
//! Per CLAUDE.md sweep discipline (size sweep with intercept capture):
//! tiny 64×64 (4 Kpx), small 256×256 (64 Kpx), medium 1024×1024 (~1 MP),
//! large 4096×4096 (16 MP). Fixed setup overhead vs. per-pixel work.
//!
//! Each predicate has two scenarios:
//!   * `pass`: every pixel satisfies the predicate (full scan)
//!   * `fail_first`: the first pixel breaks it (early-exit path)
//!
//! Run:
//!   cargo bench --bench scan_predicates --features _dev

use zenbench::prelude::*;

use zenpng::__bench_scan as scan;

fn rgba8_all_opaque_gray(w: usize, h: usize) -> Vec<u8> {
    let n = w * h;
    let mut v = Vec::with_capacity(n * 4);
    for i in 0..n {
        let g = (i * 7 + 3) as u8;
        v.extend_from_slice(&[g, g, g, 255]);
    }
    v
}

fn rgba8_first_pixel_fails_opaque(w: usize, h: usize) -> Vec<u8> {
    let mut v = rgba8_all_opaque_gray(w, h);
    if !v.is_empty() {
        v[3] = 128;
    }
    v
}

fn rgba8_first_pixel_fails_gray(w: usize, h: usize) -> Vec<u8> {
    let mut v = rgba8_all_opaque_gray(w, h);
    if v.len() >= 4 {
        v[1] = v[0].wrapping_add(1);
    }
    v
}

fn rgba8_first_pixel_fails_alpha_binary(w: usize, h: usize) -> Vec<u8> {
    let mut v = rgba8_all_opaque_gray(w, h);
    for i in 0..(w * h) {
        v[i * 4 + 3] = if i & 1 == 0 { 0 } else { 255 };
    }
    if !v.is_empty() {
        v[3] = 128;
    }
    v
}

fn rgba8_alpha_binary_pass(w: usize, h: usize) -> Vec<u8> {
    let n = w * h;
    let mut v = Vec::with_capacity(n * 4);
    for i in 0..n {
        let g = (i * 11 + 5) as u8;
        v.extend_from_slice(&[g, g, g, if i & 1 == 0 { 0 } else { 255 }]);
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

fn rgb8_first_pixel_fails(w: usize, h: usize) -> Vec<u8> {
    let mut v = rgb8_all_gray(w, h);
    if v.len() >= 3 {
        v[1] = v[0].wrapping_add(1);
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

fn be16_first_pair_fails(w: usize, h: usize) -> Vec<u8> {
    let mut v = be16_all_replicated(w, h);
    if v.len() >= 2 {
        v[1] = v[0].wrapping_add(1);
    }
    v
}

fn build_size_group(suite: &mut Suite, w: usize, h: usize, label: &'static str) {
    let pixels = (w * h) as u64;

    // Pre-build all input vectors before the group closure (they need to
    // outlive the bench closures which require 'static captures).
    let opaque_pass = rgba8_all_opaque_gray(w, h);
    let opaque_fail = rgba8_first_pixel_fails_opaque(w, h);
    let gray_pass = rgba8_all_opaque_gray(w, h);
    let gray_fail = rgba8_first_pixel_fails_gray(w, h);
    let ab_pass = rgba8_alpha_binary_pass(w, h);
    let ab_fail = rgba8_first_pixel_fails_alpha_binary(w, h);
    let rgb_pass = rgb8_all_gray(w, h);
    let rgb_fail = rgb8_first_pixel_fails(w, h);
    let be16_pass = be16_all_replicated(w, h);
    let be16_fail = be16_first_pair_fails(w, h);

    suite.group(label, move |g| {
        g.throughput(Throughput::Elements(pixels));
        g.throughput_unit("px");

        g.subgroup("is_opaque_rgba8");
        g.bench("pass", move |b| {
            b.iter(|| zenbench::black_box(scan::is_opaque_rgba8(&opaque_pass)))
        });
        g.bench("fail_first", move |b| {
            b.iter(|| zenbench::black_box(scan::is_opaque_rgba8(&opaque_fail)))
        });

        g.subgroup("is_grayscale_rgba8");
        g.bench("pass", move |b| {
            b.iter(|| zenbench::black_box(scan::is_grayscale_rgba8(&gray_pass)))
        });
        g.bench("fail_first", move |b| {
            b.iter(|| zenbench::black_box(scan::is_grayscale_rgba8(&gray_fail)))
        });

        g.subgroup("alpha_is_binary_rgba8");
        g.bench("pass", move |b| {
            b.iter(|| zenbench::black_box(scan::alpha_is_binary_rgba8(&ab_pass)))
        });
        g.bench("fail_first", move |b| {
            b.iter(|| zenbench::black_box(scan::alpha_is_binary_rgba8(&ab_fail)))
        });

        g.subgroup("is_grayscale_rgb8");
        g.bench("pass", move |b| {
            b.iter(|| zenbench::black_box(scan::is_grayscale_rgb8(&rgb_pass)))
        });
        g.bench("fail_first", move |b| {
            b.iter(|| zenbench::black_box(scan::is_grayscale_rgb8(&rgb_fail)))
        });

        g.subgroup("bit_replication_lossless_be16");
        g.bench("pass", move |b| {
            b.iter(|| zenbench::black_box(scan::bit_replication_lossless_be16(&be16_pass)))
        });
        g.bench("fail_first", move |b| {
            b.iter(|| zenbench::black_box(scan::bit_replication_lossless_be16(&be16_fail)))
        });
    });
}

fn bench_predicates(suite: &mut Suite) {
    build_size_group(suite, 64, 64, "tiny_64x64_4Kpx");
    build_size_group(suite, 256, 256, "small_256x256_64Kpx");
    build_size_group(suite, 1024, 1024, "medium_1024x1024_1MP");
    build_size_group(suite, 4096, 4096, "large_4096x4096_16MP");
}

zenbench::main!(bench_predicates);
