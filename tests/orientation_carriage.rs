//! A field-only orientation (`Metadata::orientation` with no EXIF blob, as
//! formats whose orientation is a container property and callers produce)
//! must survive PNG encoding: PNG carries orientation only in eXIf, so the
//! encoder writes a minimal EXIF holding the tag. A policy that forbids EXIF
//! refuses before any output instead of displaying the pixels re-oriented.
use std::borrow::Cow;
use std::io::Cursor;

use zencodec::decode::{Decode, DecodeJob, DecoderConfig};
use zencodec::encode::{EncodeJob, EncodePolicy, Encoder, EncoderConfig};
use zencodec::{At, CodecError, ErrorCategory, Metadata, MetadataPolicy, Orientation, PolicyKind};
use zenpixels::{PixelDescriptor, PixelSlice};
use zenpng::{PngDecoderConfig, PngEncoderConfig};

const W: u32 = 9;
const H: u32 = 5;

fn encode(meta: Metadata, policy: Option<EncodePolicy>) -> Result<Vec<u8>, At<CodecError>> {
    let data: Vec<u8> = (0..W * H * 3).map(|i| (i * 17 % 251) as u8).collect();
    let pixels = PixelSlice::new(&data, W, H, W as usize * 3, PixelDescriptor::RGB8_SRGB).unwrap();
    let mut job = PngEncoderConfig::new()
        .job()
        .with_metadata_policy(meta, MetadataPolicy::PreserveExact);
    if let Some(policy) = policy {
        // zenpng's checked setter qualifies colour only; EXIF embedding is
        // still configured through the legacy setter.
        #[allow(deprecated)]
        {
            job = job.with_policy(policy);
        }
    }
    Ok(job.encoder()?.encode(pixels)?.into_vec())
}

/// The eXIf payload as an independent reader (the png crate) sees it.
fn exif_chunk(png: &[u8]) -> Option<Vec<u8>> {
    let reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
    reader.info().exif_metadata.as_deref().map(<[u8]>::to_vec)
}

/// Orientation tag of IFD0 in a TIFF blob, read here rather than through any
/// zen parser.
fn tiff_orientation(tiff: &[u8]) -> Option<u16> {
    let le = match tiff.get(..4)? {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let u16_at = |at: usize| {
        let b: [u8; 2] = tiff.get(at..at + 2)?.try_into().ok()?;
        Some(if le {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    };
    let u32_at = |at: usize| {
        let b: [u8; 4] = tiff.get(at..at + 4)?.try_into().ok()?;
        Some(if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    };
    let ifd = u32_at(4)? as usize;
    (0..u16_at(ifd)? as usize).find_map(|i| {
        let entry = ifd + 2 + i * 12;
        // SHORT (type 3), count 1: the value sits in the first two value bytes.
        (u16_at(entry)? == 0x0112 && u16_at(entry + 2)? == 3 && u32_at(entry + 4)? == 1)
            .then(|| u16_at(entry + 8))
            .flatten()
    })
}

fn decoded_orientation(png: &[u8]) -> Orientation {
    PngDecoderConfig::new()
        .job()
        .decoder(Cow::Borrowed(png), &[])
        .unwrap()
        .decode()
        .unwrap()
        .info()
        .orientation
}

#[test]
fn a_field_only_orientation_is_written_as_exif() {
    for value in 2..=8u8 {
        let orientation = Orientation::from_exif(value).unwrap();
        let png = encode(Metadata::none().with_orientation(orientation), None).unwrap();
        let exif = exif_chunk(&png).unwrap_or_else(|| panic!("{orientation:?}: no eXIf chunk"));
        assert_eq!(
            tiff_orientation(&exif),
            Some(u16::from(value)),
            "{orientation:?}"
        );
        assert_eq!(decoded_orientation(&png), orientation);
    }
}

#[test]
fn identity_without_exif_writes_no_exif() {
    let png = encode(Metadata::none(), None).unwrap();
    assert_eq!(exif_chunk(&png), None);
    assert_eq!(decoded_orientation(&png), Orientation::Identity);
}

#[test]
fn a_policy_forbidding_exif_refuses_an_orientation_it_would_drop() {
    let no_exif = EncodePolicy::none().with_embed_exif(false);
    let error = encode(
        Metadata::none().with_orientation(Orientation::Rotate90),
        Some(no_exif),
    )
    .unwrap_err();
    assert_eq!(
        error.error().category(),
        ErrorCategory::Policy(PolicyKind::Encode),
        "{error:?}"
    );
    // Identity needs no carrier, so the same policy still encodes.
    let png = encode(Metadata::none(), Some(no_exif)).unwrap();
    assert_eq!(exif_chunk(&png), None);
}
