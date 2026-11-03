# imgalifier-avif

Pure-Rust AVIF decoder with no C or C++ dependencies (the AV1 decoder is a vendored copy of rav1d, built without assembly). Licensed `MIT AND BSD-2-Clause`.

```rust,no_run
let data = std::fs::read("photo.avif").unwrap();
let info = imgalifier_avif::avif_info(&data).unwrap();
println!("{}x{}", info.width, info.height);
let rgba = imgalifier_avif::decode_avif(&data).unwrap();
assert_eq!(rgba.data.len(), rgba.width as usize * rgba.height as usize * 4);
```

- Container and pipeline come from `imgalifier-heif`: grids, alpha (including limited-range alpha), `clap`/`irot`/`imir`, first frame of `avis` sequences.
- AV1: 8, 10 and 12 bit, 4:0:0, 4:2:0, 4:2:2, 4:4:4, film grain.
- Safety: frame size limit before allocation, bounds-checked container parsing, codec panics are returned as errors.

See `THIRD_PARTY.md` in the repository for the vendored rav1d sources.

## `image` crate integration

Enable the `image` feature and call `imgalifier_avif::register_decoding_hooks()` once at start-up; afterwards `image::open` and `ImageReader` decode these files like any built-in format (opaque pictures come out as RGB8, the rest as RGBA8). The decoder is lazy: size and alpha come from the header, pixels are decoded only in `read_image`, after `image::Limits` were applied. This is a drop-in replacement for `libheif_rs::integration::image::register_all_decoding_hooks()`.
