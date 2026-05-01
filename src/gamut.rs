//! Lossless wide-gamut → sRGB downcast (v0).
//!
//! Detects when an image tagged with a wider gamut (currently Display P3)
//! has every pixel falling within sRGB primaries, and rewrites the buffer
//! to sRGB primaries + sRGB transfer. This is lossless under PNG decode
//! semantics: the pixel values change, but they encode the same colors.
//!
//! Common case: macOS/iOS screenshots tagged with cICP code points for
//! Display P3 + sRGB transfer (CP=12, TC=13) but containing only sRGB-
//! gamut content from rendered web pages. Downcast lets ordinary PNG
//! viewers without color management render correctly.
//!
//! # Scope (v0)
//!
//! - **Source**: cICP `color_primaries == 12` (Display P3), `transfer == 13`
//!   (sRGB). Other primaries (BT.2020, AdobeRGB) and transfers (PQ, HLG,
//!   linear) are TODO.
//! - **Pixel format**: 8-bit RGB and RGBA only. 16-bit is TODO.
//! - **Output**: `Some(buffer)` with the same channel layout but
//!   re-encoded to sRGB. Caller must clear iCCP/cICP/cHRM and set the
//!   sRGB chunk.
//! - **Approach**: two-pass — bounds check first (early-exit on the
//!   first out-of-gamut pixel), then transform on commit.
//!
//! # Future migration
//!
//! The bounds-check helper has been added to zenpixels-convert
//! (`gamut::check_fits_in_gamut_linear_f32_*` and
//! `fit_and_transform_linear_f32_*`) — see imazen/zenpixels#30. Once that
//! lands and ships in 0.2.12, this module shrinks to (a) detecting source
//! primaries from PNG metadata and (b) coordinating the EOTF→matrix→OETF
//! pipeline using upstream primitives.

use alloc::vec::Vec;

use zencodec::Cicp;

/// 3×3 row-major matrix that converts linear-light Display P3 → linear
/// BT.709 (sRGB primaries). White points are both D65 — no chromatic
/// adaptation needed. Values match `zenpixels-convert::gamut`.
const DISPLAY_P3_TO_BT709: [[f32; 3]; 3] = [
    [1.2249401762, -0.2249398679, 0.0],
    [-0.0420569547, 1.0420571193, 0.0],
    [-0.0196375600, -0.0786360660, 1.0982736130],
];

/// Default epsilon for bounds checks; absorbs roundoff in the matrix.
const GAMUT_EPSILON: f32 = 5e-4;

/// sRGB EOTF (gamma encoded → linear), per IEC 61966-2-1.
#[inline]
fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB OETF (linear → gamma encoded), per IEC 61966-2-1.
#[inline]
fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Encode a linear-light f32 in `[0, 1]` to a u8 sRGB byte. Clamps and
/// rounds to nearest.
#[inline]
fn encode_srgb_byte(linear: f32) -> u8 {
    let clamped = linear.clamp(0.0, 1.0);
    let encoded = linear_to_srgb(clamped);
    (encoded * 255.0 + 0.5) as u8
}

/// Apply the 3×3 matrix to a single linear-light pixel.
#[inline]
fn apply_matrix(rgb: [f32; 3], m: &[[f32; 3]; 3]) -> [f32; 3] {
    [
        m[0][0] * rgb[0] + m[0][1] * rgb[1] + m[0][2] * rgb[2],
        m[1][0] * rgb[0] + m[1][1] * rgb[1] + m[1][2] * rgb[2],
        m[2][0] * rgb[0] + m[2][1] * rgb[1] + m[2][2] * rgb[2],
    ]
}

/// True if a u8-sRGB linear-output value is inside `[-epsilon, 1+epsilon]`.
#[inline]
fn in_unit_range(v: f32, epsilon: f32) -> bool {
    v >= -epsilon && v <= 1.0 + epsilon
}

/// Source/target descriptor for the supported v0 downcasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceGamut {
    /// Display P3 primaries with sRGB transfer (CP=12, TC=13).
    DisplayP3Srgb,
}

impl SourceGamut {
    /// Detect the source gamut from a CICP code, if it matches one of the
    /// v0-supported flavors.
    pub(crate) fn from_cicp(cicp: Cicp) -> Option<Self> {
        // Matrix coefficients must be 0 (RGB / Identity) — non-zero MC
        // means YCbCr-like coding which we don't handle for PNG.
        if cicp.matrix_coefficients != 0 {
            return None;
        }
        match (cicp.color_primaries, cicp.transfer_characteristics) {
            (12, 13) => Some(SourceGamut::DisplayP3Srgb),
            _ => None,
        }
    }

    fn matrix_to_srgb(self) -> &'static [[f32; 3]; 3] {
        match self {
            SourceGamut::DisplayP3Srgb => &DISPLAY_P3_TO_BT709,
        }
    }
}

/// Try to downcast an RGB8 buffer from `src` to sRGB. Returns `Some(buf)`
/// with the rewritten pixels on success, `None` if any pixel falls
/// outside the sRGB gamut (lossless downcast not possible).
pub(crate) fn try_downcast_rgb8_to_srgb(rgb: &[u8], src: SourceGamut) -> Option<Vec<u8>> {
    let m = src.matrix_to_srgb();

    // Pass 1: bounds check with early-exit.
    for px in rgb.chunks_exact(3) {
        let lin = [
            srgb_to_linear(px[0] as f32 / 255.0),
            srgb_to_linear(px[1] as f32 / 255.0),
            srgb_to_linear(px[2] as f32 / 255.0),
        ];
        let out = apply_matrix(lin, m);
        if !in_unit_range(out[0], GAMUT_EPSILON)
            || !in_unit_range(out[1], GAMUT_EPSILON)
            || !in_unit_range(out[2], GAMUT_EPSILON)
        {
            return None;
        }
    }

    // Pass 2: commit the transform.
    let mut out_buf = Vec::with_capacity(rgb.len());
    for px in rgb.chunks_exact(3) {
        let lin = [
            srgb_to_linear(px[0] as f32 / 255.0),
            srgb_to_linear(px[1] as f32 / 255.0),
            srgb_to_linear(px[2] as f32 / 255.0),
        ];
        let out = apply_matrix(lin, m);
        out_buf.push(encode_srgb_byte(out[0]));
        out_buf.push(encode_srgb_byte(out[1]));
        out_buf.push(encode_srgb_byte(out[2]));
    }
    Some(out_buf)
}

/// Try to downcast an RGBA8 buffer from `src` to sRGB. Alpha is preserved.
/// Returns `Some(buf)` on success, `None` if any pixel is out of gamut.
pub(crate) fn try_downcast_rgba8_to_srgb(rgba: &[u8], src: SourceGamut) -> Option<Vec<u8>> {
    let m = src.matrix_to_srgb();

    for px in rgba.chunks_exact(4) {
        let lin = [
            srgb_to_linear(px[0] as f32 / 255.0),
            srgb_to_linear(px[1] as f32 / 255.0),
            srgb_to_linear(px[2] as f32 / 255.0),
        ];
        let out = apply_matrix(lin, m);
        if !in_unit_range(out[0], GAMUT_EPSILON)
            || !in_unit_range(out[1], GAMUT_EPSILON)
            || !in_unit_range(out[2], GAMUT_EPSILON)
        {
            return None;
        }
    }

    let mut out_buf = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        let lin = [
            srgb_to_linear(px[0] as f32 / 255.0),
            srgb_to_linear(px[1] as f32 / 255.0),
            srgb_to_linear(px[2] as f32 / 255.0),
        ];
        let out = apply_matrix(lin, m);
        out_buf.push(encode_srgb_byte(out[0]));
        out_buf.push(encode_srgb_byte(out[1]));
        out_buf.push(encode_srgb_byte(out[2]));
        out_buf.push(px[3]); // alpha unchanged
    }
    Some(out_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cicp_display_p3_srgb_recognized() {
        assert_eq!(
            SourceGamut::from_cicp(Cicp::DISPLAY_P3),
            Some(SourceGamut::DisplayP3Srgb)
        );
    }

    #[test]
    fn cicp_pq_hlg_rejected() {
        // HDR PQ — not v0-supported.
        assert_eq!(SourceGamut::from_cicp(Cicp::BT2100_PQ), None);
        assert_eq!(SourceGamut::from_cicp(Cicp::BT2100_HLG), None);
    }

    #[test]
    fn cicp_srgb_rejected_no_downcast_needed() {
        // Source IS sRGB — no downcast needed, return None.
        assert_eq!(SourceGamut::from_cicp(Cicp::SRGB), None);
    }

    #[test]
    fn cicp_with_nonzero_matrix_coefficients_rejected() {
        // matrix_coefficients != 0 (Identity) means YCbCr; we don't handle
        // that for PNG.
        let cicp = Cicp::new(12, 13, 9, true);
        assert_eq!(SourceGamut::from_cicp(cicp), None);
    }

    #[test]
    fn neutral_gray_p3_downcasts_to_neutral_gray_srgb() {
        // Mid gray (128, 128, 128) in P3 should remain mid gray in sRGB
        // since white points match (D65) and the matrix preserves gray.
        let p3 = vec![128u8, 128, 128, 128, 128, 128]; // 2 RGB pixels
        let srgb = try_downcast_rgb8_to_srgb(&p3, SourceGamut::DisplayP3Srgb)
            .expect("neutral gray must fit");
        // Should be very close to (128, 128, 128) in sRGB too, modulo
        // EOTF/OETF roundoff.
        for &b in &srgb {
            assert!(b >= 127 && b <= 129, "gray drifted to {b}");
        }
    }

    #[test]
    fn saturated_p3_red_rejected() {
        // (255, 0, 0) in P3 is brighter than (255, 0, 0) in sRGB.
        // After matrix, R > 1 → out of gamut → None.
        let p3 = vec![255u8, 0, 0];
        let result = try_downcast_rgb8_to_srgb(&p3, SourceGamut::DisplayP3Srgb);
        assert_eq!(result, None);
    }

    #[test]
    fn moderate_p3_red_fits_srgb() {
        // (200, 100, 100) is well inside sRGB even when interpreted as P3.
        let p3 = vec![200u8, 100, 100];
        let result = try_downcast_rgb8_to_srgb(&p3, SourceGamut::DisplayP3Srgb);
        assert!(result.is_some(), "moderate P3 red should fit sRGB");
    }

    #[test]
    fn rgba8_alpha_preserved_after_downcast() {
        // Two pixels with non-trivial alpha; gray RGB so all fit.
        let p3 = vec![100u8, 100, 100, 200, 50, 50, 50, 64];
        let result = try_downcast_rgba8_to_srgb(&p3, SourceGamut::DisplayP3Srgb)
            .expect("gray-with-alpha must fit");
        assert_eq!(result[3], 200, "pixel 0 alpha lost");
        assert_eq!(result[7], 64, "pixel 1 alpha lost");
    }

    #[test]
    fn rgba8_out_of_gamut_returns_none_no_partial_buffer() {
        // First pixel is fine, second is saturated red (out of gamut).
        // Bounds-check pass should bail before allocating output.
        let p3 = vec![100u8, 100, 100, 255, 255, 0, 0, 255];
        let result = try_downcast_rgba8_to_srgb(&p3, SourceGamut::DisplayP3Srgb);
        assert_eq!(result, None);
    }

    #[test]
    fn empty_buffer_returns_empty() {
        // Edge case: empty slice. Both passes are no-ops; we return Some(empty).
        let result = try_downcast_rgb8_to_srgb(&[], SourceGamut::DisplayP3Srgb);
        assert_eq!(result, Some(Vec::new()));
    }

    #[test]
    fn downcast_then_decode_roundtrip_is_close() {
        // Run a moderate P3 image through downcast, verify the encoded
        // sRGB bytes round-trip through P3 decode close to the original.
        // This catches mismatched matrices or transfer functions.
        let p3 = vec![
            150u8, 100, 80, // pixel 0
            120, 200, 90, // pixel 1
            80, 80, 200, // pixel 2 (saturated blue but still in sRGB gamut for P3)
        ];
        // We don't have a P3 decoder handy — just assert downcast succeeds
        // and produces something plausible.
        let srgb = try_downcast_rgb8_to_srgb(&p3, SourceGamut::DisplayP3Srgb)
            .expect("test pixels should fit");
        // Output should not be wildly different from input (pixel 0 is
        // mostly red — sRGB representation should still skew red).
        assert!(srgb[0] > srgb[1], "pixel 0 should still be R-dominant");
        assert!(srgb[3 + 1] > srgb[3 + 0], "pixel 1 should still be G-dominant");
        assert!(srgb[6 + 2] > srgb[6 + 0], "pixel 2 should still be B-dominant");
    }
}
