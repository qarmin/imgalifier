#!/usr/bin/env bash
# Vendors the Rust sources of rav1d (BSD-2-Clause, https://github.com/memorysafety/rav1d) into crates/avif/src/rav1d.
# Usage: scripts/vendor_rav1d.sh <path to a rav1d checkout>
# Only .rs files are copied (no asm, C or build script), crate-root paths are rewritten to live under this crate,
# and the C ABI exports (#[no_mangle]) are removed so no dav1d_* symbols leak into binaries that link us.
set -euo pipefail
src="${1:?path to rav1d checkout}"
dst="$(cd "$(dirname "$0")/.." && pwd)/crates/avif/src/rav1d"
rm -rf "$dst"
mkdir -p "$dst"
(cd "$src/src" && find . -name '*.rs' -print0 | xargs -0 -I{} cp --parents {} "$dst")
mv "$dst/lib.rs" "$dst/mod.rs"
cp "$src/COPYING" "$dst/COPYING"
git -C "$src" log -1 --format='rav1d commit %H (%cs)' > "$dst/VENDORED_FROM"
find "$dst" -name '*.rs' -print0 | xargs -0 sed -i -E \
  -e 's/(^|[^$A-Za-z0-9_])crate::/\1crate::rav1d::/g' \
  -e 's/\$crate::/$crate::rav1d::/g' \
  -e '/^\s*#\[(unsafe\()?no_mangle\)?\]\s*$/d' \
  -e 's/#\[(unsafe\()?no_mangle\)?\]//'

python3 -I - "$dst/mod.rs" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
i = s.index('#[cfg(not(any(feature = "bitdepth_8"')
header = '// Vendored from rav1d (BSD-2-Clause, see COPYING). Crate-level attributes were replaced; do not edit by hand,\n// change scripts/vendor_rav1d.sh instead.\n#![allow(warnings, clippy::all, clippy::pedantic, clippy::nursery, clippy::restriction)]\n\n'
open(p, 'w').write(header + s[i:])
PY

# Drop the libc, assert_matches and av-data dependencies (raw-cpuid is only used with the asm feature, which is off).
cat > "$dst/libc.rs" <<'RS'
//! The handful of C type aliases and errno values rav1d needs, so the `libc` crate is not required.
#![allow(non_camel_case_types)]

pub type ptrdiff_t = isize;
pub type intptr_t = isize;
pub type uintptr_t = usize;
pub type off_t = i64;

pub const ENOENT: i32 = 2;
pub const EAGAIN: i32 = 11;
pub const ENOMEM: i32 = 12;
pub const EINVAL: i32 = 22;
pub const ERANGE: i32 = 34;
pub const ENOPROTOOPT: i32 = 92;
RS
find "$dst" -name '*.rs' -not -name libc.rs -print0 | xargs -0 sed -i -E \
  -e 's/\buse libc::/use crate::rav1d::libc::/' \
  -e 's/([^:A-Za-z_])libc::([A-Za-z_])/\1crate::rav1d::libc::\2/g'
sed -i 's|^pub mod include {|pub(crate) mod libc;\n\npub mod include {|' "$dst/mod.rs"

python3 -I - "$dst" <<'PY'
import re, sys, pathlib
dst = pathlib.Path(sys.argv[1])

def must_sub(text, old, new, what):
    if old not in text:
        sys.exit(f"vendor_rav1d.sh: pattern for {what} not found, update the script")
    return text.replace(old, new)

def remove_blocks(text, header_regex):
    out, pos = [], 0
    for m in re.finditer(header_regex, text, flags=re.M):
        if m.start() < pos:
            continue
        i = text.index('{', m.end() - 1)
        depth = 0
        for j in range(i, len(text)):
            depth += (text[j] == '{') - (text[j] == '}')
            if depth == 0:
                break
        out.append(text[pos:m.start()])
        pos = j + 1
        while text[pos:pos + 1] == '\n':
            pos += 1
    out.append(text[pos:])
    return ''.join(out)

h = dst / 'include/dav1d/headers.rs'
s = h.read_text()
s = must_sub(s, 'use av_data::pixel;\n', '', 'av_data import in headers.rs')
s = remove_blocks(s, r'^impl (?:From|TryFrom)<[A-Za-z0-9]+> for pixel::\w+ \{')
if 'pixel::' in s:
    sys.exit('vendor_rav1d.sh: leftover pixel:: in headers.rs')
h.write_text(s)

r = dst / 'rust_api.rs'
s = r.read_text()
s = must_sub(s, 'pub use av_data::pixel;\n', '', 'av_data re-export')
s = re.sub(r'(?:[ \t]*///[^\n]*\n)*[ \t]*pub fn (?:color_primaries|transfer_characteristic|matrix_coefficients|color_range|chroma_location)\(&self\)[^{]*\{[^}]*\}\n\n?', '', s)
s = re.sub(r'\n\s*\.field\("(?:color_primaries|transfer_characteristic|matrix_coefficients|color_range|chroma_location)", &self\.\w+\(\)\)', '', s)
s = must_sub(s, '    pub fn offset(&self) -> i64 {\n        self.inner.m.offset\n    }\n', '''    pub fn offset(&self) -> i64 {
        self.inner.m.offset
    }

    /// ISO 23091-2 `matrix_coefficients` code point of the sequence header (255 for reserved values).
    pub fn matrix_coefficients_code(&self) -> u32 {
        self.inner.seq_hdr.as_ref().unwrap().mtrx as u32
    }

    /// Whether the sequence header signals full range samples.
    pub fn color_range_is_full(&self) -> bool {
        use crate::rav1d::include::dav1d::headers::Rav1dColorRange;
        matches!(self.inner.seq_hdr.as_ref().unwrap().color_range, Rav1dColorRange::Full)
    }
''', 'Picture::offset')
if 'pixel::' in s:
    sys.exit('vendor_rav1d.sh: leftover pixel:: in rust_api.rs')
r.write_text(s)

l = dst / 'lr_apply.rs'
s = l.read_text()
s = must_sub(s, 'use assert_matches::assert_matches;\n', '', 'assert_matches import in lr_apply.rs')
s = must_sub(s, 'assert_matches!(lr.r#type, Rav1dRestorationType::SgrProj(idx) => idx)',
             'match lr.r#type { Rav1dRestorationType::SgrProj(idx) => idx, _ => unreachable!() }', 'assert_matches use')
l.write_text(s)

c = dst / 'recon.rs'
s = c.read_text()
s = must_sub(s, 'use assert_matches::debug_assert_matches;\n', '', 'debug_assert_matches import')
s = must_sub(s, 'debug_assert_matches!(tx_class, TxClass::TwoD);', 'debug_assert!(matches!(tx_class, TxClass::TwoD));', 'debug_assert_matches 1')
s = must_sub(s, 'debug_assert_matches!(tx_class, TxClass::H | TxClass::V);', 'debug_assert!(matches!(tx_class, TxClass::H | TxClass::V));', 'debug_assert_matches 2')
c.write_text(s)
PY
