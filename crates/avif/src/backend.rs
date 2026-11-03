use imgalifier_heif::{ChromaFormat, CodecBackend, ColorInfo, Error, FourCc, Result, YuvImage};

use crate::config::Av1Config;
use crate::rav1d::include::dav1d::headers::Rav1dPixelLayout;
use crate::rav1d::rust_api::{Decoder, Picture, PlanarImageComponent, Settings};
use crate::rav1d::Rav1dError;

/// AV1 still-picture backend on top of the vendored rav1d decoder.
#[derive(Debug, Clone, Copy)]
pub struct Av1Backend {
    /// rav1d worker threads; 1 keeps the decode on the calling thread, 0 lets rav1d pick.
    pub threads: u32,
    /// Largest accepted frame in luma samples (rav1d rejects larger sequence headers before allocating).
    pub max_pixels: u32,
}

impl Default for Av1Backend {
    fn default() -> Self {
        Self {
            threads: 1,
            max_pixels: 1 << 27,
        }
    }
}

fn map_err(e: Rav1dError) -> Error {
    match e {
        Rav1dError::OutOfMemory => Error::LimitExceeded("AV1 decoder allocation"),
        Rav1dError::InvalidArgument | Rav1dError::OutOfRange | Rav1dError::Other => Error::Invalid("AV1 bitstream"),
        Rav1dError::UnsupportedBitstream => Error::Unsupported("AV1 stream feature"),
        Rav1dError::NoEntity | Rav1dError::TryAgain => Error::Invalid("AV1 decoder produced no picture"),
    }
}

impl CodecBackend for Av1Backend {
    fn handles(&self, item_type: FourCc) -> bool {
        &item_type == b"av01"
    }

    fn decode(&self, config: &[u8], payload: &[u8]) -> Result<YuvImage> {
        let config = Av1Config::parse(config)?;
        if payload.is_empty() {
            return Err(Error::Invalid("empty AV1 item"));
        }
        let mut settings = Settings::new();
        settings.set_n_threads(self.threads);
        settings.set_all_layers(false);
        settings.set_frame_size_limit(self.max_pixels);
        let mut decoder = Decoder::with_settings(&settings).map_err(map_err)?;
        let mut data = Vec::with_capacity(config.config_obus.len() + payload.len());
        data.extend_from_slice(config.config_obus);
        data.extend_from_slice(payload);
        match decoder.send_data(data.into_boxed_slice(), None, None, None) {
            Ok(()) | Err(Rav1dError::TryAgain) => {}
            Err(e) => return Err(map_err(e)),
        }
        // Each TryAgain either consumes more pending data or means the stream holds no picture; the cap keeps a
        // stream that never yields one from spinning forever.
        let mut budget = 64 + payload.len();
        let picture = loop {
            match decoder.get_picture() {
                Ok(p) => break p,
                Err(Rav1dError::TryAgain) => {
                    budget = budget
                        .checked_sub(1)
                        .ok_or(Error::Invalid("AV1 item holds no picture"))?;
                    decoder
                        .send_pending_data()
                        .map_err(|_| Error::Invalid("AV1 item holds no picture"))?;
                }
                Err(e) => return Err(map_err(e)),
            }
        };
        picture_to_yuv(&picture)
    }
}

fn picture_to_yuv(p: &Picture) -> Result<YuvImage> {
    let chroma = match p.pixel_layout() {
        Rav1dPixelLayout::I400 => ChromaFormat::Monochrome,
        Rav1dPixelLayout::I420 => ChromaFormat::Yuv420,
        Rav1dPixelLayout::I422 => ChromaFormat::Yuv422,
        Rav1dPixelLayout::I444 => ChromaFormat::Yuv444,
    };
    let bit_depth = u8::try_from(p.bit_depth()).map_err(|_| Error::Invalid("AV1 bit depth"))?;
    let mut yuv = YuvImage::new(p.width(), p.height(), bit_depth, chroma)?;
    let planes = if chroma == ChromaFormat::Monochrome { 1 } else { 3 };
    for idx in 0..planes {
        let comp = PlanarImageComponent::try_from(idx).map_err(|_| Error::Invalid("plane index"))?;
        let (w, h) = yuv.plane_dims(idx);
        let stride = p.stride(comp) as usize;
        let src = p.plane(comp);
        let dst = yuv.planes.get_mut(idx).ok_or(Error::Invalid("plane index"))?;
        for y in 0..h {
            let row = src.get(y * stride..).ok_or(Error::Invalid("AV1 plane too small"))?;
            let out = dst.get_mut(y * w..(y + 1) * w).ok_or(Error::Invalid("plane size"))?;
            if bit_depth == 8 {
                for (o, s) in out.iter_mut().zip(row) {
                    *o = u16::from(*s);
                }
            } else {
                for (o, s) in out.iter_mut().zip(row.chunks_exact(2)) {
                    *o = u16::from_le_bytes(s.try_into().unwrap_or_default());
                }
            }
        }
    }
    yuv.color = ColorInfo {
        primaries: 2,
        transfer: 2,
        matrix: u16::try_from(p.matrix_coefficients_code()).unwrap_or(2),
        full_range: p.color_range_is_full(),
    };
    Ok(yuv)
}
