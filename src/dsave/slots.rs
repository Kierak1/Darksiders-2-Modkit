//! Multi-save management: list the `dssave*.dsav` slot files in a directory
//! with a per-file summary (size/mtime, `SaveGameInfo`, last-saved location
//! records, item and equipped counts).
//!
//! `save-copy-item` itself lives in [`super::items::copy_item`]; this module
//! only lists slots and locates the default save directory.

use super::parse::parse;
use crate::Result;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One `dssave*.dsav` file.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotSummary {
    pub path: PathBuf,
    pub bytes: u64,
    /// File mtime, when available.
    pub modified: Option<SystemTime>,
    /// `SaveGameInfo.counter`.
    pub counter: u32,
    /// `SaveGameInfo.global_version` (`0x0D015155` on the stock saves).
    pub global_version: u32,
    /// `SaveGameInfo.flags` (DLC bitmask).
    pub flags: u32,
    /// The module `n1` "last saved location" records:
    /// `[name, world, region, waypoint, load-region]`.
    pub locations: Vec<[String; 5]>,
    /// Total items in all lists.
    pub items: usize,
    /// Items flagged equipped (`SaveItem.b816 == 1`).
    pub equipped: usize,
}

/// List every `dssave*.dsav` in `dir`, sorted by file name. A file that does
/// not parse fails the listing with the file's path in the message.
pub fn list_slots(dir: &Path) -> Result<Vec<SlotSummary>> {
    let entries = std::fs::read_dir(dir)?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n.starts_with("dssave")
                        && Path::new(n)
                            .extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("dsav"))
                })
        })
        .collect();
    paths.sort();
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let save = parse(&data).map_err(|e| format!("{}: {e}", path.display()))?;
        let bytes = data.len() as u64;
        let modified = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok());
        let equipped = save
            .items()
            .iter()
            .filter(|r| {
                save.item(r.index)
                    .and_then(|i| i.save_body())
                    .is_some_and(|s| s.b816 == 1)
            })
            .count();
        out.push(SlotSummary {
            path,
            bytes,
            modified,
            counter: save.info.counter,
            global_version: save.info.global_version,
            flags: save.info.flags,
            locations: save.module.loc_records.clone(),
            items: save.items().len(),
            equipped,
        });
    }
    Ok(out)
}

/// The user's save directory: `DS2_SAVE_DIR` when set, else the first
/// `Documents\My Games\Darksiders2\*\dssave0.dsav` parent.
pub fn default_save_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("DS2_SAVE_DIR") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var_os("USERPROFILE")?;
    let root = PathBuf::from(home)
        .join("Documents")
        .join("My Games")
        .join("Darksiders2");
    // Candidates: the root itself (some profiles write `dssave*.dsav` straight
    // into it) plus every subdirectory holding a `dssave0.dsav`. The newest
    // `dssave0.dsav` wins, so the game's current profile is picked.
    let mut candidates: Vec<(SystemTime, PathBuf)> = Vec::new();
    let mut push = |dir: PathBuf| {
        if let Ok(md) = std::fs::metadata(dir.join("dssave0.dsav")) {
            candidates.push((md.modified().unwrap_or(SystemTime::UNIX_EPOCH), dir));
        }
    };
    push(root.clone());
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                push(path);
            }
        }
    }
    candidates.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    candidates.into_iter().next().map(|(_, dir)| dir)
}

/// `YYYY-MM-DD HH:MM:SS` (UTC) for a file mtime.
pub fn format_time(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        // Clock before the epoch: print the epoch rather than fail.
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = super::write::civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}
