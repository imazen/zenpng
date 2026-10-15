//! `push_decoder` and one-shot `decode` are two paths for one operation:
//! on every PNG they must agree on the descriptor and the pixels.

use std::borrow::Cow;
use std::path::PathBuf;

use zencodec::decode::{Decode, DecodeJob, DecodeRowSink, DecoderConfig, SinkError};
use zenpixels::{PixelDescriptor, PixelSliceMut};
use zenpng::PngDecoderConfig;

#[derive(Default)]
struct Rows {
    width: u32,
    desc: Option<PixelDescriptor>,
    rows: Vec<u8>,
    strip: Vec<u8>,
    strip_rows: u32,
}

impl Rows {
    fn ship(&mut self) {
        let row = self.width as usize * self.desc.map_or(0, |d| d.bytes_per_pixel());
        for r in 0..self.strip_rows as usize {
            let s = &self.strip[r * row..(r + 1) * row];
            self.rows.extend_from_slice(s);
        }
        self.strip_rows = 0;
    }
}

impl DecodeRowSink for Rows {
    fn begin(&mut self, w: u32, _h: u32, d: PixelDescriptor) -> Result<(), SinkError> {
        self.width = w;
        self.desc = Some(d);
        Ok(())
    }
    fn provide_next_buffer(
        &mut self,
        _y: u32,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        self.ship();
        let row = width as usize * descriptor.bytes_per_pixel();
        self.strip.clear();
        self.strip.resize(row * height as usize, 0);
        self.strip_rows = height;
        Ok(PixelSliceMut::new(&mut self.strip, width, height, row, descriptor).unwrap())
    }
    fn finish(&mut self) -> Result<(), SinkError> {
        self.ship();
        Ok(())
    }
}

fn files() -> Vec<PathBuf> {
    let mut v = Vec::new();
    // `$ZENPNG_CODEC_CORPUS/pngsuite` (a local checkout, read-only: the i686
    // cross container cannot write the codec-corpus cache) or the cache.
    let suite = match std::env::var_os("ZENPNG_CODEC_CORPUS") {
        Some(d) => PathBuf::from(d).join("pngsuite"),
        None => codec_corpus::Corpus::new()
            .expect("codec-corpus cache unavailable")
            .github_repo("imazen/codec-corpus", "pngsuite", "main")
            .expect("fetching imazen/codec-corpus pngsuite (main) failed"),
    };
    for dir in [
        suite,
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/idot")),
    ] {
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "png") {
                v.push(p);
            }
        }
    }
    v.sort();
    assert!(v.len() > 100, "{} files", v.len());
    v
}

#[test]
fn push_decoder_matches_one_shot_descriptor_and_pixels() {
    let mut checked = 0;
    let mut mismatches = Vec::new();
    for path in files() {
        let data = std::fs::read(&path).unwrap();
        let Ok(out) = PngDecoderConfig::new()
            .job()
            .decoder(Cow::Borrowed(&data), &[])
            .and_then(|d| d.decode())
        else {
            continue;
        };
        let mut sink = Rows::default();
        let pushed =
            PngDecoderConfig::new()
                .job()
                .push_decoder(Cow::Borrowed(&data), &mut sink, &[]);
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let Ok(_) = pushed else {
            mismatches.push(format!("{name}: one-shot ok, push failed"));
            continue;
        };
        let s = out.pixels();
        let row = s.width() as usize * s.descriptor().bytes_per_pixel();
        let want: Vec<u8> = (0..s.rows())
            .flat_map(|y| s.row(y)[..row].to_vec())
            .collect();
        if sink.desc != Some(s.descriptor()) {
            mismatches.push(format!(
                "{name}: push {:?} vs one-shot {:?}",
                sink.desc,
                s.descriptor()
            ));
        } else if sink.rows != want {
            mismatches.push(format!("{name}: pixels differ"));
        }
        checked += 1;
    }
    assert!(checked > 100, "only {checked} files decoded");
    assert!(
        mismatches.is_empty(),
        "{} of {checked}:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}
