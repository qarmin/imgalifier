use crate::container::{Clap, Property};
use crate::error::{Error, Result};
use crate::image::RgbaImage;

/// Applies `clap`, `irot` and `imir` in association order (the order mandated by HEIF).
pub fn apply_transformative_properties<'a>(
    mut img: RgbaImage,
    props: impl Iterator<Item = &'a Property>,
) -> Result<RgbaImage> {
    for p in props {
        img = match p {
            Property::Clap(c) => crop(&img, c)?,
            Property::Irot(a) => rotate_ccw(&img, *a),
            Property::Imir(axis) => mirror(&img, *axis),
            _ => img,
        };
    }
    Ok(img)
}

fn ratio(n: u32, d: u32) -> Result<f64> {
    if d == 0 {
        return Err(Error::Invalid("clap with zero denominator"));
    }
    Ok(f64::from(n) / f64::from(d))
}

fn crop(img: &RgbaImage, c: &Clap) -> Result<RgbaImage> {
    let cw = ratio(c.width.0, c.width.1)?.round();
    let ch = ratio(c.height.0, c.height.1)?.round();
    let h_off = f64::from(c.h_off.0) / f64::from(c.h_off.1.max(1));
    let v_off = f64::from(c.v_off.0) / f64::from(c.v_off.1.max(1));
    // Ties round down to match libheif; the spec leaves half-pixel apertures open.
    let left = (h_off + f64::from(img.width - 1) / 2.0 - (cw - 1.0) / 2.0).floor();
    let top = (v_off + f64::from(img.height - 1) / 2.0 - (ch - 1.0) / 2.0).floor();
    if cw < 1.0 || ch < 1.0 || left < 0.0 || top < 0.0 {
        return Err(Error::Invalid("clap outside of the image"));
    }
    let (left, top, cw, ch) = (left as u32, top as u32, cw as u32, ch as u32);
    if left + cw > img.width || top + ch > img.height {
        return Err(Error::Invalid("clap outside of the image"));
    }
    let mut data = Vec::with_capacity(cw as usize * ch as usize * 4);
    for y in top..top + ch {
        let start = (y as usize * img.width as usize + left as usize) * 4;
        data.extend_from_slice(
            img.data
                .get(start..start + cw as usize * 4)
                .ok_or(Error::Invalid("crop"))?,
        );
    }
    Ok(RgbaImage {
        width: cw,
        height: ch,
        data,
    })
}

fn remap(img: &RgbaImage, new_w: u32, new_h: u32, f: impl Fn(u32, u32) -> (u32, u32)) -> RgbaImage {
    let mut data = vec![0; img.data.len()];
    for (i, px) in img.data.chunks_exact(4).enumerate() {
        let (x, y) = (i as u32 % img.width, i as u32 / img.width);
        let (nx, ny) = f(x, y);
        let o = (ny as usize * new_w as usize + nx as usize) * 4;
        if let Some(dst) = data.get_mut(o..o + 4) {
            dst.copy_from_slice(px);
        }
    }
    RgbaImage {
        width: new_w,
        height: new_h,
        data,
    }
}

fn rotate_ccw(img: &RgbaImage, steps: u8) -> RgbaImage {
    let (w, h) = (img.width, img.height);
    match steps & 3 {
        1 => remap(img, h, w, |x, y| (y, w - 1 - x)),
        2 => remap(img, w, h, |x, y| (w - 1 - x, h - 1 - y)),
        3 => remap(img, h, w, |x, y| (h - 1 - y, x)),
        _ => img.clone(),
    }
}

/// Axis 0 flips top-bottom, axis 1 flips left-right (matches libheif; verify against conformance files).
fn mirror(img: &RgbaImage, axis: u8) -> RgbaImage {
    let (w, h) = (img.width, img.height);
    if axis == 0 {
        remap(img, w, h, |x, y| (x, h - 1 - y))
    } else {
        remap(img, w, h, |x, y| (w - 1 - x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img_2x1() -> RgbaImage {
        RgbaImage {
            width: 2,
            height: 1,
            data: vec![1, 1, 1, 255, 2, 2, 2, 255],
        }
    }

    #[test]
    fn rotate_90_ccw_moves_right_pixel_to_top() {
        let r = rotate_ccw(&img_2x1(), 1);
        assert_eq!((r.width, r.height), (1, 2));
        assert_eq!(r.data, vec![2, 2, 2, 255, 1, 1, 1, 255]);
    }

    #[test]
    fn mirror_left_right() {
        assert_eq!(mirror(&img_2x1(), 1).data, vec![2, 2, 2, 255, 1, 1, 1, 255]);
    }

    #[test]
    fn crop_centered_single_pixel() {
        let c = Clap {
            width: (1, 1),
            height: (1, 1),
            h_off: (0, 1),
            v_off: (0, 1),
        };
        let r = crop(
            &RgbaImage {
                width: 3,
                height: 1,
                data: vec![1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3],
            },
            &c,
        )
        .unwrap();
        assert_eq!(r.data, vec![2, 2, 2, 2]);
    }
}
