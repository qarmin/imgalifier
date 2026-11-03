use std::collections::HashMap;

use super::properties::parse_property;
use super::sequence::parse_moov;
use super::{ConstructionMethod, Container, Extent, FourCc, Item, PropertyRef, Reference};
use crate::error::{Error, Limits, Result};
use crate::reader::Reader;

pub(super) struct BoxHeader<'a> {
    pub(super) kind: FourCc,
    pub(super) body: &'a [u8],
}

pub(super) fn next_box<'a>(r: &mut Reader<'a>) -> Result<BoxHeader<'a>> {
    let size32 = r.u32()?;
    let kind = r.fourcc()?;
    let header_len = if size32 == 1 { 16 } else { 8 };
    let total = match size32 {
        0 => r.remaining() as u64 + 8,
        1 => r.u64()?,
        n => u64::from(n),
    };
    let body_len = total
        .checked_sub(header_len)
        .ok_or(Error::Invalid("box smaller than its header"))?;
    let body_len = usize::try_from(body_len).map_err(|_| Error::Invalid("box size"))?;
    Ok(BoxHeader {
        kind,
        body: r.take(body_len)?,
    })
}

pub(super) fn full_box(body: &[u8]) -> Result<(u8, u32, Reader<'_>)> {
    let mut r = Reader::new(body);
    let v = r.u32()?;
    Ok(((v >> 24) as u8, v & 0x00FF_FFFF, r))
}

pub(super) fn parse_file(data: &[u8], limits: Limits) -> Result<Container<'_>> {
    let mut c = Container {
        data,
        limits,
        major_brand: [0; 4],
        compatible_brands: Vec::new(),
        primary_item: None,
        items: Vec::new(),
        properties: Vec::new(),
        references: Vec::new(),
        reference_targets: Vec::new(),
        idat: None,
        index: HashMap::new(),
        sequence: None,
    };
    let mut r = Reader::new(data);
    let (mut saw_ftyp, mut saw_meta) = (false, false);
    while !r.is_empty() {
        let b = next_box(&mut r)?;
        match &b.kind {
            b"ftyp" => {
                let mut f = Reader::new(b.body);
                c.major_brand = f.fourcc()?;
                f.skip(4)?;
                while f.remaining() >= 4 {
                    c.compatible_brands.push(f.fourcc()?);
                }
                saw_ftyp = true;
            }
            b"meta" if !saw_meta => {
                parse_meta(&mut c, b.body)?;
                saw_meta = true;
            }
            b"moov" if c.sequence.is_none() => c.sequence = parse_moov(b.body).ok().flatten(),
            _ => {}
        }
    }
    if !saw_ftyp {
        return Err(Error::Invalid("missing ftyp"));
    }
    if !saw_meta && c.sequence.is_none() {
        return Err(Error::Invalid("missing meta"));
    }
    Ok(c)
}

fn parse_meta<'a>(c: &mut Container<'a>, body: &'a [u8]) -> Result<()> {
    let (_, _, mut r) = full_box(body)?;
    let mut locations: HashMap<u32, (ConstructionMethod, Vec<Extent>)> = HashMap::new();
    let mut associations: HashMap<u32, Vec<PropertyRef>> = HashMap::new();
    while !r.is_empty() {
        let b = next_box(&mut r)?;
        match &b.kind {
            b"pitm" => {
                let (v, _, mut p) = full_box(b.body)?;
                c.primary_item = Some(if v == 0 { u32::from(p.u16()?) } else { p.u32()? });
            }
            b"iinf" => parse_iinf(c, b.body)?,
            b"iloc" => locations = parse_iloc(b.body, c.limits)?,
            b"iref" => parse_iref(c, b.body)?,
            b"iprp" => parse_iprp(c, b.body, &mut associations)?,
            b"idat" => c.idat = Some(b.body),
            _ => {}
        }
    }
    for item in &mut c.items {
        if let Some((method, extents)) = locations.remove(&item.id) {
            item.construction = method;
            item.extents = extents;
        }
        if let Some(props) = associations.remove(&item.id) {
            item.properties = props;
        }
    }
    c.index = c.items.iter().enumerate().map(|(i, it)| (it.id, i)).collect();
    Ok(())
}

fn parse_iinf(c: &mut Container<'_>, body: &[u8]) -> Result<()> {
    let (v, _, mut r) = full_box(body)?;
    let declared = if v == 0 { u32::from(r.u16()?) } else { r.u32()? };
    if declared as usize > c.limits.max_items {
        return Err(Error::LimitExceeded("item count"));
    }
    while !r.is_empty() {
        let b = next_box(&mut r)?;
        if &b.kind != b"infe" {
            continue;
        }
        let (v, flags, mut e) = full_box(b.body)?;
        let (id, item_type, name) = match v {
            0 | 1 => {
                let id = u32::from(e.u16()?);
                e.skip(2)?;
                (id, [0; 4], e.cstring())
            }
            2 | 3 => {
                let id = if v == 2 { u32::from(e.u16()?) } else { e.u32()? };
                e.skip(2)?;
                (id, e.fourcc()?, e.cstring())
            }
            _ => return Err(Error::Unsupported("infe version")),
        };
        if c.items.len() >= c.limits.max_items {
            return Err(Error::LimitExceeded("item count"));
        }
        c.items.push(Item {
            id,
            item_type,
            name,
            hidden: flags & 1 != 0,
            construction: ConstructionMethod::File,
            extents: Vec::new(),
            properties: Vec::new(),
        });
    }
    Ok(())
}

fn parse_iloc(body: &[u8], limits: Limits) -> Result<HashMap<u32, (ConstructionMethod, Vec<Extent>)>> {
    let (v, _, mut r) = full_box(body)?;
    let sizes = r.u16()?;
    let (offset_size, length_size) = ((sizes >> 12) as u8, ((sizes >> 8) & 0xF) as u8);
    let (base_size, index_size) = (((sizes >> 4) & 0xF) as u8, if v == 0 { 0 } else { (sizes & 0xF) as u8 });
    let count = if v < 2 { u32::from(r.u16()?) } else { r.u32()? };
    if count as usize > limits.max_items {
        return Err(Error::LimitExceeded("iloc item count"));
    }
    let mut out = HashMap::new();
    for _ in 0..count {
        let id = if v < 2 { u32::from(r.u16()?) } else { r.u32()? };
        let method = if v == 0 {
            ConstructionMethod::File
        } else {
            match r.u16()? & 0xF {
                0 => ConstructionMethod::File,
                1 => ConstructionMethod::Idat,
                2 => ConstructionMethod::Item,
                _ => return Err(Error::Invalid("iloc construction method")),
            }
        };
        if r.u16()? != 0 {
            return Err(Error::Unsupported("external data reference"));
        }
        let base = r.uint(base_size)?;
        let n = r.u16()?;
        let mut extents = Vec::with_capacity(usize::from(n).min(64));
        for _ in 0..n {
            r.uint(index_size)?;
            let offset = r
                .uint(offset_size)?
                .checked_add(base)
                .ok_or(Error::Invalid("extent offset overflow"))?;
            extents.push(Extent {
                offset,
                length: r.uint(length_size)?,
            });
        }
        out.insert(id, (method, extents));
    }
    Ok(out)
}

fn parse_iref(c: &mut Container<'_>, body: &[u8]) -> Result<()> {
    let (v, _, mut r) = full_box(body)?;
    while !r.is_empty() {
        let b = next_box(&mut r)?;
        let mut e = Reader::new(b.body);
        let (from, n) = if v == 0 {
            (u32::from(e.u16()?), e.u16()?)
        } else {
            (e.u32()?, e.u16()?)
        };
        let to_start = c.reference_targets.len();
        for _ in 0..n {
            c.reference_targets
                .push(if v == 0 { u32::from(e.u16()?) } else { e.u32()? });
        }
        c.references.push(Reference {
            kind: b.kind,
            from,
            to_start,
            to_len: usize::from(n),
        });
    }
    Ok(())
}

fn parse_iprp(c: &mut Container<'_>, body: &[u8], assoc: &mut HashMap<u32, Vec<PropertyRef>>) -> Result<()> {
    let mut r = Reader::new(body);
    while !r.is_empty() {
        let b = next_box(&mut r)?;
        match &b.kind {
            b"ipco" => {
                let mut p = Reader::new(b.body);
                while !p.is_empty() {
                    let pb = next_box(&mut p)?;
                    c.properties.push(parse_property(pb.kind, pb.body));
                }
            }
            b"ipma" => {
                let (v, flags, mut p) = full_box(b.body)?;
                let n = p.u32()?;
                for _ in 0..n {
                    let id = if v < 1 { u32::from(p.u16()?) } else { p.u32()? };
                    let count = p.u8()?;
                    let list = assoc.entry(id).or_default();
                    for _ in 0..count {
                        let (essential, index) = if flags & 1 != 0 {
                            let x = p.u16()?;
                            (x & 0x8000 != 0, x & 0x7FFF)
                        } else {
                            let x = p.u8()?;
                            (x & 0x80 != 0, u16::from(x & 0x7F))
                        };
                        list.push(PropertyRef { index, essential });
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}
