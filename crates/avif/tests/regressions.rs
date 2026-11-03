//! Inputs found by fuzzing; each must return (an error) promptly instead of hanging or panicking.

use std::time::{Duration, Instant};

use imgalifier_avif::decode_avif;

fn assert_prompt_error(name: &str, data: &[u8]) {
    let start = Instant::now();
    let result = decode_avif(data);
    assert!(start.elapsed() < Duration::from_secs(5), "{name} took too long");
    assert!(result.is_err(), "{name} unexpectedly decoded");
}

#[test]
fn av1_stream_without_a_picture_does_not_spin() {
    assert_prompt_error("hang_no_picture", include_bytes!("fixtures/hang_no_picture.avif"));
}

#[test]
fn empty_av1_item_is_rejected_before_reaching_the_decoder() {
    assert_prompt_error("empty_av1_item", include_bytes!("fixtures/empty_av1_item.avif"));
}
