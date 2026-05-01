//! Bandwidth-fusion benchmark: compares running three predicates as
//! separate passes (3× memory bandwidth) against fused single-pass
//! variants (1× bandwidth, both runtime-branch and const-generic-recursive).
//!
//! Workloads: success path (every pixel passes) — that's where fusion
//! pays. Plus one fail-fast scenario per buffer to confirm both fused
//! variants still early-exit cleanly.
//!
//! Run:
//!   cargo bench --bench fused_predicates --features _dev

use zenbench::prelude::*;

use zenpng::__bench_scan as scan;

fn rgba8_all_pass(w: usize, h: usize) -> Vec<u8> {
    let n = w * h;
    let mut v = Vec::with_capacity(n * 4);
    for i in 0..n {
        let g = (i * 7 + 3) as u8;
        // R = G = B (gray), alpha = 0 or 255 (binary AND opaque-mix).
        v.extend_from_slice(&[g, g, g, if i & 3 == 0 { 0 } else { 255 }]);
    }
    v
}

/// Run three predicates as three independent passes (current behavior).
fn separate_three(rgba: &[u8]) -> (bool, bool, bool) {
    (
        scan::is_opaque_rgba8(rgba),
        scan::is_grayscale_rgba8(rgba),
        scan::alpha_is_binary_rgba8(rgba),
    )
}

fn build_size_group(suite: &mut Suite, w: usize, h: usize, label: &'static str) {
    let pixels = (w * h) as u64;

    let pass_input = rgba8_all_pass(w, h);
    let mut fail_input = pass_input.clone();
    if !fail_input.is_empty() {
        // First pixel: not gray (R=10,G=20,B=30), alpha=128 (not binary, not opaque).
        // Triggers all three flips on the first chunk.
        fail_input[0] = 10;
        fail_input[1] = 20;
        fail_input[2] = 30;
        fail_input[3] = 128;
    }

    suite.group(label, move |g| {
        g.throughput(Throughput::Elements(pixels));
        g.throughput_unit("px");

        // ── Success path (full scan, fusion saves 3x bandwidth) ─────
        g.subgroup("pass_all_three_checks");
        let pass_a = pass_input.clone();
        g.bench("separate_3_passes", move |b| {
            b.iter(|| zenbench::black_box(separate_three(&pass_a)))
        });
        let pass_b = pass_input.clone();
        g.bench("fused_runtime_branch", move |b| {
            b.iter(|| {
                zenbench::black_box(scan::fused_predicates_rgba8(
                    &pass_b,
                    scan::FusedRequest::all(),
                ))
            })
        });
        let pass_c = pass_input;
        g.bench("fused_const_generic", move |b| {
            b.iter(|| {
                zenbench::black_box(scan::fused_predicates_rgba8_cg(
                    &pass_c,
                    scan::FusedRequest::all(),
                ))
            })
        });

        // ── Fail-fast (every variant should bail in chunk 0) ────────
        g.subgroup("fail_first_pixel");
        let fail_a = fail_input.clone();
        g.bench("separate_3_passes", move |b| {
            b.iter(|| zenbench::black_box(separate_three(&fail_a)))
        });
        let fail_b = fail_input.clone();
        g.bench("fused_runtime_branch", move |b| {
            b.iter(|| {
                zenbench::black_box(scan::fused_predicates_rgba8(
                    &fail_b,
                    scan::FusedRequest::all(),
                ))
            })
        });
        let fail_c = fail_input;
        g.bench("fused_const_generic", move |b| {
            b.iter(|| {
                zenbench::black_box(scan::fused_predicates_rgba8_cg(
                    &fail_c,
                    scan::FusedRequest::all(),
                ))
            })
        });
    });
}

fn bench_fused(suite: &mut Suite) {
    build_size_group(suite, 64, 64, "tiny_64x64_4Kpx");
    build_size_group(suite, 256, 256, "small_256x256_64Kpx");
    build_size_group(suite, 1024, 1024, "medium_1024x1024_1MP");
    build_size_group(suite, 4096, 4096, "large_4096x4096_16MP");
}

zenbench::main!(bench_fused);
