#![cfg(feature = "metadata-rewrite")]
// Reused unchanged expectations from zencodecs at aa21cba0.
use zencodec::metadata_rewrite::{Action, Error, RewriteRequest as ScrubRequest};
trait PlanRequest<'a> {
    fn plan(&self) -> Result<zencodec::metadata_rewrite::Plan<'a>, Error>;
}
impl<'a> PlanRequest<'a> for ScrubRequest<'a> {
    fn plan(&self) -> Result<zencodec::metadata_rewrite::Plan<'a>, Error> {
        zenpng::metadata::rewrite(self)
    }
}
mod png_scrub {
    use super::*;
    pub fn chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = (body.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        let mut c = !0u32;
        for &b in &out[4..] {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    (c >> 1) ^ 0xedb8_8320
                } else {
                    c >> 1
                };
            }
        }
        out.extend_from_slice(&(!c).to_be_bytes());
        out
    }
    pub fn fixture(extra: &[u8], sixteen: bool) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, 2, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(if sixteen {
                png::BitDepth::Sixteen
            } else {
                png::BitDepth::Eight
            });
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(if sixteen {
                    &[0, 1, 0x7f, 0xff, 0xff, 0xff, 0x55, 0x55, 0xaa, 0xaa, 0, 2]
                } else {
                    &[0, 1, 2, 3, 4, 255]
                })
                .unwrap();
        }
        out.splice(33..33, extra.iter().copied());
        out
    }
    fn pixels(bytes: &[u8]) -> Vec<u8> {
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let mut out = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut out).unwrap();
        out.truncate(info.buffer_size());
        out
    }
    #[test]
    fn removes_all_private_carriers_and_trailer_preserving_sdr_samples() {
        let extras = [
            chunk(b"tEXt", b"Author\0SECRET"),
            chunk(b"zTXt", b"SECRET-compressed-not-inflated"),
            chunk(b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0SECRET"),
            chunk(b"vpAg", b"SECRET-unknown"),
        ]
        .concat();
        let mut input = fixture(&extras, false);
        input.extend_from_slice(b"SECRET-trailer");
        let output = ScrubRequest::new(&input).plan().unwrap().to_vec().unwrap();
        assert!(!output.windows(6).any(|w| w == b"SECRET"));
        assert_eq!(pixels(&input), pixels(&output));
        assert_eq!(output, fixture(&[], false));
    }
    #[test]
    fn hdr_signals_and_sixteen_bit_samples_survive_exactly() {
        for transfer in [16, 18] {
            let extras = [
                chunk(b"cICP", &[9, transfer, 0, 1]),
                chunk(b"mDCV", &[0; 24]),
                chunk(b"cLLI", &[0; 8]),
            ]
            .concat();
            let input = fixture(&extras, true);
            let output = ScrubRequest::new(&input).plan().unwrap().to_vec().unwrap();
            assert_eq!(input, output);
            assert_eq!(pixels(&input), pixels(&output));
            assert_eq!(
                pixels(&output),
                [0, 1, 0x7f, 0xff, 0xff, 0xff, 0x55, 0x55, 0xaa, 0xaa, 0, 2]
            );
        }
    }
    #[test]
    fn unknown_color_is_never_relabelled() {
        let input = fixture(&chunk(b"cICP", &[255, 255, 0, 1]), true);
        assert_eq!(
            ScrubRequest::new(&input).plan().unwrap().to_vec().unwrap(),
            input
        );
    }
    #[test]
    fn invalid_crc_duplicates_critical_extensions_and_hdr_contracts_refuse() {
        for extra in [
            chunk(b"ABCD", b"unknown-critical"),
            [chunk(b"sRGB", &[0]), chunk(b"sRGB", &[0])].concat(),
            chunk(b"cICP", &[9, 16, 9, 0]),
            chunk(b"mDCV", &[0; 24]),
        ] {
            assert!(ScrubRequest::new(&fixture(&extra, false)).plan().is_err());
        }
        let mut input = fixture(&[], false);
        input[29] ^= 1;
        assert!(matches!(
            ScrubRequest::new(&input).plan(),
            Err(Error::Malformed("PNG CRC"))
        ));
    }
    #[test]
    fn every_truncated_prefix_refuses() {
        let input = fixture(&[], false);
        for end in 0..input.len() {
            assert!(ScrubRequest::new(&input[..end]).plan().is_err(), "{end}");
        }
    }
    #[test]
    fn compressed_icc_is_opaque_and_profile_name_is_scrubbed_without_inflation() {
        let input = fixture(
            &chunk(b"iCCP", b"SECRET profile name\0\0opaque-zlib"),
            false,
        );
        let req = ScrubRequest::new(&input);
        let plan = req.plan().unwrap();
        assert!(
            plan.report()
                .iter()
                .any(|e| e.action() == Action::OpaqueIcc)
        );
        let output = plan.to_vec().unwrap();
        assert!(!output.windows(6).any(|w| w == b"SECRET"));
        assert!(output.windows(11).any(|w| w == b"opaque-zlib"));
        assert!(
            ScrubRequest::new(&input)
                .with_icc_normalization(true)
                .plan()
                .is_err()
        );
    }
    #[test]
    fn animation_all_frames_and_timing_survive() {
        let mut input = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut input, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Sixteen);
            encoder.set_animated(2, 3).unwrap();
            encoder.set_frame_delay(17, 1000).unwrap();
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[0, 1, 0, 2, 0, 3, 255, 255])
                .unwrap();
            writer.set_frame_delay(19, 1000).unwrap();
            writer
                .write_image_data(&[0, 4, 0, 5, 0, 6, 255, 255])
                .unwrap();
        }
        let expected = input.clone();
        input.splice(33..33, chunk(b"tEXt", b"Author\0SECRET"));
        let output = ScrubRequest::new(&input).plan().unwrap().to_vec().unwrap();
        assert_eq!(output, expected);
        let mut reader = png::Decoder::new(std::io::Cursor::new(&output))
            .read_info()
            .unwrap();
        for (delay, want) in [
            (17, [0, 1, 0, 2, 0, 3, 255, 255]),
            (19, [0, 4, 0, 5, 0, 6, 255, 255]),
        ] {
            let mut buf = [0; 8];
            reader.next_frame(&mut buf).unwrap();
            assert_eq!(buf, want);
            assert_eq!(
                reader.info().frame_control.as_ref().unwrap().delay_num,
                delay
            );
        }
    }
}

#[test]
fn timeout_survives_each_crc_and_container_poll() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Deadline {
        polls: AtomicUsize,
        at: usize,
    }
    impl enough::Stop for Deadline {
        fn check(&self) -> Result<(), enough::StopReason> {
            if self.polls.fetch_add(1, Ordering::Relaxed) == self.at {
                Err(enough::StopReason::TimedOut)
            } else {
                Ok(())
            }
        }
    }
    let input = png_scrub::fixture(&png_scrub::chunk(b"tEXt", &vec![b'X'; 131072]), false);
    let count = Deadline {
        polls: AtomicUsize::new(0),
        at: usize::MAX,
    };
    let pulse = zencodec::how_far::StopOnly::borrowed(&count);
    zenpng::metadata::rewrite(&ScrubRequest::new(&input).with_pulse(&pulse)).unwrap();
    for at in 0..count.polls.load(Ordering::Relaxed) {
        let deadline = Deadline {
            polls: AtomicUsize::new(0),
            at,
        };
        assert!(
            matches!(
                zenpng::metadata::rewrite(
                    &ScrubRequest::new(&input)
                        .with_pulse(&zencodec::how_far::StopOnly::borrowed(&deadline))
                ),
                Err(Error::Stopped(enough::StopReason::TimedOut))
            ),
            "poll {at}"
        );
    }
}
