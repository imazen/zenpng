//! Compare production dispatch, forced scalar, and explicit NEON unfilters.
//! Production dispatch deliberately selects scalar for several ARM filters;
//! those cases must not be labeled as explicit NEON measurements.
//! Source modules are included to reach crate-private kernels without adding
//! public benchmark APIs. Input construction and token toggling are untimed.
//! Run: `cargo bench --bench unfilter_tiers --features _dev`.

#[cfg(target_arch = "aarch64")]
// Source modules include unit-test imports unused by this harness-free bench.
#[allow(dead_code, unused_imports)]
#[path = "../src/simd/avg.rs"]
mod avg;
#[cfg(target_arch = "aarch64")]
#[allow(dead_code, unused_imports)]
#[path = "../src/simd/fixed.rs"]
mod fixed;
#[cfg(target_arch = "aarch64")]
// Source modules include unit-test imports unused by this harness-free bench.
#[allow(dead_code, unused_imports)]
#[path = "../src/simd/paeth.rs"]
mod paeth;
#[cfg(target_arch = "aarch64")]
// Source modules include unit-test imports unused by this harness-free bench.
#[allow(dead_code, unused_imports)]
#[path = "../src/simd/sub.rs"]
mod sub;
#[cfg(target_arch = "aarch64")]
// Source modules include unit-test imports unused by this harness-free bench.
#[allow(dead_code, unused_imports)]
#[path = "../src/simd/up.rs"]
mod up;

/// Filters with a hand-written NEON kernel (Up for every bpp, the rest bpp=4).
#[cfg(target_arch = "aarch64")]
fn has_direct_neon(ft: u8, bpp: usize) -> bool {
    ft == 2 || bpp == 4
}

#[cfg(target_arch = "aarch64")]
fn direct_neon(ft: u8, row: &mut [u8], prev: &[u8], bpp: usize) {
    use archmage::SimdToken;
    let t = archmage::NeonToken::summon().expect("native NEON enabled");
    match (ft, bpp) {
        (1, 4) => sub::unfilter_sub_bpp4_impl_neon(t, row),
        (2, _) => up::unfilter_up_impl_neon(t, row, prev),
        (3, 4) => avg::unfilter_avg_bpp4_impl_neon(t, row, prev),
        (4, 4) => paeth::unfilter_paeth_bpp4_impl_neon(t, row, prev),
        _ => panic!("no explicit NEON kernel for this filter/bpp"),
    }
}

use zenbench::prelude::*;

#[cfg(target_arch = "aarch64")]
type TierToken = archmage::NeonToken;
// x86: disabling V1 (SSE2) also disables every higher tier, so "forced_scalar"
// really is the scalar (const-generic fixed-kernel) path.
#[cfg(target_arch = "x86_64")]
type TierToken = archmage::X64V1Token;

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
const TIER_NAME: &str = if cfg!(target_arch = "aarch64") {
    "neon"
} else {
    "all x86 SIMD tiers"
};

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
fn set_simd(enabled: bool) -> bool {
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
        for width in [1usize, 17, 64, 256, 1024, 1920, 4096] {
            let len = width * bpp;
            let prev: &'static [u8] = Box::leak(noise(len, 0x1234).into_boxed_slice());
            let base: &'static [u8] = Box::leak(noise(len, 0x9876).into_boxed_slice());

            for &(ft, fname) in &[(1u8, "sub"), (2, "up"), (3, "avg"), (4, "paeth")] {
                #[cfg(target_arch = "aarch64")]
                if has_direct_neon(ft, bpp) {
                    set_simd(false);
                    let mut expected = base.to_vec();
                    zenpng::__bench_unfilter_row(ft, &mut expected, prev, bpp);
                    set_simd(true);
                    let mut actual = base.to_vec();
                    direct_neon(ft, &mut actual, prev, bpp);
                    assert_eq!(actual, expected, "direct NEON {fname}/{label}");
                }
                suite.compare(format!("unfilter_{fname}/{label}/{width}"), |g| {
                    g.throughput(Throughput::Bytes(len as u64));
                    #[cfg(target_arch = "aarch64")]
                    if has_direct_neon(ft, bpp) {
                        g.bench("direct_neon", move |b| {
                            b.with_input(move || {
                                set_simd(true);
                                base.to_vec()
                            })
                            .run(move |mut row| {
                                direct_neon(ft, &mut row, prev, bpp);
                                row
                            })
                        });
                    }
                    for (arm, simd) in [("production", true), ("forced_scalar", false)] {
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
    }
    set_simd(true);
}

fn main() {
    // Ungated like benches/vs_png.rs: the resource gate capped each group at
    // ~4 rounds on shared boxes. Arms are interleaved within each round.
    let group_filter = std::env::args().find_map(|a| a.strip_prefix("--group=").map(String::from));
    let result = zenbench::run_gated(zenbench::GateConfig::disabled(), |suite| {
        if let Some(f) = &group_filter {
            suite.set_group_filter(f.clone());
        }
        bench_filters(suite);
    });
    zenbench::postprocess_result(&result);
}
