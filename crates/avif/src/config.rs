use imgalifier_heif::{ChromaFormat, Error, Result};

/// AV1CodecConfigurationRecord (`av1C`): 4 fixed bytes followed by optional configuration OBUs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Av1Config<'a> {
    pub seq_profile: u8,
    pub seq_level_idx_0: u8,
    pub bit_depth: u8,
    pub chroma: ChromaFormat,
    pub config_obus: &'a [u8],
}

impl<'a> Av1Config<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let [b0, b1, b2, _delay, obus @ ..] = data else {
            return Err(Error::Invalid("av1C shorter than 4 bytes"));
        };
        if b0 != &0x81 {
            return Err(Error::Unsupported("av1C marker/version"));
        }
        let (high_bitdepth, twelve_bit) = (b2 & 0x40 != 0, b2 & 0x20 != 0);
        let bit_depth = match (high_bitdepth, twelve_bit) {
            (false, _) => 8,
            (true, false) => 10,
            (true, true) => 12,
        };
        let (mono, sub_x, sub_y) = (b2 & 0x10 != 0, b2 & 0x08 != 0, b2 & 0x04 != 0);
        let chroma = match (mono, sub_x, sub_y) {
            (true, _, _) => ChromaFormat::Monochrome,
            (false, true, true) => ChromaFormat::Yuv420,
            (false, true, false) => ChromaFormat::Yuv422,
            (false, false, false) => ChromaFormat::Yuv444,
            (false, false, true) => return Err(Error::Invalid("av1C: invalid subsampling")),
        };
        Ok(Self {
            seq_profile: b1 >> 5,
            seq_level_idx_0: b1 & 0x1F,
            bit_depth,
            chroma,
            config_obus: obus,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_8bit_420() {
        let c = Av1Config::parse(&[0x81, 0x00, 0x0C, 0x00]).unwrap();
        assert_eq!((c.bit_depth, c.chroma), (8, ChromaFormat::Yuv420));
    }

    #[test]
    fn parses_10bit_mono() {
        let c = Av1Config::parse(&[0x81, 0x00, 0x5C, 0x00, 0x0A]).unwrap();
        assert_eq!(
            (c.bit_depth, c.chroma, c.config_obus),
            (10, ChromaFormat::Monochrome, &[0x0A][..])
        );
    }

    #[test]
    fn rejects_short_and_bad_marker() {
        Av1Config::parse(&[0x81]).unwrap_err();
        Av1Config::parse(&[0x01, 0, 0, 0]).unwrap_err();
    }
}
