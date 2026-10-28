//! Crate-level error type for the library API (AGENTS.md §2.1: libraries
//! return typed errors, never `Box<dyn Error>`).

use thiserror::Error;

/// Error returned by the library API.
///
/// `Msg` carries ad-hoc messages from parsed-input validation (bad magic,
/// out-of-range offsets, unsupported variants); the other variants wrap the
/// external error types that flow through `?`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// Ad-hoc message: bad input or unsupported data.
    #[error("{0}")]
    Msg(String),
    /// Filesystem or stream I/O.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// `mod.toml` deserialization.
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
    /// zlib stream decoding (the raw [`flate2::Decompress`] API).
    #[error(transparent)]
    Decompress(#[from] flate2::DecompressError),
    /// Integer literal parsing (`parse::<u32>`, `from_str_radix`).
    #[error(transparent)]
    ParseInt(#[from] std::num::ParseIntError),
    /// Float literal parsing (`parse::<f32>`).
    #[error(transparent)]
    ParseFloat(#[from] std::num::ParseFloatError),
}

impl From<String> for Error {
    fn from(msg: String) -> Self {
        Error::Msg(msg)
    }
}

impl From<&str> for Error {
    fn from(msg: &str) -> Self {
        Error::Msg(msg.to_owned())
    }
}
