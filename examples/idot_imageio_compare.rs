//! Compare Apple ImageIO's decoded pixels (dumped on a Mac by
//! `tests/fixtures/idot/mac/imageio_tool.swift`) with zenpng's.
//!
//! zenpng's output is converted into ImageIO's buffer layout (ImageIO widens
//! some formats, e.g. palette -> RGBA8, RGB8 -> RGBX8, 16-bit little-endian)
//! and compared byte for byte. RGBX padding bytes are ignored.
//!
//! Usage:
//!   cargo run --release --example idot_imageio_compare --features _dev -- \
//!     decode.tsv RAW_DIR idot_errors.tsv GROUP=DIR...
//!
//! Each `in/<group>/<file>` from decode.tsv is read from the DIR given for
//! its group (e.g. `tiny=tests/fixtures/idot`); a group may list several
//! comma-separated dirs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use zenflate::Unstoppable;
use zenpng::{PngDecodeConfig, decode};

#[path = "../tests/common/imageio.rs"]
mod imageio;
use imageio::{expand_indexed, to_imageio};

fn find(group: &str, name: &str, dirs: &BTreeMap<String, Vec<PathBuf>>) -> Option<PathBuf> {
    dirs.get(group)?
        .iter()
        .map(|d| d.join(name))
        .find(|p| p.exists())
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let tsv = std::fs::read_to_string(&a[0]).unwrap();
    let raw_dir = PathBuf::from(&a[1]);
    let mut errors: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for l in std::fs::read_to_string(&a[2]).unwrap().lines() {
        if let Some((f, e)) = l.split_once('\t') {
            let e = e.to_string();
            let v = errors.entry(f.to_string()).or_default();
            if !v.contains(&e) {
                v.push(e);
            }
        }
    }
    let dirs: BTreeMap<String, Vec<PathBuf>> = a[3..]
        .iter()
        .map(|g| {
            let (k, v) = g.split_once('=').expect("GROUP=DIR");
            (k.to_string(), v.split(',').map(PathBuf::from).collect())
        })
        .collect();

    println!(
        "file\tzenpng_serial\tzenpng_parallel==serial\tzenpng_vs_imageio_serial\timageio_idot_path_vs_its_serial\timageio_idot_log"
    );
    for line in tsv.lines().skip(1) {
        let f: Vec<&str> = line.split('\t').collect();
        let rel = f[0];
        let name = Path::new(rel)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let group = Path::new(rel)
            .parent()
            .and_then(|p| p.file_name())
            .map(|g| g.to_string_lossy().to_string())
            .unwrap_or_default();
        let (bpc, bpp, alpha): (usize, usize, u32) = (
            f[4].parse().unwrap_or(0),
            f[5].parse().unwrap_or(0),
            f[6].parse().unwrap_or(0),
        );
        let apple_self = match (f[8], f[9]) {
            ("-", "-") => "ImageIO rejects both".to_string(),
            ("-", _) => "ImageIO rejects only with iDOT".into(),
            (_, "-") => "ImageIO decodes only with iDOT".into(),
            _ if f[10] == "1" => "same".into(),
            _ => {
                let a =
                    std::fs::read(raw_dir.join(format!("{group}__{name}.raw"))).unwrap_or_default();
                let b = std::fs::read(raw_dir.join(format!("{group}__{name}.noidot.raw")))
                    .unwrap_or_default();
                let row = (f[2].parse::<usize>().unwrap_or(1) * bpp / 8).max(1);
                let first = a.iter().zip(&b).position(|(x, y)| x != y).unwrap_or(0);
                let n = a.iter().zip(&b).filter(|(x, y)| x != y).count();
                format!("DIFFERS: {n} bytes from row {}", first / row)
            }
        };
        let log = errors.get(rel).map(|v| v.join("; ")).unwrap_or_default();
        let Some(path) = find(&group, &name, &dirs) else {
            println!("{group}/{name}\t?\t?\tlocal file not found\t{apple_self}\t{log}");
            continue;
        };
        let data = std::fs::read(&path).unwrap();
        let serial = decode(
            &data,
            &PngDecodeConfig::default().with_max_threads(1),
            &Unstoppable,
        );
        zenpng::__set_idot_min_bytes(1);
        let par = decode(
            &data,
            &PngDecodeConfig::default().with_max_threads(0),
            &Unstoppable,
        );
        zenpng::__set_idot_min_bytes(0);
        let par_eq = match (&serial, &par) {
            (Ok(s), Ok(p)) => {
                s.pixels.copy_to_contiguous_bytes() == p.pixels.copy_to_contiguous_bytes()
            }
            (Err(_), Err(_)) => true,
            _ => false,
        };
        let vs_apple = match (&serial, f[9]) {
            (Err(e), "-") => format!("both reject ({})", e.error()),
            (Err(e), _) => format!("zenpng rejects ({}), ImageIO decodes", e.error()),
            (Ok(_), "-") => "ImageIO rejects, zenpng decodes".into(),
            (Ok(s), _) => {
                let mut apple = std::fs::read(raw_dir.join(format!("{group}__{name}.noidot.raw")))
                    .unwrap_or_default();
                let (mut bpc, mut bpp, mut alpha) = (bpc, bpp, alpha);
                let indexed = data.get(25) == Some(&3) && bpp == 8;
                if indexed && let Some(rgba) = expand_indexed(&data, &apple) {
                    apple = rgba;
                    (bpc, bpp, alpha) = (8, 32, 3);
                }
                match to_imageio(&s.pixels, bpc, bpp, alpha) {
                    Err(e) => format!("not compared: {e}"),
                    Ok((mine, _)) if mine.len() != apple.len() => {
                        format!(
                            "SIZE DIFFERS zenpng {} vs ImageIO {}",
                            mine.len(),
                            apple.len()
                        )
                    }
                    Ok((mine, mask)) => {
                        let diffs: Vec<usize> = (0..mine.len())
                            .filter(|&i| mask[i] && mine[i] != apple[i])
                            .collect();
                        if diffs.is_empty() {
                            "IDENTICAL".into()
                        } else {
                            let row = (s.info.width as usize * bpp / 8).max(1);
                            format!("DIFFERS: {} bytes from row {}", diffs.len(), diffs[0] / row)
                        }
                    }
                }
            }
        };
        println!(
            "{group}/{name}\t{}\t{par_eq}\t{vs_apple}\t{apple_self}\t{log}",
            if serial.is_ok() { "ok" } else { "err" }
        );
    }
}
