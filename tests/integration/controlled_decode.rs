//! Native PNG effects must remain behind the mandatory interpretation decision.
use std::{cell::Cell, convert::Infallible, io::Cursor, rc::Rc};
use zencodec::{
    At, CodecSet, ImageFormat,
    decode::{
        DecodeJob, DecodeRowSink, DecoderConfig, InterpretationState, InterpretationUnavailable,
        SinkError,
    },
    operation::{Decision, DecodeRequest, Question},
    storage::StorageBudget,
};
use zencodec_io::{CollectSink, PolicyDecoder, drive_from_slice};
use zenpixels::{PixelDescriptor, PixelSliceMut};
use zenpng::PngDecoderConfig;

#[derive(Default)]
struct Effects {
    sink: CollectSink,
    calls: Rc<Cell<usize>>,
}
impl DecodeRowSink for Effects {
    fn begin(&mut self, w: u32, h: u32, d: PixelDescriptor) -> Result<(), SinkError> {
        self.calls.set(self.calls.get() + 1);
        self.sink.begin(w, h, d)
    }
    fn provide_next_buffer(
        &mut self,
        y: u32,
        h: u32,
        w: u32,
        d: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        self.calls.set(self.calls.get() + 1);
        self.sink.provide_next_buffer(y, h, w, d)
    }
    fn finish(&mut self) -> Result<(), SinkError> {
        self.calls.set(self.calls.get() + 1);
        self.sink.finish()
    }
}
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/idot/{name}.png",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn released(budget: &StorageBudget) {
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
}
fn independent(data: &[u8]) -> Vec<u8> {
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    bytes.truncate(info.buffer_size());
    if info.bit_depth == png::BitDepth::Sixteen {
        for sample in bytes.as_chunks_mut::<2>().0 {
            *sample = u16::from_be_bytes(*sample).to_ne_bytes();
        }
    } else if info.color_type == png::ColorType::GrayscaleAlpha {
        bytes = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect();
    }
    bytes
}

#[test]
fn native_formats_and_adam7_match_independent_pixels_after_adoption() {
    for name in [
        "gray1_n2",
        "gray2_n2",
        "gray4_n2",
        "gray8_n2",
        "gray16_n2",
        "ga8_n2",
        "ga16_n2",
        "pal1_trns_n2",
        "pal2_trns_n2",
        "pal4_trns_n2",
        "pal8_trns_n2",
        "rgb8_n2",
        "rgb16_n2",
        "rgba8_n2",
        "rgba16_n2",
        "interlaced_with_idot",
    ] {
        let bytes = fixture(name);
        let expected = independent(&bytes);
        let native = PngDecoderConfig::new().decode(&bytes).unwrap();
        assert_eq!(
            native.pixels().contiguous_bytes().as_ref(),
            expected,
            "ordinary native {name}"
        );
        for registered in [false, true] {
            let budget = StorageBudget::new(bytes.len() + 65536, 8);
            let session = if registered {
                DecodeRequest::new()
                    .start_controlled(
                        &CodecSet::new().with_decoder(PngDecoderConfig::new()),
                        budget.clone(),
                    )
                    .unwrap()
            } else {
                PngDecoderConfig::new()
                    .job()
                    .dyn_controlled_decoder(&[], budget.clone())
                    .unwrap()
            };
            let mut sink = Effects::default();
            let effects = sink.calls.clone();
            let mut states = Vec::new();
            let callback = |question: Question<'_>| -> Result<Decision, At<Infallible>> {
                assert_eq!(effects.get(), 0, "pixels before decision: {name}");
                let Question::Boundary { facts, .. } = question else {
                    panic!("unexpected domain")
                };
                states.push(facts.state);
                Ok(Decision::UsePreset)
            };
            let mut decoder = PolicyDecoder::new(session, callback);
            let (output, stats) =
                drive_from_slice(&mut decoder, &bytes, &mut sink, None, 7).unwrap();
            assert_eq!(sink.sink.bytes(), expected, "controlled {name}");
            assert_eq!(output.native_format, native.pixels().descriptor());
            assert_eq!(stats.rows, u64::from(output.height));
            assert!(sink.sink.finished());
            drop(decoder);
            assert_eq!(
                states,
                [InterpretationState::Provisional, InterpretationState::Final]
            );
            released(&budget);
        }
    }
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in bytes {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            };
        }
    }
    !c
}
fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(&crc32(&bytes[4..]).to_be_bytes());
    bytes
}
fn insert(mut bytes: Vec<u8>, kind: &[u8; 4], payload: &[u8], late: bool) -> Vec<u8> {
    let offset = if late { bytes.len() - 12 } else { 33 };
    bytes.splice(offset..offset, chunk(kind, payload));
    bytes
}
fn unavailable(bytes: &[u8], why: InterpretationUnavailable) {
    let budget = StorageBudget::new(bytes.len() + 65536, 8);
    let session = PngDecoderConfig::new()
        .job()
        .dyn_controlled_decoder(&[], budget.clone())
        .unwrap();
    let mut sink = Effects::default();
    let effects = sink.calls.clone();
    let mut reasons = Vec::new();
    let callback = |q: Question<'_>| -> Result<Decision, At<Infallible>> {
        assert_eq!(effects.get(), 0);
        let Question::Boundary { facts, .. } = q else {
            panic!("unexpected domain")
        };
        if facts.state == InterpretationState::Unavailable {
            reasons.push(facts.unavailable.unwrap());
        }
        Ok(Decision::UsePreset)
    };
    let mut decoder = PolicyDecoder::new(session, callback);
    let error = drive_from_slice(&mut decoder, bytes, &mut sink, None, 1).unwrap_err();
    assert!(
        matches!(
            error,
            zencodec_io::DriveFailure::Step(zencodec::decode::StepError::Codec(
                zencodec_io::PolicyDecodeError::Policy(_)
            ))
        ),
        "{error}"
    );
    assert_eq!(sink.calls.get(), 0);
    // Even retaining the failed policy adapter must release native input.
    released(&budget);
    drop(decoder);
    assert_eq!(reasons, [why]);
}
#[test]
fn late_malformed_legacy_color_and_sequences_refuse_before_sink_effects() {
    use InterpretationUnavailable as Why;
    let bytes = fixture("rgb8_n2");
    for tag in [
        b"sRGB", b"cICP", b"iCCP", b"gAMA", b"cHRM", b"mDCV", b"cLLI",
    ] {
        let payload = match tag {
            b"sRGB" => vec![0],
            b"cICP" => vec![1, 13, 0, 1],
            _ => vec![],
        };
        unavailable(
            &insert(bytes.clone(), tag, &payload, true),
            Why::LateColorMetadata,
        );
    }
    unavailable(
        &insert(bytes.clone(), b"iCCP", b"broken\0\0", false),
        Why::IncompleteColorMetadata,
    );
    unavailable(
        &insert(bytes.clone(), b"gAMA", &45455u32.to_be_bytes(), false),
        Why::UnsupportedColor,
    );
    unavailable(
        &insert(bytes.clone(), b"cICP", &[1, 13, 1, 1], false),
        Why::IncompleteColorMetadata,
    );
    unavailable(
        &insert(bytes.clone(), b"sRGB", &[4], false),
        Why::IncompleteColorMetadata,
    );
    unavailable(
        &insert(
            insert(bytes.clone(), b"sRGB", &[0], false),
            b"sRGB",
            &[0],
            false,
        ),
        Why::IncompleteColorMetadata,
    );
    unavailable(
        &insert(bytes, b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0], false),
        Why::UnqualifiedOutput,
    );
}
#[test]
fn required_layout_refuses_and_preference_falls_back_without_relabeling() {
    let bytes = fixture("rgb8_n2");
    let codecs = CodecSet::new().with_decoder(PngDecoderConfig::new());
    for required in [true, false] {
        let budget = StorageBudget::new(bytes.len() + 65536, 8);
        let request = if required {
            DecodeRequest::new().with_output(PixelDescriptor::RGBA8_SRGB)
        } else {
            DecodeRequest::new().preferring_output(vec![PixelDescriptor::RGBA8_SRGB])
        };
        let session = request
            .with_format(ImageFormat::Png)
            .start_controlled(&codecs, budget.clone())
            .unwrap();
        let mut decoder = PolicyDecoder::new(session, zencodec::operation::PolicySet::new());
        let mut sink = Effects::default();
        let result = drive_from_slice(&mut decoder, &bytes, &mut sink, None, 7);
        if required {
            assert!(result.is_err());
            assert_eq!(sink.calls.get(), 0);
        } else {
            assert_eq!(result.unwrap().0.native_format, PixelDescriptor::RGB8_SRGB);
        }
        drop(decoder);
        released(&budget);
    }
}
#[test]
fn refusal_and_callback_unwind_release_input_without_sink_effects() {
    use zencodec::operation::Decline;
    let bytes = fixture("rgb8_n2");
    for panic_at in [None, Some(1), Some(2)] {
        let budget = StorageBudget::new(bytes.len() + 65536, 8);
        let session = PngDecoderConfig::new()
            .job()
            .dyn_controlled_decoder(&[], budget.clone())
            .unwrap();
        let mut count = 0;
        let callback = |_: Question<'_>| -> Result<Decision, At<Infallible>> {
            count += 1;
            assert_ne!(panic_at, Some(count), "application callback panic");
            Ok(if count == 2 {
                Decision::decline(Decline::ApplicationRule)
            } else {
                Decision::UsePreset
            })
        };
        let mut decoder = PolicyDecoder::new(session, callback);
        let mut sink = Effects::default();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            drive_from_slice(&mut decoder, &bytes, &mut sink, None, 7)
        }));
        if panic_at.is_some() {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().is_err());
        }
        released(&budget);
        assert_eq!(sink.calls.get(), 0);
    }
}

#[test]
fn late_orientation_is_final_before_native_decode_and_resume_is_mandatory() {
    use zencodec::decode::BoundaryAction;
    use zencodec_io::{ControlledEvent, ControlledStepper, SliceCursor};
    let exif = [
        b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
    ];
    let bytes = insert(fixture("rgb8_n2"), b"eXIf", &exif, true);
    let budget = StorageBudget::new(bytes.len() + 65536, 8);
    let session = PngDecoderConfig::new()
        .job()
        .dyn_controlled_decoder(&[], budget.clone())
        .unwrap();
    let mut sink = Effects::default();
    let effects = sink.calls.clone();
    let mut stage = ControlledStepper::new(session, &mut sink);
    let mut cursor = SliceCursor::new(7);
    for state in [InterpretationState::Provisional, InterpretationState::Final] {
        let boundary = loop {
            if let ControlledEvent::Suspended(b) = cursor
                .next(&bytes, &mut stage, &zencodec::NoPulse)
                .unwrap()
                .into_inner()
            {
                break b;
            }
        };
        assert_eq!(boundary.facts().state, state);
        assert_eq!(effects.get(), 0);
        let reads = cursor.stats().bytes_read;
        for _ in 0..3 {
            let ControlledEvent::Suspended(repeated) = cursor
                .next(&bytes, &mut stage, &zencodec::NoPulse)
                .unwrap()
                .into_inner()
            else {
                panic!("a suspension progressed without its reply")
            };
            assert_eq!(repeated, boundary);
            assert_eq!(cursor.stats().bytes_read, reads);
        }
        let action = if state == InterpretationState::Provisional {
            assert_eq!(reads, 33);
            BoundaryAction::ContinueReading
        } else {
            assert_eq!(
                stage.info().unwrap().orientation,
                zencodec::Orientation::Rotate90
            );
            assert_eq!(
                stage.info().unwrap().embedded_metadata.exif.as_deref(),
                Some(exif.as_slice())
            );
            BoundaryAction::AdoptInterpretation
        };
        stage
            .resume(boundary.id(), action, &zencodec::NoPulse)
            .unwrap();
    }
    loop {
        if cursor
            .next(&bytes, &mut stage, &zencodec::NoPulse)
            .unwrap()
            .is_finished()
        {
            break;
        }
    }
    assert_eq!(stage.sink().sink.bytes(), independent(&bytes));
    drop(stage);
    released(&budget);
}
#[test]
fn native_truncation_and_storage_rejection_release_retained_owners() {
    let mut short = fixture("rgb8_n2");
    // Valid header/chunk CRC, but the same IDAT stream now promises one extra row.
    let h = u32::from_be_bytes(short[20..24].try_into().unwrap());
    short[20..24].copy_from_slice(&(h + 1).to_be_bytes());
    let crc = crc32(&short[12..29]);
    short[29..33].copy_from_slice(&crc.to_be_bytes());
    for (bytes, cap) in [
        (short.as_slice(), short.len() + 65536),
        (short.as_slice(), 6144),
    ] {
        let budget = StorageBudget::new(cap, 8);
        let session = PngDecoderConfig::new()
            .job()
            .dyn_controlled_decoder(&[], budget.clone())
            .unwrap();
        let mut decoder = PolicyDecoder::new(session, zencodec::operation::PolicySet::new());
        let mut sink = Effects::default();
        let error = drive_from_slice(&mut decoder, bytes, &mut sink, None, 3).unwrap_err();
        assert!(!sink.sink.finished());
        released(&budget);
        let mut cause: &dyn std::error::Error = &error;
        let mut native = false;
        let mut storage = false;
        loop {
            native |= cause.downcast_ref::<zenpng::PngError>().is_some();
            storage |= cause
                .downcast_ref::<zencodec::storage::StorageError>()
                .is_some();
            match cause.source() {
                Some(next) => cause = next,
                None => break,
            }
        }
        if cap == 6144 {
            assert!(storage, "{error}");
            assert_eq!(sink.calls.get(), 0);
        } else {
            assert!(native, "{error}");
        }
    }
}
#[test]
fn cancellation_at_either_suspension_drops_native_input() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    #[derive(Clone)]
    struct Cancel(Arc<AtomicBool>);
    impl enough::Stop for Cancel {
        fn check(&self) -> Result<(), enough::StopReason> {
            if self.0.load(Ordering::Relaxed) {
                Err(enough::StopReason::Cancelled)
            } else {
                Ok(())
            }
        }
    }
    use zencodec_io::{ControlledEvent, ControlledStepper, SliceCursor};
    let bytes = fixture("rgba16_n2");
    for when in [1, 2] {
        let budget = StorageBudget::new(bytes.len() + 65536, 8);
        let session = PngDecoderConfig::new()
            .job()
            .dyn_controlled_decoder(&[], budget.clone())
            .unwrap();
        let mut sink = Effects::default();
        let mut stage = ControlledStepper::new(session, &mut sink);
        let mut cursor = SliceCursor::new(7);
        let cancel = Cancel(Arc::new(AtomicBool::new(false)));
        let pulse = zencodec::how_far::StopOnly::borrowed(&cancel);
        let mut decisions = 0;
        loop {
            if let ControlledEvent::Suspended(b) = cursor
                .next(&bytes, &mut stage, &pulse)
                .unwrap()
                .into_inner()
            {
                decisions += 1;
                if decisions == when {
                    cancel.0.store(true, Ordering::Relaxed);
                    assert!(cursor.next(&bytes, &mut stage, &pulse).is_err());
                    break;
                }
                stage
                    .resume(
                        b.id(),
                        zencodec::decode::BoundaryAction::ContinueReading,
                        &pulse,
                    )
                    .unwrap();
            }
        }
        released(&budget);
        assert_eq!(stage.sink().calls.get(), 0);
    }
}

#[test]
fn read_at_uses_same_boundaries_and_input_limits_precede_reads() {
    use zencodec::source::ReadAt;
    struct Source<'a> {
        bytes: &'a [u8],
        reads: Rc<Cell<usize>>,
    }
    impl ReadAt for Source<'_> {
        type Error = std::io::Error;
        fn len(&mut self) -> Result<u64, Self::Error> {
            Ok(self.bytes.len() as u64)
        }
        fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error> {
            self.reads.set(self.reads.get() + 1);
            let start = usize::try_from(offset).unwrap();
            let bytes = self
                .bytes
                .get(start..start + out.len())
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::UnexpectedEof))?;
            out.copy_from_slice(bytes);
            Ok(())
        }
    }
    let bytes = fixture("pal4_trns_n2");
    for cap in [0, bytes.len() as u64] {
        let budget = StorageBudget::new(bytes.len() + 65536, 8);
        let job = PngDecoderConfig::new()
            .job()
            .with_limits(zencodec::ResourceLimits::none().with_max_input_bytes(cap));
        let session = job.dyn_controlled_decoder(&[], budget.clone()).unwrap();
        let mut decoder = PolicyDecoder::new(session, zencodec::operation::PolicySet::new());
        let reads = Rc::new(Cell::new(0));
        let mut source = Source {
            bytes: &bytes,
            reads: reads.clone(),
        };
        let mut sink = Effects::default();
        let result = zencodec_io::drive_from_read_at(
            &mut decoder,
            &mut source,
            &mut sink,
            None,
            zencodec_io::DriveOptions::new(0, 37),
        );
        if cap == 0 {
            assert!(result.is_err());
            assert_eq!(reads.get(), 0);
            assert_eq!(sink.calls.get(), 0);
        } else {
            result.unwrap();
            assert_eq!(sink.sink.bytes(), independent(&bytes));
            assert!(reads.get() > 0);
        }
        drop(decoder);
        released(&budget);
    }
}

#[test]
fn recognized_carriers_cannot_disappear_through_lenient_parsing() {
    let bytes = fixture("rgb8_n2");
    let mut bad_crc = insert(bytes.clone(), b"cICP", &[12, 13, 0, 1], false);
    bad_crc[48] ^= 1;
    unavailable(&bad_crc, InterpretationUnavailable::IncompleteColorMetadata);
    unavailable(
        &insert(bytes, b"cICP", &[255, 255, 0, 1], false),
        InterpretationUnavailable::UnsupportedColor,
    );
}
#[test]
#[allow(deprecated)] // Exercises the legacy best-effort policy setter on purpose.
fn native_hdr_and_icc_keep_exact_samples_and_reported_interpretation() {
    use zencodec::decode::DecodePolicy;
    let profile = zenpixels_convert::icc_profiles::DISPLAY_P3_V4;
    let mut compressor = zenflate::Compressor::new(zenflate::CompressionLevel::new(1));
    let mut compressed = vec![0; zenflate::Compressor::zlib_compress_bound(profile.len())];
    let len = compressor
        .zlib_compress(profile, &mut compressed, enough::Unstoppable)
        .unwrap();
    compressed.truncate(len);
    let mut carrier = b"display profile\0\0".to_vec();
    carrier.extend(compressed);
    let icc = insert(fixture("rgb8_n2"), b"iCCP", &carrier, false);
    let hdr = insert(fixture("rgb16_n2"), b"cICP", &[9, 16, 0, 1], false);
    for bytes in [&icc, &hdr] {
        let ordinary = PngDecoderConfig::new().decode(bytes).unwrap();
        let budget = StorageBudget::new(bytes.len() + 65536, 8);
        let session = PngDecoderConfig::new()
            .job()
            .dyn_controlled_decoder(&[], budget.clone())
            .unwrap();
        let mut decoder = PolicyDecoder::new(session, zencodec::operation::PolicySet::new());
        let mut sink = Effects::default();
        let (output, _) = drive_from_slice(&mut decoder, bytes, &mut sink, None, 3).unwrap();
        assert_eq!(output.native_format, ordinary.pixels().descriptor());
        assert_eq!(sink.sink.bytes(), independent(bytes));
        drop(decoder);
        released(&budget);
    }
    assert_eq!(
        PngDecoderConfig::new()
            .probe(&icc)
            .unwrap()
            .source_color
            .icc_profile
            .as_deref(),
        Some(profile)
    );
    let budget = StorageBudget::new(icc.len() + 65536, 8);
    let session = PngDecoderConfig::new()
        .job()
        .with_policy(DecodePolicy::none().with_allow_icc(false))
        .dyn_controlled_decoder(&[], budget.clone())
        .unwrap();
    let mut reason = None;
    let callback = |q: Question<'_>| -> Result<Decision, At<Infallible>> {
        if let Question::Boundary { facts, .. } = q {
            reason = facts.unavailable;
        }
        Ok(Decision::UsePreset)
    };
    let mut decoder = PolicyDecoder::new(session, callback);
    let mut sink = Effects::default();
    assert!(drive_from_slice(&mut decoder, &icc, &mut sink, None, 3).is_err());
    released(&budget);
    drop(decoder);
    assert_eq!(
        reason,
        Some(InterpretationUnavailable::ForbiddenColorMetadata)
    );
    assert_eq!(sink.calls.get(), 0);
}

#[test]
fn auxiliary_and_unknown_chunks_cannot_be_mistaken_for_absent_gain_maps() {
    let bytes = fixture("rgb8_n2");
    for tag in [b"gmAP", b"gdAT", b"gMAP", b"gDAT", b"vpAg", b"CgBI"] {
        for late in [false, true] {
            unavailable(
                &insert(bytes.clone(), tag, &[], late),
                InterpretationUnavailable::UnqualifiedOutput,
            );
        }
    }
    let mut trailing = bytes;
    trailing.extend_from_slice(b"uninspected suffix");
    unavailable(&trailing, InterpretationUnavailable::UnqualifiedOutput);
}

#[test]
fn malformed_hdr_and_duplicate_legacy_carriers_are_unavailable() {
    let bytes = fixture("rgb8_n2");
    for tag in [b"gAMA", b"cHRM", b"mDCV", b"cLLI"] {
        unavailable(
            &insert(bytes.clone(), tag, &[], false),
            InterpretationUnavailable::IncompleteColorMetadata,
        );
    }
    unavailable(
        &insert(
            insert(bytes, b"gAMA", &45455u32.to_be_bytes(), false),
            b"gAMA",
            &45455u32.to_be_bytes(),
            false,
        ),
        InterpretationUnavailable::IncompleteColorMetadata,
    );
}

#[test]
fn gray_alpha_sixteen_reports_the_same_default_transfer_as_other_png_layouts() {
    use zencodec::decode::StreamingDecode;
    for bytes in [
        fixture("ga16_n2"),
        insert(fixture("gray16_n2"), b"tRNS", &[0, 1], false),
    ] {
        let descriptor = PixelDescriptor::GRAYA16_SRGB;
        let config = PngDecoderConfig::new();
        assert_eq!(
            config
                .clone()
                .job()
                .output_info(&bytes)
                .unwrap()
                .native_format,
            descriptor
        );
        let image = config.clone().decode(&bytes).unwrap();
        assert_eq!(image.pixels().descriptor(), descriptor);
        assert_eq!(
            image.pixels().contiguous_bytes().as_ref(),
            independent(&bytes)
        );
        let mut stream = config
            .job()
            .streaming_decoder(bytes.as_slice().into(), &[])
            .unwrap();
        while let Some((_, rows)) = stream.next_batch().unwrap() {
            assert_eq!(rows.descriptor(), descriptor);
        }
    }
}
