use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromaFormat {
    Monochrome,
    Yuv420,
    Yuv422,
    Yuv444,
}

impl ChromaFormat {
    /// Horizontal and vertical subsampling shifts of the chroma planes.
    pub const fn shifts(self) -> (u32, u32) {
        match self {
            Self::Monochrome | Self::Yuv444 => (0, 0),
            Self::Yuv420 => (1, 1),
            Self::Yuv422 => (1, 0),
        }
    }
}

/// Colour description from `colr`/nclx or the codec bitstream (ISO 23091-2 code points).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorInfo {
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    pub full_range: bool,
}

impl Default for ColorInfo {
    fn default() -> Self {
        Self {
            primaries: 2,
            transfer: 2,
            matrix: 2,
            full_range: true,
        }
    }
}

/// Decoded planar picture; all bit depths are stored as `u16` samples, chroma planes are subsampled.
#[derive(Debug, Clone)]
pub struct YuvImage {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub chroma: ChromaFormat,
    pub color: ColorInfo,
    pub planes: [Vec<u16>; 3],
}

impl YuvImage {
    pub fn new(width: u32, height: u32, bit_depth: u8, chroma: ChromaFormat) -> Result<Self> {
        if width == 0 || height == 0 || !(1..=16).contains(&bit_depth) {
            return Err(Error::Invalid("bad picture dimensions or bit depth"));
        }
        let (w, h) = (width as usize, height as usize);
        let luma = w.checked_mul(h).ok_or(Error::LimitExceeded("picture size"))?;
        let (cw, ch) = chroma_dims(width, height, chroma);
        let c = if chroma == ChromaFormat::Monochrome { 0 } else { cw * ch };
        Ok(Self {
            width,
            height,
            bit_depth,
            chroma,
            color: ColorInfo::default(),
            planes: [vec![0; luma], vec![0; c], vec![0; c]],
        })
    }

    /// Width and height of plane `idx` (0 = luma).
    pub fn plane_dims(&self, idx: usize) -> (usize, usize) {
        if idx == 0 || self.chroma == ChromaFormat::Monochrome {
            (self.width as usize, self.height as usize)
        } else {
            chroma_dims(self.width, self.height, self.chroma)
        }
    }
}

pub(crate) fn chroma_dims(width: u32, height: u32, chroma: ChromaFormat) -> (usize, usize) {
    let (sx, sy) = chroma.shifts();
    (
        ((width + (1 << sx) - 1) >> sx) as usize,
        ((height + (1 << sy) - 1) >> sy) as usize,
    )
}

/// Final 8-bit RGBA picture (straight alpha), row-major, no padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}
