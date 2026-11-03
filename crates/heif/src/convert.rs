use crate::image::{ChromaFormat, RgbaImage, YuvImage};

struct Matrix {
    kr: f32,
    kb: f32,
}

/// Chromaticities (R, G, B, white point as CIE xy) of the ISO 23091-2 colour primaries that matter for photos.
fn primaries_xy(code: u16) -> Option<[(f64, f64); 4]> {
    const D65: (f64, f64) = (0.3127, 0.3290);
    Some(match code {
        1 => [(0.640, 0.330), (0.300, 0.600), (0.150, 0.060), D65],
        4 => [(0.670, 0.330), (0.210, 0.710), (0.140, 0.080), (0.310, 0.316)],
        5 => [(0.640, 0.330), (0.290, 0.600), (0.150, 0.060), D65],
        6 | 7 => [(0.630, 0.340), (0.310, 0.595), (0.155, 0.070), D65],
        8 => [(0.681, 0.319), (0.243, 0.692), (0.145, 0.049), (0.310, 0.316)],
        9 => [(0.708, 0.292), (0.170, 0.797), (0.131, 0.046), D65],
        11 => [(0.680, 0.320), (0.265, 0.690), (0.150, 0.060), (0.314, 0.351)],
        12 => [(0.680, 0.320), (0.265, 0.690), (0.150, 0.060), D65],
        22 => [(0.630, 0.340), (0.295, 0.605), (0.155, 0.077), D65],
        _ => return None,
    })
}

/// Luma coefficients (Kr, Kb) derived from the primaries: the Y row of the RGB to XYZ matrix.
fn derived_coefficients(primaries: u16) -> Option<Matrix> {
    let [(xr, yr), (xg, yg), (xb, yb), (xw, yw)] = primaries_xy(primaries)?;
    // Solve [xr/yr xg/yg xb/yb; 1 1 1; zr/yr zg/yg zb/yb] * S = white (X, 1, Z); S are the luminance weights.
    let col = |x: f64, y: f64| [x / y, 1.0, (1.0 - x - y) / y];
    let (r, g, b) = (col(xr, yr), col(xg, yg), col(xb, yb));
    let w = [xw / yw, 1.0, (1.0 - xw - yw) / yw];
    let det = |a: [f64; 3], b: [f64; 3], c: [f64; 3]| {
        a[0] * (b[1] * c[2] - b[2] * c[1]) - b[0] * (a[1] * c[2] - a[2] * c[1]) + c[0] * (a[1] * b[2] - a[2] * b[1])
    };
    let d = det(r, g, b);
    if d.abs() < 1e-9 {
        return None;
    }
    Some(Matrix {
        kr: (det(w, g, b) / d) as f32,
        kb: (det(r, g, w) / d) as f32,
    })
}

/// Matrix coefficients per ISO 23091-2; `None` means identity (GBR). Unspecified falls back to BT.601 like libheif.
fn matrix_for(mc: u16, primaries: u16) -> Option<Matrix> {
    match mc {
        0 => None,
        1 => Some(Matrix { kr: 0.2126, kb: 0.0722 }),
        9 | 10 => Some(Matrix { kr: 0.2627, kb: 0.0593 }),
        12 => Some(derived_coefficients(primaries).unwrap_or(Matrix { kr: 0.299, kb: 0.114 })),
        _ => Some(Matrix { kr: 0.299, kb: 0.114 }),
    }
}

/// Converts to 8-bit RGBA with nearest-neighbour chroma upsampling; `alpha` is an optional monochrome plane.
pub fn yuv_to_rgba(yuv: &YuvImage, alpha: Option<&YuvImage>) -> RgbaImage {
    yuv_to_rgba_threaded(yuv, alpha, 1)
}

/// Like [`yuv_to_rgba`], converting horizontal bands on up to `threads` threads (the result is identical).
pub fn yuv_to_rgba_threaded(yuv: &YuvImage, alpha: Option<&YuvImage>, threads: usize) -> RgbaImage {
    let (w, h) = (yuv.width as usize, yuv.height as usize);
    let max = f32::from(((1u32 << yuv.bit_depth) - 1) as u16);
    let matrix = matrix_for(yuv.color.matrix, yuv.color.primaries);
    let (sx, sy) = yuv.chroma.shifts();
    let (cw, _) = yuv.plane_dims(1);
    let (y_off, y_scale, c_scale) = if yuv.color.full_range {
        (0.0, 1.0 / max, 1.0 / max)
    } else {
        let s = (1u32 << yuv.bit_depth.saturating_sub(8)) as f32;
        (16.0 * s, 1.0 / (219.0 * s), 1.0 / (224.0 * s))
    };
    let c_mid = f32::from((1u32 << (yuv.bit_depth - 1)) as u16);
    let (alpha_off, alpha_scale) = alpha.map_or((0.0, 1.0), |a| {
        let s = (1u32 << a.bit_depth.saturating_sub(8)) as f32;
        let amax = f32::from(((1u32 << a.bit_depth) - 1) as u16);
        if a.color.full_range {
            (0.0, 1.0 / amax)
        } else {
            (16.0 * s, 1.0 / (219.0 * s))
        }
    });
    let mut data = vec![255u8; w * h * 4];
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let convert = |first_row: usize, out: &mut [u8]| {
        for (j, px) in out.chunks_exact_mut(4).enumerate() {
            let i = first_row * w + j;
            let (x, y) = (i % w, i / w);
            let luma = yuv.planes[0].get(i).copied().unwrap_or(0);
            let l = (f32::from(luma) - y_off) * y_scale;
            let (r, g, b) = if yuv.chroma == ChromaFormat::Monochrome {
                (l, l, l)
            } else {
                let ci = (y >> sy) * cw + (x >> sx);
                let cb = (f32::from(yuv.planes[1].get(ci).copied().unwrap_or(0)) - c_mid) * c_scale;
                let cr = (f32::from(yuv.planes[2].get(ci).copied().unwrap_or(0)) - c_mid) * c_scale;
                match &matrix {
                    _ if yuv.color.matrix == 16 => {
                        let d = i32::from(yuv.bit_depth);
                        let mid = 1i32 << (d - 1);
                        let y = i32::from(luma);
                        let cg = i32::from(yuv.planes[1].get(ci).copied().unwrap_or(0)) - mid;
                        let co = i32::from(yuv.planes[2].get(ci).copied().unwrap_or(0)) - mid;
                        let t = y - (cg >> 1);
                        let g = cg + t;
                        let b = t - (co >> 1);
                        let r = b + co;
                        let scale = 1.0 / ((1u32 << (d - 2)) - 1) as f32;
                        (r as f32 * scale, g as f32 * scale, b as f32 * scale)
                    }
                    _ if yuv.color.matrix == 8 => (l - cb + cr, l + cb, l - cb - cr),
                    None => (cr + 0.5, l, cb + 0.5),
                    Some(m) => {
                        let kg = 1.0 - m.kr - m.kb;
                        let r = l + 2.0 * (1.0 - m.kr) * cr;
                        let b = l + 2.0 * (1.0 - m.kb) * cb;
                        (r, (l - m.kr * r - m.kb * b) / kg, b)
                    }
                }
            };
            let [pr, pg, pb, pa] = px else { continue };
            *pr = to8(r);
            *pg = to8(g);
            *pb = to8(b);
            if let Some(a) = alpha {
                if let Some(&av) = a.planes[0].get(y * a.width as usize + x) {
                    *pa = to8((f32::from(av) - alpha_off) * alpha_scale);
                }
            }
        }
    };
    let threads = threads.clamp(1, h.max(1));
    if threads == 1 || w * h < (1 << 18) {
        convert(0, &mut data);
    } else {
        let rows_per_band = h.div_ceil(threads);
        std::thread::scope(|scope| {
            for (n, band) in data.chunks_mut(rows_per_band * w * 4).enumerate() {
                let convert = &convert;
                scope.spawn(move || convert(n * rows_per_band, band));
            }
        });
    }
    RgbaImage {
        width: yuv.width,
        height: yuv.height,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::ColorInfo;

    #[test]
    fn full_range_gray_stays_gray() {
        let mut y = YuvImage::new(2, 2, 8, ChromaFormat::Yuv420).unwrap();
        y.color = ColorInfo {
            matrix: 1,
            full_range: true,
            ..ColorInfo::default()
        };
        y.planes[0].fill(128);
        y.planes[1].fill(128);
        y.planes[2].fill(128);
        let rgba = yuv_to_rgba(&y, None);
        assert!(
            rgba.data
                .chunks_exact(4)
                .all(|p| p[0] == 128 && p[1] == 128 && p[2] == 128 && p[3] == 255)
        );
    }

    #[test]
    fn ycgco_re_is_the_exact_inverse_of_the_forward_transform() {
        // RGB (200, 100, 50) at 8 bit, coded in 10-bit planes: Co = R - B, t = B + Co/2, Cg = G - t, Y = t + Cg/2.
        let (r, g, b) = (200i32, 100i32, 50i32);
        let co = r - b;
        let t = b + (co >> 1);
        let cg = g - t;
        let luma = t + (cg >> 1);
        let mut y = YuvImage::new(1, 1, 10, ChromaFormat::Yuv444).unwrap();
        y.color = ColorInfo {
            matrix: 16,
            full_range: true,
            ..ColorInfo::default()
        };
        y.planes[0][0] = luma as u16;
        y.planes[1][0] = (cg + 512) as u16;
        y.planes[2][0] = (co + 512) as u16;
        assert_eq!(&yuv_to_rgba(&y, None).data[..3], &[200, 100, 50]);
    }

    #[test]
    fn derived_coefficients_reproduce_the_standard_ones() {
        let bt709 = derived_coefficients(1).unwrap();
        assert!((bt709.kr - 0.2126).abs() < 1e-3 && (bt709.kb - 0.0722).abs() < 1e-3);
        let bt2020 = derived_coefficients(9).unwrap();
        assert!((bt2020.kr - 0.2627).abs() < 1e-3 && (bt2020.kb - 0.0593).abs() < 1e-3);
        let p3 = derived_coefficients(12).unwrap();
        assert!((p3.kr - 0.2290).abs() < 1e-3 && (p3.kb - 0.0793).abs() < 1e-3);
    }

    #[test]
    fn ycgco_gray_stays_gray() {
        let mut y = YuvImage::new(1, 1, 8, ChromaFormat::Yuv444).unwrap();
        y.color = ColorInfo {
            matrix: 8,
            full_range: true,
            ..ColorInfo::default()
        };
        y.planes[0][0] = 100;
        y.planes[1][0] = 128;
        y.planes[2][0] = 128;
        assert_eq!(&yuv_to_rgba(&y, None).data[..3], &[100, 100, 100]);
    }

    #[test]
    fn limited_range_black_and_white() {
        let mut y = YuvImage::new(2, 1, 8, ChromaFormat::Yuv444).unwrap();
        y.color = ColorInfo {
            matrix: 6,
            full_range: false,
            ..ColorInfo::default()
        };
        y.planes[0].copy_from_slice(&[16, 235]);
        y.planes[1].fill(128);
        y.planes[2].fill(128);
        let rgba = yuv_to_rgba(&y, None);
        assert_eq!(&rgba.data[..3], &[0, 0, 0]);
        assert_eq!(&rgba.data[4..7], &[255, 255, 255]);
    }
}
