//! Effective-job planning runs the adapter's own preparation: the plan must
//! equal the timing of the row mode `begin_declared` constructs, for the
//! colour the rows declare, without allocating rows or starting workers.

use std::io::Cursor;
use std::sync::Arc;

use zencodec::encode::{
    BoxedError, EncodeDeclaration, EncodeJob as _, EncodePlan, Encoder as _, EncoderConfig as _,
    InputRequirement, OutputPhase, OutputTiming,
};
use zencodec::output_patch::OutputAccess;
use zencodec::{ResourceLimits, ThreadingPolicy};
use zenpixels::{Cicp, ColorContext, ColorPrimaries, PixelDescriptor, PixelSlice};
use zenpixels_convert::icc_profiles::DISPLAY_P3_V4;
use zenpng::{Compression, PngEncodeJob, PngEncoder, PngEncoderConfig};

fn declaration(
    width: u32,
    height: u32,
    descriptor: PixelDescriptor,
    context: Option<&ColorContext>,
    access: OutputAccess,
) -> EncodeDeclaration<'_> {
    let declaration = EncodeDeclaration::new(width, height, descriptor).with_access(access);
    match context {
        Some(context) => declaration.with_color(context),
        None => declaration,
    }
}

/// PNG rows are one ordered pass that emits in that (final) pass, and the
/// plan does not report colour carriers yet; only the timing varies.
fn row_timing(plan: EncodePlan, declaration: &EncodeDeclaration<'_>) -> OutputTiming {
    assert_eq!(plan.input, InputRequirement::Rows);
    assert_eq!(plan.output_phase, OutputPhase::FinalPass);
    assert_eq!(plan.carriers, None, "carrier evidence is not reported");
    plan.validate(declaration).unwrap();
    plan.timing
}

/// `EncodeJob::plan_encode` for a row declaration, as its output timing.
fn plan_rows(
    job: &PngEncodeJob,
    width: u32,
    height: u32,
    descriptor: PixelDescriptor,
    context: Option<&ColorContext>,
    access: OutputAccess,
) -> Result<OutputTiming, BoxedError> {
    let declaration = declaration(width, height, descriptor, context, access);
    job.plan_encode(&declaration)
        .map(|plan| row_timing(plan, &declaration))
}

/// `Encoder::begin_declared`, as its output timing.
fn begin_declared(
    encoder: &mut PngEncoder,
    width: u32,
    height: u32,
    descriptor: PixelDescriptor,
    context: Option<&ColorContext>,
    access: OutputAccess,
) -> Result<OutputTiming, zencodec::At<zencodec::CodecError>> {
    let declaration = declaration(width, height, descriptor, context, access);
    encoder
        .begin_declared(&declaration)
        .map(|plan| row_timing(plan, &declaration))
}

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
                            let planned = plan_rows(
                                &job,
                                width,
                                height,
                                descriptor,
                                context.as_ref(),
                                access,
                            );
                            let actual = begin_declared(
                                &mut job.encoder().unwrap(),
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
            plan_rows(
                &job,
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
        let planned = plan_rows(&job, 640, 900, p3, Some(&context), OutputAccess::Forward).unwrap();
        let one_shot = job.clone().encoder().unwrap().encode(pixels()).unwrap();
        let mut rows = job.encoder().unwrap();
        let timing = begin_declared(
            &mut rows,
            640,
            900,
            p3,
            Some(&context),
            OutputAccess::Forward,
        )
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

/// zencodec-io's planned row adapter (`row_encode::encode_job`, ideal-api
/// `1f77b8a3`) drives the real job: it plans and admits before constructing
/// the encoder, begins with the source's colour context and writes the file
/// the direct declared row path writes, in every row mode (stored, strips,
/// pre-filtered, buffered). An independent reader sees the declared profile.
/// A refused declaration fails during preparation with the job's own error
/// and writes nothing.
#[test]
fn zencodec_io_planned_row_adapter_writes_the_direct_row_file() {
    use zencodec::operation::{Decision, PolicyResolver, Question};
    use zencodec::storage::StorageBudget;
    use zencodec_io::row_encode::{self, Options, Raster, Stage};
    use zenpng::DowncastFlags;

    struct FirstSchedule;
    impl PolicyResolver for FirstSchedule {
        type Error = std::convert::Infallible;
        fn resolve(
            &mut self,
            question: Question<'_>,
        ) -> Result<Decision, zencodec::At<Self::Error>> {
            let Question::OutputSchedule { choices, .. } = question else {
                panic!("unexpected policy question")
            };
            Ok(Decision::Choose(choices.first().unwrap().id()))
        }
    }
    let (w, h) = (640u32, 900u32);
    let (data, stride) = strided_rgb(w, h);
    let p3 = PixelDescriptor::RGB8_SRGB.with_primaries(ColorPrimaries::DisplayP3);
    let icc = Arc::new(ColorContext::from_icc(DISPLAY_P3_V4));
    let contradiction = Arc::new(ColorContext::from_cicp(Cicp::SRGB));
    let pixels = |context: &Arc<ColorContext>| {
        PixelSlice::new(&data, w, h, stride, p3)
            .unwrap()
            .with_color_context(context.clone())
    };
    let effort = |e| PngEncoderConfig::new().with_compression(Compression::Effort(e));
    let configs = [
        ("stored", effort(0)),
        ("pre-filtered", effort(1)),
        ("strips", effort(5).with_downcast(DowncastFlags::none())),
        ("buffered", effort(19)),
    ];
    for (mode, config) in configs {
        let declaration = EncodeDeclaration::new(w, h, p3).with_color(&icc);
        let mut direct = config.clone().job().encoder().unwrap();
        direct.begin_declared(&declaration).unwrap();
        let mut expected = Vec::new();
        for y in (0..h).step_by(37) {
            direct
                .push_rows(pixels(&icc).sub_rows(y, 37.min(h - y)))
                .unwrap();
            direct.take_output(&mut expected);
        }
        expected.extend_from_slice(direct.finish().unwrap().data());

        let storage = StorageBudget::new(64 << 20, 64);
        let mut written = Vec::new();
        let done = row_encode::encode_job(
            config.clone().job(),
            Raster::new(pixels(&icc)),
            &mut written,
            FirstSchedule,
            Options::new(storage.clone(), u64::MAX),
            &zencodec::NoPulse,
        )
        .unwrap_or_else(|e| panic!("{mode}: {e:?}"));
        assert_eq!(written, expected, "{mode}");
        assert_eq!(done.emitted, written.len() as u64, "{mode}");
        assert_eq!(storage.reserved_bytes(), 0, "{mode}");
        let reader = png::Decoder::new(Cursor::new(&written))
            .read_info()
            .unwrap();
        assert_eq!(
            reader.info().icc_profile.as_deref(),
            Some(DISPLAY_P3_V4),
            "{mode}"
        );

        let mut refused = Vec::new();
        let failure = row_encode::encode_job(
            config.job(),
            Raster::new(pixels(&contradiction)),
            &mut refused,
            FirstSchedule,
            Options::new(storage.clone(), u64::MAX),
            &zencodec::NoPulse,
        )
        .expect_err(mode);
        assert_eq!(failure.error().stage, Stage::Prepare, "{mode}");
        assert!(refused.is_empty(), "{mode}");
        std::error::Error::source(failure.error())
            .and_then(|e| e.downcast_ref::<zencodec::At<zencodec::CodecError>>())
            .expect("the job's own error survives the adapter");
        assert_eq!(storage.reserved_bytes(), 0, "{mode}");
    }
}

/// The type-erased path forwards both hooks: the same plan from the erased
/// job and from the encoder it builds, and a planning refusal that still
/// downcasts to the job's own error.
#[test]
fn erased_hooks_forward_the_plan_and_the_native_error() {
    use zencodec::encode::DynEncoderConfig;
    let p3 = PixelDescriptor::RGB8_SRGB.with_primaries(ColorPrimaries::DisplayP3);
    let contradiction = ColorContext::from_cicp(Cicp::SRGB);
    let (mut planned, mut refused) = (0, 0);
    for effort in EFFORTS {
        let config = PngEncoderConfig::new().with_compression(Compression::Effort(effort));
        for (descriptor, color) in [
            (PixelDescriptor::RGB8_SRGB, None),
            (p3, Some(&contradiction)),
        ] {
            let declaration = declaration(640, 900, descriptor, color, OutputAccess::Forward);
            let typed = config.clone().job().plan_encode(&declaration);
            let erased_job = DynEncoderConfig::dyn_job(&config);
            match (typed, erased_job.plan_encode(&declaration)) {
                (Ok(typed), Ok(erased)) => {
                    assert_eq!(typed, erased, "effort {effort}");
                    let started = erased_job
                        .into_encoder()
                        .unwrap()
                        .begin_declared(&declaration)
                        .unwrap();
                    typed.validate_started(&started).unwrap();
                    planned += 1;
                }
                (Err(typed), Err(erased)) => {
                    let typed = typed
                        .downcast_ref::<zencodec::At<zencodec::CodecError>>()
                        .expect("typed planning boxes the native error");
                    let erased = erased
                        .downcast_ref::<zencodec::At<zencodec::CodecError>>()
                        .expect("erased planning keeps the native error");
                    assert_eq!(
                        zencodec::CategorizedError::category(typed),
                        zencodec::CategorizedError::category(erased)
                    );
                    refused += 1;
                }
                (typed, erased) => panic!("typed {typed:?} / erased {erased:?}; effort {effort}"),
            }
        }
    }
    assert_eq!((planned, refused), (EFFORTS.len(), EFFORTS.len()));
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
        assert!(begin_declared(&mut enc, 64, 32, p3, None, OutputAccess::Forward).is_err());
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
    let job = PngEncoderConfig::new()
        .job()
        .with_pulse(zencodec::how_far::SharedPulse::new(
            zencodec::how_far::StopOnly::new(Expired),
        ));
    let error = plan_rows(
        &job,
        32,
        32,
        PixelDescriptor::RGB8_SRGB,
        None,
        OutputAccess::Forward,
    )
    .unwrap_err();
    let error = error
        .downcast_ref::<zencodec::At<zencodec::CodecError>>()
        .expect("the job's own error, boxed");
    assert_eq!(
        zencodec::CategorizedError::category(error),
        zencodec::ErrorCategory::Stopped(zencodec::enough::StopReason::TimedOut)
    );
}
