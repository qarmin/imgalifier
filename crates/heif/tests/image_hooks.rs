//! The `image` crate integration. Tests that need a real file use the Czkawka `zune_heic_test/samples` dir
//! (set IMGALIFIER_SAMPLES to its `samples` directory) and are skipped when the variable is not set.

#![cfg(feature = "image")]

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Once;

fn register() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| assert!(imgalifier_heif::register_decoding_hooks()));
}

fn sample(rel: &str) -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("IMGALIFIER_SAMPLES")?).join(rel);
    path.exists().then_some(path)
}

#[test]
fn image_crate_opens_heic_through_the_hook() {
    let Some(path) = sample("nokiatech_heif_conformance/conformance_files__C001.heic") else {
        return;
    };
    register();
    let img = image::open(&path).unwrap();
    let reference = imgalifier_heif::decode_heic(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!((img.width(), img.height()), (reference.width, reference.height));
    assert_eq!(
        img.to_rgb8().get_pixel(0, 0).0,
        [reference.data[0], reference.data[1], reference.data[2]]
    );
}

#[test]
fn second_registration_reports_the_clash() {
    register();
    assert!(!imgalifier_heif::register_decoding_hooks());
}

#[test]
fn dimension_limits_are_applied_before_any_pixels_are_decoded() {
    let Some(path) = sample("nokiatech_heif_conformance/conformance_files__C001.heic") else {
        return;
    };
    register();
    let mut reader = image::ImageReader::open(&path).unwrap();
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16);
    reader.limits(limits);
    assert!(matches!(reader.decode(), Err(image::ImageError::Limits(_))));
}

#[test]
fn mis_named_files_fall_back_to_the_other_decoders() {
    register();
    let dir = std::env::temp_dir().join(format!("imgalifier_fallback_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("actually_a_bmp.heic");
    let mut bytes = Vec::new();
    image::RgbImage::from_pixel(3, 2, image::Rgb([10, 20, 30]))
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Bmp)
        .unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let img = image::open(&path).unwrap();
    assert_eq!((img.width(), img.height()), (3, 2));
    assert_eq!(img.to_rgb8().get_pixel(1, 1).0, [10, 20, 30]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn garbage_with_a_heic_name_is_an_error_not_a_panic() {
    register();
    let dir = std::env::temp_dir().join(format!("imgalifier_garbage_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("garbage.heic");
    std::fs::write(&path, [0xde, 0xad, 0xbe, 0xef, 1, 2, 3]).unwrap();
    image::open(&path).unwrap_err();
    std::fs::remove_dir_all(&dir).unwrap();
}
