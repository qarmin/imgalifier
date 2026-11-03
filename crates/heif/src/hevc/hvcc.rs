use crate::error::{Error, Result};
use crate::reader::Reader;

/// HEVCDecoderConfigurationRecord (`hvcC`): NAL length field size plus the parameter set NAL units.
#[derive(Debug, Clone)]
pub struct HevcConfig<'a> {
    pub chroma_format_idc: u8,
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
    pub length_size: u8,
    pub parameter_sets: Vec<&'a [u8]>,
}

impl<'a> HevcConfig<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        if r.u8()? != 1 {
            return Err(Error::Unsupported("hvcC configuration version"));
        }
        r.skip(1 + 4 + 6 + 1 + 2 + 1)?;
        let chroma_format_idc = r.u8()? & 3;
        let bit_depth_luma = (r.u8()? & 7) + 8;
        let bit_depth_chroma = (r.u8()? & 7) + 8;
        r.skip(2)?;
        let length_size = (r.u8()? & 3) + 1;
        let arrays = r.u8()?;
        let mut parameter_sets = Vec::new();
        for _ in 0..arrays {
            r.skip(1)?;
            for _ in 0..r.u16()? {
                let len = usize::from(r.u16()?);
                parameter_sets.push(r.take(len)?);
            }
        }
        Ok(Self {
            chroma_format_idc,
            bit_depth_luma,
            bit_depth_chroma,
            length_size,
            parameter_sets,
        })
    }

    /// Annex B stream (start-code framed) of the parameter sets followed by the length-prefixed `payload` NAL units.
    pub fn annex_b(&self, payload: &[u8]) -> Result<Vec<u8>> {
        const START: [u8; 4] = [0, 0, 0, 1];
        let mut out = Vec::with_capacity(payload.len() + 256);
        for ps in &self.parameter_sets {
            out.extend_from_slice(&START);
            out.extend_from_slice(ps);
        }
        let n = usize::from(self.length_size);
        let mut rest = payload;
        while !rest.is_empty() {
            let (len_bytes, tail) = rest.split_at_checked(n).ok_or(Error::Invalid("truncated NAL length"))?;
            let len = len_bytes.iter().fold(0usize, |a, &b| (a << 8) | usize::from(b));
            let (nal, tail) = tail
                .split_at_checked(len)
                .ok_or(Error::Invalid("NAL length exceeds data"))?;
            rest = tail;
            // Some encoders pad with empty or 1-byte NAL units, which have no header to parse.
            if nal.len() < 2 {
                continue;
            }
            out.extend_from_slice(&START);
            out.extend_from_slice(nal);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> Vec<u8> {
        let mut d = vec![
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0xF0,
            0,
            0xFC,
            0xFC | 1,
            0xF8,
            0xF8,
            0,
            0,
            0x03,
            1,
        ];
        d.extend_from_slice(&[0x21, 0, 1, 0, 3, 0x42, 0x01, 0xAA]);
        d
    }

    #[test]
    fn parses_minimal_record() {
        let d = record();
        let c = HevcConfig::parse(&d).unwrap();
        assert_eq!((c.chroma_format_idc, c.bit_depth_luma, c.length_size), (1, 8, 4));
        assert_eq!(c.parameter_sets, vec![&[0x42, 0x01, 0xAA][..]]);
    }

    #[test]
    fn builds_annex_b() {
        let d = record();
        let c = HevcConfig::parse(&d).unwrap();
        let s = c.annex_b(&[0, 0, 0, 2, 0x26, 0x01]).unwrap();
        assert_eq!(s, [0, 0, 0, 1, 0x42, 1, 0xAA, 0, 0, 0, 1, 0x26, 1]);
        c.annex_b(&[0, 0, 0, 9, 1]).unwrap_err();
    }
}
