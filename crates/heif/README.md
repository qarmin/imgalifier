# imgalifier-heif

Pure-Rust HEIF/HEIC decoder with no C or C++ dependencies. MIT licensed.

```rust,no_run
let data = std::fs::read("photo.heic").unwrap();
let info = imgalifier_heif::heic_info(&data).unwrap();
println!("{}x{}", info.width, info.height);
let rgba = imgalifier_heif::decode_heic(&data).unwrap();
assert_eq!(rgba.data.len(), rgba.width as usize * rgba.height as usize * 4);
```

- Uncompressed HEIF (`unci`, ISO 23001-17): component, pixel, mixed, row and tile-component interleave, tiles, bit packing, 4:2:0/4:2:2 YCbCr, RGB(A), alpha, zlib/deflate/brotli (feature `unci-compression`, on by default). Checked pixel for pixel against libheif 1.21 on 181 of 182 sample files.
- Container: `meta` items, `iloc` (file and `idat`), `iref`, properties (`ispe`, `pixi`, `colr`, `irot`, `imir`, `clap`, `auxC`, ...), grids, overlays, alpha planes, first frame of image sequences.
- HEVC: intra pictures (Main, Main 10, Main Still Picture, range extensions 8-12 bit 4:0:0 to 4:4:4); no inter prediction yet.
- Output: 8-bit RGBA. Grid tiles and the colour conversion run on all cores by default (`Limits::max_threads`).
- Safety: bounds-checked parsing, configurable limits, codec panics are returned as errors.

The container, grid, alpha, transform and colour-conversion code is codec independent and is reused by `imgalifier-avif` (build with `default-features = false` to leave out HEVC).

The HEVC decoder is derived from heifer-hevc-dec (MIT OR Apache-2.0); see `THIRD_PARTY.md` in the repository.

## `image` crate integration

Enable the `image` feature and call `imgalifier_heif::register_decoding_hooks()` once at start-up; afterwards `image::open` and `ImageReader` decode these files like any built-in format (opaque pictures come out as RGB8, the rest as RGBA8). The decoders are lazy: size and colour type come from the header, pixels are decoded only in `read_image`, after `image::Limits` (dimensions, `max_alloc`) were applied, so oversized files are rejected cheaply. A file without an `ftyp` box (for example a JPEG named `.heic`) is handed to the other decoders registered with `image`. This is a drop-in replacement for `libheif_rs::integration::image::register_all_decoding_hooks()`.
