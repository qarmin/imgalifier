//! Codec-independent decoding pipeline: item resolution, grids, alpha, colour conversion and transforms.

use crate::container::{Colr, Container, FourCc, GridInfo, Item, OverlayInfo, Property, SequenceFrame};
use crate::convert::yuv_to_rgba_threaded;
use crate::error::{Error, Limits, Result};
use crate::image::{ColorInfo, RgbaImage, YuvImage};
use crate::overlay;
use crate::transform::apply_transformative_properties;

/// A still-picture codec (HEVC for HEIC, AV1 for AVIF) plugged into the shared pipeline.
pub trait CodecBackend: Sync {
    fn handles(&self, item_type: FourCc) -> bool;

    /// `config` is the body of the `hvcC`/`av1C` property, `payload` the item data.
    fn decode(&self, config: &[u8], payload: &[u8]) -> Result<YuvImage>;

    /// Whether the codec's own colour range signalling applies to alpha planes.
    /// HEVC (HEIF) alpha is always full range, AV1 (AVIF) alpha follows the sequence header range.
    fn alpha_honors_range(&self) -> bool {
        true
    }
}

/// Basic facts about the primary image, available without decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    /// Output size after `clap`, `irot` and `imir`.
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    /// Bits per channel from `pixi`, when present.
    pub bit_depth: Option<u8>,
    /// The file is an image sequence without a primary item; only the first frame is decoded.
    pub is_sequence: bool,
}

pub struct Decoder<'a, B> {
    container: Container<'a>,
    backend: B,
    limits: Limits,
}

impl<'a, B: CodecBackend> Decoder<'a, B> {
    pub fn new(data: &'a [u8], backend: B, limits: Limits) -> Result<Self> {
        Ok(Self {
            container: Container::parse(data, limits)?,
            backend,
            limits,
        })
    }

    pub const fn container(&self) -> &Container<'a> {
        &self.container
    }

    /// Decodes the primary image. A panic inside a codec (a bug triggered by hostile input) becomes an error.
    pub fn decode_primary(&self) -> Result<RgbaImage> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match (self.container.primary_item, &self.container.sequence) {
                (None, Some(frame)) => self.decode_sequence_frame(frame),
                _ => self.decode_rgba(self.container.primary()?, 0),
            }
        }))
        .unwrap_or(Err(Error::Invalid("decoder panicked on this input")))
    }

    /// First frame of an image sequence (files without a primary item), converted without alpha.
    fn decode_sequence_frame(&self, frame: &SequenceFrame) -> Result<RgbaImage> {
        if !self.backend.handles(frame.codec) {
            return Err(Error::Unsupported("sequence codec"));
        }
        self.check_size(frame.width, frame.height)?;
        let yuv = self
            .backend
            .decode(&frame.config, self.container.sequence_sample(frame)?)?;
        Ok(yuv_to_rgba_threaded(&yuv, None, self.worker_threads()))
    }

    /// Full RGBA picture of an item: derived overlays are composed in RGBA, coded and grid items go through YUV.
    fn decode_rgba(&self, item: &Item, depth: u8) -> Result<RgbaImage> {
        let rgba = if &item.item_type == b"iovl" {
            self.decode_overlay(item, depth)?
        } else {
            let planes = self.decode_item(item, depth)?;
            let mut rgba = yuv_to_rgba_threaded(&planes.yuv, planes.alpha.as_ref(), self.worker_threads());
            if planes.alpha.is_some() && self.container.references_from(item.id, *b"prem").next().is_some() {
                unpremultiply(&mut rgba.data);
            }
            rgba
        };
        let props = self.container.item_properties(item).map(|(p, _)| p);
        apply_transformative_properties(rgba, props)
    }

    fn decode_overlay(&self, item: &Item, depth: u8) -> Result<RgbaImage> {
        if depth > 0 {
            return Err(Error::Invalid("nested derived images"));
        }
        let ids: Vec<u32> = self.container.references_from(item.id, *b"dimg").collect();
        if ids.len() > self.limits.max_grid_tiles {
            return Err(Error::LimitExceeded("overlay layers"));
        }
        let info = OverlayInfo::parse(&self.container.item_data(item)?, ids.len())?;
        self.check_size(info.width, info.height)?;
        let mut layers = Vec::with_capacity(ids.len());
        for id in ids {
            let layer = self
                .container
                .item(id)
                .ok_or(Error::Invalid("missing overlay layer item"))?;
            layers.push(self.decode_rgba(layer, depth + 1)?);
        }
        Ok(overlay::compose(&info, &layers))
    }

    /// Size and format of the primary image without decoding any pixels.
    pub fn info(&self) -> Result<ImageInfo> {
        let Some(item) = self.container.primary_item.and_then(|id| self.container.item(id)) else {
            let frame = self
                .container
                .sequence
                .as_ref()
                .ok_or(Error::Invalid("no image in file"))?;
            return Ok(ImageInfo {
                width: frame.width,
                height: frame.height,
                has_alpha: false,
                bit_depth: None,
                is_sequence: true,
            });
        };
        let (mut width, mut height) = self.ispe(item).ok_or(Error::Invalid("missing ispe"))?;
        for (p, _) in self.container.item_properties(item) {
            match p {
                Property::Clap(c) if c.width.1 != 0 && c.height.1 != 0 => {
                    width = c.width.0 / c.width.1;
                    height = c.height.0 / c.height.1;
                }
                Property::Irot(a) if a & 1 == 1 => std::mem::swap(&mut width, &mut height),
                _ => {}
            }
        }
        let bit_depth = self.container.find_property(item, |p| match p {
            Property::Pixi { bits_per_channel } => bits_per_channel.first().copied(),
            _ => None,
        });
        let has_alpha = self.container.referrers(item.id, *b"auxl").into_iter().any(|id| {
            self.container.item(id).is_some_and(|aux| {
                self.container
                    .item_properties(aux)
                    .any(|(p, _)| matches!(p, Property::AuxC { aux_type } if is_alpha_urn(aux_type)))
            })
        });
        Ok(ImageInfo {
            width,
            height,
            has_alpha,
            bit_depth,
            is_sequence: false,
        })
    }

    fn check_size(&self, w: u32, h: u32) -> Result<()> {
        if u64::from(w) * u64::from(h) > self.limits.max_pixels {
            return Err(Error::LimitExceeded("image pixels"));
        }
        Ok(())
    }

    /// Decodes a coded or grid item to YUV plus its alpha plane, if any.
    fn decode_item(&self, item: &Item, depth: u8) -> Result<Planes> {
        if depth > 2 {
            return Err(Error::Invalid("nested derived images"));
        }
        let (mut yuv, tile_alpha) = if self.backend.handles(item.item_type) {
            let config = self
                .container
                .find_property(item, |p| match p {
                    Property::CodecConfig { data, .. } => Some(data.as_slice()),
                    _ => None,
                })
                .ok_or(Error::Invalid("missing codec configuration property"))?;
            if let Some((w, h)) = self.ispe(item) {
                self.check_size(w, h)?;
            }
            (self.backend.decode(config, &self.container.item_data(item)?)?, None)
        } else if &item.item_type == b"grid" {
            self.decode_grid(item, depth)?
        } else if &item.item_type == b"unci" {
            self.decode_uncompressed(item)?
        } else if &item.item_type == b"iden" {
            let source = self
                .container
                .references_from(item.id, *b"dimg")
                .next()
                .ok_or(Error::Invalid("iden without a source image"))?;
            let source = self
                .container
                .item(source)
                .ok_or(Error::Invalid("missing iden source item"))?;
            let planes = self.decode_item(source, depth + 1)?;
            (planes.yuv, planes.alpha)
        } else {
            return Err(Error::Unsupported("primary item type"));
        };
        if let Some(Colr::Nclx {
            primaries,
            transfer,
            matrix,
            full_range,
        }) = self.container.find_property(item, |p| match p {
            Property::Colr(c @ Colr::Nclx { .. }) => Some(c.clone()),
            _ => None,
        }) {
            yuv.color = ColorInfo {
                primaries,
                transfer,
                matrix,
                full_range,
            };
        }
        let alpha = match self.decode_alpha(item, depth)? {
            Some(a) => Some(a),
            None => tile_alpha,
        };
        Ok(Planes { yuv, alpha })
    }

    fn worker_threads(&self) -> usize {
        match self.limits.max_threads {
            0 => std::thread::available_parallelism().map_or(1, usize::from),
            n => n,
        }
    }

    /// Decodes one batch of grid tiles, in parallel when there is more than one.
    #[expect(
        clippy::needless_collect,
        reason = "every thread must be spawned before the first join"
    )]
    fn decode_tiles(&self, ids: &[u32], depth: u8) -> Vec<Result<Planes>> {
        let one = |id: u32| {
            let item = self
                .container
                .item(id)
                .ok_or(Error::Invalid("missing grid tile item"))?;
            self.decode_item(item, depth)
        };
        if let [id] = ids {
            return vec![one(*id)];
        }
        std::thread::scope(|scope| {
            let handles: Vec<_> = ids.iter().map(|&id| scope.spawn(move || one(id))).collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or(Err(Error::Invalid("decoder panicked on this input")))
                })
                .collect()
        })
    }

    /// Raw samples described by the `cmpd` and `uncC` properties; returns the planes and an optional alpha plane.
    fn decode_uncompressed(&self, item: &Item) -> Result<(YuvImage, Option<YuvImage>)> {
        let (w, h) = self.ispe(item).ok_or(Error::Invalid("unci without ispe"))?;
        self.check_size(w, h)?;
        let unc = self
            .container
            .find_property(item, |p| match p {
                Property::UncC(u) => Some(u),
                _ => None,
            })
            .ok_or(Error::Invalid("unci without uncC"))?;
        let cmpd: Vec<u16> = match (
            self.container.find_property(item, |p| match p {
                Property::Cmpd(t) => Some(t.clone()),
                _ => None,
            }),
            unc.profile,
        ) {
            (Some(t), _) => t,
            (None, Some(p)) if &p == b"rgb3" => vec![4, 5, 6],
            (None, Some(p)) if &p == b"rgba" => vec![4, 5, 6, 7],
            (None, Some(p)) if &p == b"abgr" => vec![7, 6, 5, 4],
            _ => return Err(Error::Invalid("unci without cmpd")),
        };
        let raw = self.container.item_data(item)?;
        let data = match self.container.find_property(item, |p| match p {
            Property::CmpC(kind) => Some(*kind),
            _ => None,
        }) {
            None => raw,
            Some(kind) => {
                let units = self.container.find_property(item, |p| match p {
                    Property::Icbr(r) => Some(r.as_slice()),
                    _ => None,
                });
                let base = item.extents.first().map_or(0, |e| e.offset);
                std::borrow::Cow::Owned(crate::uncompressed::decompress(
                    kind,
                    &raw,
                    units,
                    base,
                    self.limits.max_item_bytes,
                )?)
            }
        };
        crate::uncompressed::decode(unc, &cmpd, w, h, &data)
    }

    fn ispe(&self, item: &Item) -> Option<(u32, u32)> {
        self.container.find_property(item, |p| match p {
            Property::Ispe { width, height } => Some((*width, *height)),
            _ => None,
        })
    }

    /// Returns the grid canvas and, when every tile carries its own alpha item, the composed alpha canvas.
    fn decode_grid(&self, item: &Item, depth: u8) -> Result<(YuvImage, Option<YuvImage>)> {
        let grid = GridInfo::parse(&self.container.item_data(item)?)?;
        self.check_size(grid.width, grid.height)?;
        let count = grid.rows as usize * grid.columns as usize;
        if count > self.limits.max_grid_tiles {
            return Err(Error::LimitExceeded("grid tiles"));
        }
        let tiles: Vec<u32> = self.container.references_from(item.id, *b"dimg").collect();
        if tiles.len() != count {
            return Err(Error::Invalid("grid tile count differs from rows * columns"));
        }
        let mut out: Option<YuvImage> = None;
        let mut alpha_out: Option<YuvImage> = None;
        let mut all_alpha = true;
        let batch_size = self.worker_threads().min(tiles.len()).max(1);
        let mut decoded = tiles
            .chunks(batch_size)
            .flat_map(|batch| self.decode_tiles(batch, depth + 1))
            .enumerate();
        for (n, tile) in &mut decoded {
            let tile = tile?;
            let dst = match out.as_mut() {
                Some(d) => d,
                None => {
                    let mut d = YuvImage::new(grid.width, grid.height, tile.yuv.bit_depth, tile.yuv.chroma)?;
                    d.color = tile.yuv.color;
                    out.insert(d)
                }
            };
            if (tile.yuv.bit_depth, tile.yuv.chroma) != (dst.bit_depth, dst.chroma) {
                return Err(Error::Unsupported("grid tiles with different formats"));
            }
            let (col, row) = (n % grid.columns as usize, n / grid.columns as usize);
            let (x0, y0) = (col * tile.yuv.width as usize, row * tile.yuv.height as usize);
            blit(dst, &tile.yuv, x0, y0);
            match tile.alpha {
                Some(a) if all_alpha => {
                    let canvas = match alpha_out.as_mut() {
                        Some(c) => c,
                        None => {
                            let mut c = YuvImage::new(grid.width, grid.height, a.bit_depth, a.chroma)?;
                            c.color = a.color;
                            alpha_out.insert(c)
                        }
                    };
                    if (a.bit_depth, a.chroma) == (canvas.bit_depth, canvas.chroma) {
                        blit(canvas, &a, x0, y0);
                    } else {
                        all_alpha = false;
                    }
                }
                _ => all_alpha = false,
            }
        }
        let image = out.ok_or(Error::Invalid("empty grid"))?;
        Ok((image, if all_alpha { alpha_out } else { None }))
    }

    fn decode_alpha(&self, item: &Item, depth: u8) -> Result<Option<YuvImage>> {
        for id in self.container.referrers(item.id, *b"auxl") {
            let Some(aux) = self.container.item(id) else { continue };
            let is_alpha = self
                .container
                .item_properties(aux)
                .any(|(p, _)| matches!(p, Property::AuxC { aux_type } if is_alpha_urn(aux_type)));
            if !is_alpha {
                continue;
            }
            let mut alpha = self.decode_item(aux, depth + 1)?.yuv;
            if !self.backend.alpha_honors_range() {
                alpha.color.full_range = true;
            }
            let (w, h) = self.ispe(item).unwrap_or((alpha.width, alpha.height));
            if (alpha.width, alpha.height) != (w, h) {
                return Err(Error::Unsupported("alpha plane with a different size than the image"));
            }
            return Ok(Some(alpha));
        }
        Ok(None)
    }
}

struct Planes {
    yuv: YuvImage,
    alpha: Option<YuvImage>,
}

/// Copies `tile` into `dst` at luma position (x0, y0), clipping at the destination edges.
fn blit(dst: &mut YuvImage, tile: &YuvImage, x0: usize, y0: usize) {
    let (sx, sy) = dst.chroma.shifts();
    for p in 0..3 {
        let (dw, dh) = dst.plane_dims(p);
        let (tw, th) = tile.plane_dims(p);
        let (px, py) = if p == 0 { (x0, y0) } else { (x0 >> sx, y0 >> sy) };
        let (Some(dplane), Some(tplane)) = (dst.planes.get_mut(p), tile.planes.get(p)) else {
            continue;
        };
        for row in 0..th.min(dh.saturating_sub(py)) {
            let n = tw.min(dw.saturating_sub(px));
            if let (Some(d), Some(s)) = (
                dplane.get_mut((py + row) * dw + px..(py + row) * dw + px + n),
                tplane.get(row * tw..row * tw + n),
            ) {
                d.copy_from_slice(s);
            }
        }
    }
}

/// Alpha URNs: AVIF/MIAF `...:alpha`, HEVC `urn:mpeg:hevc:2015:auxid:1`, legacy `urn:mpeg:mpegB:cicp:systems:auxiliary:alpha`.
fn is_alpha_urn(urn: &str) -> bool {
    urn.contains("alpha") || urn == "urn:mpeg:hevc:2015:auxid:1"
}

/// Converts premultiplied-alpha RGBA to straight alpha in place.
fn unpremultiply(data: &mut [u8]) {
    for px in data.chunks_exact_mut(4) {
        if let [r, g, b, a] = px {
            if *a != 0 && *a != 255 {
                let a = u32::from(*a);
                for c in [r, g, b] {
                    *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::unpremultiply;

    #[test]
    fn unpremultiply_restores_straight_colour() {
        let mut px = [50, 100, 0, 128, 9, 9, 9, 0, 7, 7, 7, 255];
        unpremultiply(&mut px);
        assert_eq!(px, [100, 199, 0, 128, 9, 9, 9, 0, 7, 7, 7, 255]);
    }
}
