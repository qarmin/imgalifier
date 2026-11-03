#![no_main]

use imgalifier_avif::{Av1Backend, Decoder, Limits};

const LIMITS: Limits = Limits { max_items: 256, max_item_bytes: 16 << 20, max_pixels: 1 << 18, max_grid_tiles: 256, max_threads: 1 };

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let backend = Av1Backend { threads: 1, max_pixels: 1 << 18 };
    if let Ok(d) = Decoder::new(data, backend, LIMITS) {
        let _ = d.decode_primary();
    }
});
