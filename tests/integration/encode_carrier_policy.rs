//! Forbidding the ICC profile (`embed_icc: false`) never relabels pixels. PNG
//! must then refuse, unless a carrier that remains (a cICP from the metadata
//! or the native builder) says what the profile said. One-shot, row and
//! animation encodes; every output is read back with the png crate.
use std::io::Cursor;

use zencodec::encode::{AnimationFrameEncoder, EncodeJob, EncodePolicy, Encoder, EncoderConfig};
use zencodec::{At, Cicp, CodecError, ErrorCategory, Metadata, MetadataPolicy, PolicyKind};
use zenpixels::{PixelDescriptor, PixelSlice};
use zenpixels_convert::icc_profiles::{DISPLAY_P3_V4, REC2020_V4};
use zenpng::{DowncastFlags, PngEncoderConfig};

const W: u32 = 7;
const H: u32 = 5;
const P3_CICP: Cicp = Cicp::new(12, 13, 0, true);

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
    assert_eq!(
        zenpixels::icc::profile_color_space(&profile),
        Some(zenpixels::ColorModel::Rgb)
    );
    profile
}

#[derive(Clone, Copy, Debug)]
enum Route {
    OneShot,
    Rows,
    Animation,
}

/// Encodes under `embed_icc: false`.
fn encode(route: Route, meta: Metadata, native: Option<Cicp>) -> Result<Vec<u8>, At<CodecError>> {
    encode_with(
        route,
        meta,
        native,
        Some(EncodePolicy::none().with_embed_icc(false)),
    )
}

fn encode_with(
    route: Route,
    meta: Metadata,
    native: Option<Cicp>,
    policy: Option<EncodePolicy>,
) -> Result<Vec<u8>, At<CodecError>> {
    let data = samples();
    let pixels =
        || PixelSlice::new(&data, W, H, W as usize * 4, PixelDescriptor::RGBA8_SRGB).unwrap();
    let mut job = PngEncoderConfig::new()
        .with_downcast(DowncastFlags::none())
        .with_cicp(native)
        .job()
        .with_metadata_policy(meta, MetadataPolicy::PreserveExact)
        .with_canvas_size(W, H);
    // The checked setter does not admit embed_icc yet; this is the legacy
    // setter's effect, which the checked one will reuse.
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
            encoder.begin(W, H, PixelDescriptor::RGBA8_SRGB)?;
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

/// iCCP, cICP (primaries, transfer, matrix, full range) and the samples.
type Read = (Option<Vec<u8>>, Option<(u8, u8, u8, bool)>, Vec<u8>);

/// The colour chunks and samples as the png crate reads them.
fn read(file: &[u8]) -> Read {
    let mut decoder = png::Decoder::new(Cursor::new(file));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().unwrap();
    let info = reader.info();
    let icc = info.icc_profile.as_ref().map(|icc| icc.to_vec());
    let cicp = info.coding_independent_code_points.map(|c| {
        (
            c.color_primaries,
            c.transfer_function,
            c.matrix_coefficients,
            c.is_video_full_range_image,
        )
    });
    let mut samples = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut samples).unwrap();
    samples.truncate(frame.buffer_size());
    (icc, cicp, samples)
}

#[test]
fn a_forbidden_profile_with_no_equivalent_carrier_refuses() {
    for route in [Route::OneShot, Route::Rows, Route::Animation] {
        for (name, profile) in [
            ("Display P3", DISPLAY_P3_V4.to_vec()),
            ("Rec. 2020", REC2020_V4.to_vec()),
            ("unidentified", unidentified()),
        ] {
            let result = encode(route, Metadata::none().with_icc(profile), None);
            let error = match result {
                Ok(file) => panic!(
                    "{route:?}, {name}: wrote a file whose colour chunks are {:?}",
                    {
                        let (icc, cicp, _) = read(&file);
                        (icc.map(|p| p.len()), cicp)
                    }
                ),
                Err(error) => error,
            };
            assert_eq!(
                error.error().category(),
                ErrorCategory::Policy(PolicyKind::Encode),
                "{route:?}, {name}: {error}"
            );
        }
    }
}

#[test]
fn a_remaining_cicp_with_the_same_meaning_carries_the_colour() {
    for route in [Route::OneShot, Route::Rows, Route::Animation] {
        for (name, meta, native) in [
            (
                "metadata cICP",
                Metadata::none().with_icc(DISPLAY_P3_V4).with_cicp(P3_CICP),
                None,
            ),
            (
                "native cICP",
                Metadata::none().with_icc(DISPLAY_P3_V4),
                Some(P3_CICP),
            ),
        ] {
            let file = encode(route, meta, native)
                .unwrap_or_else(|error| panic!("{route:?}, {name}: {error}"));
            let (icc, cicp, decoded) = read(&file);
            assert!(icc.is_none(), "{route:?}, {name}: the policy forbids iCCP");
            assert_eq!(cicp, Some((12, 13, 0, true)), "{route:?}, {name}");
            assert_eq!(decoded, samples(), "{route:?}, {name}: samples");
        }
    }
}

#[test]
fn nothing_to_forbid_changes_nothing() {
    for route in [Route::OneShot, Route::Rows, Route::Animation] {
        let forbidden = encode(route, Metadata::none(), None)
            .unwrap_or_else(|error| panic!("{route:?}: {error}"));
        let unpoliced = encode_with(route, Metadata::none(), None, None).unwrap();
        assert_eq!(forbidden, unpoliced, "{route:?}");
        assert!(read(&forbidden).0.is_none(), "{route:?}");
    }
}
