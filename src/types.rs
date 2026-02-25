//! Core types for PNG encoding configuration.
//!
//! These are zenpng's own types, independent of the `png` crate backend.

/// PNG compression effort.
///
/// Controls the trade-off between encoding speed and output file size.
/// Higher effort produces smaller files but takes longer.
///
/// Named presets map to effort levels on a 0-30 scale. Each ~3 effort
/// points roughly doubles encoding time. Use [`Effort`](Self::Effort)
/// for fine-grained control between presets.
///
/// | Preset | Effort | Description |
/// |---------|--------|-------------|
/// | `None` | 0 | Uncompressed |
/// | `Fastest` | 2 | Single filter, turbo DEFLATE |
/// | `Fast` | 6 | 5 strategies, fast-ht screen only |
/// | `Balanced` | 10 | 9 strategies, screen + lazy refine |
/// | `Thorough` | 13 | 9 strategies, screen + lazy+ refine |
/// | `High` | 16 | Screen + lazy2 multi-tier refine |
/// | `Aggressive` | 20 | Screen + near-optimal multi-tier |
/// | `Best` | 24 | Brute-force + near-optimal |
/// | `Crush` | 28 | Full sweep + zopfli |
/// | `Maniac` | 30 | Maximum everything |
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Compression {
    /// No compression (uncompressed DEFLATE blocks). Maximum speed, maximum size.
    None,
    /// Fastest compression. Good for large images where speed matters more than size.
    Fastest,
    /// Fast compression. Better ratio than `Fastest` with modest speed cost.
    Fast,
    /// Balanced compression (default). Good trade-off for most images.
    #[default]
    Balanced,
    /// Thorough compression. Double-lazy DEFLATE matching.
    Thorough,
    /// High compression. Multi-tier lazy2 DEFLATE refinement.
    High,
    /// Aggressive compression. Near-optimal DEFLATE multi-tier.
    Aggressive,
    /// Best compression with zenflate. Near-optimal multi-pass parser with
    /// brute-force per-row filter selection. ~10x slower than `Balanced`.
    Best,
    /// Ultra compression. Uses zenflate near-optimal for filter selection, then
    /// zopfli for final DEFLATE compression. ~50x slower than `Balanced`, but
    /// produces the smallest files. Requires the `zopfli` feature; falls back
    /// to `Best` if the feature is not enabled.
    Crush,
    /// Maximum possible compression. Full brute-force sweep and zopfli with
    /// maximum effort. Extremely slow — minutes per megapixel.
    /// Requires the `zopfli` feature; falls back to `Best` if not enabled.
    Maniac,
    /// Explicit effort level (0-200).
    ///
    /// Provides fine-grained control between the named presets. Each ~3 effort
    /// points roughly doubles encoding time. Named presets are equivalent to
    /// specific effort values (e.g., `Balanced` = `Effort(10)`).
    ///
    /// Effort 0-30 uses zenflate's standard compression strategies.
    /// Effort 31+ enables zenflate's FullOptimal (Zopfli-style forward DP)
    /// compression in Phase 4 with `(effort-16)*2` iterations. Higher effort
    /// values run more iterations for smaller output at the cost of time.
    Effort(u32),
}

impl Compression {
    /// Get the effort level for this compression setting.
    pub fn effort(self) -> u32 {
        match self {
            Compression::None => 0,
            Compression::Fastest => 2,
            Compression::Fast => 6,
            Compression::Balanced => 10,
            Compression::Thorough => 13,
            Compression::High => 16,
            Compression::Aggressive => 20,
            Compression::Best => 24,
            Compression::Crush => 28,
            Compression::Maniac => 30,
            Compression::Effort(e) => e.min(200),
        }
    }
}

/// PNG row filter strategy.
///
/// Currently only automatic multi-strategy selection is supported. The encoder
/// tries 8 strategies (5 single-filter + 3 adaptive heuristics) and keeps the
/// smallest result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Filter {
    /// Automatic multi-strategy filter selection (recommended).
    #[default]
    Auto,
}
