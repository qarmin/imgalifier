use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
#[expect(clippy::error_impl_error, reason = "conventional crate-level error name")]
pub enum Error {
    /// The data is not a valid container or bitstream.
    Invalid(&'static str),
    /// Valid input that uses a feature this decoder does not implement.
    Unsupported(&'static str),
    /// A configured resource limit was exceeded.
    LimitExceeded(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(m) => write!(f, "invalid data: {m}"),
            Self::Unsupported(m) => write!(f, "unsupported: {m}"),
            Self::LimitExceeded(m) => write!(f, "limit exceeded: {m}"),
        }
    }
}

impl std::error::Error for Error {}

/// Resource limits applied while parsing and decoding untrusted input.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_items: usize,
    pub max_item_bytes: usize,
    pub max_pixels: u64,
    pub max_grid_tiles: usize,
    /// Worker threads for decoding grid tiles; 0 uses the available parallelism, 1 disables threading.
    pub max_threads: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_items: 4096,
            max_item_bytes: 512 << 20,
            max_pixels: 1 << 28,
            max_grid_tiles: 4096,
            max_threads: 0,
        }
    }
}
