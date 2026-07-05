//! PNG error types with `whereat` location tracking.

use whereat::At;

/// Result type alias using `At<PngError>` for automatic location tracking.
pub type Result<T> = core::result::Result<T, At<PngError>>;

/// Errors from PNG encode/decode operations.
///
/// Each variant maps to exactly one coarse [`zencodec::ErrorCategory`] (see the
/// [`CategorizedError`](zencodec::CategorizedError) impl) so consumers can route
/// on the category — HTTP status, retry policy, logging — without matching this
/// enum directly.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PngError {
    /// Corrupt or invalid PNG bitstream content (bad chunk, CRC mismatch,
    /// invalid structure). Maps to [`ErrorCategory::MalformedImage`].
    ///
    /// [`ErrorCategory::MalformedImage`]: zencodec::ErrorCategory::MalformedImage
    #[error("PNG decode error: {0}")]
    Decode(alloc::string::String),

    /// Input ended before a needed field/chunk was complete (truncated stream,
    /// missing IDAT, empty PNG). Maps to [`ErrorCategory::UnexpectedEof`].
    ///
    /// [`ErrorCategory::UnexpectedEof`]: zencodec::ErrorCategory::UnexpectedEof
    #[error("unexpected end of PNG data: {0}")]
    Truncated(alloc::string::String),

    /// Data does not begin with the PNG signature — this isn't a PNG file at
    /// all, as opposed to a recognized-but-corrupt PNG (that's
    /// [`Decode`](Self::Decode)). Maps to [`ErrorCategory::UnsupportedImageType`].
    ///
    /// [`ErrorCategory::UnsupportedImageType`]: zencodec::ErrorCategory::UnsupportedImageType
    #[error("not a PNG file: {0}")]
    NotPng(alloc::string::String),

    /// A structurally-valid PNG feature this codec does not implement
    /// (unknown filter type, unsupported color-type/bit-depth combination).
    /// Maps to [`ErrorCategory::UnsupportedImageFeature`].
    ///
    /// [`ErrorCategory::UnsupportedImageFeature`]: zencodec::ErrorCategory::UnsupportedImageFeature
    #[error("unsupported PNG feature: {0}")]
    UnsupportedFeature(alloc::string::String),

    /// Invalid caller-supplied parameters or configuration (bad dimensions,
    /// inconsistent streaming usage, quantizer config, mismatched buffer size).
    /// Maps to [`ErrorCategory::InvalidParameters`].
    ///
    /// [`ErrorCategory::InvalidParameters`]: zencodec::ErrorCategory::InvalidParameters
    #[error("invalid input: {0}")]
    InvalidInput(alloc::string::String),

    /// The input is valid and could be processed, but a configured decode
    /// policy declined it (e.g. animation forbidden, progressive/interlaced
    /// content forbidden). The request was understood and *declined*, so it
    /// is neither malformed nor unsupported. Maps to
    /// [`ErrorCategory::PolicyRejected`].
    ///
    /// [`ErrorCategory::PolicyRejected`]: zencodec::ErrorCategory::PolicyRejected
    #[error("rejected by policy: {0}")]
    PolicyRejected(alloc::string::String),

    /// An unsupported operation, including pixel-format negotiation failures.
    /// Delegates its category to the wrapped [`zencodec::UnsupportedOperation`]
    /// (`PixelFormat` → [`ErrorCategory::UnsupportedPixelFormat`], otherwise
    /// [`ErrorCategory::UnsupportedOperation`]).
    ///
    /// [`ErrorCategory::UnsupportedPixelFormat`]: zencodec::ErrorCategory::UnsupportedPixelFormat
    /// [`ErrorCategory::UnsupportedOperation`]: zencodec::ErrorCategory::UnsupportedOperation
    #[error("unsupported operation: {0}")]
    Unsupported(zencodec::UnsupportedOperation),

    /// Output sink / I/O write failure. Maps to [`ErrorCategory::Io`].
    ///
    /// [`ErrorCategory::Io`]: zencodec::ErrorCategory::Io
    #[error("I/O error: {0}")]
    Io(alloc::string::String),

    /// A configured resource limit was exceeded. Wraps the typed
    /// [`zencodec::LimitExceeded`] so the [`LimitKind`](zencodec::LimitKind) is
    /// preserved; delegates its category to
    /// [`ErrorCategory::LimitsExceeded`]`(kind)`.
    ///
    /// [`ErrorCategory::LimitsExceeded`]: zencodec::ErrorCategory::LimitsExceeded
    #[error("resource limit exceeded: {0}")]
    Limit(zencodec::LimitExceeded),

    /// Memory acquisition failed: a fallible allocation returned an error, or a
    /// size computation overflowed the platform's address space (so the buffer
    /// can never be allocated). Maps to [`ErrorCategory::OutOfMemory`].
    ///
    /// [`ErrorCategory::OutOfMemory`]: zencodec::ErrorCategory::OutOfMemory
    #[error("limit exceeded: {0}")]
    LimitExceeded(alloc::string::String),

    /// Operation stopped by cooperative cancellation. Delegates its category to
    /// the wrapped [`enough::StopReason`] (`TimedOut` →
    /// [`ErrorCategory::TimedOut`], otherwise [`ErrorCategory::Cancelled`]).
    ///
    /// [`ErrorCategory::TimedOut`]: zencodec::ErrorCategory::TimedOut
    /// [`ErrorCategory::Cancelled`]: zencodec::ErrorCategory::Cancelled
    #[error("stopped: {0}")]
    Stopped(enough::StopReason),

    /// Quantization error (zenquant backend). Maps to
    /// [`ErrorCategory::Internal`].
    ///
    /// [`ErrorCategory::Internal`]: zencodec::ErrorCategory::Internal
    #[cfg(feature = "quantize")]
    #[error("quantize error: {0}")]
    Quantize(#[from] zenquant::QuantizeError),
}

impl From<enough::StopReason> for PngError {
    fn from(reason: enough::StopReason) -> Self {
        PngError::Stopped(reason)
    }
}

impl From<zencodec::UnsupportedOperation> for PngError {
    fn from(op: zencodec::UnsupportedOperation) -> Self {
        PngError::Unsupported(op)
    }
}

impl From<zencodec::LimitExceeded> for PngError {
    fn from(limit: zencodec::LimitExceeded) -> Self {
        PngError::Limit(limit)
    }
}

// Codec-agnostic error taxonomy (zencodec PR #103). Maps every `PngError`
// variant to exactly one coarse `ErrorCategory` so consumers can route on the
// category without naming this enum. `zencodec` is a non-optional dependency, so
// this impl is unconditional.
impl zencodec::CategorizedError for PngError {
    fn codec_name(&self) -> Option<&'static str> {
        Some("zenpng")
    }

    fn category(&self) -> zencodec::ErrorCategory {
        use zencodec::ErrorCategory as C;
        match self {
            // Corrupt / invalid bitstream content.
            PngError::Decode(_) => C::MalformedImage,

            // Truncated input / unexpected end of data.
            PngError::Truncated(_) => C::UnexpectedEof,

            // Missing/incorrect PNG signature — this isn't a PNG at all.
            PngError::NotPng(_) => C::UnsupportedImageType,

            // A valid PNG feature we don't implement.
            PngError::UnsupportedFeature(_) => C::UnsupportedImageFeature,

            // Bad caller parameters / configuration / usage.
            PngError::InvalidInput(_) => C::InvalidParameters,

            // Understood and declined by a configured decode policy.
            PngError::PolicyRejected(_) => C::PolicyRejected,

            // Output sink / I/O failures.
            PngError::Io(_) => C::Io(zencodec::CodecIoKind::opaque()),

            // Memory acquisition failure (alloc failed or address-space overflow).
            PngError::LimitExceeded(_) => C::OutOfMemory,

            // Delegate to the wrapped zencodec cause types — they carry their
            // own `CategorizedError` impl (kind / pixel-format / stop-reason).
            PngError::Unsupported(op) => op.category(),
            PngError::Limit(limit) => limit.category(),
            PngError::Stopped(reason) => reason.category(),

            // Quantizer backend failure. Delegating would need zenquant itself
            // to impl `CategorizedError` (out of scope here); treat as internal.
            #[cfg(feature = "quantize")]
            PngError::Quantize(_) => C::Internal,
        }
    }
}

/// Categorize a [`ProbeError`]: structural pre-decode probe failures.
///
/// `ProbeError` is caller-facing (returned by [`crate::detect::probe`]), so it
/// carries its own [`CategorizedError`](zencodec::CategorizedError) impl too.
impl zencodec::CategorizedError for crate::detect::ProbeError {
    fn codec_name(&self) -> Option<&'static str> {
        Some("zenpng")
    }

    fn category(&self) -> zencodec::ErrorCategory {
        use crate::detect::ProbeError;
        use zencodec::ErrorCategory as C;
        match self {
            // Not enough bytes yet / structure cut short → need more input.
            ProbeError::TooShort | ProbeError::Truncated => C::UnexpectedEof,
            // Signature absent → this isn't a PNG at all.
            ProbeError::NotPng => C::UnsupportedImageType,
        }
    }
}

/// Bridge a bare [`PngError`] into the shared
/// [`CodecError`](zencodec::CodecError) envelope (Pattern B).
///
/// `.start_at()` begins the location trace; [`CodecError::of`] then reads the
/// [`category`](zencodec::CategorizedError::category) *and* the
/// [`codec_name`](zencodec::CategorizedError::codec_name) from the value, keeping
/// the trace on the outside. With this, `?`/`.into()` on a bare `PngError`
/// auto-wraps into the envelope the zencodec trait impls return.
///
/// Already-located `At<PngError>` values convert via `.map_err(CodecError::of)`
/// instead — the orphan rule forbids a `From<At<PngError>>` impl here (`At` is
/// not a fundamental type, so `At<PngError>` is not a local type).
///
/// [`CodecError::of`]: zencodec::CodecError::of
impl From<PngError> for At<zencodec::CodecError> {
    #[track_caller]
    fn from(e: PngError) -> Self {
        use whereat::ErrorAtExt;
        zencodec::CodecError::of(e.start_at())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use whereat::at;
    use zencodec::{CategorizedError, ErrorCategory as C, LimitKind as L};

    #[test]
    fn error_display_decode() {
        let e = PngError::Decode("bad chunk".into());
        assert!(e.to_string().contains("bad chunk"));
    }

    #[test]
    fn error_display_invalid_input() {
        let e = PngError::InvalidInput("wrong size".into());
        assert!(e.to_string().contains("wrong size"));
    }

    #[test]
    fn error_display_limit_exceeded() {
        let e = PngError::LimitExceeded("too big".into());
        assert!(e.to_string().contains("too big"));
    }

    #[test]
    fn error_from_stop_reason() {
        let reason = enough::StopReason::Cancelled;
        let e: PngError = reason.into();
        assert!(matches!(e, PngError::Stopped(_)));
    }

    #[test]
    fn error_from_unsupported_operation() {
        let op = zencodec::UnsupportedOperation::RowLevelEncode;
        let e: PngError = op.into();
        assert!(matches!(e, PngError::Unsupported(_)));
        assert!(e.to_string().contains("unsupported operation"));
    }

    #[test]
    fn error_from_limit_exceeded() {
        let limit = zencodec::LimitExceeded::Pixels { actual: 9, max: 4 };
        let e: PngError = limit.into();
        assert!(matches!(e, PngError::Limit(_)));
    }

    #[test]
    fn error_with_whereat() {
        fn inner() -> Result<()> {
            Err(at!(PngError::Decode("test".into())))
        }

        fn outer() -> Result<()> {
            inner().map_err(|e| e.at())?;
            Ok(())
        }

        let err = outer().unwrap_err();
        assert!(err.frame_count() >= 1);
    }

    // Every `PngError` variant maps to its documented `ErrorCategory`.
    #[test]
    fn error_category_mapping() {
        assert_eq!(PngError::Decode("x".into()).codec_name(), Some("zenpng"));

        assert_eq!(PngError::Decode("x".into()).category(), C::MalformedImage);
        assert_eq!(PngError::Truncated("x".into()).category(), C::UnexpectedEof);
        assert_eq!(
            PngError::NotPng("x".into()).category(),
            C::UnsupportedImageType
        );
        assert_eq!(
            PngError::UnsupportedFeature("x".into()).category(),
            C::UnsupportedImageFeature
        );
        assert_eq!(
            PngError::InvalidInput("x".into()).category(),
            C::InvalidParameters
        );
        assert_eq!(
            PngError::PolicyRejected("x".into()).category(),
            C::PolicyRejected
        );
        assert_eq!(
            PngError::Io("x".into()).category(),
            C::Io(zencodec::CodecIoKind::opaque())
        );
        assert_eq!(
            PngError::LimitExceeded("x".into()).category(),
            C::OutOfMemory
        );

        // Delegated arms.
        assert_eq!(
            PngError::Unsupported(zencodec::UnsupportedOperation::PixelFormat).category(),
            C::UnsupportedPixelFormat
        );
        assert_eq!(
            PngError::Unsupported(zencodec::UnsupportedOperation::RowLevelEncode).category(),
            C::UnsupportedOperation
        );
        assert_eq!(
            PngError::Limit(zencodec::LimitExceeded::Memory { actual: 9, max: 4 }).category(),
            C::LimitsExceeded(L::Memory)
        );
        assert_eq!(
            PngError::Limit(zencodec::LimitExceeded::Frames { actual: 9, max: 4 }).category(),
            C::LimitsExceeded(L::Frames)
        );
        assert_eq!(
            PngError::Stopped(enough::StopReason::Cancelled).category(),
            C::Cancelled
        );
        assert_eq!(
            PngError::Stopped(enough::StopReason::TimedOut).category(),
            C::TimedOut
        );
    }

    // `At<PngError>` forwards both the category and the codec name.
    #[test]
    fn error_category_through_at() {
        let err: At<PngError> = at!(PngError::Truncated("eof".into()));
        assert_eq!(err.category(), C::UnexpectedEof);
        assert_eq!(err.codec_name(), Some("zenpng"));
    }

    #[test]
    #[cfg(feature = "quantize")]
    fn error_quantize_is_internal() {
        let e: PngError = zenquant::QuantizeError::ZeroDimension.into();
        assert!(matches!(e, PngError::Quantize(_)));
        assert_eq!(e.category(), C::Internal);
    }

    // ProbeError categories.
    #[test]
    fn probe_error_category_mapping() {
        use crate::detect::ProbeError;
        assert_eq!(ProbeError::TooShort.codec_name(), Some("zenpng"));
        assert_eq!(ProbeError::TooShort.category(), C::UnexpectedEof);
        assert_eq!(ProbeError::Truncated.category(), C::UnexpectedEof);
        assert_eq!(ProbeError::NotPng.category(), C::UnsupportedImageType);
    }
}
