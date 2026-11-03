#![no_main]

use imgalifier_heif::hevc::HevcBackend;
use imgalifier_heif::{Decoder, Limits};

// Small limits keep every input fast and below the fuzzer's RSS cap.
const LIMITS: Limits = Limits { max_items: 256, max_item_bytes: 16 << 20, max_pixels: 1 << 22, max_grid_tiles: 256, max_threads: 1 };

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(d) = Decoder::new(data, HevcBackend::default(), LIMITS) {
        let _ = d.decode_primary();
    }
});
