//! Effective-job planning runs the adapter's own preparation: the plan must
//! equal the timing of the row mode `begin_declared` constructs, for the
//! colour the rows declare, without allocating rows or starting workers.

use std::io::Cursor;
use std::sync::Arc;

use zencodec::encode::{EncodeJob as _, Encoder as _, EncoderConfig as _};
use zencodec::output_patch::OutputAccess;
use zencodec::{ResourceLimits, ThreadingPolicy};
use zenpixels::{Cicp, ColorContext, ColorPrimaries, PixelDescriptor, PixelSlice};
use zenpixels_convert::icc_profiles::DISPLAY_P3_V4;
use zenpng::{Compression, PngEncoderConfig};

const EFFORTS: [u32; 9] = [0, 1, 2, 5, 7, 13, 16, 19, 22];

/// Declarations whose interpretation, layout or depth differ, and a
/// contradiction that must refuse in both planning and startup.
fn declarations() -> Vec<(&'static str, PixelDescriptor, Option<ColorContext>)> {
    let p3 = PixelDescriptor::RGB8_SRGB.with_primaries(ColorPrimaries::DisplayP3);
    vec![
        ("rgb8", PixelDescriptor::RGB8_SRGB, None),
        ("rgba8", PixelDescriptor::RGBA8_SRGB, None),
        ("gray8", PixelDescriptor::GRAY8_SRGB, None),
        ("rgb16", PixelDescriptor::RGB16_SRGB, None),
        ("p3 descriptor", p3, None),
        ("p3 icc", p3, Some(ColorContext::from_icc(DISPLAY_P3_V4))),
        ("linear f32", PixelDescriptor::RGBF32_LINEAR, None),
        (
            "contradiction",
            p3,
            Some(ColorContext::from_cicp(Cicp::SRGB)),
        ),
    ]
}

fn limits(parallel: bool) -> ResourceLimits {
    ResourceLimits::none().with_threading(if parallel {
        ThreadingPolicy::Parallel
    } else {
        ThreadingPolicy::Sequential
    })
}

/// The plan equals the constructed row mode's timing for every effort,
/// geometry (including multi-strip images), declaration, destination access
/// and threading policy, or both refuse.
#[test]
fn effective_plan_matches_native_startup() {
    let (mut cells, mut refusals, mut streamed, mut strips) = (0, 0, 0, 0);
    for effort in EFFORTS {
        // Downcast analysis is whole-image: only without it can rows stream
        // in strips.
        for downcast in [true, false] {
            for (width, height) in [(1, 1), (640, 33), (640, 900)] {
                for (what, descriptor, context) in declarations() {
                    for access in [OutputAccess::Forward, OutputAccess::SeekBack] {
                        for parallel in [false, true] {
                            let mut config = PngEncoderConfig::new()
                                .with_compression(Compression::Effort(effort));
                            if !downcast {
                                config = config.with_downcast(zenpng::DowncastFlags::none());
                            }
                            let job = config.job().with_limits(limits(parallel));
                            let planned =
                                job.plan_rows(width, height, descriptor, context.as_ref(), access);
                            let actual = job.encoder().unwrap().begin_declared(
                                width,
                                height,
                                descriptor,
                                context.as_ref(),
                                access,
                            );
                            match (planned, actual) {
                                (Ok(planned), Ok(actual)) => {
                                    assert_eq!(
                                        planned, actual,
                                        "effort {effort} {width}x{height} {what} {access:?} \
                                     parallel={parallel}"
                                    );
                                    streamed += usize::from(planned.streams());
                                    strips += usize::from(effort > 0 && planned.streams());
                                }
                                (Err(_), Err(_)) => refusals += 1,
                                (planned, actual) => panic!(
                                    "planning/startup disagreement for effort {effort} {what}: \
                                 {planned:?} / {actual:?}"
                                ),
                            }
                            cells += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cells, EFFORTS.len() * 2 * 3 * 8 * 2 * 2);
    assert_eq!(
        refusals,
        EFFORTS.len() * 2 * 3 * 2 * 2,
        "only the contradiction refuses"
    );
    // Effort 0 streams every accepted image (7 declarations); strip efforts
    // stream multi-strip images when downcast analysis is off.
    assert_eq!(streamed - strips, 2 * 3 * 7 * 2 * 2, "effort 0");
    assert!(strips > 0, "no strip-streamed cell");
}

fn strided_rgb(width: u32, height: u32) -> (Vec<u8>, usize) {
    let stride = (width as usize + 3) * 3;
    let mut data = vec![0xa5; stride * height as usize];
    for y in 0..height as usize {
        for x in 0..width as usize * 3 {
            data[y * stride + x] = ((x * 31 + y * 17) % 256) as u8;
        }
    }
    (data, stride)
}

/// Planning is repeatable, `begin_output` without a context is the plan,
/// and planning leaves the job's encode byte-identical.
#[test]
fn planning_is_repeatable_and_leaves_the_job_unchanged() {
    let (data, stride) = strided_rgb(640, 900);
    let pixels = || PixelSlice::new(&data, 640, 900, stride, PixelDescriptor::RGB8_SRGB).unwrap();
    for effort in EFFORTS {
        let job = PngEncoderConfig::new()
            .with_compression(Compression::Effort(effort))
            .job();
        let expected = job.clone().encoder().unwrap().encode(pixels()).unwrap();
        let plan = || {
            job.plan_rows(
                640,
                900,
                PixelDescriptor::RGB8_SRGB,
                None,
                OutputAccess::Forward,
            )
            .unwrap()
        };
        let first = plan();
        assert_eq!(first, plan());
        let timing = job
            .clone()
            .encoder()
            .unwrap()
            .begin_output(640, 900, PixelDescriptor::RGB8_SRGB, OutputAccess::SeekBack)
            .unwrap();
        assert_eq!(timing, first, "effort {effort}");
        let actual = job.encoder().unwrap().encode(pixels()).unwrap();
        assert_eq!(actual.data(), expected.data(), "effort {effort}");
    }
}

/// A declared colour is written before any row, rows stream on the planned
/// schedule, and the file's colour chunks match the one-shot encode of the
/// same pixels.
#[test]
fn declared_colour_reaches_the_header_and_rows_match_one_shot_colour() {
    let (data, stride) = strided_rgb(640, 900);
    let p3 = PixelDescriptor::RGB8_SRGB.with_primaries(ColorPrimaries::DisplayP3);
    let context = Arc::new(ColorContext::from_icc(DISPLAY_P3_V4));
    let pixels = || {
        PixelSlice::new(&data, 640, 900, stride, p3)
            .unwrap()
            .with_color_context(context.clone())
    };
    for effort in [0, 1, 5, 19] {
        let job = PngEncoderConfig::new()
            .with_compression(Compression::Effort(effort))
            .job();
        let planned = job
            .plan_rows(640, 900, p3, Some(&context), OutputAccess::Forward)
            .unwrap();
        let one_shot = job.clone().encoder().unwrap().encode(pixels()).unwrap();
        let mut rows = job.encoder().unwrap();
        let timing = rows
            .begin_declared(640, 900, p3, Some(&context), OutputAccess::Forward)
            .unwrap();
        assert_eq!(timing, planned);
        let mut file = Vec::new();
        for y in (0..900).step_by(37) {
            rows.push_rows(pixels().sub_rows(y, 37.min(900 - y)))
                .unwrap();
            if !timing.streams() {
                assert!(rows.pending_output().is_empty(), "effort {effort}");
            }
            rows.take_output(&mut file);
        }
        file.extend_from_slice(rows.finish().unwrap().data());
        let read = |bytes: &[u8]| {
            let reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
            let info = reader.info();
            (
                info.icc_profile.as_ref().map(|icc| icc.to_vec()),
                info.coding_independent_code_points
                    .map(|c| (c.color_primaries, c.transfer_function)),
            )
        };
        let (icc, cicp) = read(&file);
        assert_eq!(icc.as_deref(), Some(DISPLAY_P3_V4), "effort {effort}");
        assert_eq!((icc, cicp), read(one_shot.data()), "effort {effort}");
    }
}

/// A refused declaration leaves the encoder as it was. The limit refusal
/// comes after the Display P3 colour was bound, so a later declaration must
/// see neither that colour nor the refused canvas.
#[test]
fn a_refused_declaration_leaves_the_encoder_unchanged() {
    let (data, stride) = strided_rgb(8, 8);
    let p3 = PixelDescriptor::RGB8_SRGB.with_primaries(ColorPrimaries::DisplayP3);
    for effort in [0, 1, 7] {
        let mut enc = PngEncoderConfig::new()
            .with_compression(Compression::Effort(effort))
            .job()
            .with_limits(ResourceLimits::none().with_max_pixels(100))
            .encoder()
            .unwrap();
        assert!(
            enc.begin_declared(64, 32, p3, None, OutputAccess::Forward)
                .is_err()
        );
        enc.begin(8, 8, PixelDescriptor::RGB8_SRGB).unwrap();
        enc.push_rows(PixelSlice::new(&data, 8, 8, stride, PixelDescriptor::RGB8_SRGB).unwrap())
            .unwrap();
        let mut file = Vec::new();
        enc.take_output(&mut file);
        file.extend_from_slice(enc.finish().unwrap().data());
        let reader = png::Decoder::new(Cursor::new(&file)).read_info().unwrap();
        assert_eq!((reader.info().width, reader.info().height), (8, 8));
        assert!(
            reader.info().coding_independent_code_points.is_none(),
            "effort {effort}"
        );
        assert!(reader.info().icc_profile.is_none(), "effort {effort}");
    }
}

/// Cancellation refuses planning and keeps its original reason.
#[test]
fn cancelled_planning_refuses_with_its_reason() {
    struct Expired;
    impl zencodec::enough::Stop for Expired {
        fn check(&self) -> Result<(), zencodec::enough::StopReason> {
            Err(zencodec::enough::StopReason::TimedOut)
        }
    }
    let error = PngEncoderConfig::new()
        .job()
        .with_pulse(zencodec::how_far::SharedPulse::new(
            zencodec::how_far::StopOnly::new(Expired),
        ))
        .plan_rows(
            32,
            32,
            PixelDescriptor::RGB8_SRGB,
            None,
            OutputAccess::Forward,
        )
        .unwrap_err();
    assert_eq!(
        zencodec::CategorizedError::category(&error),
        zencodec::ErrorCategory::Stopped(zencodec::enough::StopReason::TimedOut)
    );
}
