//! The colour chunks the zencodec adapter writes must describe the samples
//! the file stores, taken from the current pixel interpretation (descriptor
//! plus `PixelSlice` colour context) when the caller supplied no colour
//! metadata. The checked `EncodeRequest` path supplies none. One-shot and row
//! encodes of the same pixels must agree. Linear input is stored as 8-bit
//! sRGB, so a description of the linear samples no longer applies.
//!
//! Every output is read back with image-png (the `png` crate), an independent
//! reader, for both the samples and the cICP/iCCP chunks.

use std::io::Cursor;
use std::sync::Arc;

use zencodec::encode::{EncodeJob as _, Encoder as _, EncoderConfig as _};
use zencodec::operation::EncodeRequest;
use zencodec::{CodecSet, ImageFormat, Metadata, MetadataPolicy};
use zenpixels::{Cicp, ColorContext, ColorPrimaries, PixelDescriptor, PixelSlice};
use zenpixels_convert::icc_profiles::DISPLAY_P3_V4;
use zenpng::{Compression, PngEncoderConfig};

const W: u32 = 24;
const H: u32 = 20;

fn colour_at(x: u32, y: u32) -> [u8; 3] {
    [(x * 9 + 7) as u8, (y * 11 + 3) as u8, ((x + y) * 5) as u8]
}

/// A strided raster with whole padding pixels and an aligned first sample.
struct Raster {
    buf: Vec<u8>,
    start: usize,
    stride: usize,
}

impl Raster {
    fn new(bytes_per_pixel: usize, padding_pixels: usize) -> Self {
        let stride = (W as usize + padding_pixels) * bytes_per_pixel;
        let buf = vec![0xa5; stride * H as usize + 16];
        let start = buf.as_ptr().align_offset(16);
        Self { buf, start, stride }
    }

    fn row(&mut self, y: u32) -> &mut [u8] {
        let at = self.start + y as usize * self.stride;
        &mut self.buf[at..at + self.stride]
    }

    fn slice(&self, descriptor: PixelDescriptor) -> PixelSlice<'_> {
        PixelSlice::new(&self.buf[self.start..], W, H, self.stride, descriptor).unwrap()
    }
}

fn rgb8() -> Raster {
    let mut raster = Raster::new(3, 2);
    for y in 0..H {
        for x in 0..W {
            raster.row(y)[x as usize * 3..][..3].copy_from_slice(&colour_at(x, y));
        }
    }
    raster
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear f32 RGB whose 8-bit sRGB encoding is the patch image.
fn rgbf32_linear() -> Raster {
    let mut raster = Raster::new(12, 1);
    for y in 0..H {
        for x in 0..W {
            for (c, v) in colour_at(x, y).into_iter().enumerate() {
                let linear = srgb_to_linear(f32::from(v) / 255.0).to_ne_bytes();
                raster.row(y)[x as usize * 12 + c * 4..][..4].copy_from_slice(&linear);
            }
        }
    }
    raster
}

fn p3(base: PixelDescriptor) -> PixelDescriptor {
    base.with_primaries(ColorPrimaries::DisplayP3)
}

struct Read {
    samples: Vec<u8>,
    icc: Option<Vec<u8>>,
    cicp: Option<(u8, u8, u8, bool)>,
}

/// Decode with the independent reader.
fn oracle(file: &[u8]) -> Read {
    let mut reader = png::Decoder::new(Cursor::new(file))
        .read_info()
        .expect("png header");
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
    assert_eq!((info.width, info.height), (W, H));
    let mut samples = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut samples).expect("png samples");
    samples.truncate(frame.buffer_size());
    Read { samples, icc, cicp }
}

/// Every sample decodes exactly (PNG is lossless) or, for linear input
/// stored as 8-bit sRGB, within one code value.
fn assert_samples(read: &Read, tolerance: u8, what: &str) {
    for y in 0..H {
        for x in 0..W {
            let at = (y * W + x) as usize * 3;
            let got = &read.samples[at..at + 3];
            let want = colour_at(x, y);
            for c in 0..3 {
                assert!(
                    got[c].abs_diff(want[c]) <= tolerance,
                    "{what}: ({x},{y}) decoded {got:?}, stored samples are {want:?}"
                );
            }
        }
    }
}

fn checked(pixels: PixelSlice<'_>) -> Result<Vec<u8>, String> {
    let codecs = CodecSet::new().with_encoder(PngEncoderConfig::new());
    EncodeRequest::new(ImageFormat::Png)
        .encode(&codecs, pixels)
        .map(|outcome| outcome.into_output().into_vec())
        .map_err(|error| format!("{error:?}"))
}

#[allow(deprecated)] // Exercises the published best-effort job setters.
fn with_metadata(meta: Metadata, pixels: PixelSlice<'_>) -> Result<Vec<u8>, String> {
    PngEncoderConfig::new()
        .job()
        .with_metadata_policy(meta, MetadataPolicy::PreserveExact)
        .encoder()
        .and_then(|encoder| encoder.encode(pixels))
        .map(|output| output.into_vec())
        .map_err(|error| format!("{error:?}"))
}

const P3: (u8, u8, u8, bool) = (12, 13, 0, true);

/// Issue 154: the checked request carries no metadata, so the pixels' own
/// colour must reach the file. The context ICC is carried verbatim; a
/// context CICP becomes cICP. The samples are not converted.
#[test]
fn checked_request_binds_the_current_pixel_colour() {
    let raster = rgb8();
    let icc = raster
        .slice(PixelDescriptor::RGB8_SRGB)
        .with_color_context(Arc::new(ColorContext::from_icc(DISPLAY_P3_V4)));
    let read = oracle(&checked(icc).unwrap());
    assert_eq!(read.icc.as_deref(), Some(DISPLAY_P3_V4), "context ICC");
    assert!(
        read.cicp.is_none() || read.cicp == Some(P3),
        "{:?}",
        read.cicp
    );
    assert_samples(&read, 0, "context ICC");

    let cicp = raster
        .slice(PixelDescriptor::RGB8_SRGB)
        .with_color_context(Arc::new(ColorContext::from_cicp(Cicp::DISPLAY_P3)));
    let read = oracle(&checked(cicp).unwrap());
    assert_eq!(read.cicp, Some(P3), "context CICP");
    assert_samples(&read, 0, "context CICP");
}

/// One-shot and row encodes of the same Display P3 pixels write the same
/// colour, for every row mode.
#[test]
fn row_encodes_write_the_same_colour_as_one_shot() {
    let raster = rgb8();
    let desc = p3(PixelDescriptor::RGB8_SRGB);
    for compression in [
        Compression::Effort(0),
        Compression::Effort(1),
        Compression::Effort(5),
        Compression::Effort(19),
    ] {
        let config = PngEncoderConfig::new().with_compression(compression);
        let one_shot = config
            .clone()
            .job()
            .encoder()
            .unwrap()
            .encode(raster.slice(desc))
            .unwrap();
        let mut rows = config.job().encoder().unwrap();
        rows.begin(W, H, desc).unwrap();
        let mut file = Vec::new();
        for y in (0..H).step_by(3) {
            rows.push_rows(raster.slice(desc).sub_rows(y, 3.min(H - y)))
                .unwrap();
            rows.take_output(&mut file);
        }
        file.extend_from_slice(rows.finish().unwrap().data());
        let (once, streamed) = (oracle(one_shot.data()), oracle(&file));
        assert_eq!(once.cicp, Some(P3), "{compression:?} one-shot");
        assert_eq!(streamed.cicp, once.cicp, "{compression:?} rows");
        assert_eq!(streamed.icc, once.icc, "{compression:?} rows");
        assert_samples(&streamed, 0, "rows");
    }
}

/// f32 input is stored as 8-bit sRGB: the written colour is the primaries
/// with the sRGB transfer, whatever described the linear samples.
#[test]
fn linear_input_writes_the_colour_of_the_stored_samples() {
    let raster = rgbf32_linear();
    let desc = p3(PixelDescriptor::RGBF32_LINEAR);
    for (what, context) in [
        ("descriptor", None),
        (
            "context CICP",
            Some(ColorContext::from_cicp(Cicp::new(12, 8, 0, true))),
        ),
    ] {
        let mut pixels = raster.slice(desc);
        if let Some(context) = context {
            pixels = pixels.with_color_context(Arc::new(context));
        }
        let read = oracle(&checked(pixels).unwrap_or_else(|e| panic!("{what}: {e}")));
        assert_eq!(read.cicp, Some(P3), "{what}");
        assert_samples(&read, 1, what);
    }
    // A profile describing the linear samples, supplied as metadata, would be
    // a stale carrier for the stored sRGB samples: refuse.
    let linear_p3 =
        match zenpixels_convert::icc_profiles::synthesize_icc_for_cicp(Cicp::new(12, 8, 0, true)) {
            zenpixels_convert::icc_profiles::SynthesizedIcc::Profile(bytes) => bytes.into_owned(),
            other => panic!("icc-db must synthesize linear P3: {other:?}"),
        };
    assert!(with_metadata(Metadata::none().with_icc(linear_p3), raster.slice(desc)).is_err());
}

/// Metadata or native configuration contradicting the pixels' explicit
/// colour is a stale carrier: refuse instead of relabelling.
#[test]
fn stale_carriers_contradicting_the_current_context_refuse() {
    let raster = rgb8();
    let srgb_now = || {
        raster
            .slice(PixelDescriptor::RGB8_SRGB)
            .with_color_context(Arc::new(ColorContext::from_cicp(Cicp::SRGB)))
    };
    assert!(with_metadata(Metadata::none().with_icc(DISPLAY_P3_V4), srgb_now()).is_err());
    assert!(with_metadata(Metadata::none().with_cicp(Cicp::DISPLAY_P3), srgb_now()).is_err());
    let configured = PngEncoderConfig::new()
        .with_cicp(Some(Cicp::DISPLAY_P3))
        .job()
        .encoder()
        .unwrap()
        .encode(srgb_now());
    assert!(configured.is_err(), "native cICP contradicting the pixels");
}

/// Default sRGB pixels keep their exact bytes whether or not they say so.
#[test]
fn default_srgb_bytes_are_identical_across_routes() {
    let raster = rgb8();
    let plain = checked(raster.slice(PixelDescriptor::RGB8_SRGB)).unwrap();
    let explicit = checked(
        raster
            .slice(PixelDescriptor::RGB8_SRGB)
            .with_color_context(Arc::new(ColorContext::from_cicp(Cicp::SRGB))),
    )
    .unwrap();
    assert_eq!(plain, explicit);
    let read = oracle(&plain);
    assert_eq!((read.icc, read.cicp), (None, None));
}
