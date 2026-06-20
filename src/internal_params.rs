//! Internal-params bundle for cross-codec uniformity (`__expert` feature).
//!
//! [`InternalParams`] collects the encoder knobs that codec-calibration
//! sweeps and the picker training pipeline want to drive externally,
//! mirroring `zenjpeg::encode::internal_params::InternalParams` so a
//! single picker model can emit the same bundle shape for every codec in
//! the zen family.
//!
//! Production callers should use [`EncodeConfig::with_compression`] /
//! [`with_filter`](EncodeConfig::with_filter) and the other per-axis
//! builder methods directly. Reach for [`InternalParams`] only when you
//! need to vary calibration axes from outside the codec — e.g., from a
//! Pareto sweep harness or a learned picker that emits per-image axis
//! values.
//!
//! Each field is `Option<_>`. `None` means "leave the
//! [`EncodeConfig`]'s existing value alone." This is partial-merge, the
//! same shape every zen codec's bundle uses, so callers can override one
//! axis at a time without spelling out the rest.
//!
//! PNG is lossless, so its single rate-distortion-relevant tunable is the
//! compression **effort** ([`Compression`]) — exactly the one axis
//! [`crate::sweep::SweepVariant`] varies. `parallel` is included because
//! it is a public encoder setter ([`EncodeConfig::with_parallel`]) that a
//! picker may want to drive, even though the byte-identity sweep pins it
//! off (thread-dependent chunking is not a rate-distortion knob — see
//! `crate::sweep`).

#![cfg(feature = "__expert")]

use crate::encode::EncodeConfig;
use crate::types::Compression;

/// Bundle of advanced encoder tuning knobs. Expert-only.
///
/// Intended for codec calibration sweeps and the picker training
/// pipeline. Production callers should rely on the per-axis builder
/// methods on [`EncodeConfig`] instead.
///
/// Every field is `Option<_>`. `None` means "leave the
/// [`EncodeConfig`]'s existing value alone." Apply with
/// [`EncodeConfig::with_internal_params`].
///
/// `#[non_exhaustive]` so adding a new axis is a non-breaking change.
///
/// ```ignore
/// # #[cfg(feature = "__expert")]
/// # {
/// use zenpng::{Compression, EncodeConfig};
/// use zenpng::internal_params::InternalParams;
///
/// let cfg = EncodeConfig::default()
///     .with_internal_params(InternalParams {
///         compression: Some(Compression::Fast),
///         ..Default::default()
///     });
/// # }
/// ```
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct InternalParams {
    /// Override the compression effort dial (any [`Compression`]
    /// variant). This is PNG's only rate-distortion-relevant axis and the
    /// sole axis [`crate::sweep::SweepVariant`] varies.
    ///
    /// Applied via [`EncodeConfig::with_compression`].
    pub compression: Option<Compression>,

    /// Toggle multi-threaded screening and refinement.
    ///
    /// Applied via [`EncodeConfig::with_parallel`]. Note the byte-identity
    /// sweep pins this off (see `crate::sweep`); it is exposed here only
    /// because it is a public encoder setter a picker might drive.
    pub parallel: Option<bool>,
}

impl EncodeConfig {
    /// Apply an [`InternalParams`] bundle, overriding each axis whose
    /// field is `Some(_)` and leaving the rest untouched (partial-merge).
    ///
    /// Each `Some` field routes through the corresponding builder setter,
    /// so this is exactly equivalent to calling those setters by hand.
    ///
    /// Cross-codec uniformity entry point (`__expert`-gated): mirrors
    /// `zenjpeg`'s `EncoderConfig::with_internal_params` so external
    /// pipelines can drive every zen codec with one bundle shape.
    #[must_use]
    pub fn with_internal_params(mut self, params: InternalParams) -> Self {
        if let Some(compression) = params.compression {
            self = self.with_compression(compression);
        }
        if let Some(parallel) = params.parallel {
            self = self.with_parallel(parallel);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline() -> EncodeConfig {
        EncodeConfig::default()
    }

    /// Empty `InternalParams` (all `None`) leaves the config bytewise
    /// equivalent to the constructor default — debug-format equality
    /// is a coarse but reliable check that no field flipped.
    #[test]
    fn default_internal_params_is_noop() {
        let cfg = baseline();
        let cfg2 = baseline().with_internal_params(InternalParams::default());
        assert_eq!(format!("{cfg:?}"), format!("{cfg2:?}"));
    }

    #[test]
    fn compression_field_applies() {
        let cfg = baseline().with_internal_params(InternalParams {
            compression: Some(Compression::Fast),
            ..Default::default()
        });
        assert_eq!(cfg.compression, Compression::Fast);
        // And the resolved effort matches the preset.
        assert_eq!(cfg.compression.effort(), Compression::Fast.effort());
    }

    #[test]
    fn compression_effort_variant_applies() {
        let cfg = baseline().with_internal_params(InternalParams {
            compression: Some(Compression::Effort(15)),
            ..Default::default()
        });
        assert_eq!(cfg.compression, Compression::Effort(15));
    }

    #[test]
    fn parallel_field_applies() {
        // Default is false; setting Some(true) must flip it.
        assert!(!baseline().parallel);
        let cfg = baseline().with_internal_params(InternalParams {
            parallel: Some(true),
            ..Default::default()
        });
        assert!(cfg.parallel);
    }

    #[test]
    fn parallel_none_leaves_value_alone() {
        // Start from parallel=true, then apply a bundle that doesn't touch
        // it — the true must survive.
        let cfg = baseline()
            .with_parallel(true)
            .with_internal_params(InternalParams {
                compression: Some(Compression::Turbo),
                ..Default::default()
            });
        assert!(
            cfg.parallel,
            "parallel=None must not reset an existing true"
        );
        assert_eq!(cfg.compression, Compression::Turbo);
    }

    /// Both fields together: each non-default field produces an observable
    /// state change vs the baseline.
    #[test]
    fn full_permutation_round_trip() {
        let cfg = baseline().with_internal_params(InternalParams {
            compression: Some(Compression::Intense),
            parallel: Some(true),
        });
        assert_eq!(cfg.compression, Compression::Intense);
        assert!(cfg.parallel);
    }
}
