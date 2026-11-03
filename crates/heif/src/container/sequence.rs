//! First frame of an image sequence (`moov`/`trak`): enough to show a thumbnail of animated AVIF and HEIC sequences.

use super::boxes::{full_box, next_box};
use super::{Extent, FourCc};
use crate::error::{Error, Result};
use crate::reader::Reader;

/// The first sample of the first video track whose codec the caller can decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceFrame {
    /// Sample entry type: `av01`, `hvc1` or `hev1`.
    pub codec: FourCc,
    /// Body of the `av1C`/`hvcC` box of the sample entry.
    pub config: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Location of the first sample in the file.
    pub sample: Extent,
}

pub(super) fn parse_moov(body: &[u8]) -> Result<Option<SequenceFrame>> {
    let mut r = Reader::new(body);
    while !r.is_empty() {
        let b = next_box(&mut r)?;
        if &b.kind == b"trak" {
            if let Some(frame) = parse_trak(b.body)? {
                return Ok(Some(frame));
            }
        }
    }
    Ok(None)
}

fn find_child(body: &[u8], kind: [u8; 4]) -> Result<Option<&[u8]>> {
    let mut r = Reader::new(body);
    while !r.is_empty() {
        let b = next_box(&mut r)?;
        if b.kind == kind {
            return Ok(Some(b.body));
        }
    }
    Ok(None)
}

fn parse_trak(body: &[u8]) -> Result<Option<SequenceFrame>> {
    let Some(mdia) = find_child(body, *b"mdia")? else {
        return Ok(None);
    };
    let Some(hdlr) = find_child(mdia, *b"hdlr")? else {
        return Ok(None);
    };
    let (_, _, mut h) = full_box(hdlr)?;
    h.skip(4)?;
    if !matches!(&h.fourcc()?, b"vide" | b"pict") {
        return Ok(None);
    }
    let Some(stbl) = find_child(mdia, *b"minf")?
        .map(|m| find_child(m, *b"stbl"))
        .transpose()?
        .flatten()
    else {
        return Ok(None);
    };
    let Some(stsd) = find_child(stbl, *b"stsd")? else {
        return Ok(None);
    };
    let (_, _, mut r) = full_box(stsd)?;
    if r.u32()? == 0 {
        return Ok(None);
    }
    let entry = next_box(&mut r)?;
    if !matches!(&entry.kind, b"av01" | b"hvc1" | b"hev1") {
        return Ok(None);
    }
    let mut e = Reader::new(entry.body);
    e.skip(6 + 2 + 16)?;
    let (width, height) = (u32::from(e.u16()?), u32::from(e.u16()?));
    e.skip(4 + 4 + 4 + 2 + 32 + 2 + 2)?;
    let config_kind = if &entry.kind == b"av01" { *b"av1C" } else { *b"hvcC" };
    let Some(config) = find_child(e.rest(), config_kind)? else {
        return Ok(None);
    };

    let offset = first_chunk_offset(stbl)?;
    let length = first_sample_size(stbl)?;
    Ok(Some(SequenceFrame {
        codec: entry.kind,
        config: config.to_vec(),
        width,
        height,
        sample: Extent {
            offset,
            length: u64::from(length),
        },
    }))
}

fn first_chunk_offset(stbl: &[u8]) -> Result<u64> {
    if let Some(stco) = find_child(stbl, *b"stco")? {
        let (_, _, mut r) = full_box(stco)?;
        if r.u32()? == 0 {
            return Err(Error::Invalid("empty stco"));
        }
        return Ok(u64::from(r.u32()?));
    }
    let co64 = find_child(stbl, *b"co64")?.ok_or(Error::Invalid("missing chunk offsets"))?;
    let (_, _, mut r) = full_box(co64)?;
    if r.u32()? == 0 {
        return Err(Error::Invalid("empty co64"));
    }
    r.u64()
}

fn first_sample_size(stbl: &[u8]) -> Result<u32> {
    let stsz = find_child(stbl, *b"stsz")?.ok_or(Error::Invalid("missing stsz"))?;
    let (_, _, mut r) = full_box(stsz)?;
    let constant = r.u32()?;
    let count = r.u32()?;
    if count == 0 {
        return Err(Error::Invalid("empty stsz"));
    }
    if constant != 0 { Ok(constant) } else { r.u32() }
}
