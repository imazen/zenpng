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
/// | `High` | 9 | Double lazy |
/// | `Best` | 12 | Near-optimal |
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
    /// High compression. Best ratio before the slower near-optimal parser.
    High,
    /// Maximum compression. Near-optimal parser, ~3x slower than `Balanced`.
    Best,
}

impl Compression {
    /// Convert to zenflate compression level (0-12).
    pub(crate) fn to_zenflate_level(self) -> u8 {
        match self {
            Compression::None => 0,
            Compression::Fastest => 1,
            Compression::Fast => 4,
            Compression::Balanced => 6,
            Compression::High => 9,
            Compression::Best => 12,
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
