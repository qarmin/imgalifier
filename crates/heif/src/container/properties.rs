use super::FourCc;
use crate::error::Result;
use crate::reader::Reader;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Colr {
    Nclx {
        primaries: u16,
        transfer: u16,
        matrix: u16,
        full_range: bool,
    },
    Icc(Vec<u8>),
}

/// Clean aperture as rational numbers (width, height, horizontal and vertical centre offset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clap {
    pub width: (u32, u32),
    pub height: (u32, u32),
    pub h_off: (i32, u32),
    pub v_off: (i32, u32),
}

/// One component of an uncompressed (`unci`) image as listed in `uncC`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UncComponent {
    /// Index into the `cmpd` component list.
    pub index: u16,
    pub bit_depth: u8,
    /// 0 unsigned integer, 1 float, 2 complex.
    pub format: u8,
    /// Bytes each sample is aligned to; 0 means bit-packed.
    pub align_size: u8,
}

/// Layout description of an uncompressed image (`uncC`, ISO/IEC 23001-17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UncC {
    /// 0 = full resolution chroma, 1 = 4:2:2, 2 = 4:2:0, 3 = 4:1:1.
    pub sampling: u8,
    /// 0 component, 1 mixed, 2 pixel, 3 row, 4 tile-component, 5 multi-Y.
    pub interleave: u8,
    pub block_size: u8,
    pub components_little_endian: bool,
    pub block_pad_lsb: bool,
    pub block_little_endian: bool,
    pub block_reversed: bool,
    pub pad_unknown: bool,
    pub pixel_size: u32,
    pub row_align_size: u32,
    pub tile_align_size: u32,
    pub tile_cols: u32,
    pub tile_rows: u32,
    pub components: Vec<UncComponent>,
    /// Profile fourcc of a version 1 `uncC` (`rgb3`, `rgba`, `abgr`), which has no `cmpd` and no component list.
    pub profile: Option<[u8; 4]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Property {
    /// Component types of `cmpd` (0 mono, 1 Y, 2 Cb, 3 Cr, 4 R, 5 G, 6 B, 7 alpha, 12 padding, ...).
    Cmpd(Vec<u16>),
    UncC(UncC),
    /// Compression of the item data (`cmpC`): `defl`, `zlib` or `brot`.
    CmpC(FourCc),
    /// Compressed unit byte ranges (`icbr`): (offset, size) pairs, offsets are file positions.
    Icbr(Vec<(u64, u64)>),
    Ispe {
        width: u32,
        height: u32,
    },
    Pixi {
        bits_per_channel: Vec<u8>,
    },
    Colr(Colr),
    /// Rotation in 90 degree steps, counter-clockwise.
    Irot(u8),
    /// Mirror axis: 0 = about the horizontal axis (vertical flip), 1 = about the vertical axis.
    Imir(u8),
    Clap(Clap),
    /// Codec configuration record body (`hvcC` or `av1C`), parsed by the codec crate.
    CodecConfig {
        kind: FourCc,
        data: Vec<u8>,
    },
    AuxC {
        aux_type: String,
    },
    Clli {
        max_cll: u16,
        max_fall: u16,
    },
    Other(FourCc),
}

pub(super) fn parse_property(kind: FourCc, body: &[u8]) -> Property {
    parse_known(kind, body).ok().flatten().unwrap_or(Property::Other(kind))
}

fn parse_known(kind: FourCc, body: &[u8]) -> Result<Option<Property>> {
    let mut r = Reader::new(body);
    Ok(Some(match &kind {
        b"ispe" => {
            r.skip(4)?;
            Property::Ispe {
                width: r.u32()?,
                height: r.u32()?,
            }
        }
        b"pixi" => {
            r.skip(4)?;
            let n = r.u8()?;
            Property::Pixi {
                bits_per_channel: r.take(usize::from(n))?.to_vec(),
            }
        }
        b"colr" => match &r.fourcc()? {
            b"nclx" => Property::Colr(Colr::Nclx {
                primaries: r.u16()?,
                transfer: r.u16()?,
                matrix: r.u16()?,
                full_range: r.u8()? & 0x80 != 0,
            }),
            b"rICC" | b"prof" => Property::Colr(Colr::Icc(r.rest().to_vec())),
            _ => return Ok(None),
        },
        b"irot" => Property::Irot(r.u8()? & 3),
        b"imir" => Property::Imir(r.u8()? & 1),
        b"clap" => Property::Clap(Clap {
            width: (r.u32()?, r.u32()?),
            height: (r.u32()?, r.u32()?),
            h_off: (r.i32()?, r.u32()?),
            v_off: (r.i32()?, r.u32()?),
        }),
        b"hvcC" | b"av1C" => Property::CodecConfig {
            kind,
            data: body.to_vec(),
        },
        b"auxC" => {
            r.skip(4)?;
            Property::AuxC { aux_type: r.cstring() }
        }
        b"cmpd" => {
            let n = r.u32()?;
            let mut types = Vec::with_capacity((n as usize).min(64));
            for _ in 0..n {
                let t = r.u16()?;
                if t >= 0x8000 {
                    r.cstring_skip()?;
                }
                types.push(t);
            }
            Property::Cmpd(types)
        }
        b"uncC" => {
            let version = r.u32()? >> 24;
            if version == 1 {
                let profile = r.fourcc()?;
                let depth8 = |i: u16| UncComponent {
                    index: i,
                    bit_depth: 8,
                    format: 0,
                    align_size: 1,
                };
                let n: u16 = if &profile == b"rgb3" { 3 } else { 4 };
                return Ok(Some(Property::UncC(UncC {
                    sampling: 0,
                    interleave: 1,
                    block_size: 0,
                    components_little_endian: false,
                    block_pad_lsb: false,
                    block_little_endian: false,
                    block_reversed: false,
                    pad_unknown: false,
                    pixel_size: 0,
                    row_align_size: 0,
                    tile_align_size: 0,
                    tile_cols: 1,
                    tile_rows: 1,
                    components: (0..n).map(depth8).collect(),
                    profile: Some(profile),
                })));
            }
            if version != 0 {
                return Ok(None);
            }
            r.skip(4)?;
            let count = r.u32()?;
            let mut components = Vec::with_capacity((count as usize).min(64));
            for _ in 0..count {
                components.push(UncComponent {
                    index: r.u16()?,
                    bit_depth: r.u8()?.wrapping_add(1),
                    format: r.u8()?,
                    align_size: r.u8()?,
                });
            }
            let sampling = r.u8()?;
            let interleave = r.u8()?;
            let block_size = r.u8()?;
            let flags = r.u8()?;
            Property::UncC(UncC {
                sampling,
                interleave,
                block_size,
                components_little_endian: flags & 0x80 != 0,
                block_pad_lsb: flags & 0x40 != 0,
                block_little_endian: flags & 0x20 != 0,
                block_reversed: flags & 0x10 != 0,
                pad_unknown: flags & 0x08 != 0,
                pixel_size: r.u32()?,
                row_align_size: r.u32()?,
                tile_align_size: r.u32()?,
                tile_cols: r.u32()?.wrapping_add(1),
                tile_rows: r.u32()?.wrapping_add(1),
                components,
                profile: None,
            })
        }
        b"cmpC" => {
            r.skip(4)?;
            Property::CmpC(r.fourcc()?)
        }
        b"icbr" => {
            let version = r.u32()? >> 24;
            let n = r.u32()?;
            let mut ranges = Vec::with_capacity((n as usize).min(4096));
            for _ in 0..n {
                ranges.push(if version == 0 {
                    (u64::from(r.u32()?), u64::from(r.u32()?))
                } else {
                    (r.u64()?, r.u64()?)
                });
            }
            Property::Icbr(ranges)
        }
        b"clli" => Property::Clli {
            max_cll: r.u16()?,
            max_fall: r.u16()?,
        },
        _ => return Ok(None),
    }))
}
