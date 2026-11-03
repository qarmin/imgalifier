# Roadmap

Ideas and known gaps, roughly by value. Nothing here is required for the current feature set.

## Decoding features

- HEVC inter prediction (P/B slices): needed for HEIC files that code the primary image with inter frames and for frames after the first one of image sequences. Photos from phones do not use it.
- Full image sequences (`avis`, HEIC `msf1`): only the first frame is decoded today; expose all frames with durations.
- `mini` box (brand `mif3`) of newer libavif, `tmap` gain maps (HDR), thumbnails and auxiliary images (depth) as API, EXIF / XMP / ICC accessors.
- YCgCo-Ro (matrix 17) and ICtCp (matrix 14) conversion; both need a reference decoder or sample files to verify against.
- Other codecs inside HEIF: JPEG, AVC, VVC, JPEG 2000 payloads (`jpeg`, `avc1`, `vvc1`, `j2k1`), `lhv1` layers.
- Output: bilinear chroma upsampling (nearest neighbour today), 16-bit RGBA, optional ICC based colour management.

## Quality

- `clap` apertures with half-pixel positions combined with rotation or mirroring decode one row or column off compared with libheif (`kimono.mirror-vertical.rotate270.crop.avif`).
- Grids whose `dimg` list mixes colour and alpha tiles (`color_grid_alpha_grid_tile_shared_in_dimg.avif`, `..._shared_tiles.avif`).
- Alpha item without the primary item's `irot`/`imir` is not treated specially.
- Fuzzing for longer than an hour per target, with a CI job; rav1d is the main `unsafe` surface (about 590 uses).

## Performance

- AV1 decodes single threaded by default (`Av1Backend::threads`); large AVIF files would gain from rav1d's tile/frame threading once it is proven stable for stills.
- HEVC: CABAC renormalises one bit at a time, transforms have no zero-coefficient shortcuts, no SIMD, a single picture (not a grid) decodes on one thread. WPP and tile threading are possible.
- YUV to RGBA is scalar `f32` per pixel (multi-threaded by band); a fixed-point or SIMD path would be faster.
- 8-bit pictures are stored as `u16` samples internally.

## Dependencies and vendored code

- The vendored rav1d still pulls `parking_lot`, `atomig`, `strum`, `zerocopy` 0.7, `bitflags`, `paste`, `static_assertions`, `to_method` and `cfg-if`; `parking_lot` and `atomig` could be replaced by std types. Unused C-API wrappers could be pruned further.
- Track upstream rav1d and heifer-hevc-dec for fixes and re-vendor (`just vendor-rav1d <path>`).
