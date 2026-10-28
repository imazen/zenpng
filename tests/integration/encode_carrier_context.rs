//! A colour context is the pixels' own, authoritative description. Forbidding
//! the ICC profile (`embed_icc: false`) never drops a context profile: PNG
//! refuses unless a remaining cICP says the same, whether the profile is
//! alone or repeated as job metadata, on every encoder (one-shot, declared
//! rows, animation frames). A refused row encode has written nothing. Job
//! metadata gets the same refusal (`encode_carrier_policy`); there is no
//! best-effort exception for the legacy setter.
use std::io::Cursor;
use std::sync::Arc;

use zencodec::encode::{
    AnimationFrameEncoder, EncodeDeclaration, EncodeJob, EncodePolicy, Encoder, EncoderConfig,
};
use zencodec::{At, CodecError, ErrorCategory, Metadata, MetadataPolicy, PolicyKind};
use zenpixels::{Cicp, ColorContext, ColorModel, PixelDescriptor, PixelSlice};
use zenpixels_convert::icc_profiles::DISPLAY_P3_V4;
use zenpng::{DowncastFlags, PngEncoderConfig};

const W: u32 = 7;
const H: u32 = 5;

/// Opaque RGBA (the animation encoder's frame format; still encodes keep it
/// with downcasting off).
fn samples() -> Vec<u8> {
    (0..W * H * 4)
        .map(|i| {
            if i % 4 == 3 {
                255
            } else {
                (i * 37 % 251) as u8
            }
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
enum Route {
    OneShot,
    Rows,
    Animation,
}

const ROUTES: [Route; 3] = [Route::OneShot, Route::Rows, Route::Animation];

/// `policy` goes through the legacy setter (the checked one does not admit
/// `embed_icc` yet).
fn encode(
    route: Route,
    meta: Metadata,
    context: Option<Arc<ColorContext>>,
    policy: Option<EncodePolicy>,
) -> Result<Vec<u8>, At<CodecError>> {
    let data = samples();
    let pixels = || {
        let pixels =
            PixelSlice::new(&data, W, H, W as usize * 4, PixelDescriptor::RGBA8_SRGB).unwrap();
        match &context {
            Some(context) => pixels.with_color_context(Arc::clone(context)),
            None => pixels,
        }
    };
    let mut job = PngEncoderConfig::new()
        .with_downcast(DowncastFlags::none())
        .job()
        .with_metadata_policy(meta, MetadataPolicy::PreserveExact)
        .with_canvas_size(W, H);
    if let Some(policy) = policy {
        #[allow(deprecated)]
        {
            job = job.with_policy(policy);
        }
    }
    Ok(match route {
        Route::OneShot => job.encoder()?.encode(pixels())?.into_vec(),
        Route::Rows => {
            let mut encoder = job.encoder()?;
            let mut declaration = EncodeDeclaration::new(W, H, PixelDescriptor::RGBA8_SRGB);
            if let Some(context) = &context {
                declaration = declaration.with_color(context);
            }
            if let Err(error) = encoder.begin_declared(&declaration) {
                assert!(
                    encoder.pending_output().is_empty(),
                    "a refused row encode wrote bytes"
                );
                return Err(error);
            }
            encoder.push_rows(pixels())?;
            encoder.finish()?.into_vec()
        }
        Route::Animation => {
            let mut encoder = job.animation_frame_encoder()?;
            encoder.push_frame(pixels(), 20, None)?;
            encoder.finish(None)?.into_vec()
        }
    })
}

fn forbid() -> Option<EncodePolicy> {
    Some(EncodePolicy::none().with_embed_icc(false))
}

/// iCCP and cICP (primaries, transfer, matrix, full range).
type Chunks = (Option<Vec<u8>>, Option<(u8, u8, u8, bool)>);

/// The colour chunks as the png crate reads them.
fn chunks(file: &[u8]) -> Chunks {
    let reader = png::Decoder::new(Cursor::new(file)).read_info().unwrap();
    let info = reader.info();
    (
        info.icc_profile.as_ref().map(|icc| icc.to_vec()),
        info.coding_independent_code_points.map(|c| {
            (
                c.color_primaries,
                c.transfer_function,
                c.matrix_coefficients,
                c.is_video_full_range_image,
            )
        }),
    )
}

/// An RGB profile zenpixels cannot identify (identification hashes the bytes
/// after the header): Display P3 with its last byte changed.
fn unidentified() -> Vec<u8> {
    let mut profile = DISPLAY_P3_V4.to_vec();
    let last = profile.len() - 1;
    profile[last] ^= 1;
    assert!(
        zenpixels::ColorProfileSource::Icc(&profile)
            .resolve()
            .is_none()
    );
    profile
}

fn with(profile: &[u8], at: usize, bytes: &[u8]) -> Vec<u8> {
    let mut out = profile.to_vec();
    out[at..at + bytes.len()].copy_from_slice(bytes);
    out
}

/// Context profiles: Display P3, an unidentified RGB profile, and bytes lcms
/// 2.17 refused to open as a profile (zenjpeg
/// `benchmarks/icc_fixture_lcms_probe_2026-10-28.txt`), which zenpixels does
/// not identify either.
fn context_profiles() -> Vec<(&'static str, Vec<u8>)> {
    let p3 = DISPLAY_P3_V4;
    let truncated = p3[..100].to_vec();
    // The header's colour-space field alone does not make a profile.
    assert_eq!(
        zenpixels::icc::profile_color_space(&truncated),
        Some(ColorModel::Rgb)
    );
    vec![
        ("Display P3", p3.to_vec()),
        ("an unidentified profile", unidentified()),
        (
            "21 bytes lcms 2.17 rejects",
            b"fake icc profile data".to_vec(),
        ),
        ("a truncated header lcms 2.17 rejects", truncated),
        ("no acsp signature", with(p3, 36, b"xxxx")),
        (
            "a tag table past the end",
            with(p3, 128, &30u32.to_be_bytes()),
        ),
    ]
}

#[test]
fn forbidding_a_colour_context_profile_refuses() {
    for route in ROUTES {
        for (what, profile) in context_profiles() {
            let context = Some(Arc::new(ColorContext::from_icc(profile.clone())));
            let alone = encode(route, Metadata::none(), context.clone(), forbid());
            assert!(alone.is_err(), "{route:?}, {what}: dropped the context");
            let repeated = Metadata::none().with_icc(profile.clone());
            let both = encode(route, repeated, context, forbid());
            assert!(
                both.is_err(),
                "{route:?}, {what}: dropped the context repeated as metadata"
            );
        }
        // Complete RGB profiles reach the carrier decision: policy refusals.
        for profile in [DISPLAY_P3_V4.to_vec(), unidentified()] {
            let context = Some(Arc::new(ColorContext::from_icc(profile)));
            let error = encode(route, Metadata::none(), context, forbid()).unwrap_err();
            assert_eq!(
                error.error().category(),
                ErrorCategory::Policy(PolicyKind::Encode),
                "{route:?}: {error}"
            );
        }
    }
}

#[test]
fn a_context_cicp_carries_the_colour_when_the_profile_is_forbidden() {
    for route in ROUTES {
        let context = Some(Arc::new(ColorContext::from_cicp(Cicp::DISPLAY_P3)));
        let file = encode(route, Metadata::none(), context, forbid())
            .unwrap_or_else(|error| panic!("{route:?}: {error}"));
        assert_eq!(chunks(&file), (None, Some((12, 13, 0, true))), "{route:?}");
    }
}
