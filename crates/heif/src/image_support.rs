//! Integration with the `image` crate: lazy decoders that honour `image::Limits`, and hook registration.

use std::ffi::OsString;
use std::io::Read;

use image::error::{DecodingError, ImageFormatHint, LimitError, LimitErrorKind};
use image::hooks::{GenericReader, register_decoding_hook, register_format_detection_hook};
use image::{ColorType, ImageDecoder, ImageError, ImageResult};

use crate::decode::ImageInfo;
use crate::error::{Limits, Result};
use crate::image::RgbaImage;

/// Describes how to register one format (HEIC or AVIF) with the `image` crate.
#[derive(Clone, Copy)]
pub struct Format {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    /// `ftyp` major brands used to recognise files by content.
    pub brands: &'static [&'static [u8; 4]],
    pub limits: Limits,
    /// Cheap header-only inspection (size, alpha); no pixels are decoded.
    pub info: fn(&[u8], Limits) -> Result<ImageInfo>,
    pub decode: fn(&[u8], Limits) -> Result<RgbaImage>,
}

enum State {
    Lazy { data: Vec<u8>, format: Format },
    Ready(RgbaImage),
}

/// Decoder that knows the size and colour type from the header and decodes the pixels only in `read_image`,
/// after the caller had the chance to set limits.
pub struct HeifImageDecoder {
    width: u32,
    height: u32,
    has_alpha: bool,
    state: State,
}

impl HeifImageDecoder {
    fn ready(image: RgbaImage, has_alpha: bool) -> Self {
        Self {
            width: image.width,
            height: image.height,
            has_alpha,
            state: State::Ready(image),
        }
    }

    fn decoding_error(format: &str, e: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> ImageError {
        ImageError::Decoding(DecodingError::new(ImageFormatHint::Name(format.to_owned()), e))
    }

    fn bytes_per_pixel(&self) -> u64 {
        if self.has_alpha { 4 } else { 3 }
    }
}

impl ImageDecoder for HeifImageDecoder {
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn color_type(&self) -> ColorType {
        if self.has_alpha {
            ColorType::Rgba8
        } else {
            ColorType::Rgb8
        }
    }

    fn set_limits(&mut self, limits: image::Limits) -> ImageResult<()> {
        limits.check_dimensions(self.width, self.height)?;
        if let Some(max_alloc) = limits.max_alloc {
            // the RGBA picture produced by the decoder is allocated before it is narrowed to RGB
            if u64::from(self.width) * u64::from(self.height) * 4 > max_alloc.max(self.bytes_per_pixel()) {
                return Err(ImageError::Limits(LimitError::from_kind(
                    LimitErrorKind::InsufficientMemory,
                )));
            }
        }
        Ok(())
    }

    fn read_image(self, buf: &mut [u8]) -> ImageResult<()> {
        let (width, height, has_alpha) = (self.width, self.height, self.has_alpha);
        let rgba = match self.state {
            State::Ready(img) => img,
            State::Lazy { data, format } => {
                (format.decode)(&data, format.limits).map_err(|e| Self::decoding_error(format.name, e))?
            }
        };
        if (rgba.width, rgba.height) != (width, height) {
            return Err(Self::decoding_error(
                "heif",
                "decoded size differs from the size in the file header",
            ));
        }
        if has_alpha {
            buf.copy_from_slice(&rgba.data);
        } else {
            for (dst, src) in buf.chunks_exact_mut(3).zip(rgba.data.chunks_exact(4)) {
                dst.copy_from_slice(src.get(..3).unwrap_or_default());
            }
        }
        Ok(())
    }

    fn read_image_boxed(self: Box<Self>, buf: &mut [u8]) -> ImageResult<()> {
        (*self).read_image(buf)
    }
}

fn make_decoder(format: Format, data: Vec<u8>) -> ImageResult<HeifImageDecoder> {
    if data.get(4..8) != Some(b"ftyp") {
        // Mis-named files (a JPEG called .heic) are common: let the other registered decoders have a go.
        let img = image::load_from_memory(&data)?;
        let has_alpha = img.color().has_alpha();
        let rgba = img.to_rgba8();
        return Ok(HeifImageDecoder::ready(
            RgbaImage {
                width: rgba.width(),
                height: rgba.height(),
                data: rgba.into_raw(),
            },
            has_alpha,
        ));
    }
    match (format.info)(&data, format.limits) {
        Ok(info) => Ok(HeifImageDecoder {
            width: info.width,
            height: info.height,
            has_alpha: info.has_alpha,
            state: State::Lazy { data, format },
        }),
        // No usable header information (e.g. no `ispe`): decode right away to learn the size.
        Err(_) => {
            let image =
                (format.decode)(&data, format.limits).map_err(|e| HeifImageDecoder::decoding_error(format.name, e))?;
            Ok(HeifImageDecoder::ready(image, false))
        }
    }
}

/// Registers `format` for its extensions and brands. Returns false when an extension was already taken.
pub fn register(format: Format) -> bool {
    let mut all_new = true;
    for ext in format.extensions {
        let hook: image::hooks::DecodingHook = Box::new(move |mut reader: GenericReader<'_>| {
            let mut data = Vec::new();
            reader.read_to_end(&mut data).map_err(ImageError::IoError)?;
            Ok(Box::new(make_decoder(format, data)?) as Box<dyn ImageDecoder + '_>)
        });
        all_new &= register_decoding_hook(OsString::from(ext), hook);
    }
    if let Some(first) = format.extensions.first() {
        for brand in format.brands {
            let signature: &'static [u8] = Box::leak(Box::new([
                0, 0, 0, 0, b'f', b't', b'y', b'p', brand[0], brand[1], brand[2], brand[3],
            ]));
            const MASK: &[u8] = &[0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
            register_format_detection_hook(OsString::from(first), signature, Some(MASK));
        }
    }
    all_new
}
