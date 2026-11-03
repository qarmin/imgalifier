use crate::container::OverlayInfo;
use crate::image::RgbaImage;

/// Paints `layers` (with their offsets) over a canvas filled with the overlay colour, using straight-alpha "over".
pub fn compose(info: &OverlayInfo, layers: &[RgbaImage]) -> RgbaImage {
    let (cw, ch) = (info.width as usize, info.height as usize);
    let fill = info.canvas_fill.map(|v| (v >> 8) as u8);
    let mut data: Vec<u8> = fill.iter().copied().cycle().take(cw * ch * 4).collect();
    for (layer, &(ox, oy)) in layers.iter().zip(&info.offsets) {
        for ly in 0..layer.height as usize {
            let Some(y) = usize::try_from(i64::from(oy) + ly as i64).ok().filter(|&y| y < ch) else {
                continue;
            };
            for lx in 0..layer.width as usize {
                let Some(x) = usize::try_from(i64::from(ox) + lx as i64).ok().filter(|&x| x < cw) else {
                    continue;
                };
                let src = layer
                    .data
                    .get((ly * layer.width as usize + lx) * 4..)
                    .and_then(|s| s.get(..4));
                let dst = data.get_mut((y * cw + x) * 4..).and_then(|d| d.get_mut(..4));
                if let (Some(s), Some(d)) = (src, dst) {
                    blend(s, d);
                }
            }
        }
    }
    RgbaImage {
        width: info.width,
        height: info.height,
        data,
    }
}

fn blend(src: &[u8], dst: &mut [u8]) {
    let (Some(&[sr, sg, sb, sa]), Some([dr, dg, db, da])) = (src.first_chunk::<4>(), dst.first_chunk_mut::<4>()) else {
        return;
    };
    if sa == 255 {
        (*dr, *dg, *db, *da) = (sr, sg, sb, sa);
        return;
    }
    let (sa, a) = (u32::from(sa), u32::from(*da));
    let out_a = sa * 255 + a * (255 - sa);
    if out_a == 0 {
        (*dr, *dg, *db, *da) = (0, 0, 0, 0);
        return;
    }
    let mix = |s: u8, d: u8| ((u32::from(s) * sa * 255 + u32::from(d) * a * (255 - sa) + out_a / 2) / out_a) as u8;
    (*dr, *dg, *db) = (mix(sr, *dr), mix(sg, *dg), mix(sb, *db));
    *da = out_a.div_ceil(255).min(255) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_layer_replaces_canvas_and_clips() {
        let info = OverlayInfo {
            canvas_fill: [0, 0, 0, 0xFFFF],
            width: 2,
            height: 1,
            offsets: vec![(1, 0)],
        };
        let layer = RgbaImage {
            width: 2,
            height: 1,
            data: vec![9, 9, 9, 255, 7, 7, 7, 255],
        };
        let out = compose(&info, &[layer]);
        assert_eq!(out.data, vec![0, 0, 0, 255, 9, 9, 9, 255]);
    }

    #[test]
    fn half_transparent_blends() {
        let info = OverlayInfo {
            canvas_fill: [0, 0, 0, 0xFFFF],
            width: 1,
            height: 1,
            offsets: vec![(0, 0)],
        };
        let layer = RgbaImage {
            width: 1,
            height: 1,
            data: vec![200, 100, 0, 128],
        };
        let out = compose(&info, &[layer]);
        assert_eq!(out.data, vec![100, 50, 0, 255]);
    }
}
