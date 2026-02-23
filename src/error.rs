//! PNG error types.

/// Errors from PNG encode/decode operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PngError {
    /// PNG decoding error.
    #[error("PNG decode error: {0}")]
    Decode(alloc::string::String),

    /// Invalid input (dimensions, buffer size, etc.).
    #[error("invalid input: {0}")]
    InvalidInput(alloc::string::String),

    /// Resource limit exceeded.
    #[error("limit exceeded: {0}")]
    LimitExceeded(alloc::string::String),

    /// Operation stopped by cooperative cancellation.
    #[error("stopped: {0}")]
    Stopped(enough::StopReason),

    /// Quantization error.
    #[cfg(feature = "quantize")]
    #[error("quantize error: {0}")]
    Quantize(#[from] zenquant::QuantizeError),
}

impl From<enough::StopReason> for PngError {
    fn from(reason: enough::StopReason) -> Self {
        PngError::Stopped(reason)
    }
}

impl From<zencodec_types::UnsupportedOperation> for PngError {
    fn from(op: zencodec_types::UnsupportedOperation) -> Self {
        PngError::InvalidInput(alloc::format!("unsupported operation: {op}"))
    }
}
