# imgalifier

Pure-Rust, MIT-licensed HEIC/HEIF and AVIF decoders with zero C/C++ dependencies, meant to replace the native `libheif`/`libavif` features in Czkawka.

| crate | purpose |
|---|---|
| `imgalifier-heif` (`crates/heif`) | HEIF/HEIC: ISOBMFF container, grids, overlays, alpha, `clap`/`irot`/`imir`, image sequences (first frame), uncompressed HEIF (`unci`), HEVC intra decoder, YUV to RGBA conversion, resource limits, `image` crate hooks |
| `imgalifier-avif` (`crates/avif`) | AVIF: AV1 backend (vendored rav1d, no assembly) plugged into the `heif` pipeline, `image` crate hooks |

Both crates share one pipeline: a codec only implements `CodecBackend` (`handles` + `decode` returning a `YuvImage`), everything else (items, grids, alpha, colour conversion, transforms, size limits) lives once in `imgalifier-heif`. See `ROADMAP.md` for known gaps and ideas, `THIRD_PARTY.md` for vendored code.

## Layout

- `crates/heif`, `crates/avif` - the two crates (examples: `inspect`, `compare`, `bench`, `to_png`)
- `fuzz/` - cargo-fuzz targets `heic`, `avif`, `container`
- `justfile` - `just test`, `just clippy`, `just compare-heic <samples> <refs>`, `just fuzz <target> <corpus>`; the sample sets used for comparisons are not part of the repository
- `scripts/vendor_rav1d.sh` - regenerates the vendored rav1d sources in `crates/avif/src/rav1d`

## Using it in Czkawka

Czkawka decodes through the `image` crate hooks, so the swap is one line per format: replace `libheif_rs::integration::image::register_all_decoding_hooks()` with `imgalifier_heif::register_decoding_hooks()` (feature `image`), and add `imgalifier_avif::register_decoding_hooks()` instead of the native `image/avif-native` feature. Orientation (`irot`/`imir`) is already applied, like with libheif. Run the decoders on as many files in parallel as you like: tile and colour conversion threads default to all cores, set `Limits::max_threads` to 1 when the caller already parallelises over files.

## Created with AI

This project was created with the help of AI (Claude, by Anthropic), working under human direction: the code and the tests were written by the AI. The decoders were checked against libheif, libavif and sample files, and fuzzed, but they have not had the long production use of the libraries they are meant to replace. Review the code and test it on your own data before relying on it.
