//! Uncompressed HEIF images (`unci`, ISO/IEC 23001-17): raw samples described by `cmpd` and `uncC`.

use crate::container::UncC;
use crate::error::{Error, Result};
use crate::image::{ChromaFormat, ColorInfo, YuvImage};

const MONO: u16 = 0;
const Y: u16 = 1;
const CB: u16 = 2;
const CR: u16 = 3;
const RED: u16 = 4;
const GREEN: u16 = 5;
const BLUE: u16 = 6;
const ALPHA: u16 = 7;
const PADDING: u16 = 12;

/// Where a stored component goes in the output planes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    Luma,
    Cb,
    Cr,
    Alpha,
    Skip,
}

struct Comp {
    role: Role,
    depth: u8,
    align: u8,
}

struct Layout {
    comps: Vec<Comp>,
    chroma: ChromaFormat,
    rgb: bool,
    out_depth: u8,
}

fn layout(unc: &UncC, cmpd: &[u16]) -> Result<Layout> {
    let types: Vec<u16> = unc
        .components
        .iter()
        .map(|c| {
            cmpd.get(usize::from(c.index))
                .copied()
                .ok_or(Error::Invalid("unci component index"))
        })
        .collect::<Result<_>>()?;
    let has = |t: u16| types.contains(&t);
    let rgb = has(RED) && has(GREEN) && has(BLUE);
    let ycc = has(Y) && has(CB) && has(CR);
    let mono = (has(MONO) || has(Y)) && !ycc && !rgb;
    if !(rgb || ycc || mono) {
        return Err(Error::Unsupported("unci component combination"));
    }
    let mut comps = Vec::new();
    for (c, &t) in unc.components.iter().zip(&types) {
        let role = match t {
            // RGB is stored as GBR planes (identity matrix): green is "luma", blue "Cb", red "Cr".
            MONO | Y | GREEN => Role::Luma,
            CB | BLUE => Role::Cb,
            CR | RED => Role::Cr,
            ALPHA => Role::Alpha,
            PADDING => Role::Skip,
            _ => return Err(Error::Unsupported("unci component type")),
        };
        if c.format != 0 || c.bit_depth > 16 {
            return Err(Error::Unsupported("unci sample format"));
        }
        comps.push(Comp {
            role,
            depth: c.bit_depth,
            align: c.align_size,
        });
    }
    let chroma = if mono {
        ChromaFormat::Monochrome
    } else {
        match (rgb, unc.sampling) {
            (true, _) | (_, 0) => ChromaFormat::Yuv444,
            (_, 1) => ChromaFormat::Yuv422,
            (_, 2) => ChromaFormat::Yuv420,
            _ => return Err(Error::Unsupported("unci 4:1:1 sampling")),
        }
    };
    let out_depth = comps
        .iter()
        .filter(|c| c.role != Role::Skip && c.role != Role::Alpha)
        .map(|c| c.depth)
        .max()
        .unwrap_or(8)
        .clamp(8, 16);
    Ok(Layout {
        comps,
        chroma,
        rgb,
        out_depth,
    })
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn read(&mut self, n: u8) -> Result<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = self
                .data
                .get(self.pos / 8)
                .ok_or(Error::Invalid("unci data ends early"))?;
            v = (v << 1) | u32::from((byte >> (7 - self.pos % 8)) & 1);
            self.pos += 1;
        }
        Ok(v)
    }

    fn align_to_byte(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }

    fn align_to(&mut self, bytes: u32) {
        self.align_to_byte();
        if bytes > 1 {
            let b = bytes as usize;
            self.pos = (self.pos / 8).div_ceil(b) * b * 8;
        }
    }

    /// Pads the row that started at bit `start` to a multiple of `bytes` (relative to the row start).
    fn align_row(&mut self, start: usize, bytes: u32) {
        self.align_to_byte();
        if bytes > 1 {
            let b = bytes as usize;
            self.pos = start + (self.pos - start).div_ceil(b * 8) * b * 8;
        }
    }

    fn sample(&mut self, c: &Comp, little_endian: bool) -> Result<u32> {
        if c.align == 0 {
            return self.read(c.depth);
        }
        self.align_to_byte();
        let n = usize::from(c.align);
        let start = self.pos / 8;
        let bytes = self
            .data
            .get(start..start + n)
            .ok_or(Error::Invalid("unci data ends early"))?;
        let v = if little_endian {
            bytes.iter().rev().fold(0u64, |a, &b| (a << 8) | u64::from(b))
        } else {
            bytes.iter().fold(0u64, |a, &b| (a << 8) | u64::from(b))
        };
        self.pos += n * 8;
        Ok((v & ((1u64 << c.depth) - 1)) as u32)
    }
}

/// Widens by repeating the sample's bits (so full scale maps to full scale), narrows by dropping low bits.
fn scale(v: u32, from: u8, to: u8) -> u16 {
    if from >= to {
        return (v >> (from - to)) as u16;
    }
    let (mut acc, mut have) = (u64::from(v), from);
    while have < to {
        acc = (acc << from) | u64::from(v);
        have += from;
    }
    (acc >> (have - to)) as u16
}

/// Decodes the raw data into planes (plus an alpha plane when the image has one).
pub fn decode(unc: &UncC, cmpd: &[u16], width: u32, height: u32, data: &[u8]) -> Result<(YuvImage, Option<YuvImage>)> {
    let lay = layout(unc, cmpd)?;
    // Every pixel needs at least one bit; refuse absurd sizes before allocating the planes.
    if u64::from(width) * u64::from(height) > data.len() as u64 * 8 {
        return Err(Error::Invalid("unci data too small for the image size"));
    }
    let mut yuv = YuvImage::new(width, height, lay.out_depth, lay.chroma)?;
    let alpha_depth = lay.comps.iter().find(|c| c.role == Role::Alpha).map(|c| c.depth);
    let mut alpha = alpha_depth
        .map(|d| YuvImage::new(width, height, d, ChromaFormat::Monochrome))
        .transpose()?;
    if unc.tile_cols == 0 || unc.tile_rows == 0 || width % unc.tile_cols != 0 || height % unc.tile_rows != 0 {
        return Err(Error::Unsupported("unci tile grid does not divide the image"));
    }
    let (tw, th) = ((width / unc.tile_cols) as usize, (height / unc.tile_rows) as usize);
    let mut bits = Bits { data, pos: 0 };
    if unc.interleave == 4 {
        // Tile-component: every component is stored as its own sequence of tiles.
        for ci in 0..lay.comps.len() {
            for ty in 0..unc.tile_rows as usize {
                for tx in 0..unc.tile_cols as usize {
                    bits.align_to(unc.tile_align_size);
                    let mut sink = Sink {
                        lay: &lay,
                        yuv: &mut yuv,
                        alpha: alpha.as_mut(),
                        x0: tx * tw,
                        y0: ty * th,
                    };
                    read_component(&mut bits, unc, &lay, ci, tw, th, &mut sink)?;
                }
            }
        }
        return Ok(finish(yuv, alpha, &lay));
    }
    for ty in 0..unc.tile_rows as usize {
        for tx in 0..unc.tile_cols as usize {
            bits.align_to(unc.tile_align_size);
            let mut sink = Sink {
                lay: &lay,
                yuv: &mut yuv,
                alpha: alpha.as_mut(),
                x0: tx * tw,
                y0: ty * th,
            };
            read_tile(&mut bits, unc, &lay, tw, th, &mut sink)?;
        }
    }
    Ok(finish(yuv, alpha, &lay))
}

fn finish(mut yuv: YuvImage, alpha: Option<YuvImage>, lay: &Layout) -> (YuvImage, Option<YuvImage>) {
    yuv.color = if lay.rgb {
        ColorInfo {
            primaries: 2,
            transfer: 2,
            matrix: 0,
            full_range: true,
        }
    } else {
        ColorInfo {
            primaries: 2,
            transfer: 2,
            matrix: 6,
            full_range: true,
        }
    };
    (yuv, alpha)
}

struct Sink<'a> {
    lay: &'a Layout,
    yuv: &'a mut YuvImage,
    alpha: Option<&'a mut YuvImage>,
    x0: usize,
    y0: usize,
}

impl Sink<'_> {
    /// Stores sample `v` of component `ci` at tile-local position (x, y) of that component's grid.
    fn put(&mut self, ci: usize, x: usize, y: usize, v: u32) {
        let Some(c) = self.lay.comps.get(ci) else { return };
        let (sx, sy) = self.yuv.chroma.shifts();
        let (plane, gx, gy) = match c.role {
            Role::Luma => (0, self.x0 + x, self.y0 + y),
            Role::Cb => (1, (self.x0 >> sx) + x, (self.y0 >> sy) + y),
            Role::Cr => (2, (self.x0 >> sx) + x, (self.y0 >> sy) + y),
            Role::Alpha => {
                if let Some(a) = self.alpha.as_mut() {
                    let w = a.width as usize;
                    if let Some(s) = a
                        .planes
                        .first_mut()
                        .and_then(|p| p.get_mut((self.y0 + y) * w + self.x0 + x))
                    {
                        *s = v as u16;
                    }
                }
                return;
            }
            Role::Skip => return,
        };
        let (pw, _) = self.yuv.plane_dims(plane);
        let out = scale(v, c.depth, self.lay.out_depth);
        if let Some(s) = self.yuv.planes.get_mut(plane).and_then(|p| p.get_mut(gy * pw + gx)) {
            *s = out;
        }
    }

    fn dims(&self, role: Role, tw: usize, th: usize) -> (usize, usize) {
        let (sx, sy) = self.yuv.chroma.shifts();
        match role {
            Role::Cb | Role::Cr => (tw >> sx, th >> sy),
            _ => (tw, th),
        }
    }
}

fn read_tile(bits: &mut Bits<'_>, unc: &UncC, lay: &Layout, tw: usize, th: usize, sink: &mut Sink<'_>) -> Result<()> {
    let le = unc.components_little_endian;
    match unc.interleave {
        0 => {
            for ci in 0..lay.comps.len() {
                read_component(bits, unc, lay, ci, tw, th, sink)?;
            }
        }
        1 => read_pixel_interleaved(bits, unc, lay, tw, th, sink)?,
        2 => read_mixed(bits, unc, lay, tw, th, sink)?,
        3 => {
            for y in 0..th {
                for (ci, c) in lay.comps.iter().enumerate() {
                    let (w, h) = sink.dims(c.role, tw, th);
                    if y >= h {
                        continue;
                    }
                    let row_start = bits.pos;
                    for x in 0..w {
                        let v = bits.sample(c, le)?;
                        sink.put(ci, x, y, v);
                    }
                    bits.align_row(row_start, unc.row_align_size);
                }
            }
        }
        _ => return Err(Error::Unsupported("unci interleave type")),
    }
    Ok(())
}

fn read_pixel_interleaved(
    bits: &mut Bits<'_>,
    unc: &UncC,
    lay: &Layout,
    tw: usize,
    th: usize,
    sink: &mut Sink<'_>,
) -> Result<()> {
    let le = unc.components_little_endian;
    let (sx, sy) = sink.yuv.chroma.shifts();
    let (bw, bh) = (1usize << sx, 1usize << sy);
    for by in 0..th.div_ceil(bh) {
        let row_start = bits.pos;
        for bx in 0..tw.div_ceil(bw) {
            let start = bits.pos;
            // One block holds bw*bh luma samples and one sample of every other component.
            for (ci, c) in lay.comps.iter().enumerate() {
                let n = if c.role == Role::Luma { bw * bh } else { 1 };
                for i in 0..n {
                    let v = bits.sample(c, le)?;
                    if c.role == Role::Luma {
                        sink.put(ci, bx * bw + i % bw, by * bh + i / bw, v);
                    } else {
                        sink.put(ci, bx, by, v);
                    }
                }
            }
            if unc.pixel_size > 0 {
                bits.pos = start + unc.pixel_size as usize * 8;
            }
        }
        bits.align_row(row_start, unc.row_align_size);
    }
    Ok(())
}

/// One component plane of a tile, rows padded to `row_align_size`.
fn read_component(
    bits: &mut Bits<'_>,
    unc: &UncC,
    lay: &Layout,
    ci: usize,
    tw: usize,
    th: usize,
    sink: &mut Sink<'_>,
) -> Result<()> {
    let c = lay.comps.get(ci).ok_or(Error::Invalid("unci component"))?;
    let (w, h) = sink.dims(c.role, tw, th);
    for y in 0..h {
        let row_start = bits.pos;
        for x in 0..w {
            let v = bits.sample(c, unc.components_little_endian)?;
            sink.put(ci, x, y, v);
        }
        bits.align_row(row_start, unc.row_align_size);
    }
    Ok(())
}

/// Mixed interleave: planar luma-like components and the Cb/Cr components interleaved on the chroma grid, in listing order.
fn read_mixed(bits: &mut Bits<'_>, unc: &UncC, lay: &Layout, tw: usize, th: usize, sink: &mut Sink<'_>) -> Result<()> {
    let chroma: Vec<usize> = lay
        .comps
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c.role, Role::Cb | Role::Cr))
        .map(|(i, _)| i)
        .collect();
    let mut chroma_done = false;
    for (ci, c) in lay.comps.iter().enumerate() {
        if matches!(c.role, Role::Cb | Role::Cr) {
            if chroma_done {
                continue;
            }
            chroma_done = true;
            let (cw, ch) = sink.dims(Role::Cb, tw, th);
            for y in 0..ch {
                let row_start = bits.pos;
                for x in 0..cw {
                    for &cj in &chroma {
                        let cc = lay.comps.get(cj).ok_or(Error::Invalid("unci component"))?;
                        let v = bits.sample(cc, unc.components_little_endian)?;
                        sink.put(cj, x, y, v);
                    }
                }
                bits.align_row(row_start, unc.row_align_size);
            }
        } else {
            read_component(bits, unc, lay, ci, tw, th, sink)?;
        }
    }
    Ok(())
}

/// Inflates the item data. `units` are (offset, size) ranges as file positions (or relative to the item when they do
/// not fall inside it); without them the whole item is one compressed unit.
pub fn decompress(
    kind: [u8; 4],
    item: &[u8],
    units: Option<&[(u64, u64)]>,
    item_file_offset: u64,
    limit: usize,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut unit = |bytes: &[u8]| -> Result<()> {
        let remaining = limit
            .checked_sub(out.len())
            .ok_or(Error::LimitExceeded("decompressed item size"))?;
        out.extend(inflate(kind, bytes, remaining)?);
        Ok(())
    };
    match units {
        None if matches!(&kind, b"zlib" | b"defl") => {
            let remaining = limit;
            out = inflate_concatenated(kind, item, remaining)?;
        }
        None => unit(item)?,
        Some(ranges) => {
            for &(offset, size) in ranges {
                let (offset, size) = (
                    usize::try_from(offset).map_err(|_| Error::Invalid("icbr"))?,
                    usize::try_from(size).map_err(|_| Error::Invalid("icbr"))?,
                );
                let base = usize::try_from(item_file_offset).map_err(|_| Error::Invalid("icbr"))?;
                let start = if offset >= base && offset - base + size <= item.len() {
                    offset - base
                } else {
                    offset
                };
                unit(
                    item.get(start..start.checked_add(size).ok_or(Error::Invalid("icbr"))?)
                        .ok_or(Error::Invalid("icbr range outside of item"))?,
                )?;
            }
        }
    }
    Ok(out)
}

#[cfg(feature = "unci-compression")]
fn inflate(kind: [u8; 4], data: &[u8], limit: usize) -> Result<Vec<u8>> {
    match &kind {
        b"zlib" => miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(data, limit)
            .map_err(|_| Error::Invalid("zlib stream")),
        b"defl" => miniz_oxide::inflate::decompress_to_vec_with_limit(data, limit)
            .map_err(|_| Error::Invalid("deflate stream")),
        b"brot" => {
            struct Capped<'a>(&'a mut Vec<u8>, usize);
            impl std::io::Write for Capped<'_> {
                fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                    if self.0.len() + buf.len() > self.1 {
                        return Err(std::io::Error::other("decompressed size limit"));
                    }
                    self.0.extend_from_slice(buf);
                    Ok(buf.len())
                }

                fn flush(&mut self) -> std::io::Result<()> {
                    Ok(())
                }
            }
            let mut out = Vec::new();
            brotli_decompressor::BrotliDecompress(&mut std::io::Cursor::new(data), &mut Capped(&mut out, limit))
                .map_err(|_| Error::Invalid("brotli stream"))?;
            Ok(out)
        }
        _ => Err(Error::Unsupported("unci compression type")),
    }
}

/// Several zlib or raw deflate streams stored back to back (one per tile or row) without a range table.
#[cfg(feature = "unci-compression")]
fn inflate_concatenated(kind: [u8; 4], data: &[u8], limit: usize) -> Result<Vec<u8>> {
    use miniz_oxide::inflate::stream::{InflateState, inflate};
    use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

    let format = if &kind == b"zlib" {
        DataFormat::Zlib
    } else {
        DataFormat::Raw
    };
    let mut state = InflateState::new_boxed(format);
    let mut chunk = vec![0u8; 1 << 16];
    let (mut out, mut pos) = (Vec::new(), 0usize);
    while pos < data.len() {
        let r = inflate(
            &mut state,
            data.get(pos..).unwrap_or_default(),
            &mut chunk,
            MZFlush::None,
        );
        out.extend_from_slice(chunk.get(..r.bytes_written).unwrap_or_default());
        pos += r.bytes_consumed;
        if out.len() > limit {
            return Err(Error::LimitExceeded("decompressed item size"));
        }
        match r.status {
            Ok(MZStatus::StreamEnd) => state = InflateState::new_boxed(format),
            Ok(_) | Err(MZError::Buf) if r.bytes_consumed + r.bytes_written > 0 => {}
            _ => return Err(Error::Invalid("deflate stream")),
        }
    }
    Ok(out)
}

#[cfg(not(feature = "unci-compression"))]
fn inflate_concatenated(_kind: [u8; 4], _data: &[u8], _limit: usize) -> Result<Vec<u8>> {
    Err(Error::Unsupported("compressed unci needs the unci-compression feature"))
}

#[cfg(not(feature = "unci-compression"))]
fn inflate(_kind: [u8; 4], _data: &[u8], _limit: usize) -> Result<Vec<u8>> {
    Err(Error::Unsupported("compressed unci needs the unci-compression feature"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::UncComponent;

    fn unc(interleave: u8, comps: &[(u8, u8)]) -> UncC {
        UncC {
            sampling: 0,
            interleave,
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
            components: comps
                .iter()
                .enumerate()
                .map(|(i, &(depth, align))| UncComponent {
                    index: i as u16,
                    bit_depth: depth,
                    format: 0,
                    align_size: align,
                })
                .collect(),
            profile: None,
        }
    }

    #[test]
    fn pixel_interleaved_rgba() {
        let (yuv, alpha) = decode(
            &unc(1, &[(8, 1); 4]),
            &[RED, GREEN, BLUE, ALPHA],
            2,
            1,
            &[10, 20, 30, 40, 11, 21, 31, 41],
        )
        .unwrap();
        // RGB is stored as G, B, R planes with the identity matrix
        assert_eq!(
            (yuv.planes[0].clone(), yuv.planes[1].clone(), yuv.planes[2].clone()),
            (vec![20, 21], vec![30, 31], vec![10, 11])
        );
        assert_eq!(alpha.unwrap().planes[0], vec![40, 41]);
    }

    #[test]
    fn component_interleaved_with_row_padding() {
        let mut u = unc(0, &[(8, 0)]);
        u.row_align_size = 4;
        let (yuv, _) = decode(&u, &[MONO], 3, 2, &[1, 2, 3, 0, 4, 5, 6, 0]).unwrap();
        assert_eq!(yuv.planes[0], vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn bit_packed_565_widens_by_bit_replication() {
        // R=31, G=63, B=31 packed MSB first in two bytes: full scale in every channel.
        let (yuv, _) = decode(
            &unc(1, &[(5, 0), (6, 0), (5, 0)]),
            &[RED, GREEN, BLUE],
            1,
            1,
            &[0xFF, 0xFF],
        )
        .unwrap();
        assert_eq!(
            (yuv.bit_depth, yuv.planes[0][0], yuv.planes[1][0], yuv.planes[2][0]),
            (8, 255, 255, 255)
        );
    }

    #[test]
    fn truncated_data_is_an_error() {
        decode(&unc(1, &[(8, 1); 3]), &[RED, GREEN, BLUE], 2, 2, &[0; 5]).unwrap_err();
    }

    #[cfg(feature = "unci-compression")]
    #[test]
    fn concatenated_zlib_streams_are_all_decoded() {
        let a = miniz_oxide::deflate::compress_to_vec_zlib(&[1, 2, 3], 6);
        let b = miniz_oxide::deflate::compress_to_vec_zlib(&[4, 5], 6);
        let data = [a, b].concat();
        assert_eq!(decompress(*b"zlib", &data, None, 0, 1024).unwrap(), vec![1, 2, 3, 4, 5]);
    }

    #[cfg(feature = "unci-compression")]
    #[test]
    fn decompression_respects_the_size_limit() {
        let data = miniz_oxide::deflate::compress_to_vec_zlib(&[0; 4096], 6);
        decompress(*b"zlib", &data, None, 0, 100).unwrap_err();
    }
}
