//! NEON-vs-scalar for the PNG inverse filters — the decode hot path.
//!
//! zenpng's existing benches all cover the scan predicates (is_opaque,
//! is_grayscale, ...). The inverse filters are where PNG decode time actually
//! goes, and `bench_unfilter_row` was already exposed under `_dev` for exactly
//! this purpose — but nothing used it, so the four filter kernels had never
//! been measured against their own scalar fallback on any architecture.
//!
//! Sub and Paeth carry a per-byte serial dependency (each output feeds the
//! next), so they are the ones most likely to lose to the autovectoriser;
//! Up and Avg are more parallel. This tells us which.
//!
//! Run: `cargo bench --bench unfilter_tiers --features _dev`
//! Do NOT pass `-C target-cpu=native` (the tier then cannot be disabled).

use zenbench::prelude::*;

#[cfg(target_arch = "aarch64")]
type TierToken = archmage::NeonToken;
#[cfg(target_arch = "x86_64")]
type TierToken = archmage::X64V3Token;

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
const TIER_NAME: &str = if cfg!(target_arch = "aarch64") {
    "neon"
} else {
    "v3(avx2)"
};

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
fn set_simd(enabled: bool) -> bool {
    use archmage::SimdToken;
    TierToken::dangerously_disable_token_process_wide(!enabled).is_ok()
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
fn set_simd(_enabled: bool) -> bool {
    false
}

fn noise(n: usize, seed: u32) -> Vec<u8> {
    let mut s = seed | 1;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 24) as u8
        })
        .collect()
}

fn bench_filters(suite: &mut Suite) {
    if !set_simd(true) || !set_simd(false) {
        eprintln!("[unfilter_tiers] SIMD tier not toggleable here. Skipping.");
        return;
    }
    set_simd(true);
    eprintln!("[unfilter_tiers] comparing {TIER_NAME} vs forced scalar");

    // 1920-px rows at 3 and 4 bytes/px — the shapes a real decode unfilters.
    for &(bpp, label) in &[(3usize, "rgb8"), (4usize, "rgba8")] {
        let width = 1920usize;
        let len = width * bpp;
        let prev: &'static [u8] = Box::leak(noise(len, 0x1234).into_boxed_slice());
        let base: &'static [u8] = Box::leak(noise(len, 0x9876).into_boxed_slice());

        for &(ft, fname) in &[(1u8, "sub"), (2, "up"), (3, "avg"), (4, "paeth")] {
            suite.compare(format!("unfilter_{fname}/{label}"), |g| {
                g.throughput(Throughput::Bytes(len as u64));
                for (arm, simd) in [(TIER_NAME, true), ("scalar", false)] {
                    g.bench(arm, move |b| {
                        b.with_input(move || {
                            set_simd(simd);
                            base.to_vec()
                        })
                        .run(move |mut row| {
                            zenpng::__bench_unfilter_row(ft, &mut row, prev, bpp);
                            row
                        })
                    });
                }
            });
        }
    }
    set_simd(true);
}

zenbench::main!(bench_filters);
