//! Decodes every AVIF sample and compares the result pixel by pixel with libheif reference PNGs.
//!
//! Usage: compare <samples_dir> <reference_out_dir> [name_filter]
//! The reference for `samples/<origin>/<file>` is `<out>/<origin>__<file>/libheif.png` (layout of the Czkawka
//! `zune_heic_test` harness). Prints one line per file and a summary.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::indexing_slicing,
    reason = "developer tool"
)]

use std::fmt::Write as _;
use std::panic::catch_unwind;
use std::path::Path;

use imgalifier_avif::decode_avif;

fn load_png(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let mut dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let bytes = buf.get(..info.buffer_size())?;
    let step = if info.bit_depth == png::BitDepth::Sixteen { 2 } else { 1 };
    let samples: Vec<u8> = bytes.chunks(step).map(|c| c[0]).collect();
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => samples,
        png::ColorType::Rgb => samples.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::Grayscale => samples.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => samples.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((info.width, info.height, rgba))
}

/// Reference dumped by a libavif helper: u32 LE width, height, transform flags, then RGBA8 (libavif does not apply
/// irot/imir/clap itself, so files with transforms are only compared when the sizes happen to agree).
fn load_rgba(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let bytes = std::fs::read(path).ok()?;
    let word = |i: usize| {
        bytes
            .get(i * 4..i * 4 + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    Some((word(0)?, word(1)?, bytes.get(12..)?.to_vec()))
}

fn classify(a: &[u8], b: &[u8]) -> (&'static str, u8, f64) {
    let mut max = 0u8;
    let mut sq = 0f64;
    for (x, y) in a.iter().zip(b) {
        let d = x.abs_diff(*y);
        max = max.max(d);
        sq += f64::from(d) * f64::from(d);
    }
    let mse = sq / a.len().max(1) as f64;
    let psnr = if mse == 0.0 {
        99.0
    } else {
        10.0 * (255.0f64 * 255.0 / mse).log10()
    };
    let class = match (max, psnr) {
        (0, _) => "identical",
        (1..=2, _) => "close",
        (_, p) if p >= 40.0 => "minor",
        _ => "DIFFERENT",
    };
    (class, max, psnr)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (samples, out) = (Path::new(&args[1]), Path::new(&args[2]));
    let filter = args.get(3).cloned().unwrap_or_default();
    let mut summary = std::collections::BTreeMap::<String, u32>::new();
    let mut files = Vec::new();
    for origin in std::fs::read_dir(samples).into_iter().flatten().flatten() {
        for f in std::fs::read_dir(origin.path()).into_iter().flatten().flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            if ["avif"].contains(&ext.as_str()) && name.contains(&filter) {
                let key = format!("{}__{name}", origin.file_name().to_string_lossy());
                files.push((key, f.path()));
            }
        }
    }
    files.sort();
    for (key, path) in files {
        let reference = match std::env::var_os("LIBAVIF_REF").or_else(|| std::env::var_os("LIBHEIF_REF")) {
            Some(dir) => load_rgba(&Path::new(&dir).join(format!("{key}.rgba"))),
            None => load_png(&out.join(&key).join("libheif.png")),
        };
        let Ok(data) = std::fs::read(&path) else { continue };
        let result = catch_unwind(|| decode_avif(&data));
        let (class, detail) = match (result, reference) {
            (Err(_), _) => ("PANIC".to_owned(), String::new()),
            (Ok(Err(e)), Some(_)) => ("ERROR(libheif ok)".to_owned(), e.to_string()),
            (Ok(Err(e)), None) => ("both failed".to_owned(), e.to_string()),
            (Ok(Ok(img)), None) => (
                "libheif failed, ours ok".to_owned(),
                format!("{}x{}", img.width, img.height),
            ),
            (Ok(Ok(img)), Some((w, h, _))) if (img.width, img.height) != (w, h) => (
                "SIZE MISMATCH".to_owned(),
                format!("ours {}x{} libheif {w}x{h}", img.width, img.height),
            ),
            (Ok(Ok(img)), Some((_, _, rgba))) => {
                let (c, max, psnr) = classify(&img.data, &rgba);
                let mut sums = [0u64; 4];
                for (i, (x, y)) in img.data.iter().zip(&rgba).enumerate() {
                    sums[i % 4] += u64::from(x.abs_diff(*y));
                }
                let n = (img.data.len() / 4).max(1) as u64;
                let m = sums.map(|v| v / n);
                (
                    c.to_owned(),
                    format!(
                        "max {max} psnr {psnr:.1} mean|d| rgba {m:?} px0 {:?} vs {:?}",
                        &img.data[..4],
                        &rgba[..4]
                    ),
                )
            }
        };
        let mut line = String::new();
        let _ = write!(line, "{class:<24} {key} {detail}");
        println!("{line}");
        *summary.entry(class).or_default() += 1;
    }
    eprintln!("--- summary");
    for (k, v) in summary {
        eprintln!("{v:>5} {k}");
    }
}
