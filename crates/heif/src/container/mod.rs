//! ISOBMFF/HEIF box parser shared by the HEIC and AVIF decoders.

mod boxes;
mod properties;
mod sequence;

use std::borrow::Cow;
use std::collections::HashMap;

pub use properties::{Clap, Colr, Property, UncC, UncComponent};
pub use sequence::SequenceFrame;

use crate::error::{Error, Limits, Result};

pub type FourCc = [u8; 4];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstructionMethod {
    File,
    Idat,
    Item,
}

#[derive(Debug, Clone, Copy)]
pub struct PropertyRef {
    pub index: u16,
    pub essential: bool,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub id: u32,
    pub item_type: FourCc,
    pub name: String,
    pub hidden: bool,
    pub construction: ConstructionMethod,
    pub extents: Vec<Extent>,
    pub properties: Vec<PropertyRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reference {
    pub kind: FourCc,
    pub from: u32,
    pub to_start: usize,
    pub to_len: usize,
}

/// Parsed `meta` box of a HEIF/AVIF file; item payloads are borrowed lazily from the input.
#[derive(Debug)]
pub struct Container<'a> {
    data: &'a [u8],
    limits: Limits,
    pub major_brand: FourCc,
    pub compatible_brands: Vec<FourCc>,
    pub primary_item: Option<u32>,
    pub items: Vec<Item>,
    pub properties: Vec<Property>,
    references: Vec<Reference>,
    reference_targets: Vec<u32>,
    idat: Option<&'a [u8]>,
    index: HashMap<u32, usize>,
    /// First frame of the first decodable video track, when the file has a `moov` box.
    pub sequence: Option<SequenceFrame>,
}

impl<'a> Container<'a> {
    pub fn parse(data: &'a [u8], limits: Limits) -> Result<Self> {
        boxes::parse_file(data, limits)
    }

    pub fn has_brand(&self, brand: FourCc) -> bool {
        self.major_brand == brand || self.compatible_brands.contains(&brand)
    }

    pub fn item(&self, id: u32) -> Option<&Item> {
        self.index.get(&id).and_then(|&i| self.items.get(i))
    }

    /// Bytes of the first sequence sample (see [`Container::sequence`]).
    pub fn sequence_sample(&self, frame: &SequenceFrame) -> Result<&'a [u8]> {
        let start = usize::try_from(frame.sample.offset).map_err(|_| Error::Invalid("sample offset"))?;
        let len = usize::try_from(frame.sample.length).map_err(|_| Error::Invalid("sample size"))?;
        if len > self.limits.max_item_bytes {
            return Err(Error::LimitExceeded("sample size"));
        }
        let end = start.checked_add(len).ok_or(Error::Invalid("sample overflow"))?;
        self.data
            .get(start..end)
            .ok_or(Error::Invalid("sample outside of file"))
    }

    pub fn primary(&self) -> Result<&Item> {
        let id = self.primary_item.ok_or(Error::Invalid("missing pitm"))?;
        self.item(id).ok_or(Error::Invalid("primary item not found"))
    }

    /// Resolved (1-based `ipco` index) properties of an item in association order.
    pub fn item_properties<'s>(&'s self, item: &'s Item) -> impl Iterator<Item = (&'s Property, bool)> + 's {
        item.properties.iter().filter_map(|r| {
            let idx = usize::from(r.index).checked_sub(1)?;
            self.properties.get(idx).map(|p| (p, r.essential))
        })
    }

    pub fn find_property<'s, T>(&'s self, item: &'s Item, f: impl Fn(&'s Property) -> Option<T>) -> Option<T> {
        self.item_properties(item).find_map(|(p, _)| f(p))
    }

    /// Targets of every `kind` reference starting at `from`, in file order.
    pub fn references_from(&self, from: u32, kind: FourCc) -> impl Iterator<Item = u32> + '_ {
        self.references
            .iter()
            .filter(move |r| r.from == from && r.kind == kind)
            .flat_map(|r| {
                self.reference_targets
                    .get(r.to_start..r.to_start + r.to_len)
                    .unwrap_or_default()
                    .iter()
                    .copied()
            })
    }

    /// All `iref` entries as (type, from item, target items).
    pub fn all_references(&self) -> impl Iterator<Item = (FourCc, u32, &[u32])> + '_ {
        self.references.iter().map(|r| {
            (
                r.kind,
                r.from,
                self.reference_targets
                    .get(r.to_start..r.to_start + r.to_len)
                    .unwrap_or_default(),
            )
        })
    }

    /// Items that reference `to` with `kind` (e.g. the alpha item that has `auxl` -> primary).
    pub fn referrers(&self, to: u32, kind: FourCc) -> Vec<u32> {
        self.references
            .iter()
            .filter(|r| r.kind == kind)
            .filter(|r| {
                self.reference_targets
                    .get(r.to_start..r.to_start + r.to_len)
                    .unwrap_or_default()
                    .contains(&to)
            })
            .map(|r| r.from)
            .collect()
    }

    /// Concatenated payload of an item. Borrowed when it is a single extent in the file or `idat`.
    pub fn item_data(&self, item: &Item) -> Result<Cow<'a, [u8]>> {
        let base = match item.construction {
            ConstructionMethod::File => self.data,
            ConstructionMethod::Idat => self.idat.ok_or(Error::Invalid("idat construction without idat box"))?,
            ConstructionMethod::Item => return Err(Error::Unsupported("iloc construction method 2")),
        };
        let slice_of = |e: &Extent| -> Result<&'a [u8]> {
            let start = usize::try_from(e.offset).map_err(|_| Error::Invalid("extent offset"))?;
            let end = if e.length == 0 {
                base.len()
            } else {
                start
                    .checked_add(usize::try_from(e.length).map_err(|_| Error::Invalid("extent length"))?)
                    .ok_or(Error::Invalid("extent overflow"))?
            };
            base.get(start..end).ok_or(Error::Invalid("extent outside of file"))
        };
        if let [single] = item.extents.as_slice() {
            return slice_of(single).map(Cow::Borrowed);
        }
        let mut out = Vec::new();
        for e in &item.extents {
            let s = slice_of(e)?;
            // Overlapping extents could otherwise turn a tiny file into a huge allocation.
            if out.len() + s.len() > self.limits.max_item_bytes.min(base.len()) {
                return Err(Error::LimitExceeded("item size"));
            }
            out.extend_from_slice(s);
        }
        Ok(Cow::Owned(out))
    }
}

/// Grid derivation (`grid` item payload).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridInfo {
    pub rows: u32,
    pub columns: u32,
    pub width: u32,
    pub height: u32,
}

impl GridInfo {
    pub fn parse(payload: &[u8]) -> Result<Self> {
        let mut r = crate::reader::Reader::new(payload);
        if r.u8()? != 0 {
            return Err(Error::Unsupported("grid version"));
        }
        let wide = r.u8()? & 1 != 0;
        let rows = u32::from(r.u8()?) + 1;
        let columns = u32::from(r.u8()?) + 1;
        let (width, height) = if wide {
            (r.u32()?, r.u32()?)
        } else {
            (u32::from(r.u16()?), u32::from(r.u16()?))
        };
        Ok(Self {
            rows,
            columns,
            width,
            height,
        })
    }
}

/// Overlay derivation (`iovl` item payload): canvas colour, size and the per-layer offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayInfo {
    /// RGBA fill colour, 16 bits per channel.
    pub canvas_fill: [u16; 4],
    pub width: u32,
    pub height: u32,
    pub offsets: Vec<(i32, i32)>,
}

impl OverlayInfo {
    pub fn parse(payload: &[u8], layers: usize) -> Result<Self> {
        let mut r = crate::reader::Reader::new(payload);
        if r.u8()? != 0 {
            return Err(Error::Unsupported("overlay version"));
        }
        let wide = r.u8()? & 1 != 0;
        let canvas_fill = [r.u16()?, r.u16()?, r.u16()?, r.u16()?];
        let (width, height) = if wide {
            (r.u32()?, r.u32()?)
        } else {
            (u32::from(r.u16()?), u32::from(r.u16()?))
        };
        let mut offsets = Vec::with_capacity(layers.min(1024));
        for _ in 0..layers {
            offsets.push(if wide {
                (r.i32()?, r.i32()?)
            } else {
                (i32::from(r.u16()? as i16), i32::from(r.u16()? as i16))
            });
        }
        Ok(Self {
            canvas_fill,
            width,
            height,
            offsets,
        })
    }
}
