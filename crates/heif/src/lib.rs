//! Pure-Rust HEIF/HEIC decoder.
//!
//! The crate is split into a codec-independent part (container, grid, alpha, transforms, colour conversion;
//! reused by `imgalifier-avif`) and the HEVC still-picture decoder behind the `hevc` feature.

mod container;
mod convert;
mod decode;
mod error;
mod image;
#[cfg(feature = "image")]
pub mod image_support;
mod overlay;
mod reader;
mod transform;
mod uncompressed;

#[cfg(feature = "hevc")]
pub mod hevc;

pub use container::{
    Clap, Colr, ConstructionMethod, Container, Extent, FourCc, GridInfo, Item, OverlayInfo, Property, SequenceFrame,
    UncC, UncComponent,
};
pub use convert::{yuv_to_rgba, yuv_to_rgba_threaded};
pub use decode::{CodecBackend, Decoder, ImageInfo};
pub use error::{Error, Limits, Result};
pub use image::{ChromaFormat, ColorInfo, RgbaImage, YuvImage};
pub use transform::apply_transformative_properties;

/// Decodes the primary image of a HEIC/HEIF file to 8-bit RGBA.
#[cfg(feature = "hevc")]
pub fn decode_heic(data: &[u8]) -> Result<RgbaImage> {
    Decoder::new(data, hevc::HevcBackend::default(), Limits::default())?.decode_primary()
}

/// Reads size and format of the primary image of a HEIC/HEIF file without decoding it.
#[cfg(feature = "hevc")]
pub fn heic_info(data: &[u8]) -> Result<ImageInfo> {
    Decoder::new(data, hevc::HevcBackend::default(), Limits::default())?.info()
}

/// Registers the HEIC/HEIF decoder with the `image` crate for the `heic`, `heif` and `hif` extensions.
/// Returns false when another decoder already claimed one of them.
#[cfg(all(feature = "hevc", feature = "image"))]
pub fn register_decoding_hooks() -> bool {
    register_decoding_hooks_with(Limits::default())
}

/// Like [`register_decoding_hooks`] with custom limits, e.g. `max_threads: 1` when the caller already decodes many
/// files in parallel. The decoders are lazy: size and colour type come from the header, pixels are decoded in
/// `read_image` after `image::Limits` were applied. Files without an `ftyp` box (a JPEG named `.heic`) are passed to
/// the other decoders registered with `image`.
#[cfg(all(feature = "hevc", feature = "image"))]
pub fn register_decoding_hooks_with(limits: Limits) -> bool {
    fn info(data: &[u8], limits: Limits) -> Result<ImageInfo> {
        Decoder::new(data, hevc::HevcBackend::default(), limits)?.info()
    }
    fn decode(data: &[u8], limits: Limits) -> Result<RgbaImage> {
        Decoder::new(data, hevc::HevcBackend::default(), limits)?.decode_primary()
    }
    image_support::register(image_support::Format {
        name: "heic",
        extensions: &["heic", "heif", "hif"],
        brands: &[b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"mif1", b"msf1"],
        limits,
        info,
        decode,
    })
}
