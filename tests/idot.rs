//! Apple `iDOT` parallel decode: every file in the `png-idot` corpus must
//! decode to exactly what the single-threaded decoder produces.
//!
//! Corpus: `codec-corpus` dataset `png-idot` (real Apple files, Buchanan's
//! adversarial "ambiguous PNG" samples, and generated cases with a
//! `manifest.json`). Set `ZENPNG_IDOT_CORPUS=<dir>` to use a local copy
//! instead of the cached download.
//!
//! With the `_dev` feature the test also checks *which* files took the
//! parallel path: every Apple file and every manifest entry marked
//! `parallel_usable`, and none of the others.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use zenflate::Unstoppable;
use zenpng::{PngDecodeConfig, decode};

fn corpus_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("ZENPNG_IDOT_CORPUS") {
        return PathBuf::from(d);
    }
    // `png-idot` landed on codec-corpus main after the crate's v1.1.0 data
    // tag, so `Corpus::get("png-idot")` cannot see it yet. Switch to
    // `.get("png-idot")` once a codec-corpus release includes the dataset.
    codec_corpus::Corpus::new()
        .expect("codec-corpus cache unavailable")
        .github_repo("imazen/codec-corpus", "png-idot", "main")
        .expect("fetching imazen/codec-corpus png-idot (main) failed")
}

fn pngs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .collect();
    v.sort();
    assert!(!v.is_empty(), "no PNGs in {}", dir.display());
    v
}

/// `manifest.json` → (file name, parallel_usable). Tiny scanner; the file is
/// generated with one key per line.
fn manifest(dir: &Path) -> Vec<(String, bool)> {
    let text = std::fs::read_to_string(dir.join("manifest.json")).unwrap();
    let mut out = Vec::new();
    let mut current = None;
    for line in text.lines() {
        let l = line.trim();
        if l.ends_with(": {") && l.starts_with('"') {
            current = Some(l.trim_end_matches(": {").trim_matches('"').to_string());
        } else if let Some(rest) = l.strip_prefix("\"parallel_usable\": ") {
            out.push((current.take().unwrap(), rest.starts_with("true")));
        }
    }
    assert!(!out.is_empty());
    out
}

/// Decode serially and with each thread budget; results must be identical.
fn assert_parallel_equals_serial(path: &Path, base: PngDecodeConfig) {
    let data = std::fs::read(path).unwrap();
    let serial = decode(&data, &base.clone().with_max_threads(1), &Unstoppable);
    for threads in [0usize, 2, 3, 8] {
        let par = decode(&data, &base.clone().with_max_threads(threads), &Unstoppable);
        match (&serial, &par) {
            (Ok(s), Ok(p)) => {
                assert!(
                    s.pixels.copy_to_contiguous_bytes() == p.pixels.copy_to_contiguous_bytes(),
                    "{}: pixels differ with max_threads={threads}",
                    path.display()
                );
                assert_eq!(
                    s.warnings,
                    p.warnings,
                    "{}: warnings differ",
                    path.display()
                );
                assert_eq!(s.info.width, p.info.width);
                assert_eq!(s.info.height, p.info.height);
            }
            (Err(s), Err(p)) => {
                assert_eq!(
                    format!("{:?}", s.error()),
                    format!("{:?}", p.error()),
                    "{}: errors differ",
                    path.display()
                );
            }
            _ => panic!(
                "{}: serial ok={} but max_threads={threads} ok={}",
                path.display(),
                serial.is_ok(),
                par.is_ok()
            ),
        }
    }
}

#[test]
fn idot_corpus_parallel_equals_serial() {
    let root = corpus_dir();
    let can_parallel = std::thread::available_parallelism().map_or(1, |n| n.get()) >= 2;

    let mut expectations: Vec<(PathBuf, bool)> = Vec::new();
    for p in pngs(&root.join("apple")) {
        expectations.push((p, true));
    }
    for p in pngs(&root.join("adversarial")) {
        // Both are 1440 px wide RGB with segment tables that do not describe
        // the serial stream: they must never complete in parallel.
        expectations.push((p, false));
    }
    let synth = root.join("synthetic");
    let listed = manifest(&synth);
    assert_eq!(
        listed.len(),
        pngs(&synth).len(),
        "manifest and folder disagree"
    );
    for (name, usable) in listed {
        expectations.push((synth.join(name), usable));
    }

    for (path, usable) in &expectations {
        #[cfg(feature = "_dev")]
        let before = zenpng::__idot_stats();

        assert_parallel_equals_serial(path, PngDecodeConfig::default());
        assert_parallel_equals_serial(path, PngDecodeConfig::strict());

        #[cfg(feature = "_dev")]
        {
            let completed = zenpng::__idot_stats().0 - before.0;
            if *usable && can_parallel {
                assert!(
                    completed > 0,
                    "{}: expected the parallel path to complete",
                    path.display()
                );
            } else {
                assert_eq!(
                    completed,
                    0,
                    "{}: parallel path completed on a file it must not use",
                    path.display()
                );
            }
        }
        #[cfg(not(feature = "_dev"))]
        let _ = (usable, can_parallel);
    }
}
