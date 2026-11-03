//! Pure-Rust AVIF decoder: the HEIF container and pipeline come from `imgalifier-heif`, this crate adds the AV1
//! still-picture backend.

mod backend;
mod config;

pub mod rav1d;

pub use backend::Av1Backend;
pub use config::Av1Config;
pub use imgalifier_heif::{Decoder, Error, ImageInfo, Limits, Result, RgbaImage, YuvImage};

fn backend_for(limits: &Limits) -> Av1Backend {
    Av1Backend {
        max_pixels: u32::try_from(limits.max_pixels).unwrap_or(u32::MAX),
        ..Av1Backend::default()
    }
}

/// Decodes the primary image of an AVIF file to 8-bit RGBA.
pub fn decode_avif(data: &[u8]) -> Result<RgbaImage> {
    Decoder::new(data, backend_for(&Limits::default()), Limits::default())?.decode_primary()
}

/// Reads size and format of the primary image of an AVIF file without decoding it.
pub fn avif_info(data: &[u8]) -> Result<ImageInfo> {
    Decoder::new(data, backend_for(&Limits::default()), Limits::default())?.info()
}

/// Registers the AVIF decoder with the `image` crate for the `avif` extension.
/// Returns false when another decoder already claimed it.
#[cfg(feature = "image")]
pub fn register_decoding_hooks() -> bool {
    register_decoding_hooks_with(Limits::default())
}

/// Like [`register_decoding_hooks`] with custom limits, e.g. `max_threads: 1` when the caller already decodes many
/// files in parallel. The decoder is lazy: size and alpha come from the header, pixels are decoded in `read_image`
/// after `image::Limits` were applied. Files without an `ftyp` box are passed to the other decoders registered with
/// `image`.
#[cfg(feature = "image")]
pub fn register_decoding_hooks_with(limits: Limits) -> bool {
    fn info(data: &[u8], limits: Limits) -> Result<ImageInfo> {
        Decoder::new(data, backend_for(&limits), limits)?.info()
    }
    fn decode(data: &[u8], limits: Limits) -> Result<RgbaImage> {
        Decoder::new(data, backend_for(&limits), limits)?.decode_primary()
    }
    imgalifier_heif::image_support::register(imgalifier_heif::image_support::Format {
        name: "avif",
        extensions: &["avif"],
        brands: &[b"avif", b"avis"],
        limits,
        info,
        decode,
    })
}
