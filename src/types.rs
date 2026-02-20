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
/// | `Crush` | 12+zopfli | Near-optimal filter eval, zopfli final compression |
/// | `Budget(ms)` | adaptive | Best result within wall-clock budget |
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
    /// Maximum compression with zenflate. Near-optimal parser, ~3x slower than `Balanced`.
    Best,
    /// Ultra compression. Uses zenflate near-optimal for filter selection, then
    /// zopfli for final DEFLATE compression. ~50x slower than `Balanced`, but
    /// produces the smallest files. Requires the `zopfli` feature; falls back
    /// to `Best` if the feature is not enabled.
    Crush,
    /// Best result within a wall-clock time budget (milliseconds).
    /// Internally uses the same progressive engine as other levels, enabling
    /// all strategies (including zopfli when the feature is enabled) and
    /// letting the deadline control how far to go.
    Budget(u32),
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
            Compression::Best | Compression::Crush | Compression::Budget(_) => 12,
        }
    }

    /// Whether this level uses zopfli for final compression.
    pub(crate) fn use_zopfli(self) -> bool {
        matches!(self, Compression::Crush | Compression::Budget(_)) && cfg!(feature = "zopfli")
    }

    /// Return the explicit time budget in milliseconds, if any.
    pub(crate) fn budget_ms(self) -> Option<u32> {
        match self {
            Compression::Budget(ms) => Some(ms),
            _ => None,
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
