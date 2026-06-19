//! Sweep-plan construction over the PNG encoder knob space.
//!
//! Port of the variant-generation playbook
//! (`zenjpeg/docs/VARIANT_GENERATION.md`). PNG is lossless, so the
//! **entire curated space is trial-class**: decoded pixels are
//! identical across every cell, `min(bytes)` comparisons are exact, and
//! no quality grid exists — each stratum is one cell.
//!
//! Deliberately excluded from the curated axes, with reasons:
//!
//! - `near_lossless_bits` — changes pixels (metric-class; sweep it in
//!   metric-scored fleet runs, never in the trial-class axes).
//! - `Crush`/`Maniac`/`Brag`/`Minutes` — minutes-per-megapixel tiers;
//!   they remain constructible (`variant_from_cell_id` parses them) but
//!   don't ride the default axes where a harness or fleet would pay
//!   for them implicitly.
//! - `Compression::Effort(n)` — continuous spelling of the presets;
//!   the fingerprint hashes the RESOLVED effort
//!   ([`Compression::effort`]), so `Effort(13)` aliases `Balanced`
//!   (pattern 4: resolution is the identity, not the spelling).
//! - `Filter` — single variant (`Auto`); not an axis.
//! - metadata chunks (gamma/CICP/text/…) — orthogonal container bytes,
//!   not rate-distortion knobs.
//! - `parallel` — pinned `false` in every cell (playbook pattern 9:
//!   thread-dependent chunking would make bytes machine-dependent and
//!   poison content-addressed ledgers); hashed so the pin is part of
//!   the identity.
//!
//! Step provenance: the named presets ARE the ship-derived steps
//! (each maps to a documented effort via [`Compression::effort`]).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::Compression;

/// One encode variant. PNG has a single (lossless) mode; `parallel` is
/// pinned off per playbook pattern 9.
#[derive(Clone, Debug)]
pub struct SweepVariant {
    /// Compression preset (named tiers; resolved effort is the
    /// fingerprint identity).
    pub compression: Compression,
}

impl SweepVariant {
    /// Build the encode config for this variant (parallel pinned off).
    #[must_use]
    pub fn build(&self) -> crate::EncodeConfig {
        crate::EncodeConfig::default()
            .with_compression(self.compression)
            .with_parallel(false)
    }

    fn id(&self) -> String {
        format!("png-{}", preset_token(self.compression))
    }
}

fn preset_token(c: Compression) -> String {
    match c {
        Compression::None => "none".into(),
        Compression::Fastest => "fastest".into(),
        Compression::Turbo => "turbo".into(),
        Compression::Fast => "fast".into(),
        Compression::Balanced => "balanced".into(),
        Compression::Thorough => "thorough".into(),
        Compression::High => "high".into(),
        Compression::Aggressive => "aggressive".into(),
        Compression::Intense => "intense".into(),
        Compression::Crush => "crush".into(),
        Compression::Maniac => "maniac".into(),
        Compression::Brag => "brag".into(),
        Compression::Minutes => "minutes".into(),
        Compression::Effort(n) => format!("e{n}"),
    }
}

/// Reconstruct the [`SweepVariant`] a cell id denotes. Grammar:
/// `png-<preset>` (named tiers) or `png-e<effort>` (explicit). The
/// renderer and parser move in lockstep
/// (`cell_ids_roundtrip_to_their_variants`); evolution is
/// additive-only. Note `png-e13` and `png-balanced` are distinct
/// SPELLINGS of one identity — the fingerprint merges them, and the
/// planner emits only canonical preset names.
pub fn variant_from_cell_id(id: &str) -> Result<SweepVariant, String> {
    let Some(tok) = id.strip_prefix("png-") else {
        return Err(format!("cell id {id:?} is not a png- id"));
    };
    let compression = match tok {
        "none" => Compression::None,
        "fastest" => Compression::Fastest,
        "turbo" => Compression::Turbo,
        "fast" => Compression::Fast,
        "balanced" => Compression::Balanced,
        "thorough" => Compression::Thorough,
        "high" => Compression::High,
        "aggressive" => Compression::Aggressive,
        "intense" => Compression::Intense,
        "crush" => Compression::Crush,
        "maniac" => Compression::Maniac,
        "brag" => Compression::Brag,
        "minutes" => Compression::Minutes,
        e if e.starts_with('e') => {
            let n: u32 = e[1..]
                .parse()
                .map_err(|err| format!("bad effort in {id:?}: {err}"))?;
            Compression::Effort(n)
        }
        other => return Err(format!("unknown compression token {other:?} in {id:?}")),
    };
    Ok(SweepVariant { compression })
}

/// Byte-identity fingerprint over RESOLVED state: the effort value
/// ([`Compression::effort`] — so `Effort(13)` ≡ `Balanced`) plus the
/// pinned `parallel = false`. Equal fingerprints produce identical
/// bytes for the same input.
#[must_use]
pub fn fingerprint(variant: &SweepVariant) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut write = |b: u8| {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for b in variant.compression.effort().to_le_bytes() {
        write(b);
    }
    write(0); // parallel pinned false
    h
}

/// Coarse compute-cost tier of a variant (`0` = cheapest). Higher tiers
/// run more DEFLATE/filter passes and cost more CPU per encode. Made
/// public so the fleet harness and pickers can bound their candidate set
/// the same way [`plan_constrained`] does, and so a trained **scalar
/// head** can read compute alongside bytes. It is an **ordinal proxy**,
/// not a calibrated millisecond estimate — compare tiers, don't read
/// absolute cost into them.
///
/// PNG *has* a single compute dial — the compression **effort**
/// ([`Compression::effort`], which tops out at 200) — so the tier is
/// simply that effort saturated into `u8`. Effort 200 already fits a
/// `u8`, so the `.min(255)` is a defensive cap that never fires in
/// practice; it only matters if a future preset ever exceeds 255.
/// `Effort(n)` and the named preset it resolves to therefore share a
/// tier, matching the fingerprint's resolved-effort identity.
#[must_use]
pub fn compute_tier(variant: &SweepVariant) -> u8 {
    variant.compression.effort().min(u32::from(u8::MAX)) as u8
}

/// Axes: compression presets, most-important first.
#[derive(Clone, Debug)]
pub struct SweepAxes {
    /// Compression tiers (index 0 = production default).
    pub compression: Vec<Compression>,
}

impl SweepAxes {
    /// The RD-front tiers: default + the fast and strong ends of the
    /// standard pipeline.
    #[must_use]
    pub fn rd_core() -> Self {
        Self {
            compression: vec![
                Compression::Balanced,
                Compression::Fast,
                Compression::Intense,
            ],
        }
    }

    /// Every standard-pipeline tier (the minutes-per-MP zopfli tiers
    /// stay out — see module docs).
    #[must_use]
    pub fn modes_full() -> Self {
        let mut axes = Self::rd_core();
        axes.compression.extend([
            Compression::Fastest,
            Compression::Turbo,
            Compression::Thorough,
            Compression::High,
            Compression::Aggressive,
            Compression::None,
        ]);
        axes
    }

    /// The densest principled effort ladder — every distinct standard
    /// tier **plus** the heavy minutes-per-MP tiers that
    /// [`modes_full`](Self::modes_full) holds back. This is the data a
    /// trained **scalar head** (a continuous regression on the picker's
    /// compute dial) needs: the full compute-vs-bytes curve, not just the
    /// fast and strong ends. Effort is PNG's single continuous-ish axis,
    /// so densifying it *is* the dense sweep — the curve has one knob.
    ///
    /// Default-first (`Balanced`, the production default at index 0), then
    /// the standard pipeline in ascending effort (Fastest → Turbo → Fast
    /// → Thorough → High → Aggressive → Intense), then the heavy tiers
    /// `modes_full` excludes (Crush, Maniac) so the expensive end of the
    /// compute axis is covered. No-op/aliasing spellings are omitted; any
    /// `Effort(n)` that resolved to one of these would fingerprint-merge
    /// anyway ([`fingerprint`] hashes the resolved effort, pattern 4).
    /// The FullOptimal-recompressor `Brag`/`Minutes` tiers (efforts 31 and
    /// 200) stay out even here: each costs minutes per megapixel for a
    /// sliver of extra ratio, so the curve they'd add is dominated by
    /// runaway wall time, not signal a scalar head can use — the same
    /// rationale that keeps them off [`modes_full`].
    #[must_use]
    pub fn scalar_dense() -> Self {
        Self {
            compression: vec![
                Compression::Balanced,
                Compression::Fastest,
                Compression::Turbo,
                Compression::Fast,
                Compression::Thorough,
                Compression::High,
                Compression::Aggressive,
                Compression::Intense,
                Compression::Crush,
                Compression::Maniac,
            ],
        }
    }
}

/// One encode cell.
#[derive(Clone, Debug)]
pub struct SweepCell {
    /// Stable id (`png-<preset>`).
    pub id: String,
    /// The variant to encode with.
    pub variant: SweepVariant,
    /// Byte-identity fingerprint of the resolved state.
    pub fingerprint: u64,
    /// Ids merged into this cell (identical resolved effort).
    pub aliases: Vec<String>,
    /// Axis deviation from the default (0 = production default).
    pub deviations: u8,
}

/// The finite plan. PNG's space is small enough that no budget ladder
/// exists: the full `modes_full` is 9 cells.
#[derive(Clone, Debug)]
pub struct SweepPlan {
    /// Deduplicated cells, default first.
    pub cells: Vec<SweepCell>,
    /// Candidates merged by resolved-effort identity.
    pub duplicates_merged: usize,
    /// Cell ids dropped because their [`compute_tier`] exceeded the
    /// `compute_limit` passed to [`plan_constrained`] — the explicit
    /// no-silent-caps report for the compute constraint (empty in the
    /// unconstrained [`plan`] path).
    pub compute_tier_skipped: Vec<String>,
}

/// Build the plan for the given axes. Equivalent to
/// [`plan_constrained`]`(axes, None, None)` — the full, unconstrained
/// curated space.
#[must_use]
pub fn plan(axes: &SweepAxes) -> SweepPlan {
    plan_constrained(axes, None, None)
}

/// Build the plan, optionally bounded by a compute budget and/or a
/// deviation scope.
///
/// - `compute_limit`: if `Some(max)`, cells whose [`compute_tier`] is
///   `> max` are dropped and their ids recorded in
///   [`SweepPlan::compute_tier_skipped`] (never silently capped) — the
///   compute-resource constraint a CPU-bound fleet or a "fast configs
///   only" picker asks for.
/// - `max_deviations`: if `Some(n)`, only cells within `n` axis
///   deviations of the default survive (`1` = main-effects only). PNG is
///   single-axis so at most one deviation is ever possible; the parameter
///   exists for **cross-codec API uniformity** — the fleet and picker
///   call this same shape on every codec.
///
/// `compute_limit` is applied first, then `max_deviations`.
#[must_use]
pub fn plan_constrained(
    axes: &SweepAxes,
    compute_limit: Option<u8>,
    max_deviations: Option<u8>,
) -> SweepPlan {
    let mut cells: Vec<SweepCell> = Vec::new();
    let mut merged = 0usize;
    for (i, &compression) in axes.compression.iter().enumerate() {
        let variant = SweepVariant { compression };
        let fp = fingerprint(&variant);
        let id = variant.id();
        if let Some(c) = cells.iter_mut().find(|c| c.fingerprint == fp) {
            c.aliases.push(id);
            merged += 1;
        } else {
            cells.push(SweepCell {
                id,
                variant,
                fingerprint: fp,
                aliases: Vec::new(),
                deviations: u8::from(i != 0),
            });
        }
    }

    let mut compute_tier_skipped = Vec::new();
    if let Some(max) = compute_limit {
        cells.retain(|c| {
            if compute_tier(&c.variant) <= max {
                true
            } else {
                compute_tier_skipped.push(c.id.clone());
                false
            }
        });
    }
    if let Some(n) = max_deviations {
        cells.retain(|c| c.deviations <= n);
    }

    SweepPlan {
        cells,
        duplicates_merged: merged,
        compute_tier_skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_ids_roundtrip_to_their_variants() {
        let p = plan(&SweepAxes::modes_full());
        assert!(p.cells.len() >= 8);
        for cell in &p.cells {
            for id in core::iter::once(&cell.id).chain(cell.aliases.iter()) {
                let v = variant_from_cell_id(id).unwrap_or_else(|e| panic!("{id}: {e}"));
                assert_eq!(fingerprint(&v), cell.fingerprint, "drift for {id}");
            }
        }
        // The excluded heavy tiers and explicit efforts still parse
        // (constructible, just not curated).
        for id in ["png-crush", "png-minutes", "png-e42"] {
            variant_from_cell_id(id).unwrap();
        }
    }

    #[test]
    fn effort_spelling_aliases_its_preset() {
        // Resolution is the identity, not the spelling (pattern 4):
        // Balanced resolves to effort 13, so Effort(13) is the same cell.
        let named = SweepVariant {
            compression: Compression::Balanced,
        };
        let spelled = SweepVariant {
            compression: Compression::Effort(named.compression.effort()),
        };
        assert_eq!(fingerprint(&named), fingerprint(&spelled));
        assert_ne!(
            fingerprint(&named),
            fingerprint(&SweepVariant {
                compression: Compression::Fast
            })
        );
    }

    #[test]
    fn malformed_ids_error() {
        for bad in ["png-warp", "png-ex", "jpg-balanced"] {
            assert!(variant_from_cell_id(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn plan_is_default_first_and_deterministic() {
        let a = plan(&SweepAxes::rd_core());
        assert_eq!(a.cells[0].id, "png-balanced");
        assert_eq!(a.cells[0].deviations, 0);
        let b = plan(&SweepAxes::rd_core());
        for (x, y) in a.cells.iter().zip(&b.cells) {
            assert_eq!(x.id, y.id);
            assert_eq!(x.fingerprint, y.fingerprint);
        }
    }

    #[test]
    fn compute_tier_orders_cost() {
        // Effort is the compute dial: a cheap preset must tier strictly
        // below an expensive one.
        let cheap = SweepVariant {
            compression: Compression::Fast,
        };
        let pricey = SweepVariant {
            compression: Compression::Intense,
        };
        assert!(
            compute_tier(&cheap) < compute_tier(&pricey),
            "Fast must cost less than Intense"
        );
        // Tier == resolved effort. Effort tops out at 200 (< 255), so it
        // fits u8 directly; the .min(255) is a defensive saturation that
        // never actually fires in practice.
        assert_eq!(
            compute_tier(&SweepVariant {
                compression: Compression::Maniac
            }),
            30
        );
        assert_eq!(
            compute_tier(&SweepVariant {
                compression: Compression::Minutes
            }),
            200,
            "effort 200 fits u8 unchanged"
        );
    }

    #[test]
    fn scalar_dense_spans_the_compute_curve() {
        // A trained scalar head needs many distinct compute tiers across
        // the cells, not 1-2.
        let p = plan(&SweepAxes::scalar_dense());
        assert_eq!(p.cells[0].id, "png-balanced", "default still first");
        let mut tiers: Vec<u8> = p.cells.iter().map(|c| compute_tier(&c.variant)).collect();
        tiers.sort_unstable();
        tiers.dedup();
        assert!(
            tiers.len() >= 6,
            "scalar_dense too sparse for a scalar head: {} distinct tiers",
            tiers.len()
        );
    }

    #[test]
    fn plan_constrained_drops_expensive_and_matches_plan() {
        let unconstrained = plan(&SweepAxes::scalar_dense());
        let limit = 13u8; // Balanced's effort: keeps the fast end only.
        let limited = plan_constrained(&SweepAxes::scalar_dense(), Some(limit), None);
        assert!(!limited.cells.is_empty());
        assert!(
            limited.cells.len() < unconstrained.cells.len(),
            "the compute limit must drop the expensive cells"
        );
        assert!(
            limited
                .cells
                .iter()
                .all(|c| compute_tier(&c.variant) <= limit),
            "every surviving cell must be within budget"
        );
        assert!(
            !limited.compute_tier_skipped.is_empty(),
            "dropped cells must be reported, never silently capped"
        );

        // The unconstrained delegate must equal plan() cell-for-cell.
        let via_constrained = plan_constrained(&SweepAxes::scalar_dense(), None, None);
        let direct = plan(&SweepAxes::scalar_dense());
        assert_eq!(via_constrained.cells.len(), direct.cells.len());
        for (x, y) in via_constrained.cells.iter().zip(&direct.cells) {
            assert_eq!(x.id, y.id);
            assert_eq!(x.fingerprint, y.fingerprint);
        }
        assert!(via_constrained.compute_tier_skipped.is_empty());
    }
}
