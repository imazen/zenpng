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
}

/// Build the plan for the given axes.
#[must_use]
pub fn plan(axes: &SweepAxes) -> SweepPlan {
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
    SweepPlan {
        cells,
        duplicates_merged: merged,
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
}
