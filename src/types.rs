//! Core types for PNG encoding configuration.
//!
//! These are zenpng's own types, independent of the `png` crate backend.

/// PNG compression effort.
///
/// Controls the trade-off between encoding speed and output file size.
/// Higher effort produces smaller files but takes longer.
///
/// Named presets are placed at Pareto-optimal points on the effort curve,
/// approximately log-spaced in encode time (each step ~2x slower).
/// Use [`Effort`](Self::Effort) for fine-grained control between presets.
///
/// | Preset | Effort | Description |
/// |---------|--------|-------------|
/// | `None` | 0 | Uncompressed |
/// | `Fastest` | 1 | 1 strategy (Paeth), turbo DEFLATE |
/// | `Turbo` | 2 | 3 strategies, turbo DEFLATE |
/// | `Fast` | 7 | 5 strategies, FastHt screen-only |
/// | `Balanced` | 13 | 9 strategies, screen + lazy refine |
/// | `Thorough` | 17 | 9 strategies, lazy2 multi-tier + brute-force |
/// | `High` | 19 | Near-optimal multi-tier + brute-force |
/// | `Aggressive` | 22 | Near-optimal + extended brute-force |
/// | `Intense` | 24 | Full brute-force + near-optimal |
/// | `Crush` | 27 | Full brute-force + beam search + zenzop |
/// | `Maniac` | 30 | Maximum standard pipeline |
/// | `Minutes` | 200 | Full pipeline + 184 FullOptimal iterations |
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Compression {
    /// No compression (uncompressed DEFLATE blocks). Maximum speed, maximum size.
    None,
    /// Fastest compression. Single strategy (Paeth) with turbo DEFLATE.
    Fastest,
    /// Turbo compression. 3 strategies with turbo DEFLATE.
    Turbo,
    /// Fast compression. 5 strategies, FastHt screen-only — the sweet spot of
    /// the fast range.
    Fast,
    /// Balanced compression (default). Good trade-off for most images.
    #[default]
    Balanced,
    /// Thorough compression. Lazy2 multi-tier refinement with brute-force.
    Thorough,
    /// High compression. Near-optimal DEFLATE with brute-force.
    High,
    /// Aggressive compression. Near-optimal with extended brute-force.
    Aggressive,
    /// Intense compression. Full brute-force filter sweep with near-optimal
    /// DEFLATE. The strongest level before zenzop enters the picture.
    Intense,
    /// Ultra compression. Full brute-force sweep, beam search, and zenzop
    /// recompression. Requires the `zopfli` feature; falls back to `Intense`
    /// if the feature is not enabled.
    Crush,
    /// Maximum standard-pipeline compression. Full brute-force sweep, beam
    /// search, and zenzop with maximum effort. Requires the `zopfli` feature;
    /// falls back to `Intense` if not enabled.
    Maniac,
    /// Extreme compression with FullOptimal recompression (184 iterations).
    /// Runs the full Maniac pipeline plus iterative forward-DP DEFLATE
    /// parsing. Expect minutes per megapixel. Produces the smallest
    /// possible output. Requires the `zopfli` feature for best results.
    Minutes,
    /// Explicit effort level (0-200).
    ///
    /// Provides fine-grained control between the named presets. Named presets
    /// are equivalent to specific effort values (e.g., `Balanced` = `Effort(13)`).
    ///
    /// Effort 0-30 uses zenflate's standard compression strategies.
    /// Effort 31+ adds FullOptimal recompression with `effort - 16` iterations.
    /// With the `zopfli` feature, effort 31+ uses zenzop (enhanced zopfli fork)
    /// for even better results.
    Effort(u32),
}

impl Compression {
    /// Get the effort level for this compression setting.
    pub fn effort(self) -> u32 {
        match self {
            Compression::None => 0,
            Compression::Fastest => 1,
            Compression::Turbo => 2,
            Compression::Fast => 7,
            Compression::Balanced => 13,
            Compression::Thorough => 17,
            Compression::High => 19,
            Compression::Aggressive => 22,
            Compression::Intense => 24,
            Compression::Crush => 27,
            Compression::Maniac => 30,
            Compression::Minutes => 200,
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
