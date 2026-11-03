//! Times decode_heic over the given files: bench <files...>

#![expect(clippy::print_stdout, reason = "developer tool")]

use std::time::Instant;

fn main() {
    for path in std::env::args().skip(1) {
        let Ok(data) = std::fs::read(&path) else { continue };
        let t = Instant::now();
        let r = imgalifier_heif::decode_heic(&data);
        let dt = t.elapsed();
        match r {
            Ok(img) => println!(
                "{:>8.1} ms {:>5.1} MP/s {}x{} {}",
                dt.as_secs_f64() * 1e3,
                f64::from(img.width) * f64::from(img.height) / 1e6 / dt.as_secs_f64(),
                img.width,
                img.height,
                path
            ),
            Err(e) => println!("error {e} {path}"),
        }
    }
}
