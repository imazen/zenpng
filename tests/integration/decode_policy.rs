//! Checked decode policy: the metadata gates (`allow_icc`, `allow_exif`,
//! `allow_xmp`) and refusing truncated input (`allow_truncated: false`) hold
//! on every decode route the job creates: probe, one-shot, row streaming,
//! push, incremental, APNG frames and controlled. Every other field is
//! refused, and a refusal leaves the accepted policy in place.
use std::borrow::Cow;
use std::error::Error;

use zencodec::decode::{
    AnimationFrameDecoder, Decode, DecodeJob, DecodePolicy, DecodeRowSink, DecoderConfig,
    DynDecoderConfig, IncrementalDecode, SinkError, StreamingDecode,
};
use zencodec::encode::{AnimationFrameEncoder, EncodeJob, Encoder, EncoderConfig};
use zencodec::{ImageInfo, Orientation, UnsupportedOperation};
use zencodec_io::{CollectSink, drive_from_slice};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};
use zenpng::{PngDecodeJob, PngDecoderConfig, PngEncoderConfig, PngError};

const W: u32 = 37;
const H: u32 = 23;
/// Little-endian TIFF holding only orientation 6 (Rotate90).
const EXIF: [u8; 26] = [
    b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
];
const XMP: &[u8] = b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>";
const P3: &[u8] = zenpixels_convert::icc_profiles::DISPLAY_P3_V4;

fn noise(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 24) as u8
        })
        .collect()
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
/// `(type, start, end)` of every complete chunk after the signature.
fn spans(bytes: &[u8]) -> Vec<([u8; 4], usize, usize)> {
    let mut out = Vec::new();
    let mut pos = 8;
    while pos + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        let end = pos + 12 + len;
        if end > bytes.len() {
            break;
        }
        out.push((bytes[pos + 4..pos + 8].try_into().unwrap(), pos, end));
        pos = end;
    }
    out
}
fn first(bytes: &[u8], kind: &[u8; 4]) -> (usize, usize) {
    let (_, start, end) = *spans(bytes).iter().find(|s| &s.0 == kind).unwrap();
    (start, end)
}
/// The same file with its image data in `piece`-byte IDAT chunks.
fn split_idat(bytes: &[u8], piece: usize) -> Vec<u8> {
    let mut out = bytes[..8].to_vec();
    for (kind, start, end) in spans(bytes) {
        if &kind == b"IDAT" {
            for part in bytes[start + 8..end - 4].chunks(piece) {
                out.extend(chunk(b"IDAT", part));
            }
        } else {
            out.extend_from_slice(&bytes[start..end]);
        }
    }
    out
}
fn insert(mut bytes: Vec<u8>, at: usize, chunks: &[Vec<u8>]) -> Vec<u8> {
    bytes.splice(at..at, chunks.concat());
    bytes
}
fn icc_chunk() -> Vec<u8> {
    let mut compressor = zenflate::Compressor::new(zenflate::CompressionLevel::new(1));
    let mut compressed = vec![0; zenflate::Compressor::zlib_compress_bound(P3.len())];
    let len = compressor
        .zlib_compress(P3, &mut compressed, enough::Unstoppable)
        .unwrap();
    compressed.truncate(len);
    let mut payload = b"display profile\0\0".to_vec();
    payload.extend(compressed);
    chunk(b"iCCP", &payload)
}
fn xmp_chunk() -> Vec<u8> {
    let mut payload = b"XML:com.adobe.xmp\0\0\0\0\0".to_vec();
    payload.extend_from_slice(XMP);
    chunk(b"iTXt", &payload)
}
/// iCCP and eXIf after IHDR, XMP after the image data.
fn with_metadata(png: Vec<u8>) -> Vec<u8> {
    let png = insert(png, 33, &[icc_chunk(), chunk(b"eXIf", &EXIF)]);
    let iend = png.len() - 12;
    insert(png, iend, &[xmp_chunk()])
}
/// A still RGB8 PNG whose incompressible image data spans 64-byte IDAT chunks.
fn still() -> Vec<u8> {
    let data = noise((W * H * 3) as usize, 0x2545_f491);
    let pixels = PixelSlice::new(&data, W, H, W as usize * 3, PixelDescriptor::RGB8_SRGB).unwrap();
    let png = PngEncoderConfig::new()
        .job()
        .encoder()
        .unwrap()
        .encode(pixels)
        .unwrap()
        .into_vec();
    with_metadata(split_idat(&png, 64))
}
/// A two-frame APNG.
fn animation() -> Vec<u8> {
    let mut encoder = PngEncoderConfig::new()
        .job()
        .with_canvas_size(W, H)
        .animation_frame_encoder()
        .unwrap();
    for seed in [0x1234_5677, 0x0bad_f00d] {
        let data = noise((W * H * 4) as usize, seed);
        let frame =
            PixelSlice::new(&data, W, H, W as usize * 4, PixelDescriptor::RGBA8_SRGB).unwrap();
        encoder.push_frame(frame, 20, None).unwrap();
    }
    with_metadata(encoder.finish(None).unwrap().into_vec())
}

fn job(policy: DecodePolicy) -> PngDecodeJob {
    PngDecoderConfig::new()
        .job()
        .try_with_policy(policy)
        .unwrap_or_else(|e| panic!("{policy:?} refused: {e:?}"))
}
fn packed(pixels: &PixelSlice<'_>) -> Vec<u8> {
    let row = pixels.width() as usize * pixels.descriptor().bytes_per_pixel();
    (0..pixels.rows())
        .flat_map(|y| pixels.row(y)[..row].to_vec())
        .collect()
}

/// What a route reported, and the pixels it produced.
#[derive(Debug, PartialEq)]
struct Seen {
    icc: Option<Vec<u8>>,
    exif: Option<Vec<u8>>,
    xmp: Option<Vec<u8>>,
    orientation: Orientation,
    descriptor: PixelDescriptor,
    pixels: Vec<u8>,
}
fn seen(info: &ImageInfo, descriptor: PixelDescriptor, pixels: Vec<u8>) -> Seen {
    Seen {
        icc: info.source_color.icc_profile.as_deref().map(<[u8]>::to_vec),
        exif: info.embedded_metadata.exif.as_deref().map(<[u8]>::to_vec),
        xmp: info.embedded_metadata.xmp.as_deref().map(<[u8]>::to_vec),
        orientation: info.orientation,
        descriptor,
        pixels,
    }
}
/// The metadata a policy lets through, over the unpoliced pixels: no gate
/// changes a sample or its descriptor. Orientation is display
/// interpretation, not the EXIF blob: it survives the EXIF gate.
fn expected(policy: DecodePolicy, reference: &Seen) -> Seen {
    Seen {
        icc: policy.resolve_icc(true).then(|| P3.to_vec()),
        exif: policy.resolve_exif(true).then(|| EXIF.to_vec()),
        xmp: policy.resolve_xmp(true).then(|| XMP.to_vec()),
        orientation: Orientation::Rotate90,
        descriptor: reference.descriptor,
        pixels: reference.pixels.clone(),
    }
}

fn one_shot(policy: DecodePolicy, bytes: &[u8]) -> Seen {
    let output = job(policy)
        .decoder(Cow::Borrowed(bytes), &[])
        .unwrap()
        .decode()
        .unwrap();
    seen(
        output.info(),
        output.pixels().descriptor(),
        packed(&output.pixels()),
    )
}
fn streamed(policy: DecodePolicy, bytes: &[u8]) -> Seen {
    let mut decoder = job(policy)
        .streaming_decoder(Cow::Borrowed(bytes), &[])
        .unwrap_or_else(|e| panic!("{e}"));
    let (mut rows, mut descriptor) = (Vec::new(), None);
    while let Some((_, strip)) = decoder.next_batch().unwrap() {
        descriptor = Some(strip.descriptor());
        rows.extend(packed(&strip));
    }
    seen(decoder.info(), descriptor.unwrap(), rows)
}
fn incremental(policy: DecodePolicy, bytes: &[u8]) -> Seen {
    let mut decoder = job(policy).dyn_incremental_decoder(&[]).unwrap();
    let mut sink = CollectSink::new();
    let (output, _) = drive_from_slice(&mut decoder, bytes, &mut sink, None, 11).unwrap();
    seen(
        decoder.info().unwrap(),
        output.native_format,
        sink.bytes().to_vec(),
    )
}
/// Push reports no `ImageInfo`: pixels and descriptor only.
fn pushed(policy: DecodePolicy, bytes: &[u8]) -> (PixelDescriptor, Vec<u8>) {
    let mut sink = CollectSink::new();
    let output = job(policy)
        .push_decoder(Cow::Borrowed(bytes), &mut sink, &[])
        .unwrap();
    (output.native_format, sink.bytes().to_vec())
}

/// A sink that counts every call: zero means no effect reached it.
#[derive(Default)]
struct Counted {
    sink: CollectSink,
    calls: usize,
}
impl DecodeRowSink for Counted {
    fn begin(&mut self, w: u32, h: u32, d: PixelDescriptor) -> Result<(), SinkError> {
        self.calls += 1;
        self.sink.begin(w, h, d)
    }
    fn provide_next_buffer(
        &mut self,
        y: u32,
        h: u32,
        w: u32,
        d: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        self.calls += 1;
        self.sink.provide_next_buffer(y, h, w, d)
    }
    fn finish(&mut self) -> Result<(), SinkError> {
        self.calls += 1;
        self.sink.finish()
    }
}

fn metadata_policies() -> Vec<DecodePolicy> {
    let none = DecodePolicy::none();
    vec![
        none,
        none.with_allow_icc(true)
            .with_allow_exif(true)
            .with_allow_xmp(true),
        none.with_allow_icc(false),
        none.with_allow_exif(false),
        none.with_allow_xmp(false),
        none.with_allow_exif(false).with_allow_xmp(false),
        none.with_allow_icc(false)
            .with_allow_exif(false)
            .with_allow_xmp(false)
            .with_allow_truncated(false),
    ]
}

#[test]
fn checked_setter_admits_the_metadata_gates_and_refusing_truncation_only() {
    let options = [None, Some(true), Some(false)];
    for icc in options {
        for exif in options {
            for xmp in options {
                for truncated in [None, Some(false)] {
                    let mut policy = DecodePolicy::none();
                    policy.allow_icc = icc;
                    policy.allow_exif = exif;
                    policy.allow_xmp = xmp;
                    policy.allow_truncated = truncated;
                    PngDecoderConfig::new()
                        .job()
                        .try_set_policy(policy)
                        .unwrap_or_else(|e| panic!("{policy:?}: {e:?}"));
                    // The type-erased job delegates to the same primitive.
                    PngDecoderConfig::new()
                        .dyn_job()
                        .try_set_policy(policy)
                        .unwrap_or_else(|e| panic!("dyn {policy:?}: {e}"));
                }
            }
        }
    }
    let gate = DecodePolicy::none().with_allow_exif(false);
    let mut accepted = PngDecoderConfig::new().job();
    accepted.try_set_policy(gate).unwrap();
    for refused in [
        DecodePolicy::none().with_allow_truncated(true),
        DecodePolicy::none().with_allow_progressive(false),
        DecodePolicy::none().with_allow_progressive(true),
        DecodePolicy::none().with_allow_animation(false),
        DecodePolicy::none().with_allow_animation(true),
        DecodePolicy::none().with_strict(true),
        DecodePolicy::none().with_strict(false),
        DecodePolicy::strict(),
        DecodePolicy::permissive(),
        gate.with_allow_truncated(true),
        gate.with_strict(true),
    ] {
        assert_eq!(
            accepted.try_set_policy(refused),
            Err(UnsupportedOperation::DecodePolicy),
            "{refused:?}"
        );
        let Err(error) = PngDecoderConfig::new().dyn_job().try_set_policy(refused) else {
            panic!("dyn {refused:?} accepted");
        };
        assert_eq!(
            error.downcast_ref::<UnsupportedOperation>(),
            Some(&UnsupportedOperation::DecodePolicy),
            "dyn {refused:?}"
        );
    }
    // The refusals left the accepted gate in force.
    let output = accepted
        .decoder(Cow::Owned(still()), &[])
        .unwrap()
        .decode()
        .unwrap();
    assert!(output.info().embedded_metadata.exif.is_none());
    assert_eq!(output.info().embedded_metadata.xmp.as_deref(), Some(XMP));
}

#[test]
fn metadata_gates_hold_on_every_still_route_without_changing_pixels() {
    let bytes = still();
    let reference = one_shot(DecodePolicy::none(), &bytes);
    assert_eq!(reference.descriptor, PixelDescriptor::RGB8_SRGB);
    assert_eq!(reference.pixels.len(), (W * H * 3) as usize);
    for policy in metadata_policies() {
        let want = expected(policy, &reference);
        let probed = job(policy).probe(&bytes).unwrap();
        assert_eq!(
            seen(&probed, want.descriptor, want.pixels.clone()),
            want,
            "probe {policy:?}"
        );
        assert_eq!(one_shot(policy, &bytes), want, "one-shot {policy:?}");
        assert_eq!(streamed(policy, &bytes), want, "streaming {policy:?}");
        assert_eq!(incremental(policy, &bytes), want, "incremental {policy:?}");
        assert_eq!(
            pushed(policy, &bytes),
            (want.descriptor, want.pixels.clone()),
            "push {policy:?}"
        );
    }
}

#[test]
fn metadata_gates_hold_for_apng_frames_and_their_colour_context() {
    let bytes = animation();
    let frames = |policy: DecodePolicy| {
        let mut decoder = job(policy)
            .animation_frame_decoder(Cow::Borrowed(&bytes), &[])
            .unwrap();
        let mut out = Vec::new();
        while let Some(frame) = decoder.render_next_frame(None).unwrap() {
            let pixels = frame.pixels();
            let icc = pixels
                .color_context()
                .and_then(|c| c.icc.as_deref().map(<[u8]>::to_vec));
            out.push((pixels.descriptor(), packed(pixels), icc));
        }
        (decoder.info().clone(), out)
    };
    let (_, reference) = frames(DecodePolicy::none());
    assert_eq!(reference.len(), 2);
    for policy in metadata_policies() {
        let (info, decoded) = frames(policy);
        let header = seen(&info, PixelDescriptor::RGBA8_SRGB, Vec::new());
        assert_eq!(header, expected(policy, &header), "{policy:?}");
        let icc = policy.resolve_icc(true).then(|| P3.to_vec());
        assert_eq!(decoded.len(), reference.len(), "{policy:?}");
        for (frame, (want, got)) in reference.iter().zip(&decoded).enumerate() {
            assert_eq!(
                (got.0, &got.1),
                (want.0, &want.1),
                "frame {frame}, {policy:?}"
            );
            // A forbidden profile must not ride along on the frame instead.
            assert_eq!(got.2, icc, "frame {frame} colour context, {policy:?}");
        }
    }
}

/// The input ended early: a native `Truncated` cause.
fn assert_truncated(route: &str, error: &(dyn Error + 'static)) {
    assert!(
        matches!(
            zencodec::find_cause::<PngError>(error),
            Some(PngError::Truncated(_))
        ),
        "{route}: expected a native Truncated cause, got {error}"
    );
}

#[test]
fn refusing_truncation_stops_every_route_before_any_effect() {
    let complete = still();
    let (idat, _) = first(&complete, b"IDAT");
    let (_, second_idat_end) = spans(&complete)
        .into_iter()
        .filter(|s| &s.0 == b"IDAT")
        .map(|s| (s.1, s.2))
        .nth(1)
        .unwrap();
    assert!(second_idat_end > idat);
    let (late_xmp, _) = first(&complete, b"iTXt");
    let cuts = [
        // Every chunk is complete, but the image data ends after two chunks.
        (
            "image data cut at a chunk boundary",
            &complete[..second_idat_end],
        ),
        // Complete pixels: only what follows them is missing.
        ("late XMP cut", &complete[..late_xmp + 18]),
        ("IEND missing", &complete[..complete.len() - 12]),
        ("IEND cut", &complete[..complete.len() - 1]),
    ];
    let refuse = DecodePolicy::none().with_allow_truncated(false);
    // Without the policy, a file missing only its IEND decodes in full.
    assert_eq!(
        one_shot(DecodePolicy::none(), cuts[2].1).pixels,
        one_shot(DecodePolicy::none(), &complete).pixels
    );
    // The policy changes nothing for a complete file.
    assert_eq!(
        one_shot(refuse, &complete),
        one_shot(DecodePolicy::none(), &complete)
    );
    assert_eq!(
        streamed(refuse, &complete),
        streamed(DecodePolicy::none(), &complete)
    );
    for (cut, bytes) in cuts {
        let Err(error) = job(refuse)
            .decoder(Cow::Borrowed(bytes), &[])
            .unwrap()
            .decode()
        else {
            panic!("one-shot, {cut}: decoded truncated input");
        };
        assert_truncated(&format!("one-shot, {cut}"), &error);

        let Err(error) = job(refuse).streaming_decoder(Cow::Borrowed(bytes), &[]) else {
            panic!("streaming, {cut}: the decoder started on truncated input");
        };
        assert_truncated(&format!("streaming, {cut}"), &error);

        let mut sink = Counted::default();
        let Err(error) = job(refuse).push_decoder(Cow::Borrowed(bytes), &mut sink, &[]) else {
            panic!("push, {cut}: decoded truncated input");
        };
        assert_truncated(&format!("push, {cut}"), &error);
        assert_eq!(sink.calls, 0, "push, {cut}: sink effects before refusal");

        let mut decoder = job(refuse).dyn_incremental_decoder(&[]).unwrap();
        let mut sink = Counted::default();
        let Err(error) = drive_from_slice(&mut decoder, bytes, &mut sink, None, 11) else {
            panic!("incremental, {cut}: decoded truncated input");
        };
        assert_truncated(&format!("incremental, {cut}"), &error);
        assert_eq!(sink.calls, 0, "incremental, {cut}: sink effects");
    }

    let complete = animation();
    let (frame_two, _) = first(&complete, b"fdAT");
    let cuts = [
        ("second frame missing", &complete[..frame_two]),
        ("IEND missing", &complete[..complete.len() - 12]),
    ];
    for (cut, bytes) in cuts {
        let Err(error) = job(refuse).animation_frame_decoder(Cow::Borrowed(bytes), &[]) else {
            panic!("APNG, {cut}: frames started on truncated input");
        };
        assert_truncated(&format!("APNG, {cut}"), &error);
    }
}

#[cfg(feature = "controlled-decode")]
mod controlled {
    use super::*;
    use std::convert::Infallible;
    use zencodec::At;
    use zencodec::decode::{InterpretationState, InterpretationUnavailable};
    use zencodec::operation::{Decision, Question};
    use zencodec::storage::StorageBudget;
    use zencodec_io::PolicyDecoder;

    /// Drives the controlled route, recording any unavailable interpretation.
    fn drive(
        policy: DecodePolicy,
        bytes: &[u8],
    ) -> (Result<Seen, String>, Vec<InterpretationUnavailable>, usize) {
        let budget = StorageBudget::new(bytes.len() + 65536, 8);
        let session = job(policy)
            .dyn_controlled_decoder(&[], budget.clone())
            .unwrap();
        let mut reasons = Vec::new();
        let callback = |q: Question<'_>| -> Result<Decision, At<Infallible>> {
            if let Question::Boundary { facts, .. } = q
                && facts.state == InterpretationState::Unavailable
            {
                reasons.push(facts.unavailable.unwrap());
            }
            Ok(Decision::UsePreset)
        };
        let mut decoder = PolicyDecoder::new(session, callback);
        let mut sink = Counted::default();
        let result = match drive_from_slice(&mut decoder, bytes, &mut sink, None, 7) {
            Ok((output, _)) => Ok(seen(
                decoder.info().unwrap(),
                output.native_format,
                sink.sink.bytes().to_vec(),
            )),
            Err(error) => Err(match zencodec::find_cause::<PngError>(&error) {
                Some(PngError::Truncated(_)) => "truncated".to_owned(),
                _ => error.to_string(),
            }),
        };
        drop(decoder);
        assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
        (result, reasons, sink.calls)
    }

    #[test]
    fn controlled_route_applies_the_gates_and_keeps_forbidden_icc_unavailable() {
        let bytes = still();
        let reference = one_shot(DecodePolicy::none(), &bytes);
        for policy in metadata_policies() {
            let (result, reasons, calls) = drive(policy, &bytes);
            if policy.resolve_icc(true) {
                assert_eq!(result, Ok(expected(policy, &reference)), "{policy:?}");
                assert!(reasons.is_empty(), "{policy:?}: {reasons:?}");
            } else {
                // The profile identifies these samples: without it the
                // interpretation is unavailable, so nothing reaches the sink.
                assert!(result.is_err(), "{policy:?}");
                assert_eq!(
                    reasons,
                    [InterpretationUnavailable::ForbiddenColorMetadata],
                    "{policy:?}"
                );
                assert_eq!(calls, 0, "{policy:?}");
            }
        }
    }

    #[test]
    fn controlled_route_refuses_truncation_before_any_effect() {
        let complete = still();
        let (late_xmp, _) = first(&complete, b"iTXt");
        let refuse = DecodePolicy::none().with_allow_truncated(false);
        let (whole, _, _) = drive(refuse, &complete);
        assert_eq!(
            whole,
            Ok(expected(refuse, &one_shot(DecodePolicy::none(), &complete)))
        );
        for (cut, bytes) in [
            ("late XMP cut", &complete[..late_xmp + 18]),
            ("IEND missing", &complete[..complete.len() - 12]),
            ("IEND cut", &complete[..complete.len() - 1]),
        ] {
            let (result, _, calls) = drive(refuse, bytes);
            assert_eq!(result, Err("truncated".to_owned()), "{cut}");
            assert_eq!(calls, 0, "{cut}");
        }
    }
}
