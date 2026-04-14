//! Parity test: `PngProbe::from_info` must produce the same `PngProbe`
//! values as the full-file scan in `crate::detect::probe`.
//!
//! The decoder's main path builds the probe from already-parsed decoder
//! state (see codec.rs `PngDecoder::decode`). That dedup is only sound
//! if the two construction paths agree on every field for the same input
//! PNG. This test walks the regression corpus and verifies that.

#![forbid(unsafe_code)]

use std::path::Path;

use enough::Unstoppable;
use zenpng::detect::{CompressionAssessment, PngProbe, probe};
use zenpng::{PngDecodeConfig, decode};

fn assert_probes_equal(path: &str, a: &PngProbe, b: &PngProbe) {
    assert_eq!(a.width, b.width, "{path}: width");
    assert_eq!(a.height, b.height, "{path}: height");
    assert_eq!(a.color_type, b.color_type, "{path}: color_type");
    assert_eq!(a.bit_depth, b.bit_depth, "{path}: bit_depth");
    assert_eq!(a.has_alpha, b.has_alpha, "{path}: has_alpha");
    assert_eq!(a.interlaced, b.interlaced, "{path}: interlaced");
    // ImageSequence doesn't implement PartialEq across all variants cleanly;
    // compare via debug format (stable round-trip for our cases).
    assert_eq!(
        format!("{:?}", a.sequence),
        format!("{:?}", b.sequence),
        "{path}: sequence"
    );
    assert_eq!(a.palette_size, b.palette_size, "{path}: palette_size");
    assert_eq!(a.creating_tool, b.creating_tool, "{path}: creating_tool");
    assert_eq!(
        a.compressed_data_size, b.compressed_data_size,
        "{path}: compressed_data_size"
    );
    assert_eq!(a.raw_data_size, b.raw_data_size, "{path}: raw_data_size");
    // compression_ratio is derived deterministically from the above; float
    // equality is safe because both paths do identical arithmetic.
    assert_eq!(
        a.compression_ratio, b.compression_ratio,
        "{path}: compression_ratio"
    );
    match (&a.compression_assessment, &b.compression_assessment) {
        (CompressionAssessment::Optimal, CompressionAssessment::Optimal) => {}
        (
            CompressionAssessment::Improvable {
                estimated_saving_pct: x,
            },
            CompressionAssessment::Improvable {
                estimated_saving_pct: y,
            },
        ) => assert_eq!(x, y, "{path}: estimated_saving_pct"),
        _ => panic!(
            "{path}: compression_assessment variant differs: {:?} vs {:?}",
            a.compression_assessment, b.compression_assessment
        ),
    }
    assert_eq!(
        format!("{:?}", a.recommendations),
        format!("{:?}", b.recommendations),
        "{path}: recommendations"
    );
}

fn check_dir(dir: &Path, checked: &mut u32) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("png") {
            continue;
        }
        let data = std::fs::read(&path).unwrap();
        let disp = path.display().to_string();

        // Decoder may legitimately reject some adversarial corpus files; skip
        // those — the parity claim is about successfully-decoded inputs.
        let Ok(decoded) = decode(&data, &PngDecodeConfig::none(), &Unstoppable) else {
            continue;
        };
        let Ok(from_scan) = probe(&data) else {
            continue;
        };
        let from_info = PngProbe::from_info(&decoded.info);
        assert_probes_equal(&disp, &from_scan, &from_info);
        *checked += 1;
    }
}

#[test]
fn probe_parity_across_corpora() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut checked = 0u32;
    check_dir(&root.join("tests/regression"), &mut checked);
    check_dir(&root.join("fuzz/corpus/fuzz_decode"), &mut checked);
    assert!(
        checked >= 20,
        "expected many corpus PNGs to exercise parity, got {checked}"
    );
    eprintln!("probe_parity_across_corpora: verified {checked} PNGs");
}
