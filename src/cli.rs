//! CLI command implementations, grouped by domain.
//!
//! `main.rs` dispatches commands to these modules; each domain's `run`
//! returns `Some(result)` when it owns the command and `None` otherwise.

pub mod assets;
pub mod chars;
pub mod db;
pub mod map;
pub mod mods;
pub mod obsp;
pub mod save;

/// Parse a numeric argument, accepting a `0x` hex prefix.
pub(crate) fn parse_num(s: &str) -> Result<usize, ds2_modkit::Error> {
    if let Some(h) = s.strip_prefix("0x") {
        usize::from_str_radix(h, 16).map_err(|_| "bad number".into())
    } else {
        s.parse::<usize>().map_err(|_| "bad number".into())
    }
}
