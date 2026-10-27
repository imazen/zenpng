use zencodec::decode::{DecodeJob, DecoderConfig, DynDecoderConfig};
use zencodec::{CodecErrorExt, UnsupportedOperation};
use zenpng::PngDecoderConfig;

#[test]
fn capability_matches_feature_for_generic_and_dynamic_jobs() {
    let config = PngDecoderConfig::new();
    assert_eq!(
        <PngDecoderConfig as DecoderConfig>::capabilities().inventory(),
        cfg!(feature = "inventory")
    );
    let generic = config.clone().job().inventory(b"malformed");
    let dynamic: &dyn DynDecoderConfig = &config;
    let erased = dynamic.dyn_job().inventory(b"malformed");
    if cfg!(feature = "inventory") {
        let inventory = generic.unwrap();
        inventory.validate().unwrap();
        assert_eq!(inventory, erased.unwrap());
    } else {
        assert_eq!(
            generic.unwrap_err().unsupported_operation(),
            Some(&UnsupportedOperation::Inventory)
        );
        assert_eq!(
            erased.unwrap_err().unsupported_operation(),
            Some(&UnsupportedOperation::Inventory)
        );
    }
}

#[cfg(feature = "inventory")]
#[test]
fn configured_input_limit_retains_typed_cause() {
    let config = PngDecoderConfig::new();
    let limits = zencodec::ResourceLimits::none().with_max_input_bytes(1);
    let native = config
        .clone()
        .job()
        .with_limits(limits)
        .inventory(b"long")
        .unwrap_err();
    let dynamic: &dyn DynDecoderConfig = &config;
    let mut job = dynamic.dyn_job();
    job.set_limits(limits);
    let erased = job.inventory(b"long").unwrap_err();
    let expected = zencodec::LimitExceeded::InputSize { actual: 4, max: 1 };
    assert_eq!(native.limit_exceeded(), Some(&expected));
    assert_eq!(erased.limit_exceeded(), Some(&expected));
}

#[cfg(feature = "inventory")]
#[test]
fn every_stop_poll_retains_a_one_shot_timeout() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
    use zencodec::{CategorizedError, ErrorCategory};
    struct AtPoll {
        polls: Arc<AtomicUsize>,
        fail_at: usize,
    }
    impl enough::Stop for AtPoll {
        fn check(&self) -> Result<(), enough::StopReason> {
            if self.polls.fetch_add(1, Ordering::Relaxed) == self.fail_at {
                Err(enough::StopReason::TimedOut)
            } else {
                Ok(())
            }
        }
    }
    let pixels = vec![13; 64 * 64 * 3];
    let slice = zenpixels::PixelSlice::new(
        &pixels,
        64,
        64,
        64 * 3,
        zenpixels::PixelDescriptor::RGB8_SRGB,
    )
    .unwrap();
    let encoded = zenpng::PngEncoderConfig::new()
        .job()
        .encoder()
        .unwrap()
        .encode(slice)
        .unwrap();
    for data in [b"".as_slice(), b"malformed".as_slice(), encoded.data()] {
        let polls = Arc::new(AtomicUsize::new(0));
        let config = PngDecoderConfig::new();
        config
            .clone()
            .job()
            .with_pulse(zencodec::how_far::SharedPulse::new(
                zencodec::how_far::StopOnly::new(AtPoll {
                    polls: polls.clone(),
                    fail_at: usize::MAX,
                }),
            ))
            .inventory(data)
            .unwrap();
        let total = polls.load(Ordering::Relaxed);
        assert!(total > 0);
        for fail_at in 0..total {
            let stop = || {
                zencodec::how_far::SharedPulse::new(zencodec::how_far::StopOnly::new(AtPoll {
                    polls: Arc::new(AtomicUsize::new(0)),
                    fail_at,
                }))
            };
            let generic = config
                .clone()
                .job()
                .with_pulse(stop())
                .inventory(data)
                .unwrap_err();
            let dynamic: &dyn DynDecoderConfig = &config;
            let mut job = dynamic.dyn_job();
            job.set_pulse(stop());
            let erased = job.inventory(data).unwrap_err();
            let expected = ErrorCategory::Stopped(enough::StopReason::TimedOut);
            assert_eq!(generic.category(), expected, "poll {fail_at}/{total}");
            assert_eq!(
                erased.error_category(),
                Some(expected),
                "poll {fail_at}/{total}"
            );
        }
    }
}
