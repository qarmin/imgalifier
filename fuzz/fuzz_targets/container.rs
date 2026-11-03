#![no_main]

use imgalifier_heif::{Container, Limits};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(c) = Container::parse(data, Limits::default()) {
        for item in &c.items {
            let _ = c.item_data(item);
        }
    }
});
