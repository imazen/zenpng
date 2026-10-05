//! zenpng against Apple ImageIO's own decode of the tiny iDOT fixtures.
//!
//! `tests/fixtures/idot/mac/imageio_macos27.tsv` was recorded on macOS 27.0
//! (26A428) with `tests/fixtures/idot/mac/imageio_tool.swift`: for every
//! fixture, the SHA-256 of ImageIO's decoded buffer with the file as given
//! and with its iDOT chunk removed (ImageIO's serial decoder). zenpng's
//! output, converted to ImageIO's buffer layout, must hash to the serial
//! value for every file both decode. ImageIO's iDOT path itself diverges on
//! some fixtures (boundary rows using Up/Average/Paeth, gapped tables, 1/2/4
//! bit gray); those are ImageIO bugs, recorded but not asserted here.

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

use sha2::{Digest, Sha256};
use zenflate::Unstoppable;
use zenpng::{PngDecodeConfig, decode};

mod common;
use common::imageio::{expand_indexed, to_imageio};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/idot")
}

fn sha256_hex(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

/// One palette index per pixel, unpacked from the file's scanlines.
fn palette_indices(data: &[u8]) -> Vec<u8> {
    let mut d = png::Decoder::new(std::io::Cursor::new(data));
    d.set_transformations(png::Transformations::IDENTITY);
    let mut r = d.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    let bits = info.bit_depth as u8 as usize;
    let mut idx = Vec::new();
    for row in buf[..info.buffer_size()].chunks(info.line_size) {
        for x in 0..info.width as usize {
            let bit = x * bits;
            idx.push((row[bit / 8] >> (8 - bits - bit % 8)) & ((1u16 << bits) - 1) as u8);
        }
    }
    idx
}

#[test]
fn zenpng_matches_imageio_serial_decode() {
    let tsv = std::fs::read_to_string(dir().join("mac/imageio_macos27.tsv")).unwrap();
    let mut checked = 0;
    for line in tsv.lines().filter(|l| !l.starts_with('#')).skip(1) {
        let f: Vec<&str> = line.split('\t').collect();
        let (name, bpc, bpp, alpha, apple_serial) = (
            f[0],
            f[4].parse::<usize>().unwrap(),
            f[5].parse::<usize>().unwrap(),
            f[6].parse::<u32>().unwrap(),
            f[9],
        );
        let data = std::fs::read(dir().join(name)).unwrap();
        let ours = decode(
            &data,
            &PngDecodeConfig::default().with_max_threads(1),
            &Unstoppable,
        );
        let Ok(ours) = ours else {
            // zenpng rejects only the files whose image data ends early;
            // ImageIO pads those. Make sure that's all it is.
            assert!(
                name.contains("segment0"),
                "{name}: zenpng rejected a file ImageIO decodes"
            );
            continue;
        };
        if apple_serial == "-" {
            continue;
        }
        if data[25] == 3 && bpp == 8 {
            // ImageIO kept a 1/2-bit palette image indexed: its hash covers
            // the index buffer. Check the indices (via the png crate) against
            // it, then zenpng's RGBA against those indices through PLTE/tRNS.
            let idx = palette_indices(&data);
            assert_eq!(sha256_hex(&idx), apple_serial, "{name}: indices vs ImageIO");
            let (mine, _) = to_imageio(&ours.pixels, 8, 32, 3).unwrap();
            assert_eq!(
                mine,
                expand_indexed(&data, &idx).unwrap(),
                "{name}: RGBA vs ImageIO"
            );
        } else {
            let (mine, _) =
                to_imageio(&ours.pixels, bpc, bpp, alpha).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                sha256_hex(&mine),
                apple_serial,
                "{name}: differs from ImageIO's serial decode"
            );
        }
        checked += 1;
    }
    assert!(checked >= 40, "only {checked} fixtures checked");
}
