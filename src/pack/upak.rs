use crate::pack::manifest::{FileEntry, Manifest};
use crate::Result;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub fn package_name<'a>(manifest: &'a Manifest, entry: &FileEntry) -> Result<&'a str> {
    manifest
        .packages
        .get(entry.pkg_idx as usize)
        .map(|s| s.as_str())
        .ok_or_else(|| format!("package index {} out of range", entry.pkg_idx).into())
}

pub fn entry_size(manifest: &Manifest, upak_len: u64, entry: &FileEntry) -> u64 {
    let mut same: Vec<&FileEntry> = manifest
        .files
        .iter()
        .filter(|f| f.pkg_idx == entry.pkg_idx)
        .collect();
    same.sort_by_key(|f| f.offset);
    let pos = same.iter().position(|f| {
        f.offset == entry.offset && f.name == entry.name && f.path_idx == entry.path_idx
    });
    match pos {
        Some(i) if i + 1 < same.len() => same[i + 1].offset - entry.offset,
        Some(_) => upak_len.saturating_sub(entry.offset),
        None => 0,
    }
}

pub fn carve(media_dir: &Path, manifest: &Manifest, entry: &FileEntry) -> Result<Vec<u8>> {
    let pkg = package_name(manifest, entry)?;
    let upak_path = media_dir.join(format!("{pkg}.upak"));
    let upak_len = std::fs::metadata(&upak_path)?.len();
    let size = entry_size(manifest, upak_len, entry);
    let mut file = File::open(&upak_path)?;
    file.seek(SeekFrom::Start(entry.offset))?;
    let mut buf = vec![0u8; size as usize];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

/// Read only the first `len` bytes of an entry (enough for an OBPK resource
/// header without carving the whole asset).
pub fn carve_range(
    media_dir: &Path,
    manifest: &Manifest,
    entry: &FileEntry,
    len: usize,
) -> Result<Vec<u8>> {
    let pkg = package_name(manifest, entry)?;
    let upak_path = media_dir.join(format!("{pkg}.upak"));
    let upak_len = std::fs::metadata(&upak_path)?.len();
    let size = entry_size(manifest, upak_len, entry).min(len as u64) as usize;
    let mut file = File::open(&upak_path)?;
    file.seek(SeekFrom::Start(entry.offset))?;
    let mut buf = vec![0u8; size];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

pub struct RepackResult {
    pub package: String,
    pub old_offset: u64,
    pub old_size: u64,
    pub new_size: u64,
    pub shifted_entries: usize,
    pub backup_pak: PathBuf,
    pub backup_manifest: Option<PathBuf>,
    pub in_place: bool,
}

pub struct PackageRepack {
    pub package: String,
    pub replaced: Vec<(String, u64, u64)>,
    pub shifted_entries: usize,
    pub backup_pak: PathBuf,
    pub backup_manifest: Option<PathBuf>,
    pub in_place: bool,
}

pub fn replace_asset(
    media_dir: &Path,
    manifest: &Manifest,
    manifest_bytes: &mut [u8],
    entry: &FileEntry,
    new_bytes: &[u8],
) -> Result<RepackResult> {
    let index = manifest
        .files
        .iter()
        .position(|f| {
            f.offset == entry.offset && f.name == entry.name && f.path_idx == entry.path_idx
        })
        .ok_or("entry not found in manifest")?;
    let upak_len =
        std::fs::metadata(media_dir.join(format!("{}.upak", package_name(manifest, entry)?)))?
            .len();
    let old_offset = entry.offset;
    let old_size = entry_size(manifest, upak_len, entry);
    let results = replace_assets(
        media_dir,
        manifest,
        manifest_bytes,
        &[(index, new_bytes.to_vec())],
    )?;
    let package = results.into_iter().next().ok_or("no package rewritten")?;
    Ok(RepackResult {
        package: package.package,
        old_offset,
        old_size,
        new_size: new_bytes.len() as u64,
        shifted_entries: package.shifted_entries,
        backup_pak: package.backup_pak,
        backup_manifest: package.backup_manifest,
        in_place: package.in_place,
    })
}

pub fn replace_assets(
    media_dir: &Path,
    manifest: &Manifest,
    manifest_bytes: &mut [u8],
    replacements: &[(usize, Vec<u8>)],
) -> Result<Vec<PackageRepack>> {
    replace_assets_with_progress(
        media_dir,
        manifest,
        manifest_bytes,
        replacements,
        &mut |_, _, _| {},
    )
}

pub fn replace_assets_with_progress(
    media_dir: &Path,
    manifest: &Manifest,
    manifest_bytes: &mut [u8],
    replacements: &[(usize, Vec<u8>)],
    progress: &mut dyn FnMut(&str, u64, u64),
) -> Result<Vec<PackageRepack>> {
    use std::collections::BTreeMap;

    let mut by_package: BTreeMap<u8, Vec<(usize, &[u8])>> = BTreeMap::new();
    for (index, bytes) in replacements {
        let entry = manifest
            .files
            .get(*index)
            .ok_or("entry index out of range")?;
        if entry.pkg_idx == 0xFF {
            return Err("cannot write to a reference-only entry".into());
        }
        by_package
            .entry(entry.pkg_idx)
            .or_default()
            .push((*index, bytes.as_slice()));
    }

    let mut results = Vec::new();
    for (pkg_idx, mut items) in by_package {
        let package = manifest
            .packages
            .get(pkg_idx as usize)
            .cloned()
            .ok_or("package index out of range")?;
        let pak_path = media_dir.join(format!("{package}.upak"));
        let manifest_path = media_dir.join("manifest.bin");
        let upak_len = std::fs::metadata(&pak_path)?.len();

        let mut entries: Vec<usize> = (0..manifest.files.len())
            .filter(|i| manifest.files[*i].pkg_idx == pkg_idx)
            .collect();
        entries.sort_by_key(|i| (manifest.files[*i].offset, *i));

        items.sort_by_key(|(index, _)| manifest.files[*index].offset);
        let mut replaced_info = Vec::new();
        for (index, bytes) in &items {
            let entry = &manifest.files[*index];
            let old_size = entry_size(manifest, upak_len, entry);
            replaced_info.push((entry.name.clone(), old_size, bytes.len() as u64));
        }

        let total_delta: i64 = replaced_info
            .iter()
            .map(|(_, old, new)| *new as i64 - *old as i64)
            .sum();

        if total_delta == 0 {
            let total_bytes: u64 = items.iter().map(|(_, bytes)| bytes.len() as u64).sum();
            progress(&package, 0, total_bytes);
            let mut file = std::fs::OpenOptions::new().write(true).open(&pak_path)?;
            for (index, bytes) in &items {
                file.seek(SeekFrom::Start(manifest.files[*index].offset))?;
                file.write_all(bytes)?;
            }
            progress(&package, total_bytes, total_bytes);
            results.push(PackageRepack {
                package,
                replaced: replaced_info,
                shifted_entries: 0,
                backup_pak: pak_path.with_extension("upak.bak"),
                backup_manifest: None,
                in_place: true,
            });
            continue;
        }

        let first = items
            .first()
            .map(|(index, _)| *index)
            .ok_or("empty replacement set")?;
        let first_pos = entries
            .iter()
            .position(|i| *i == first)
            .ok_or("replacement entry not in package")?;
        let first_offset = manifest.files[first].offset;

        let backup_pak = pak_path.with_extension("upak.bak");
        if !backup_pak.exists() {
            std::fs::copy(&pak_path, &backup_pak)?;
        }

        let mut src = File::open(&pak_path)?;
        let mut out: Vec<u8> = Vec::with_capacity(upak_len as usize);
        copy_range(
            &mut src,
            &mut out,
            first_offset,
            0,
            upak_len,
            &package,
            progress,
        )?;

        let replacements_by_index: BTreeMap<usize, &[u8]> =
            items.iter().map(|(i, b)| (*i, *b)).collect();

        let width = if manifest.version >= 13 { 8usize } else { 4 };
        let mut cursor = first_offset;
        let mut cum: i64 = 0;
        let mut shifted = 0usize;
        for index in entries.iter().skip(first_pos) {
            let entry = &manifest.files[*index];
            if entry.offset > cursor {
                src.seek(SeekFrom::Start(cursor))?;
                copy_range(
                    &mut src,
                    &mut out,
                    entry.offset - cursor,
                    cursor,
                    upak_len,
                    &package,
                    progress,
                )?;
            }
            let old_size = entry_size(manifest, upak_len, entry);
            let new_offset = (entry.offset as i64 + cum) as u64;
            if new_offset != entry.offset {
                let bytes: Vec<u8> = if width == 8 {
                    new_offset.to_le_bytes().to_vec()
                } else {
                    (new_offset as u32).to_le_bytes().to_vec()
                };
                manifest_bytes
                    .get_mut(entry.offset_ptr..entry.offset_ptr + width)
                    .ok_or("manifest offset field out of range")?
                    .copy_from_slice(&bytes);
                shifted += 1;
            }
            match replacements_by_index.get(index) {
                Some(bytes) => {
                    out.extend_from_slice(bytes);
                    cum += bytes.len() as i64 - old_size as i64;
                }
                None => {
                    src.seek(SeekFrom::Start(entry.offset))?;
                    let mut buf = vec![0u8; old_size as usize];
                    src.read_exact(&mut buf)?;
                    out.extend_from_slice(&buf);
                }
            }
            cursor = entry.offset + old_size;
            progress(&package, cursor.min(upak_len), upak_len);
        }
        if cursor < upak_len {
            src.seek(SeekFrom::Start(cursor))?;
            copy_range(
                &mut src,
                &mut out,
                upak_len - cursor,
                cursor,
                upak_len,
                &package,
                progress,
            )?;
        }

        let tmp_path = pak_path.with_extension("upak.new");
        let old_path = pak_path.with_extension("upak.old");
        std::fs::write(&tmp_path, &out)?;
        std::fs::rename(&pak_path, &old_path)?;
        std::fs::rename(&tmp_path, &pak_path)?;
        std::fs::remove_file(&old_path)?;

        let backup_manifest = manifest_path.with_extension("bin.bak");
        if !backup_manifest.exists() {
            std::fs::copy(&manifest_path, &backup_manifest)?;
        }
        std::fs::write(&manifest_path, &manifest_bytes[..])?;

        results.push(PackageRepack {
            package,
            replaced: replaced_info,
            shifted_entries: shifted,
            backup_pak,
            backup_manifest: Some(backup_manifest),
            in_place: false,
        });
    }

    Ok(results)
}

const COPY_CHUNK: usize = 1 << 20;

#[allow(clippy::too_many_arguments)]
fn copy_range(
    src: &mut File,
    out: &mut Vec<u8>,
    len: u64,
    base: u64,
    total: u64,
    label: &str,
    progress: &mut dyn FnMut(&str, u64, u64),
) -> Result<()> {
    let mut buf = vec![0u8; COPY_CHUNK];
    let mut done = 0u64;
    while done < len {
        let n = (len - done).min(COPY_CHUNK as u64) as usize;
        src.read_exact(&mut buf[..n])?;
        out.extend_from_slice(&buf[..n]);
        done += n as u64;
        progress(label, base + done, total);
    }
    Ok(())
}
