//! Core types for PNG encoding configuration.
//!
//! These are zenpng's own types, independent of the `png` crate backend.

/// PNG compression level.
///
/// Controls the trade-off between encoding speed and output file size.
/// Higher levels produce smaller files but take longer.
///
/// Levels map to [zenflate](https://crates.io/crates/zenflate) compression strategies:
///
/// | Variant | Level | Strategy |
/// |---------|-------|----------|
/// | `None` | 0 | Store (no compression) |
/// | `Fastest` | 1 | Hash table |
/// | `Fast` | 4 | Greedy |
/// | `Balanced` | 6 | Lazy (default) |
/// | `Thorough` | 8 | Double lazy + brute-force filter selection |
/// | `High` | 9 | Double lazy + brute-force filter selection |
/// | `Aggressive` | 10 | Near-optimal (2-pass) + brute-force filters |
/// | `Best` | 12 | Near-optimal multi-pass + dual brute-force configs |
/// | `Crush` | 12+zopfli | Near-optimal filter eval, zopfli final compression |
/// | `Obsessive` | 12+sweep+zopfli | Full brute-force sweep + zopfli |
/// | `Maniac` | 12+L6 screen+sweep+zopfli | Accurate screening + full sweep + zopfli |
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
    /// Thorough compression. Double-lazy DEFLATE with deeper brute-force context.
    Thorough,
    /// High compression. Double-lazy DEFLATE with brute-force filter selection.
    High,
    /// Aggressive compression. Near-optimal DEFLATE (2-pass) entry point.
    Aggressive,
    /// Maximum compression with zenflate. Near-optimal parser, ~3x slower than `Balanced`.
    Best,
    /// Ultra compression. Uses zenflate near-optimal for filter selection, then
    /// zopfli for final DEFLATE compression. ~50x slower than `Balanced`, but
    /// produces the smallest files. Requires the `zopfli` feature; falls back
    /// to `Best` if the feature is not enabled.
    Crush,
    /// Obsessive compression. Full brute-force filter sweep (all context/eval combos)
    /// at L12, plus zopfli. Tests many more brute-force configurations than `Crush`.
    /// Requires the `zopfli` feature; falls back to `Best` if not enabled.
    Obsessive,
    /// Maximum possible compression. Screens all heuristic strategies at L6 for
    /// more accurate ranking, full brute-force sweep, and zopfli with maximum
    /// candidates. Extremely slow — minutes per megapixel.
    /// Requires the `zopfli` feature; falls back to `Best` if not enabled.
    Maniac,
}

impl Compression {
    /// Convert to internal compression level (0-14).
    ///
    /// Levels 0-12 map directly to zenflate compression levels.
    /// Levels 13-14 use zenflate L12 internally with additional processing.
    pub(crate) fn to_zenflate_level(self) -> u8 {
        match self {
            Compression::None => 0,
            Compression::Fastest => 1,
            Compression::Fast => 4,
            Compression::Balanced => 6,
            Compression::Thorough => 8,
            Compression::High => 9,
            Compression::Aggressive => 10,
            Compression::Best => 12,
            Compression::Crush => 12,
            Compression::Obsessive => 13,
            Compression::Maniac => 14,
        }
    }

    /// Whether this level uses zopfli for final compression.
    pub(crate) fn use_zopfli(self) -> bool {
        matches!(
            self,
            Compression::Crush | Compression::Obsessive | Compression::Maniac
        ) && cfg!(feature = "zopfli")
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
