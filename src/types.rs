//! Core types for PNG encoding configuration.
//!
//! These are zenpng's own types, independent of the `png` crate backend.

/// PNG compression effort.
///
/// Controls the trade-off between encoding speed and output file size.
/// Higher effort produces smaller files but takes longer.
///
/// Against zenpng before the 2026-10-06 encoder work (median, 1024 px):
/// `Fast` is 1.7x faster and 6% smaller, `Balanced` 5% faster and 4.3%
/// smaller, `High` 0.5% smaller but 1.5x slower (see the crate README).
/// Use [`Effort`](Self::Effort) for fine-grained control between presets.
///
/// | Preset | Effort | Description |
/// |---------|--------|-------------|
/// | `None` | 0 | Uncompressed |
/// | `Fastest` | 1 | Paeth filter, zenflate png(1) |
/// | `Turbo` | 2 | Paeth or MinSum (screened), png(2) |
/// | `Fast` | 7 | Paeth or MinSum, png(12) |
/// | `Balanced` | 13 | None, Paeth or MinSum (screened at png(10)), near-optimal png(19) |
/// | `Thorough` | 17 | Balanced's screen, png(26)/png(28) + brute-force rows |
/// | `High` | 19 | Thorough + png(30), wider brute-force |
/// | `Aggressive` | 22 | + 9 strategies (best 3 refined), fork and adaptive-fork brute-force |
/// | `Intense` | 24 | + full brute-force sweep, block brute-force |
/// | `Crush` | 27 | + recompression and beam search |
/// | `Maniac` | 30 | Maximum standard pipeline (9 candidates refined) |
/// | `Brag` | 31 | Full pipeline + 15 FullOptimal iterations (beats ECT-9) |
/// | `Minutes` | 200 | Full pipeline + 184 FullOptimal iterations |
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Compression {
    /// No compression (uncompressed DEFLATE blocks). Maximum speed, maximum size.
    None,
    /// Fastest compression. Paeth filter with zenflate's png(1) (literals and
    /// zero runs); faster and smaller than the `png` crate's `Fast`.
    Fastest,
    /// Turbo compression. Paeth or MinSum, whichever compresses smaller,
    /// with png(2).
    Turbo,
    /// Fast compression. Paeth or MinSum with png(12).
    Fast,
    /// Balanced compression (default). None, Paeth or MinSum, whichever
    /// compresses smallest at png(10), with near-optimal DEFLATE (png(19)).
    /// Unfiltered rows win on line art, documents and screenshots.
    #[default]
    Balanced,
    /// Thorough compression. Balanced's screen with png(26) and png(28) plus
    /// brute-force row filtering.
    Thorough,
    /// High compression. Thorough plus png(30) and wider brute-force.
    High,
    /// Aggressive compression. Nine filter strategies (the best three
    /// refined) plus fork and adaptive-fork brute-force.
    Aggressive,
    /// Intense compression. Full brute-force filter sweep and block
    /// brute-force with near-optimal DEFLATE. The strongest level before
    /// recompression enters the picture.
    Intense,
    /// Ultra compression. Full brute-force sweep, beam search, and zenzop
    /// recompression. Requires the `zopfli` feature; falls back to `Intense`
    /// if the feature is not enabled.
    Crush,
    /// Maximum standard-pipeline compression. Full brute-force sweep, beam
    /// search, and zenzop with maximum effort. Requires the `zopfli` feature;
    /// falls back to `Intense` if not enabled.
    Maniac,
    /// SOTA compression. Full Maniac pipeline plus 15 FullOptimal iterations.
    /// Beats ECT-9 (60 zopfli iterations) on aggregate. Requires the `zopfli`
    /// feature for best results.
    Brag,
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
    #[must_use]
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
            Compression::Brag => 31,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Filter {
    /// Automatic multi-strategy filter selection (recommended).
    #[default]
    Auto,
}
