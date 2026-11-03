use crate::error::{Error, Result};

/// Bounds-checked big-endian cursor; every read fails with `Invalid` instead of panicking.
#[derive(Clone)]
pub(crate) struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) const fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub(crate) const fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(Error::Invalid("length overflow"))?;
        let s = self
            .buf
            .get(self.pos..end)
            .ok_or(Error::Invalid("unexpected end of data"))?;
        self.pos = end;
        Ok(s)
    }

    pub(crate) fn rest(&mut self) -> &'a [u8] {
        let s = self.buf.get(self.pos..).unwrap_or_default();
        self.pos = self.buf.len();
        s
    }

    pub(crate) fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }

    pub(crate) fn u8(&mut self) -> Result<u8> {
        let [b] = self.array()?;
        Ok(b)
    }

    pub(crate) fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    pub(crate) fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.array()?))
    }

    pub(crate) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    pub(crate) fn fourcc(&mut self) -> Result<[u8; 4]> {
        self.array()
    }

    /// Reads a big-endian unsigned integer of `size` bytes (0, 4 or 8 in `iloc`; 0 yields 0).
    pub(crate) fn uint(&mut self, size: u8) -> Result<u64> {
        match size {
            0 => Ok(0),
            1 => self.u8().map(u64::from),
            2 => self.u16().map(u64::from),
            4 => self.u32().map(u64::from),
            8 => self.u64(),
            _ => Err(Error::Invalid("unsupported integer field size")),
        }
    }

    /// Null-terminated UTF-8 string (lossy); stops at the end of data if there is no terminator.
    pub(crate) fn cstring(&mut self) -> String {
        let rest = self.rest();
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        String::from_utf8_lossy(rest.get(..end).unwrap_or_default()).into_owned()
    }

    /// Skips a null-terminated string, leaving the cursor after the terminator.
    pub(crate) fn cstring_skip(&mut self) -> Result<()> {
        let tail = self.buf.get(self.pos..).unwrap_or_default();
        let end = tail
            .iter()
            .position(|&b| b == 0)
            .ok_or(Error::Invalid("unterminated string"))?;
        self.pos += end + 1;
        Ok(())
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }
}
