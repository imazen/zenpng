//! Sweep-plan construction over the PNG encoder knob space.
//!
//! Port of the variant-generation playbook
//! (`zenjpeg/docs/VARIANT_GENERATION.md`). The **compression axis** is
//! lossless and therefore trial-class: decoded pixels are identical
//! across every compression cell, `min(bytes)` comparisons are exact,
//! and no quality grid exists — each compression stratum is one cell.
//!
//! The **quantize axis** ([`QuantizeSpec`]) is the one lossy axis: it
//! palette-reduces the image to `max_colors` entries via a backend
//! ([`QuantBackend::Imagequant`] or [`QuantBackend::Zenquant`]). Those
//! cells DO change pixels, so they are metric-class — a picker compares
//! their rate/distortion against the lossless cells, not `min(bytes)`.
//! The axis is a **union**, not a cross: the compression cells stay
//! truecolor (`quantize: None`), and each [`QuantizeSpec`] adds exactly
//! one cell at the default ([`Compression::Balanced`]) compression.
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

use enough::Stop;
use imgref::ImgRef;
use rgb::{Rgb, Rgba};
#[allow(unused_imports)]
use whereat::at;

use crate::Compression;

/// Palette quantizer backend for the quantize axis. Each backend is a
/// different palette-reduction algorithm gated behind its own cargo
/// feature; a build without the feature can still construct the spec and
/// parse its cell id, but [`SweepVariant::encode_png`] returns an error
/// for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QuantBackend {
    /// libimagequant — high-quality dithering (feature `imagequant`).
    /// Cell-id infix `iq`.
    Imagequant,
    /// zenquant — perceptual median-cut in OKLab (feature `quantize`).
    /// Cell-id infix `zq`.
    Zenquant,
}

/// One palette-quantize stratum: which backend, and how many colors the
/// palette is capped at. This is the lossy axis — distinct specs change
/// the decoded pixels and so fingerprint distinctly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct QuantizeSpec {
    /// Palette-reduction backend.
    pub backend: QuantBackend,
    /// Maximum palette entries (2–256).
    pub max_colors: u16,
}

/// One encode variant. The lossless **compression** preset (`parallel`
/// pinned off per playbook pattern 9) plus an optional lossy
/// **quantize** stratum. `quantize: None` is the truecolor (lossless)
/// path; `Some(spec)` palette-reduces the image first.
#[derive(Clone, Debug)]
pub struct SweepVariant {
    /// Compression preset (named tiers; resolved effort is part of the
    /// fingerprint identity).
    pub compression: Compression,
    /// Optional palette-quantize stratum. `None` = truecolor lossless.
    pub quantize: Option<QuantizeSpec>,
}

impl SweepVariant {
    /// Build the encode config for this variant (parallel pinned off).
    /// The config is identical whether or not [`Self::quantize`] is set —
    /// the quantize choice is applied at [`Self::encode_png`] time, not
    /// in the config.
    #[must_use]
    pub fn build(&self) -> crate::EncodeConfig {
        crate::EncodeConfig::default()
            .with_compression(self.compression)
            .with_parallel(false)
    }

    fn id(&self) -> String {
        let base = format!("png-{}", preset_token(self.compression));
        match self.quantize {
            None => base,
            Some(QuantizeSpec {
                backend: QuantBackend::Imagequant,
                max_colors,
            }) => format!("{base}-iq{max_colors}"),
            Some(QuantizeSpec {
                backend: QuantBackend::Zenquant,
                max_colors,
            }) => format!("{base}-zq{max_colors}"),
        }
    }

    /// Encode `img` (RGB8) to PNG bytes under this variant.
    ///
    /// - `quantize: None` → truecolor lossless via [`crate::encode_rgb8`]
    ///   (the same call a metric sweep makes for the compression cells).
    /// - `Some(spec)` → widen RGB→RGBA (opaque α=255), build the
    ///   backend's quantizer capped at `spec.max_colors`, and emit an
    ///   indexed PNG via [`crate::encode_indexed`].
    ///
    /// The quantize arm is feature-gated: a build without the backend's
    /// feature returns an [`crate::error::PngError::InvalidInput`] rather
    /// than silently encoding truecolor (which would mislabel a `-iq`/`-zq`
    /// cell as palette in the training data).
    pub fn encode_png(
        &self,
        img: ImgRef<Rgb<u8>>,
        cancel: &dyn Stop,
        deadline: &dyn Stop,
    ) -> crate::error::Result<Vec<u8>> {
        let Some(spec) = self.quantize else {
            return crate::encode_rgb8(img, None, &self.build(), cancel, deadline);
        };
        self.encode_quantized(img, spec, cancel, deadline)
    }

    /// Encode the palette-quantize arm. Split out so the feature-gated
    /// quantizer construction stays isolated and the truecolor path in
    /// [`Self::encode_png`] is unconditional.
    #[allow(unused_variables)]
    fn encode_quantized(
        &self,
        img: ImgRef<Rgb<u8>>,
        spec: QuantizeSpec,
        cancel: &dyn Stop,
        deadline: &dyn Stop,
    ) -> crate::error::Result<Vec<u8>> {
        // Widen RGB → RGBA (opaque). encode_indexed wants ImgRef<Rgba<u8>>.
        let rgba: Vec<Rgba<u8>> = img
            .pixels()
            .map(|p| Rgba::new(p.r, p.g, p.b, 255u8))
            .collect();
        let rgba_img = ImgRef::new(&rgba, img.width(), img.height());

        match spec.backend {
            QuantBackend::Imagequant => {
                #[cfg(feature = "imagequant")]
                {
                    let q = crate::ImagequantQuantizer::default().with_max_colors(spec.max_colors);
                    crate::encode_indexed(rgba_img, &self.build(), &q, None, cancel, deadline)
                }
                #[cfg(not(feature = "imagequant"))]
                {
                    Err(at!(crate::error::PngError::InvalidInput(format!(
                        "quantize cell {:?} needs the `imagequant` feature, which is not enabled",
                        self.id()
                    ))))
                }
            }
            QuantBackend::Zenquant => {
                #[cfg(feature = "quantize")]
                {
                    let q = crate::ZenquantQuantizer::new().with_max_colors(spec.max_colors);
                    crate::encode_indexed(rgba_img, &self.build(), &q, None, cancel, deadline)
                }
                #[cfg(not(feature = "quantize"))]
                {
                    Err(at!(crate::error::PngError::InvalidInput(format!(
                        "quantize cell {:?} needs the `quantize` feature, which is not enabled",
                        self.id()
                    ))))
                }
            }
        }
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
/// `png-<preset>` (named tiers) or `png-e<effort>` (explicit),
/// optionally suffixed by a quantize stratum `-iq<N>`
/// ([`QuantBackend::Imagequant`]) or `-zq<N>`
/// ([`QuantBackend::Zenquant`]), e.g. `png-balanced-iq256`. The
/// renderer and parser move in lockstep
/// (`cell_ids_roundtrip_to_their_variants`); evolution is
/// additive-only. Note `png-e13` and `png-balanced` are distinct
/// SPELLINGS of one identity — the fingerprint merges them, and the
/// planner emits only canonical preset names.
pub fn variant_from_cell_id(id: &str) -> Result<SweepVariant, String> {
    let Some(rest) = id.strip_prefix("png-") else {
        return Err(format!("cell id {id:?} is not a png- id"));
    };
    // Peel a trailing quantize stratum `-iq<N>` / `-zq<N>` off the end;
    // what's left is the compression token (parsed exactly as before).
    let (tok, quantize) = parse_quantize_suffix(rest, id)?;
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
    Ok(SweepVariant {
        compression,
        quantize,
    })
}

/// Split `rest` (the part after `png-`) into the compression token and
/// an optional [`QuantizeSpec`]. A trailing `-iq<N>` / `-zq<N>` segment
/// is the quantize stratum; everything before it is the compression
/// token. `<N>` must be a 2–256 color count.
fn parse_quantize_suffix<'a>(
    rest: &'a str,
    id: &str,
) -> Result<(&'a str, Option<QuantizeSpec>), String> {
    let Some((head, tail)) = rest.rsplit_once('-') else {
        // No '-': the whole thing is the compression token, no quantize.
        return Ok((rest, None));
    };
    let backend = tail
        .strip_prefix("iq")
        .map(|n| (QuantBackend::Imagequant, n))
        .or_else(|| tail.strip_prefix("zq").map(|n| (QuantBackend::Zenquant, n)));
    match backend {
        Some((backend, n)) => {
            let max_colors: u16 = n
                .parse()
                .map_err(|err| format!("bad quantize color count in {id:?}: {err}"))?;
            if !(2..=256).contains(&max_colors) {
                return Err(format!(
                    "quantize color count {max_colors} out of range 2..=256 in {id:?}"
                ));
            }
            Ok((
                head,
                Some(QuantizeSpec {
                    backend,
                    max_colors,
                }),
            ))
        }
        // The trailing segment isn't a quantize stratum (e.g. nothing
        // here today, but keeps the grammar additive); treat the whole
        // `rest` as the compression token.
        None => Ok((rest, None)),
    }
}

/// Byte-identity fingerprint over RESOLVED state: the effort value
/// ([`Compression::effort`] — so `Effort(13)` ≡ `Balanced`), the pinned
/// `parallel = false`, and the quantize stratum (backend discriminant +
/// `max_colors`). Equal fingerprints produce identical bytes for the
/// same input — so the truecolor cells and the palette cells fingerprint
/// distinctly, and two quantize cells differing only in backend or
/// color count never collide.
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
    // Quantize stratum. Tag 0 = truecolor (no quantize); 1 = Imagequant,
    // 2 = Zenquant, each followed by the 2-byte color count. Distinct
    // backends/counts therefore hash distinctly.
    match variant.quantize {
        None => write(0),
        Some(QuantizeSpec {
            backend,
            max_colors,
        }) => {
            write(match backend {
                QuantBackend::Imagequant => 1,
                QuantBackend::Zenquant => 2,
            });
            for b in max_colors.to_le_bytes() {
                write(b);
            }
        }
    }
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

/// The canonical quantize axis: both backends crossed with the standard
/// color-count ladder `{256, 128, 64, 32}` = 8 specs. This is the
/// MANDATORY quantize coverage — `modes_full` and `scalar_dense` always
/// carry all 8.
#[must_use]
pub fn all_quantize_specs() -> Vec<QuantizeSpec> {
    let mut v = Vec::with_capacity(8);
    for backend in [QuantBackend::Imagequant, QuantBackend::Zenquant] {
        for max_colors in [256u16, 128, 64, 32] {
            v.push(QuantizeSpec {
                backend,
                max_colors,
            });
        }
    }
    v
}

/// Axes: compression presets (most-important first) plus the quantize
/// strata. The two axes form a **union**, not a cross — see
/// [`plan_constrained`]: every compression preset is one truecolor cell,
/// and each [`QuantizeSpec`] adds one palette cell at the default
/// ([`Compression::Balanced`]) compression.
#[derive(Clone, Debug)]
pub struct SweepAxes {
    /// Compression tiers (index 0 = production default).
    pub compression: Vec<Compression>,
    /// Palette-quantize strata (each emits one cell at the default
    /// compression). Empty = no palette cells.
    pub quantize: Vec<QuantizeSpec>,
}

impl SweepAxes {
    /// The RD-front tiers: default + the fast and strong ends of the
    /// standard pipeline. No quantize strata (the RD-front is the
    /// lossless compression curve; the palette axis lives in the fuller
    /// plans).
    #[must_use]
    pub fn rd_core() -> Self {
        Self {
            compression: vec![
                Compression::Balanced,
                Compression::Fast,
                Compression::Intense,
            ],
            quantize: Vec::new(),
        }
    }

    /// Every standard-pipeline tier (the minutes-per-MP zopfli tiers
    /// stay out — see module docs) **plus the full mandatory quantize
    /// axis** (both backends × `{256,128,64,32}` = 8 palette cells).
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
        axes.quantize = all_quantize_specs();
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
            quantize: all_quantize_specs(),
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
/// exists: the full `modes_full` is 17 cells (9 truecolor compression
/// presets + 8 palette-quantize cells).
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
    // Compression axis: each preset is one truecolor (lossless) cell.
    for (i, &compression) in axes.compression.iter().enumerate() {
        let variant = SweepVariant {
            compression,
            quantize: None,
        };
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
    // Quantize axis (UNION, not cross): each spec is one palette cell at
    // the default Balanced compression. It deviates from the default cell
    // on a single axis (quantize), so deviations = 1.
    for spec in &axes.quantize {
        let variant = SweepVariant {
            compression: Compression::Balanced,
            quantize: Some(*spec),
        };
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
                deviations: 1,
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
            quantize: None,
        };
        let spelled = SweepVariant {
            compression: Compression::Effort(named.compression.effort()),
            quantize: None,
        };
        assert_eq!(fingerprint(&named), fingerprint(&spelled));
        assert_ne!(
            fingerprint(&named),
            fingerprint(&SweepVariant {
                compression: Compression::Fast,
                quantize: None,
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
            quantize: None,
        };
        let pricey = SweepVariant {
            compression: Compression::Intense,
            quantize: None,
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
                compression: Compression::Maniac,
                quantize: None,
            }),
            30
        );
        assert_eq!(
            compute_tier(&SweepVariant {
                compression: Compression::Minutes,
                quantize: None,
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

    #[test]
    fn modes_full_has_all_eight_quantize_cells() {
        let p = plan(&SweepAxes::modes_full());
        let ids: alloc::collections::BTreeSet<&str> =
            p.cells.iter().map(|c| c.id.as_str()).collect();
        // All 8 mandatory quantize cells present at the default compression.
        for id in [
            "png-balanced-iq256",
            "png-balanced-iq128",
            "png-balanced-iq64",
            "png-balanced-iq32",
            "png-balanced-zq256",
            "png-balanced-zq128",
            "png-balanced-zq64",
            "png-balanced-zq32",
        ] {
            assert!(ids.contains(id), "modes_full missing quantize cell {id}");
        }
        // Union, not cross: the truecolor compression cells are still
        // there too (9 distinct compression presets) → 9 + 8 = 17 cells.
        assert!(ids.contains("png-balanced"), "truecolor default cell gone");
        assert!(ids.contains("png-none"), "truecolor none cell gone");
        assert_eq!(
            p.cells.len(),
            17,
            "modes_full = 9 truecolor compression cells + 8 quantize cells"
        );
        // Every quantize cell deviates by exactly one axis from the default.
        for c in &p.cells {
            if c.variant.quantize.is_some() {
                assert_eq!(
                    c.deviations, 1,
                    "quantize cell {} should be 1 deviation",
                    c.id
                );
            }
        }
    }

    #[test]
    fn quantize_cell_ids_roundtrip() {
        // The 8 quantize cell ids parse back to the exact same variant
        // (fingerprint-stable), and distinct backends/counts fingerprint
        // distinctly.
        let p = plan(&SweepAxes::modes_full());
        for c in &p.cells {
            if c.variant.quantize.is_none() {
                continue;
            }
            let v = variant_from_cell_id(&c.id).unwrap_or_else(|e| panic!("{}: {e}", c.id));
            assert_eq!(
                v.quantize, c.variant.quantize,
                "quantize drift for {}",
                c.id
            );
            assert_eq!(fingerprint(&v), c.fingerprint, "fp drift for {}", c.id);
        }
        // Backend and color count both affect identity.
        let iq256 = variant_from_cell_id("png-balanced-iq256").unwrap();
        let zq256 = variant_from_cell_id("png-balanced-zq256").unwrap();
        let iq64 = variant_from_cell_id("png-balanced-iq64").unwrap();
        assert_ne!(
            fingerprint(&iq256),
            fingerprint(&zq256),
            "backend must change the fingerprint"
        );
        assert_ne!(
            fingerprint(&iq256),
            fingerprint(&iq64),
            "color count must change the fingerprint"
        );
        // A truecolor cell and any palette cell never collide.
        let truecolor = SweepVariant {
            compression: Compression::Balanced,
            quantize: None,
        };
        assert_ne!(fingerprint(&truecolor), fingerprint(&iq256));
    }

    #[test]
    fn quantize_suffix_rejects_bad_color_counts() {
        // Out of 2..=256 range, and non-numeric.
        assert!(variant_from_cell_id("png-balanced-iq1").is_err());
        assert!(variant_from_cell_id("png-balanced-iq257").is_err());
        assert!(variant_from_cell_id("png-balanced-iqxy").is_err());
    }
}
