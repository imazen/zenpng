//! Gray pixels with CICP metadata must leave with their color, in a profile a gray
//! PNG may carry.
//!
//! PNG requires a greyscale profile for color types 0 and 4. CICP is suppressed for
//! gray by the color plan, so a missing profile leaves the file undescribed, and
//! readers take it as sRGB. Every output is read back with image-png (the `png`
//! crate), an independent reader, for the color type, samples and color chunks.

use std::io::Cursor;

use zencodec::encode::{EncodeJob as _, EncodePolicy, Encoder as _, EncoderConfig as _};
use zencodec::{Cicp, ColorEmitPolicy, Metadata, MetadataPolicy};
use zenpixels::{ColorModel, PixelDescriptor, PixelSlice};
use zenpng::PngEncoderConfig;

const W: u32 = 5;
const H: u32 = 3;
/// Whole padding pixels, for the strided source rows.
const PADDING_PIXELS: usize = 2;

const GRAY_PQ: Cicp = Cicp::new(1, 16, 0, true);
const GRAY_2_2: Cicp = Cicp::new(1, 4, 0, true);
const GRAY_HLG: Cicp = Cicp::new(9, 18, 0, true);
const GRAY_BT709: Cicp = Cicp::new(1, 1, 0, true);

#[derive(Clone, Copy, Debug)]
struct Format {
    descriptor: PixelDescriptor,
    /// IHDR color type.
    color_type: png::ColorType,
    bytes_per_pixel: usize,
}

const FORMATS: [Format; 4] = [
    Format {
        descriptor: PixelDescriptor::GRAY8_SRGB,
        color_type: png::ColorType::Grayscale,
        bytes_per_pixel: 1,
    },
    Format {
        descriptor: PixelDescriptor::GRAY16_SRGB,
        color_type: png::ColorType::Grayscale,
        bytes_per_pixel: 2,
    },
    Format {
        descriptor: PixelDescriptor::GRAYA8_SRGB,
        color_type: png::ColorType::GrayscaleAlpha,
        bytes_per_pixel: 2,
    },
    Format {
        descriptor: PixelDescriptor::GRAYA16_SRGB,
        color_type: png::ColorType::GrayscaleAlpha,
        bytes_per_pixel: 4,
    },
];

/// Strided rows of varied samples, in native endianness for 16-bit formats.
fn rows(format: Format) -> (Vec<u8>, usize) {
    let stride = (W as usize + PADDING_PIXELS) * format.bytes_per_pixel;
    let mut data = vec![0xa5; stride * H as usize];
    let wide = format.bytes_per_pixel == 2 && format.color_type == png::ColorType::Grayscale
        || format.bytes_per_pixel == 4;
    for y in 0..H as usize {
        for byte in 0..W as usize * format.bytes_per_pixel {
            data[y * stride + byte] = (y * 40 + byte * 17 + 11) as u8;
        }
    }
    // Keep 16-bit values from being reducible to 8 bits, so depth stays observable.
    if wide {
        for y in 0..H as usize {
            data[y * stride] = 0x12;
            data[y * stride + 1] = 0x34;
        }
    }
    (data, stride)
}

#[allow(deprecated)] // Exercises the published best-effort job setters.
fn encode(format: Format, cicp: Cicp, policy: Option<EncodePolicy>) -> Result<Vec<u8>, String> {
    let (data, stride) = rows(format);
    let pixels = PixelSlice::new(&data, W, H, stride, format.descriptor).unwrap();
    let mut job = PngEncoderConfig::new().job().with_metadata_policy(
        Metadata::none().with_cicp(cicp),
        MetadataPolicy::PreserveExact,
    );
    if let Some(policy) = policy {
        job = job.with_policy(policy);
    }
    let encoder = job.encoder().map_err(|e| e.to_string())?;
    encoder
        .encode(pixels)
        .map(|out| out.into_vec())
        .map_err(|e| e.to_string())
}

struct Read {
    color_type: png::ColorType,
    icc: Option<Vec<u8>>,
    has_cicp: bool,
}

fn read(file: &[u8]) -> Read {
    let reader = png::Decoder::new(Cursor::new(file)).read_info().unwrap();
    let info = reader.info();
    Read {
        color_type: info.color_type,
        icc: info.icc_profile.as_ref().map(|icc| icc.to_vec()),
        has_cicp: info.coding_independent_code_points.is_some(),
    }
}

fn balanced_policies() -> [Option<EncodePolicy>; 4] {
    [
        None,
        Some(EncodePolicy::none().with_color(ColorEmitPolicy::Balanced)),
        Some(EncodePolicy::none().with_color(ColorEmitPolicy::Compatibility)),
        Some(EncodePolicy::none().with_color(ColorEmitPolicy::Compact)),
    ]
}

#[test]
fn non_default_gray_gets_a_gray_class_profile_on_every_format() {
    for format in FORMATS {
        for cicp in [GRAY_PQ, GRAY_2_2, GRAY_HLG, GRAY_BT709] {
            for policy in balanced_policies() {
                let what = format!("{format:?} {cicp:?} {policy:?}");
                let file = encode(format, cicp, policy).unwrap_or_else(|e| panic!("{what}: {e}"));
                let read = read(&file);
                assert_eq!(read.color_type, format.color_type, "{what}");
                let icc = read.icc.unwrap_or_else(|| panic!("{what}: no iCCP"));
                assert_eq!(
                    ColorModel::from_icc(&icc),
                    Some(ColorModel::Gray),
                    "{what}: profile class"
                );
            }
        }
    }
}

#[test]
fn different_transfers_get_different_profiles() {
    let format = FORMATS[0];
    let profile = |cicp| read(&encode(format, cicp, None).unwrap()).icc.unwrap();
    assert_ne!(profile(GRAY_PQ), profile(GRAY_2_2));
    assert_ne!(profile(GRAY_PQ), profile(GRAY_HLG));
    assert_ne!(profile(GRAY_2_2), profile(GRAY_BT709));
}

#[test]
fn a_re_encode_of_the_output_keeps_its_profile() {
    let format = FORMATS[0];
    let first = encode(format, GRAY_PQ, None).unwrap();
    let profile = read(&first).icc.unwrap();
    let decoded = zenpng::decode(
        &first,
        &zenpng::PngDecodeConfig::none(),
        &enough::Unstoppable,
    )
    .unwrap();
    assert_eq!(
        decoded.info.icc_profile.as_deref(),
        Some(profile.as_slice())
    );
    #[allow(deprecated)] // Exercises the published best-effort job setter.
    let second = PngEncoderConfig::new()
        .job()
        .with_metadata_policy(
            Metadata::none().with_icc(profile.clone()),
            MetadataPolicy::PreserveExact,
        )
        .encoder()
        .unwrap()
        .encode(decoded.pixels.as_slice())
        .unwrap()
        .into_vec();
    assert_eq!(read(&second).icc.as_deref(), Some(profile.as_slice()));
}

#[test]
fn srgb_gray_stays_untagged() {
    for format in FORMATS {
        for cicp in [Cicp::new(1, 13, 0, true), Cicp::new(2, 2, 0, true)] {
            let read = read(&encode(format, cicp, None).unwrap());
            assert_eq!(read.icc, None, "{format:?} {cicp:?}");
            assert!(!read.has_cicp, "{format:?} {cicp:?}");
        }
    }
}

/// With the profile forbidden and CICP suppressed, nothing would describe these
/// samples. That refuses, as a forbidden profile with no equal carrier does.
#[test]
fn forbidding_the_only_carrier_refuses_instead_of_relabeling() {
    let policy = || Some(EncodePolicy::none().with_embed_icc(false));
    for format in FORMATS {
        for cicp in [GRAY_PQ, GRAY_2_2, GRAY_HLG, GRAY_BT709] {
            assert!(
                encode(format, cicp, policy()).is_err(),
                "{format:?} {cicp:?} wrote gray pixels that read as sRGB"
            );
        }
        // The sRGB default means the same with no carrier at all.
        assert!(encode(format, Cicp::new(1, 13, 0, true), policy()).is_ok());
    }
}

/// A palette holds RGB colors: reading one as gray would plan a gray profile, or
/// suppress its cICP, on a later encode of the same image.
#[test]
fn palette_source_does_not_report_as_gray() {
    use zencodec::decode::{DecodeJob as _, DecoderConfig as _};
    for (trns, want) in [(false, 3), (true, 4)] {
        let mut file = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut file, 2, 1);
            encoder.set_color(png::ColorType::Indexed);
            encoder.set_palette(vec![10, 20, 30, 200, 100, 50]);
            if trns {
                encoder.set_trns(vec![128]);
            }
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0, 1]).unwrap();
        }
        let info = zenpng::PngDecoderConfig::new().job().probe(&file).unwrap();
        assert_eq!(info.source_color.channel_count, Some(want), "trns={trns}");
    }
}
