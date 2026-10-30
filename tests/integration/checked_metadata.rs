//! Required metadata is an obligation on the retained native job.
use zencodec::{
    Metadata, MetadataPolicy,
    encode::{EncodeJob, Encoder, EncoderConfig},
};
use zenpixels::{
    PixelDescriptor, PixelSlice,
    hdr::{ContentLightLevel, MasteringDisplay},
};
use zenpng::{Compression, DowncastFlags, PngEncoderConfig};

fn config() -> PngEncoderConfig {
    PngEncoderConfig::new()
        .with_compression(Compression::Effort(1))
        .with_downcast(DowncastFlags::none())
}

fn encode(job: zenpng::PngEncodeJob) -> Vec<u8> {
    let data = [7, 19, 43, 255];
    let enc = job.encoder().unwrap();
    enc.encode(PixelSlice::new(&data, 1, 1, 4, PixelDescriptor::RGBA8_SRGB).unwrap())
        .unwrap()
        .into_vec()
}

fn encode_hdr(job: zenpng::PngEncodeJob) -> Vec<u8> {
    let data = [7, 19, 43, 255];
    let descriptor = PixelDescriptor::RGBA8_SRGB
        .with_primaries(zenpixels::ColorPrimaries::Bt2020)
        .with_transfer(zenpixels::TransferFunction::Pq);
    job.encoder()
        .unwrap()
        .encode(PixelSlice::new(&data, 1, 1, 4, descriptor).unwrap())
        .unwrap()
        .into_vec()
}

struct PullSource {
    data: [u8; 4],
    reads: std::rc::Rc<std::cell::Cell<usize>>,
}
impl zencodec_io::row_encode::Source for PullSource {
    type Error = std::convert::Infallible;
    fn description(&self) -> zencodec_io::row_encode::Description {
        zencodec_io::row_encode::Description::new(1, 1, PixelDescriptor::RGBA8_SRGB)
    }
    fn read_rows(
        &mut self,
        first: u32,
        mut rows: zenpixels::PixelSliceMut<'_>,
        _pulse: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), Self::Error> {
        assert_eq!(first, 0);
        assert_eq!(rows.rows(), 1);
        rows.row_mut(0).copy_from_slice(&self.data);
        self.reads.set(self.reads.get() + 1);
        Ok(())
    }
}

#[test]
fn accepted_cll_survives_refused_replacement_and_can_be_cleared() {
    let expected = ContentLightLevel::new(1000, 400);
    let mut job = config().job();
    job.try_set_metadata_policy(
        Metadata::none().with_content_light_level(expected),
        MetadataPolicy::PreserveExact,
    )
    .unwrap();
    let unsupported = Metadata::none().with_mastering_display(MasteringDisplay::new(
        [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046]],
        [0.3127, 0.3290],
        1000.0,
        0.005,
    ));
    assert!(
        job.try_set_metadata_policy(unsupported, MetadataPolicy::PreserveExact)
            .is_err()
    );
    let output = encode(job.clone());
    assert_eq!(
        zenpng::probe(&output).unwrap().content_light_level,
        Some(expected)
    );
    job.try_set_metadata_policy(Metadata::none(), MetadataPolicy::PreserveExact)
        .unwrap();
    assert!(
        zenpng::probe(&encode(job))
            .unwrap()
            .content_light_level
            .is_none()
    );
}

#[test]
fn native_raw_override_cannot_replace_required_cll() {
    let mut job = config().with_content_light_level_raw(Some([1, 2])).job();
    assert!(
        job.try_set_metadata_policy(
            Metadata::none().with_content_light_level(ContentLightLevel::new(1000, 400)),
            MetadataPolicy::PreserveExact,
        )
        .is_err()
    );
}

fn chunks(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut pos = 8;
    while pos + 12 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let tag = data[pos + 4..pos + 8].try_into().unwrap();
        out.push((tag, &data[pos + 8..pos + 8 + len]));
        pos += 12 + len;
    }
    out
}

fn assert_readback(bytes: &[u8], expected: &Metadata, samples: &[u8]) {
    let decoded = zenpng::PngDecoderConfig::new().decode(bytes).unwrap();
    zencodec_testkit::check_metadata_retained(expected, &decoded.metadata()).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    if let Some(icc) = expected.icc_profile.as_deref() {
        assert_eq!(reader.info().icc_profile.as_deref(), Some(icc));
    }
    let mut output = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut output).unwrap();
    assert_eq!(&output[..info.buffer_size()], samples);
    let entries = chunks(bytes);
    if let Some(xmp) = expected.xmp.as_deref() {
        assert!(
            entries
                .iter()
                .any(|(tag, payload)| tag == b"iTXt" && payload.ends_with(xmp))
        );
    }
    if let Some(exif) = expected.exif.as_deref() {
        assert!(
            entries
                .iter()
                .any(|(tag, payload)| tag == b"eXIf" && *payload == exif)
        );
    }
    if let Some(cll) = expected.content_light_level {
        let mut payload = Vec::new();
        payload.extend_from_slice(&(u32::from(cll.max_content_light_level) * 10_000).to_be_bytes());
        payload.extend_from_slice(
            &(u32::from(cll.max_frame_average_light_level) * 10_000).to_be_bytes(),
        );
        assert!(
            entries
                .iter()
                .any(|(tag, found)| tag == b"cLLI" && *found == payload)
        );
    }
    if let Some(cicp) = expected.cicp {
        let expected_cicp = [
            cicp.color_primaries,
            cicp.transfer_characteristics,
            cicp.matrix_coefficients,
            u8::from(cicp.full_range),
        ];
        assert!(
            entries
                .iter()
                .any(|(tag, payload)| tag == b"cICP" && *payload == expected_cicp)
        );
    }
    if expected.mastering_display.is_some() {
        let mut expected_raw = [0u8; 24];
        expected_raw[16..20].copy_from_slice(&1_000_000u32.to_be_bytes());
        assert!(
            entries
                .iter()
                .any(|(tag, payload)| tag == b"mDCV" && *payload == expected_raw)
        );
        assert!(
            entries
                .iter()
                .any(|(tag, payload)| tag == b"mDCV" && payload.len() == 24)
        );
    }
}

#[test]
fn checked_carriers_survive_still_rows_pull_apng_and_erasure() {
    use zencodec::encode::AnimationFrameEncoder;
    let exif = [b'I', b'I', 42, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let expected = Metadata::none()
        .with_exif(exif.as_slice())
        .with_xmp(b"<xmp>checked</xmp>".as_slice())
        .with_icc(zenpixels_convert::icc_profiles::DISPLAY_P3_V4)
        .with_content_light_level(ContentLightLevel::new(1000, 400));
    let data = [7, 19, 43, 255];
    let pixels = || PixelSlice::new(&data, 1, 1, 4, PixelDescriptor::RGBA8_SRGB).unwrap();
    let job = || {
        config()
            .job()
            .try_with_metadata_policy(expected.clone(), MetadataPolicy::PreserveExact)
            .unwrap()
    };
    let direct = job().encoder().unwrap().encode(pixels()).unwrap();
    assert_readback(direct.data(), &expected, &data);
    let mut rows = job().with_canvas_size(1, 1).encoder().unwrap();
    rows.begin(1, 1, PixelDescriptor::RGBA8_SRGB).unwrap();
    rows.push_rows(pixels()).unwrap();
    let result = rows.finish().unwrap();
    assert_readback(result.data(), &expected, &data);
    let reads = std::rc::Rc::new(std::cell::Cell::new(0));
    let source = PullSource {
        data,
        reads: reads.clone(),
    };
    let mut pulled = Vec::new();
    let completed = zencodec_io::row_encode::encode_job(
        job(),
        source,
        &mut pulled,
        zencodec::operation::PolicySet::new(),
        zencodec_io::row_encode::Options::new(
            zencodec::storage::StorageBudget::new(1 << 20, 4096),
            1 << 20,
        ),
        None,
    )
    .unwrap();
    assert!(
        reads.get() > 0,
        "the shared driver must read the pull source"
    );
    assert_eq!(completed.emitted, pulled.len() as u64);
    assert_readback(&pulled, &expected, &data);
    let unsupported = job()
        .with_canvas_size(1, 1)
        .encoder()
        .unwrap()
        .encode_from(&mut |_, _| panic!("unsupported native pull must not call the source"))
        .unwrap_err();
    use zencodec::CodecErrorExt as _;
    assert_eq!(
        unsupported.unsupported_operation(),
        Some(&zencodec::UnsupportedOperation::PullEncode)
    );
    let mut animation = job()
        .with_canvas_size(1, 1)
        .animation_frame_encoder()
        .unwrap();
    animation.push_frame(pixels(), 20, None).unwrap();
    let result = animation.finish(None).unwrap();
    assert_readback(result.data(), &expected, &data);
    let codecs = zencodec::CodecSet::new().with_send_encoder(config());
    let mut erased = codecs.encode_job(zencodec::ImageFormat::Png).unwrap();
    erased
        .try_set_metadata_policy(expected.clone(), MetadataPolicy::PreserveExact)
        .unwrap();
    let result = erased.into_encoder().unwrap().encode(pixels()).unwrap();
    assert_readback(result.data(), &expected, &data);
}

#[test]
fn representable_mastering_and_cicp_round_trip_and_quantized_values_refuse() {
    let m = MasteringDisplay::new([[0.0; 2]; 3], [0.0; 2], 100.0, 0.0);
    let metadata = Metadata::none()
        .with_cicp(zenpixels::Cicp::new(9, 16, 0, true))
        .with_mastering_display(m);
    let mut job = config().job();
    job.try_set_metadata_policy(metadata.clone(), MetadataPolicy::PreserveExact)
        .unwrap();
    assert_readback(&encode_hdr(job.clone()), &metadata, &[7, 19, 43, 255]);
    let nonexact = MasteringDisplay::new([[0.1234567; 2]; 3], [0.0; 2], 100.0, 0.0);
    assert!(
        job.try_set_metadata_policy(
            Metadata::none()
                .with_cicp(zenpixels::Cicp::new(9, 16, 0, true))
                .with_mastering_display(nonexact),
            MetadataPolicy::PreserveExact
        )
        .is_err()
    );
    assert_readback(&encode_hdr(job), &metadata, &[7, 19, 43, 255]);
}

#[test]
#[allow(deprecated)] // These controls deliberately exercise legacy setters after admission.
fn conflicting_legacy_setters_refuse_before_encoder_or_header() {
    use zencodec::encode::EncodePolicy;
    let expected = Metadata::none().with_xmp(b"<checked/>".as_slice());
    let make = || {
        config()
            .job()
            .try_with_metadata_policy(expected.clone(), MetadataPolicy::PreserveExact)
            .unwrap()
    };
    assert!(make().with_metadata(Metadata::none()).encoder().is_err());
    assert!(
        make()
            .with_policy(EncodePolicy::none().with_embed_xmp(false))
            .encoder()
            .is_err()
    );
}

#[test]
fn unsupported_and_native_conflicts_refuse_atomically() {
    use zenpixels::hdr::DiffuseWhite;
    let good = Metadata::none().with_xmp(b"<retained/>".as_slice());
    let mut job = config().job();
    job.try_set_metadata_policy(good.clone(), MetadataPolicy::PreserveExact)
        .unwrap();
    for bad in [
        Metadata::none().with_diffuse_white(DiffuseWhite::new(203.0)),
        Metadata::none().with_xmp(&b"\xff"[..]),
        Metadata::none().with_xmp(&b"x\0y"[..]),
        Metadata::none().with_xmp(&b""[..]),
        Metadata::none().with_icc(&b""[..]),
        Metadata::none().with_cicp(zenpixels::Cicp::new(1, 13, 1, true)),
    ] {
        let error = job
            .try_set_metadata_policy(bad, MetadataPolicy::PreserveExact)
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<zencodec::UnsupportedOperation>(),
            Some(&zencodec::UnsupportedOperation::MetadataRetention)
        );
        assert_readback(&encode(job.clone()), &good, &[7, 19, 43, 255]);
    }
    let mastering = MasteringDisplay::new([[0.0; 2]; 3], [0.0; 2], 100.0, 0.0);
    let record = Metadata::none()
        .with_cicp(zenpixels::Cicp::new(9, 16, 0, true))
        .with_mastering_display(mastering);
    let mut conflict = config().with_mastering_display_raw(Some([0; 24])).job();
    assert!(
        conflict
            .try_set_metadata_policy(record, MetadataPolicy::PreserveExact)
            .is_err()
    );
}

#[test]
fn ordinary_erased_job_owns_checked_metadata() {
    use zencodec::encode::DynEncoderConfig;
    let metadata = Metadata::none().with_xmp(b"<erased/>".as_slice());
    let mut job = DynEncoderConfig::dyn_job(&config());
    job.try_set_metadata_policy(metadata.clone(), MetadataPolicy::PreserveExact)
        .unwrap();
    let data = [7, 19, 43, 255];
    let pixels = PixelSlice::new(&data, 1, 1, 4, PixelDescriptor::RGBA8_SRGB).unwrap();
    let output = job.into_encoder().unwrap().encode(pixels).unwrap();
    assert_readback(output.data(), &metadata, &data);
}

#[test]
fn field_orientation_and_float_color_cannot_be_silently_rewritten() {
    use zencodec::encode::EncodePolicy;
    let oriented = Metadata::none().with_orientation(zencodec::Orientation::Rotate90);
    let job = config()
        .job()
        .try_with_metadata_policy(oriented.clone(), MetadataPolicy::PreserveExact)
        .unwrap();
    assert_readback(&encode(job), &oriented, &[7, 19, 43, 255]);
    #[allow(deprecated)]
    let conflict = config()
        .job()
        .try_with_metadata_policy(oriented, MetadataPolicy::PreserveExact)
        .unwrap()
        .with_policy(EncodePolicy::none().with_embed_exif(false));
    assert!(conflict.encoder().is_err());
    let linear = zenpixels::Cicp::new(1, 8, 0, true);
    let job = config()
        .job()
        .try_with_metadata_policy(
            Metadata::none().with_cicp(linear),
            MetadataPolicy::PreserveExact,
        )
        .unwrap();
    let data = [0.5_f32, 0.5, 0.5];
    let bytes: &[u8] = bytemuck::cast_slice(&data);
    let pixels = PixelSlice::new(bytes, 1, 1, 12, PixelDescriptor::RGBF32_LINEAR).unwrap();
    assert!(job.clone().encoder().unwrap().encode(pixels).is_err());
    let mut rows = job.with_canvas_size(1, 1).encoder().unwrap();
    assert!(rows.begin(1, 1, PixelDescriptor::RGBF32_LINEAR).is_err());
    assert!(rows.pending_output().is_empty());
}

#[test]
fn discarding_policy_cannot_leave_native_hdr_from_before_replacement() {
    let mut job = config()
        .with_content_light_level_raw(Some([123, 456]))
        .job();
    let discard = MetadataPolicy::Custom(zencodec::MetadataFields::DISCARD_ALL);
    let err = job
        .try_set_metadata_policy(Metadata::none(), discard)
        .unwrap_err();
    assert_eq!(
        err.downcast_ref::<zencodec::UnsupportedOperation>(),
        Some(&zencodec::UnsupportedOperation::MetadataRetention)
    );
}

#[test]
#[allow(deprecated)] // Exercise the published policy setter after checked admission.
fn explicit_color_policy_cannot_substitute_retained_icc() {
    let meta = Metadata::none().with_icc(zenpixels_convert::icc_profiles::DISPLAY_P3_V4);
    let job = config()
        .job()
        .try_with_metadata_policy(meta.clone(), MetadataPolicy::PreserveExact)
        .unwrap();
    assert_readback(&encode(job.clone()), &meta, &[7, 19, 43, 255]);
    let conflict = job.with_policy(zencodec::encode::EncodePolicy::none().with_embed_icc(false));
    assert!(conflict.encoder().is_err());
}

#[test]
fn streaming_animation_preserves_checked_record_and_refuses_conflicting_context_before_header() {
    use std::sync::Arc;
    use zencodec::encode::AnimationFrameEncoder;
    let metadata = Metadata::none()
        .with_icc(zenpixels_convert::icc_profiles::DISPLAY_P3_V4)
        .with_xmp(b"<animation/>".as_slice());
    let job = || {
        config()
            .job()
            .try_with_metadata_policy(metadata.clone(), MetadataPolicy::PreserveExact)
            .unwrap()
            .with_canvas_size(1, 1)
    };
    let data = [7, 19, 43, 255];
    let make_pixels = || PixelSlice::new(&data, 1, 1, 4, PixelDescriptor::RGBA8_SRGB).unwrap();
    let mut rejected = job().animation_frame_encoder().unwrap();
    rejected
        .begin(1, 1, PixelDescriptor::RGBA8_SRGB, Some(1))
        .unwrap();
    let context = Arc::new(zenpixels::ColorContext::from_cicp(zenpixels::Cicp::new(
        9, 16, 0, true,
    )));
    assert!(
        rejected
            .push_frame(make_pixels().with_color_context(context), 20, None)
            .is_err()
    );
    assert!(rejected.pending_output().is_empty());
    let mut animation = job().animation_frame_encoder().unwrap();
    animation
        .begin(1, 1, PixelDescriptor::RGBA8_SRGB, Some(1))
        .unwrap();
    animation.push_frame(make_pixels(), 20, None).unwrap();
    let mut result = Vec::new();
    animation.take_output(&mut result);
    result.extend_from_slice(animation.finish(None).unwrap().data());
    assert_readback(&result, &metadata, &data);
}

#[test]
#[allow(deprecated)] // Exercise reinsertion through the legacy job setter.
fn discarded_or_cleared_metadata_cannot_be_reinserted_by_legacy_setters() {
    let original = Metadata::none().with_xmp(b"<private/>".as_slice());
    let mut job = config().job();
    job.try_set_metadata_policy(original.clone(), MetadataPolicy::Web)
        .unwrap();
    let output = encode(job.clone());
    assert!(zenpng::probe(&output).unwrap().xmp.is_none());
    assert!(
        job.clone()
            .with_metadata(original.clone())
            .encoder()
            .is_err()
    );
    assert!(
        job.clone()
            .with_metadata(original.clone())
            .animation_frame_encoder()
            .is_err()
    );
    job.try_set_metadata_policy(Metadata::none(), MetadataPolicy::PreserveExact)
        .unwrap();
    assert!(job.with_metadata(original).encoder().is_err());
}
