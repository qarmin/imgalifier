//! HEVC (H.265) still-picture decoder.
//!
//! The picture decoding modules (`bitreader`, `cabac`, `contexts`, `decoder`, `filters`, `nal`, `params`, `recon`,
//! `scan`, `sei`, `slice`, `syntax`) are vendored from heifer-hevc-dec 0.3.26 (https://github.com/HarpeLm/Heifer,
//! MIT OR Apache-2.0, see THIRD_PARTY.md) and are being improved in place; `mod.rs` and `hvcc.rs` are ours.

#![allow(missing_docs, clippy::all, clippy::pedantic, clippy::nursery, clippy::restriction)]

pub mod bitreader;
pub mod cabac;
pub mod contexts;
pub mod decoder;
pub mod filters;
pub mod nal;
pub mod params;
pub mod recon;
pub mod scan;
pub mod sei;
pub mod slice;
pub mod syntax;
#[cfg(test)]
mod testutil;

mod hvcc;

use std::fmt;

pub use hvcc::HevcConfig;

use crate::container::FourCc;
use crate::decode::CodecBackend;
use crate::image::{ChromaFormat, ColorInfo, YuvImage};

/// Errors produced while decoding an HEVC bitstream.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    UnexpectedEof,
    Invalid(&'static str),
    Unimplemented(&'static str),
    HashMismatch(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => f.write_str("unexpected end of bitstream"),
            Self::Invalid(m) => write!(f, "invalid bitstream: {m}"),
            Self::Unimplemented(m) => write!(f, "not implemented: {m}"),
            Self::HashMismatch(k) => write!(f, "decoded picture does not match its {k} hash"),
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for crate::error::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::UnexpectedEof => Self::Invalid("HEVC bitstream ended unexpectedly"),
            Error::Invalid(m) | Error::HashMismatch(m) => Self::Invalid(m),
            Error::Unimplemented(m) => Self::Unsupported(m),
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct HevcBackend {
    pub verify_hash: bool,
}

impl CodecBackend for HevcBackend {
    fn handles(&self, item_type: FourCc) -> bool {
        matches!(&item_type, b"hvc1" | b"hev1")
    }

    fn decode(&self, config: &[u8], payload: &[u8]) -> crate::error::Result<YuvImage> {
        let config = HevcConfig::parse(config)?;
        let stream = config.annex_b(payload)?;
        let options = decoder::DecodeOptions {
            verify_hash: self.verify_hash,
            ..Default::default()
        };
        let frame = decoder::decode_picture(&stream, options)?;
        frame_to_yuv(&frame)
    }

    fn alpha_honors_range(&self) -> bool {
        false
    }
}

fn frame_to_yuv(frame: &recon::Frame) -> crate::error::Result<YuvImage> {
    let chroma = match frame.chroma_format {
        0 => ChromaFormat::Monochrome,
        1 => ChromaFormat::Yuv420,
        2 => ChromaFormat::Yuv422,
        _ => ChromaFormat::Yuv444,
    };
    let depth = frame.bit_depth[0];
    let mut planes = frame.planes.clone();
    if chroma != ChromaFormat::Monochrome && frame.bit_depth[1] != depth {
        let (from, to) = (u32::from(frame.bit_depth[1]), u32::from(depth));
        for p in planes.iter_mut().skip(1) {
            for v in p.iter_mut() {
                *v = if to >= from {
                    *v << (to - from)
                } else {
                    *v >> (from - to)
                };
            }
        }
    }
    let mut yuv = YuvImage::new(frame.widths[0], frame.heights[0], depth, chroma)?;
    if planes.iter().zip(yuv.planes.iter()).any(|(a, b)| a.len() != b.len()) {
        return Err(crate::error::Error::Invalid("HEVC plane size mismatch"));
    }
    yuv.planes = planes;
    yuv.color = ColorInfo {
        matrix: u16::from(frame.matrix_coeffs),
        full_range: frame.full_range,
        ..ColorInfo::default()
    };
    Ok(yuv)
}
