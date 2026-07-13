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
    /// invalid structure). Maps to
    /// [`ErrorCategory::Image`]`(`[`ImageError::Malformed`]`)`.
    ///
    /// [`ErrorCategory::Image`]: zencodec::ErrorCategory::Image
    /// [`ImageError::Malformed`]: zencodec::ImageError::Malformed
    #[error("PNG decode error: {0}")]
    Decode(alloc::string::String),

    /// Input ended before a needed field/chunk was complete (truncated stream,
    /// missing IDAT, empty PNG). Maps to
    /// [`ErrorCategory::Image`]`(`[`ImageError::UnexpectedEof`]`)`.
    ///
    /// [`ErrorCategory::Image`]: zencodec::ErrorCategory::Image
    /// [`ImageError::UnexpectedEof`]: zencodec::ImageError::UnexpectedEof
    #[error("unexpected end of PNG data: {0}")]
    Truncated(alloc::string::String),

    /// Data does not begin with the PNG signature — this isn't a PNG file at
    /// all, as opposed to a recognized-but-corrupt PNG (that's
    /// [`Decode`](Self::Decode)). Maps to
    /// [`ErrorCategory::Image`]`(ImageError::Unsupported(`[`UnsupportedImageKind::Type`]`))`.
    ///
    /// [`ErrorCategory::Image`]: zencodec::ErrorCategory::Image
    /// [`UnsupportedImageKind::Type`]: zencodec::UnsupportedImageKind::Type
    #[error("not a PNG file: {0}")]
    NotPng(alloc::string::String),

    /// A structurally-valid PNG feature this codec does not implement
    /// (unknown filter type, unsupported color-type/bit-depth combination).
    /// Maps to
    /// [`ErrorCategory::Image`]`(ImageError::Unsupported(`[`UnsupportedImageKind::Feature`]`))`.
    ///
    /// [`ErrorCategory::Image`]: zencodec::ErrorCategory::Image
    /// [`UnsupportedImageKind::Feature`]: zencodec::UnsupportedImageKind::Feature
    #[error("unsupported PNG feature: {0}")]
    UnsupportedFeature(alloc::string::String),

    /// Invalid caller-supplied parameters or configuration (bad quantizer
    /// backend name/config, invalid frame count, dimensions that don't fit an
    /// operation). Maps to
    /// [`ErrorCategory::Request`]`(RequestError::Invalid(`[`InvalidKind::Parameters`]`))`.
    ///
    /// [`ErrorCategory::Request`]: zencodec::ErrorCategory::Request
    /// [`InvalidKind::Parameters`]: zencodec::InvalidKind::Parameters
    #[error("invalid input: {0}")]
    InvalidInput(alloc::string::String),

    /// A caller-supplied pixel buffer has an invalid layout — wrong size for
    /// the declared dimensions (palette, index buffer, frame data, row bytes).
    /// Maps to
    /// [`ErrorCategory::Request`]`(RequestError::Invalid(`[`InvalidKind::Buffer`]`))`.
    ///
    /// [`ErrorCategory::Request`]: zencodec::ErrorCategory::Request
    /// [`InvalidKind::Buffer`]: zencodec::InvalidKind::Buffer
    #[error("invalid pixel buffer: {0}")]
    InvalidBuffer(alloc::string::String),

    /// The streaming/animation encoder API was called out of sequence — e.g.
    /// `push_rows` with a width that doesn't match an already-established
    /// canvas width, more rows pushed than the declared canvas height,
    /// `finish()` before any rows/frames were pushed, or a pixel-data size
    /// that doesn't match what was accumulated. Maps to
    /// [`ErrorCategory::Request`]`(RequestError::Invalid(`[`InvalidKind::State`]`))`.
    ///
    /// [`ErrorCategory::Request`]: zencodec::ErrorCategory::Request
    /// [`InvalidKind::State`]: zencodec::InvalidKind::State
    #[error("invalid state: {0}")]
    InvalidState(alloc::string::String),

    /// An unsupported operation, including pixel-format negotiation failures.
    /// Delegates its category to the wrapped [`zencodec::UnsupportedOperation`]
    /// — always
    /// [`ErrorCategory::Request`]`(RequestError::Unsupported(op))`, carrying
    /// which operation (`PixelFormat` included, no longer split out).
    ///
    /// [`ErrorCategory::Request`]: zencodec::ErrorCategory::Request
    #[error("unsupported operation: {0}")]
    Unsupported(zencodec::UnsupportedOperation),

    /// The encode target's color needs a synthesized ICC/CMS transform this
    /// build will not perform itself — e.g. a CICP profile with no ICC
    /// synthesis available for it (enable the `cms` feature), and no ICC
    /// profile supplied in the metadata. CMS is the caller's job. Maps to
    /// [`ErrorCategory::Request`]`(RequestError::CmsRequired)`.
    ///
    /// [`ErrorCategory::Request`]: zencodec::ErrorCategory::Request
    #[error("colour-management transform required: {0}")]
    CmsRequired(alloc::string::String),

    /// Output sink / I/O write failure. Maps to [`ErrorCategory::Io`].
    ///
    /// [`ErrorCategory::Io`]: zencodec::ErrorCategory::Io
    #[error("I/O error: {0}")]
    Io(alloc::string::String),

    /// A configured resource limit was (or would be) exceeded. Wraps the
    /// typed [`zencodec::LimitExceeded`] so the
    /// [`LimitKind`](zencodec::LimitKind) is preserved; delegates its
    /// category to
    /// [`ErrorCategory::Resource`]`(ResourceError::Limits(kind))`. Distinct
    /// from [`OutOfMemory`](Self::OutOfMemory) — this is a configured cap the
    /// caller can raise, not genuine allocation exhaustion.
    ///
    /// [`ErrorCategory::Resource`]: zencodec::ErrorCategory::Resource
    #[error("resource limit exceeded: {0}")]
    LimitExceeded(zencodec::LimitExceeded),

    /// Memory acquisition failed: a fallible allocation returned an error, or
    /// a size computation overflowed the platform's address space (so the
    /// buffer can never be allocated). Distinct from
    /// [`LimitExceeded`](Self::LimitExceeded) — this is genuine allocation
    /// exhaustion, not a configured cap. Maps to
    /// [`ErrorCategory::Resource`]`(ResourceError::OutOfMemory)`.
    ///
    /// [`ErrorCategory::Resource`]: zencodec::ErrorCategory::Resource
    #[error("out of memory: {0}")]
    OutOfMemory(alloc::string::String),

    /// The input is valid and could be processed, but a configured
    /// [`DecodePolicy`](zencodec::decode::DecodePolicy) /
    /// [`EncodePolicy`](zencodec::encode::EncodePolicy) declined it (e.g.
    /// animation forbidden, progressive/interlaced content forbidden). The
    /// request was understood and *declined*, so it is neither malformed nor
    /// unsupported. Carries which policy family rejected it (the call site
    /// already knows). Maps to [`ErrorCategory::Policy`]`(kind)`.
    ///
    /// [`ErrorCategory::Policy`]: zencodec::ErrorCategory::Policy
    #[error("rejected by policy: {1}")]
    PolicyRejected(zencodec::PolicyKind, alloc::string::String),

    /// Operation stopped by cooperative cancellation. Delegates its category
    /// to the wrapped [`enough::StopReason`] — always
    /// [`ErrorCategory::Stopped`]`(reason)`, no lossy collapse.
    ///
    /// [`ErrorCategory::Stopped`]: zencodec::ErrorCategory::Stopped
    #[error("stopped: {0}")]
    Stopped(enough::StopReason),

    /// An internal failure not attributable to the input or the request:
    /// either a broken invariant in zenpng's own logic
    /// ([`InternalKind::Bug`](zencodec::InternalKind::Bug) — e.g. an
    /// internally-derived color type our own encoder failed to handle), or an
    /// unclassified error surfaced from a sub-component
    /// ([`InternalKind::Dependency`](zencodec::InternalKind::Dependency) — a
    /// zenflate/zenzop compression call returning something other than
    /// cancellation). The call site already knows which. Maps to
    /// [`ErrorCategory::Internal`]`(kind)`.
    ///
    /// [`ErrorCategory::Internal`]: zencodec::ErrorCategory::Internal
    #[error("{0}: {1}")]
    Internal(zencodec::InternalKind, alloc::string::String),

    /// Quantization error (zenquant backend). Most of zenquant's own variants
    /// are themselves caller-request faults (bad dimensions, out-of-range
    /// config) or a cancellation; the rest are an unclassified
    /// zenquant-internal failure. See the [`CategorizedError`] impl below for
    /// the full split — it is not a blanket [`ErrorCategory::Internal`].
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
        PngError::LimitExceeded(limit)
    }
}

// Codec-agnostic error taxonomy (zencodec's origin-first two-level
// ErrorCategory: Image/Request/Resource/Policy/Stopped/Io/Internal). Maps
// every `PngError` variant to exactly one coarse `ErrorCategory` so consumers
// can route on the category without naming this enum. `zencodec` is a
// non-optional dependency, so this impl is unconditional.
impl zencodec::CategorizedError for PngError {
    fn codec_name(&self) -> Option<&'static str> {
        Some("zenpng")
    }

    fn category(&self) -> zencodec::ErrorCategory {
        use zencodec::{
            ErrorCategory as C, ImageError, InvalidKind, RequestError, ResourceError,
            UnsupportedImageKind,
        };
        match self {
            // Image-bytes-origin: corrupt / truncated / unrecognized / an
            // implemented-format-but-unimplemented-feature.
            PngError::Decode(_) => ImageError::Malformed.into(),
            PngError::Truncated(_) => ImageError::UnexpectedEof.into(),
            PngError::NotPng(_) => UnsupportedImageKind::Type.into(),
            PngError::UnsupportedFeature(_) => UnsupportedImageKind::Feature.into(),

            // Caller-request-origin: bad parameters / bad buffer geometry / bad
            // call sequence / a CMS transform we won't perform ourselves.
            PngError::InvalidInput(_) => InvalidKind::Parameters.into(),
            PngError::InvalidBuffer(_) => InvalidKind::Buffer.into(),
            PngError::InvalidState(_) => InvalidKind::State.into(),
            PngError::CmsRequired(_) => C::Request(RequestError::CmsRequired),

            // Output sink / I/O failures.
            PngError::Io(_) => C::Io(zencodec::CodecIoKind::opaque()),

            // Memory acquisition failure (alloc failed or address-space
            // overflow) — distinct from a configured `LimitExceeded` cap.
            PngError::OutOfMemory(_) => C::Resource(ResourceError::OutOfMemory),

            // Understood and declined by a configured decode/encode policy.
            PngError::PolicyRejected(kind, _) => C::Policy(*kind),

            // A broken invariant in our own logic, or an unclassified error
            // from a sub-component (zenflate/zenzop) — the call site already
            // picked which.
            PngError::Internal(kind, _) => C::Internal(*kind),

            // Delegate to the wrapped zencodec cause types — they carry their
            // own `CategorizedError` impl (operation / limit-kind / stop-reason).
            PngError::Unsupported(op) => op.category(),
            PngError::LimitExceeded(limit) => limit.category(),
            PngError::Stopped(reason) => reason.category(),

            // Quantizer backend failure (zenquant 0.1.3). zenquant's own error
            // enum already carries enough detail to route most of it as a
            // caller-request fault instead of dumping everything into
            // `Internal` (the 13-codec audit flagged the old blanket mapping
            // as "acknowledged out-of-scope" — this closes that gap).
            #[cfg(feature = "quantize")]
            PngError::Quantize(e) => match e {
                // Bad caller-supplied config/dimensions.
                zenquant::QuantizeError::ZeroDimension
                | zenquant::QuantizeError::InvalidMaxColors(_) => InvalidKind::Parameters.into(),
                // Pixel buffer length doesn't match the declared dimensions.
                zenquant::QuantizeError::DimensionMismatch { .. } => InvalidKind::Buffer.into(),
                // `QualityNotMet` (couldn't hit the quality gate) — and any
                // future `#[non_exhaustive]` variant — is zenquant's own
                // algorithm failing to satisfy its contract, not something the
                // top-level PNG API caller did wrong: an unclassified
                // sub-component failure, not a permanent home (see
                // `InternalKind::Dependency`'s docs).
                _ => C::Internal(zencodec::InternalKind::Dependency),
            },
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
        use zencodec::{ImageError, UnsupportedImageKind};
        match self {
            // Not enough bytes yet / structure cut short → need more input.
            ProbeError::TooShort | ProbeError::Truncated => ImageError::UnexpectedEof.into(),
            // Signature absent → this isn't a PNG at all.
            ProbeError::NotPng => UnsupportedImageKind::Type.into(),
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
    use zencodec::{
        CategorizedError, ErrorCategory as C, ImageError, InternalKind, InvalidKind,
        LimitKind as L, PolicyKind, RequestError, ResourceError, UnsupportedImageKind,
    };

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
    fn error_display_out_of_memory() {
        let e = PngError::OutOfMemory("too big".into());
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
        assert!(matches!(e, PngError::LimitExceeded(_)));
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

        assert_eq!(
            PngError::Decode("x".into()).category(),
            C::Image(ImageError::Malformed)
        );
        assert_eq!(
            PngError::Truncated("x".into()).category(),
            C::Image(ImageError::UnexpectedEof)
        );
        assert_eq!(
            PngError::NotPng("x".into()).category(),
            C::Image(ImageError::Unsupported(UnsupportedImageKind::Type))
        );
        assert_eq!(
            PngError::UnsupportedFeature("x".into()).category(),
            C::Image(ImageError::Unsupported(UnsupportedImageKind::Feature))
        );
        assert_eq!(
            PngError::InvalidInput("x".into()).category(),
            C::Request(RequestError::Invalid(InvalidKind::Parameters))
        );
        assert_eq!(
            PngError::InvalidBuffer("x".into()).category(),
            C::Request(RequestError::Invalid(InvalidKind::Buffer))
        );
        assert_eq!(
            PngError::InvalidState("x".into()).category(),
            C::Request(RequestError::Invalid(InvalidKind::State))
        );
        assert_eq!(
            PngError::CmsRequired("x".into()).category(),
            C::Request(RequestError::CmsRequired)
        );
        assert_eq!(
            PngError::PolicyRejected(PolicyKind::Decode, "x".into()).category(),
            C::Policy(PolicyKind::Decode)
        );
        assert_eq!(
            PngError::PolicyRejected(PolicyKind::Encode, "x".into()).category(),
            C::Policy(PolicyKind::Encode)
        );
        assert_eq!(
            PngError::Io("x".into()).category(),
            C::Io(zencodec::CodecIoKind::opaque())
        );
        assert_eq!(
            PngError::OutOfMemory("x".into()).category(),
            C::Resource(ResourceError::OutOfMemory)
        );
        assert_eq!(
            PngError::Internal(InternalKind::Bug, "x".into()).category(),
            C::Internal(InternalKind::Bug)
        );
        assert_eq!(
            PngError::Internal(InternalKind::Dependency, "x".into()).category(),
            C::Internal(InternalKind::Dependency)
        );

        // Delegated arms.
        assert_eq!(
            PngError::Unsupported(zencodec::UnsupportedOperation::PixelFormat).category(),
            C::Request(RequestError::Unsupported(
                zencodec::UnsupportedOperation::PixelFormat
            ))
        );
        assert_eq!(
            PngError::Unsupported(zencodec::UnsupportedOperation::RowLevelEncode).category(),
            C::Request(RequestError::Unsupported(
                zencodec::UnsupportedOperation::RowLevelEncode
            ))
        );
        assert_eq!(
            PngError::LimitExceeded(zencodec::LimitExceeded::Memory { actual: 9, max: 4 })
                .category(),
            C::Resource(ResourceError::Limits(L::Memory))
        );
        assert_eq!(
            PngError::LimitExceeded(zencodec::LimitExceeded::Frames { actual: 9, max: 4 })
                .category(),
            C::Resource(ResourceError::Limits(L::Frames))
        );
        assert_eq!(
            PngError::Stopped(enough::StopReason::Cancelled).category(),
            C::Stopped(enough::StopReason::Cancelled)
        );
        assert_eq!(
            PngError::Stopped(enough::StopReason::TimedOut).category(),
            C::Stopped(enough::StopReason::TimedOut)
        );
    }

    // `At<PngError>` forwards both the category and the codec name.
    #[test]
    fn error_category_through_at() {
        let err: At<PngError> = at!(PngError::Truncated("eof".into()));
        assert_eq!(err.category(), C::Image(ImageError::UnexpectedEof));
        assert_eq!(err.codec_name(), Some("zenpng"));
    }

    // zenquant's own error variants (0.1.3: ZeroDimension, DimensionMismatch,
    // InvalidMaxColors, QualityNotMet) split across Request/Internal instead
    // of collapsing to a blanket `Internal`.
    #[test]
    #[cfg(feature = "quantize")]
    fn error_quantize_category_split() {
        let e: PngError = zenquant::QuantizeError::ZeroDimension.into();
        assert!(matches!(e, PngError::Quantize(_)));
        assert_eq!(
            e.category(),
            C::Request(RequestError::Invalid(InvalidKind::Parameters))
        );

        let e: PngError = zenquant::QuantizeError::InvalidMaxColors(9999).into();
        assert_eq!(
            e.category(),
            C::Request(RequestError::Invalid(InvalidKind::Parameters))
        );

        let e: PngError = zenquant::QuantizeError::DimensionMismatch {
            len: 1,
            width: 2,
            height: 2,
        }
        .into();
        assert_eq!(
            e.category(),
            C::Request(RequestError::Invalid(InvalidKind::Buffer))
        );

        // `QualityNotMet` — and any future `#[non_exhaustive]` variant — is
        // zenquant's own algorithm failing to satisfy its contract, not a
        // caller-request fault: an unclassified sub-component failure.
        let e: PngError = zenquant::QuantizeError::QualityNotMet {
            min_ssim2: 90.0,
            achieved_ssim2: 80.0,
        }
        .into();
        assert_eq!(e.category(), C::Internal(InternalKind::Dependency));
    }

    // ProbeError categories.
    #[test]
    fn probe_error_category_mapping() {
        use crate::detect::ProbeError;
        assert_eq!(ProbeError::TooShort.codec_name(), Some("zenpng"));
        assert_eq!(
            ProbeError::TooShort.category(),
            C::Image(ImageError::UnexpectedEof)
        );
        assert_eq!(
            ProbeError::Truncated.category(),
            C::Image(ImageError::UnexpectedEof)
        );
        assert_eq!(
            ProbeError::NotPng.category(),
            C::Image(ImageError::Unsupported(UnsupportedImageKind::Type))
        );
    }
}
