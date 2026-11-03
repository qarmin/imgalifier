//! See the HEIC counterpart: tests that need a real file are skipped unless IMGALIFIER_SAMPLES points at the Czkawka `zune_heic_test/samples` directory.

#![cfg(feature = "image")]

use std::path::PathBuf;
use std::sync::Once;

fn register() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| assert!(imgalifier_avif::register_decoding_hooks()));
}

fn sample(rel: &str) -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("IMGALIFIER_SAMPLES")?).join(rel);
    path.exists().then_some(path)
}

#[test]
fn image_crate_opens_avif_through_the_hook() {
    let Some(path) = sample("link-u_avif-sample-images/kimono.avif") else {
        return;
    };
    register();
    let img = image::open(&path).unwrap();
    let reference = imgalifier_avif::decode_avif(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!((img.width(), img.height()), (reference.width, reference.height));
}

#[test]
fn limits_stop_the_decode_before_it_starts() {
    let Some(path) = sample("link-u_avif-sample-images/kimono.avif") else {
        return;
    };
    register();
    let mut reader = image::ImageReader::open(&path).unwrap();
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(1024);
    reader.limits(limits);
    assert!(matches!(reader.decode(), Err(image::ImageError::Limits(_))));
}
