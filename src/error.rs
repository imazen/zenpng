//! PNG error types.

/// Errors from PNG encode/decode operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PngError {
    /// PNG decoding error from the underlying `png` crate.
    #[error("PNG decode error: {0}")]
    Decode(#[from] png::DecodingError),

    /// PNG encoding error from the underlying `png` crate.
    #[error("PNG encode error: {0}")]
    Encode(#[from] png::EncodingError),

    /// Invalid input (dimensions, buffer size, etc.).
    #[error("invalid input: {0}")]
    InvalidInput(alloc::string::String),

    /// Resource limit exceeded.
    #[error("limit exceeded: {0}")]
    LimitExceeded(alloc::string::String),

    /// Quantization error.
    #[cfg(feature = "quantize")]
    #[error("quantize error: {0}")]
    Quantize(#[from] zenquant::QuantizeError),
}
