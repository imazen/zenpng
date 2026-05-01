//! Scalar-vs-SIMD comparison bench.
//!
//! Decides whether the magetypes 512-bit SIMD predicates are pulling
//! their weight vs the plain scalar reference, per CLAUDE.md's "manual
//! intrinsics only when they save 10%+" guidance.
//!
//! For each predicate we run the same workload through:
//!   * `scalar_*`      — hand-written scalar reference
//!   * `simd_*`        — magetypes-generated, runtime-dispatched
//!   * (fused only) `simd_fused` and `simd_fused_cg`
//!
//! If a SIMD path doesn't beat scalar by 10%+ on the success path, the
//! magetypes V4x specialization isn't worth keeping. If a SIMD path
//! does beat scalar by ≥10%, it's pulling its weight; if it beats
//! scalar by <10% even at the largest size, the magetypes generic
//! polyfill is leaving performance on the table and a hand-written
//! AVX-512 specialization could be considered.
//!
//! Run:
//!   cargo bench --bench scalar_vs_simd --features _dev

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
    let rgb_input = rgb8_all_gray(w, h);
    let be16_input = be16_all_replicated(w, h);

    suite.group(label, move |g| {
        g.throughput(Throughput::Elements(pixels));
        g.throughput_unit("px");

        // ── is_opaque_rgba8 ─────────────────────────────────────────
        g.subgroup("is_opaque_rgba8");
        let s = rgba_input.clone();
        g.bench("scalar", move |b| {
            b.iter(|| zenbench::black_box(scan::scalar_is_opaque_rgba8(&s)))
        });
        let v = rgba_input.clone();
        g.bench("simd_512", move |b| {
            b.iter(|| zenbench::black_box(scan::is_opaque_rgba8(&v)))
        });

        // ── is_grayscale_rgba8 ─────────────────────────────────────
        g.subgroup("is_grayscale_rgba8");
        let s = rgba_input.clone();
        g.bench("scalar", move |b| {
            b.iter(|| zenbench::black_box(scan::scalar_is_grayscale_rgba8(&s)))
        });
        let v = rgba_input.clone();
        g.bench("simd_512", move |b| {
            b.iter(|| zenbench::black_box(scan::is_grayscale_rgba8(&v)))
        });

        // ── alpha_is_binary_rgba8 ──────────────────────────────────
        g.subgroup("alpha_is_binary_rgba8");
        let s = rgba_input.clone();
        g.bench("scalar", move |b| {
            b.iter(|| zenbench::black_box(scan::scalar_alpha_is_binary_rgba8(&s)))
        });
        let v = rgba_input.clone();
        g.bench("simd_512", move |b| {
            b.iter(|| zenbench::black_box(scan::alpha_is_binary_rgba8(&v)))
        });

        // ── is_grayscale_rgb8 ──────────────────────────────────────
        g.subgroup("is_grayscale_rgb8");
        let s = rgb_input.clone();
        g.bench("scalar", move |b| {
            b.iter(|| zenbench::black_box(scan::scalar_is_grayscale_rgb8(&s)))
        });
        let v = rgb_input;
        g.bench("simd_512", move |b| {
            b.iter(|| zenbench::black_box(scan::is_grayscale_rgb8(&v)))
        });

        // ── bit_replication_lossless_be16 ──────────────────────────
        g.subgroup("bit_replication_be16");
        let s = be16_input.clone();
        g.bench("scalar", move |b| {
            b.iter(|| zenbench::black_box(scan::scalar_bit_replication_lossless_be16(&s)))
        });
        let v = be16_input;
        g.bench("simd_512", move |b| {
            b.iter(|| zenbench::black_box(scan::bit_replication_lossless_be16(&v)))
        });

        // ── fused (3-in-1) ─────────────────────────────────────────
        g.subgroup("fused_three_checks");
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
