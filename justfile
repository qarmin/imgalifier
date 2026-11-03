set shell := ["bash", "-cu"]

default:
    @just --list

## fix
# fmt + clippy --fix + fmt
[group('fix')]
fix:
    cargo +nightly fmt
    cargo clippy --fix --allow-dirty --allow-staged --all-features --all-targets
    cargo +nightly fmt
    # stable rustfmt warns that the nightly-only import options in .rustfmt.toml aren't
    # available on this channel - harmless noise on every run, so it's silenced here.
    cargo fmt 2>/dev/null

## test
# unit and integration tests of both crates, with every feature
[group('test')]
test:
    cargo test --workspace --all-features

# clippy on every target with every feature
[group('test')]
clippy:
    cargo clippy --workspace --all-targets --all-features

## samples
# The compare recipes need a sample set that is not part of this repository: `samples` holds <origin>/<file> files,
# `refs` holds <origin>__<file>/libheif.png reference pictures (layout of the Czkawka zune_heic_test harness).
# Instead of PNGs the reference can be RGBA dumps (<origin>__<file>.rgba: u32 LE width, height, flags, then RGBA8)
# made with a reference decoder: set LIBHEIF_REF (heic) or LIBAVIF_REF (avif) to their directory.

# HEIC/HEIF files vs a reference (one line per file on stdout, summary on stderr)
[group('samples')]
compare-heic samples refs *filter:
    cargo run --release -q -p imgalifier-heif --example compare -- {{samples}} {{refs}} {{filter}}

# AVIF files vs a reference
[group('samples')]
compare-avif samples refs *filter:
    cargo run --release -q -p imgalifier-avif --example compare_avif -- {{samples}} {{refs}} {{filter}}

# Parse containers only: VERBOSE=1 lists items, properties and references
[group('samples')]
inspect *files:
    cargo run --release -q -p imgalifier-heif --example inspect -- {{files}}

# Time decoding of the given HEIC files
[group('samples')]
bench *files:
    cargo run --release -q -p imgalifier-heif --example bench -- {{files}}

## fuzz
# cargo-fuzz (nightly): targets heic, avif and container; the corpus directory is yours (not tracked)
[group('fuzz')]
fuzz target corpus seconds="600":
    cd fuzz && cargo +nightly fuzz run {{target}} {{corpus}} -- -max_total_time={{seconds}} -rss_limit_mb=3000 -timeout=20 -max_len=30000

## vendoring
# Re-vendor rav1d from a local checkout (see THIRD_PARTY.md)
[group('vendoring')]
vendor-rav1d path:
    scripts/vendor_rav1d.sh {{path}}

## release
# `just publish`: dry run of both crates (tests, clippy, `cargo publish --dry-run`).
# `just publish --force`: publishes for real, imgalifier-heif first, then imgalifier-avif which depends on it.
# Needs `cargo login` and a clean git tree.

# dry run of publishing; add --force to really publish
[group('release')]
publish *args:
    #!/usr/bin/env bash
    set -euo pipefail
    force=0
    for arg in {{args}}; do
        case "$arg" in
            --force) force=1 ;;
            *) echo "unknown argument '$arg' (the only supported one is --force)" >&2; exit 2 ;;
        esac
    done
    cargo test --workspace --all-features
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    if [ "$force" = 1 ]; then
        cargo publish -p imgalifier-heif
        cargo publish -p imgalifier-avif
        echo "published imgalifier-heif and imgalifier-avif"
    else
        cargo publish --dry-run -p imgalifier-heif
        # imgalifier-avif needs the published imgalifier-heif, so cargo cannot resolve it before the real release:
        # only check that its package contents can be assembled.
        cargo package -p imgalifier-avif --list > /dev/null
        echo "dry run ok; run 'just publish --force' to publish for real"
    fi
