use crate::obsp::{self, payload as obsp_payload};
use crate::pack::{dds, manifest, obpk, upak};
use crate::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub name: Option<String>,
    #[serde(default)]
    pub patch: Vec<Patch>,
    #[serde(default)]
    pub add: Vec<Add>,
    #[serde(default)]
    pub import: Vec<Import>,
    #[serde(default)]
    pub asset: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
pub struct Patch {
    pub entry: String,
    pub property: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct Add {
    pub entry: String,
    pub property: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct Import {
    pub entry: String,
    pub file: String,
}

#[derive(Debug, Deserialize)]
pub struct Asset {
    pub asset: String,
    pub file: String,
}

#[derive(Debug)]
pub struct ApplyReport {
    pub name: String,
    pub patched: Vec<String>,
    pub added: Vec<String>,
    pub imported: Vec<String>,
    pub assets: Vec<String>,
    pub backup: Option<PathBuf>,
    pub file_size: Option<usize>,
}

pub fn infer_kind(o: &obsp::Obsp, data: &[u8], name: &str) -> Result<u8> {
    for entry in &o.entries {
        if let Ok(payload) = obsp_payload::parse_payload(o.payload(data, entry)) {
            if let Some(found) = obsp_payload::find_property(&payload.properties, name) {
                return obsp_payload::kind_of(&found.value);
            }
        }
    }
    Err(format!("no existing property named {name} to infer its type").into())
}

pub fn load_manifest(path: &Path) -> Result<Manifest> {
    let text = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}

#[derive(Debug)]
pub enum Progress {
    Step(String),
    Bytes {
        label: String,
        done: u64,
        total: u64,
    },
}

pub fn apply(media_dir: &Path, manifest_path: &Path) -> Result<ApplyReport> {
    apply_with_progress(media_dir, manifest_path, &mut |_| {})
}

pub fn apply_with_progress(
    media_dir: &Path,
    manifest_path: &Path,
    progress: &mut dyn FnMut(Progress),
) -> Result<ApplyReport> {
    let manifest = load_manifest(manifest_path)?;
    let base = manifest_path.parent().unwrap_or(Path::new("."));
    let name = manifest
        .name
        .clone()
        .unwrap_or_else(|| "unnamed mod".into());

    let mut patched = Vec::new();
    let mut added = Vec::new();
    let mut imported = Vec::new();
    let mut assets = Vec::new();
    let mut backup = None;
    let mut file_size = None;

    let needs_scripts =
        !manifest.patch.is_empty() || !manifest.add.is_empty() || !manifest.import.is_empty();
    if needs_scripts {
        progress(Progress::Step("rebuilding scripts.obsp".into()));
        let obsp_path = media_dir.join("scripts.obsp");
        let data = std::fs::read(&obsp_path)?;
        let o = obsp::parse(&data)?;
        let mut replacements: HashMap<usize, Vec<u8>> = HashMap::new();

        for patch in &manifest.patch {
            let index = o
                .find_index(&patch.entry)
                .ok_or_else(|| format!("no entry named {}", patch.entry))?;
            let payload = replacements
                .get(&index)
                .cloned()
                .unwrap_or_else(|| o.payload(&data, &o.entries[index]).to_vec());
            let parsed = obsp_payload::parse_payload(&payload)?;
            let target = obsp_payload::resolve_path(&payload, &parsed, &patch.property)
                .ok_or_else(|| format!("no property {} on {}", patch.property, patch.entry))?;
            let old = target.value.clone();
            let mut new_payload = payload;
            obsp_payload::patch_target(&mut new_payload, &target, &patch.value)
                .map_err(|e| format!("{} {}: {e}", patch.entry, patch.property))?;
            replacements.insert(index, new_payload);
            patched.push(format!(
                "{} {}: {} -> {}",
                patch.entry, patch.property, old, patch.value
            ));
        }

        for add in &manifest.add {
            let index = o
                .find_index(&add.entry)
                .ok_or_else(|| format!("no entry named {}", add.entry))?;
            let payload = replacements
                .get(&index)
                .cloned()
                .unwrap_or_else(|| o.payload(&data, &o.entries[index]).to_vec());
            let parsed = obsp_payload::parse_payload(&payload)?;
            if parsed.get(&add.property).is_some() {
                return Err(
                    format!("{} already has {}; use [[patch]]", add.entry, add.property).into(),
                );
            }
            let kind = infer_kind(&o, &data, &add.property)?;
            let mut new_payload = payload;
            obsp_payload::add_property(&mut new_payload, &parsed, &add.property, kind, &add.value)
                .map_err(|e| format!("{} {}: {e}", add.entry, add.property))?;
            replacements.insert(index, new_payload);
            added.push(format!("{} {} = {}", add.entry, add.property, add.value));
        }

        for imp in &manifest.import {
            let index = o
                .find_index(&imp.entry)
                .ok_or_else(|| format!("no entry named {}", imp.entry))?;
            let path = resolve(base, &imp.file);
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
            imported.push(format!(
                "{} <- {} ({} bytes)",
                imp.entry,
                path.display(),
                bytes.len()
            ));
            replacements.insert(index, bytes);
        }

        if !replacements.is_empty() {
            let rebuilt = obsp::rebuild(&data, &o, &replacements)?;
            let bak = media_dir.join("scripts.obsp.bak");
            if !bak.exists() {
                std::fs::copy(&obsp_path, &bak)?;
            }
            std::fs::write(&obsp_path, &rebuilt)?;
            backup = Some(bak);
            file_size = Some(rebuilt.len());
        }
    }

    if !manifest.asset.is_empty() {
        let mut pending = Vec::new();
        for asset in &manifest.asset {
            let path = resolve(base, &asset.file);
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
            pending.push((asset.asset.clone(), path, bytes));
        }

        let manifest_file = media_dir.join("manifest.bin");
        let mut manifest_bytes = std::fs::read(&manifest_file)?;
        let m = crate::pack::manifest::parse(&manifest_bytes)?;
        let mut replacements = Vec::new();
        for (asset, path, bytes) in &pending {
            let index = m
                .find_index(asset)
                .ok_or_else(|| format!("asset not found in manifest: {asset}"))?;
            let entry = &m.files[index];
            let upak_len = m
                .packages
                .get(entry.pkg_idx as usize)
                .and_then(|name| std::fs::metadata(media_dir.join(format!("{name}.upak"))).ok())
                .map(|md| md.len())
                .unwrap_or(0);
            let old_size = upak::entry_size(&m, upak_len, entry);
            assets.push(format!(
                "{} <- {} ({} -> {} bytes)",
                m.full_name(entry),
                path.display(),
                old_size,
                bytes.len()
            ));
            replacements.push((index, bytes.clone()));
        }

        let results = upak::replace_assets_with_progress(
            media_dir,
            &m,
            &mut manifest_bytes,
            &replacements,
            &mut |label, done, total| {
                progress(Progress::Bytes {
                    label: label.to_string(),
                    done,
                    total,
                })
            },
        )?;
        for result in &results {
            if !result.in_place {
                assets.push(format!(
                    "[{}] {} asset(s), {} offsets shifted",
                    result.package,
                    result.replaced.len(),
                    result.shifted_entries
                ));
            }
        }
    }

    if patched.is_empty() && added.is_empty() && imported.is_empty() && assets.is_empty() {
        return Err("manifest contains no patches, adds, imports, or assets".into());
    }

    Ok(ApplyReport {
        name,
        patched,
        added,
        imported,
        assets,
        backup,
        file_size,
    })
}

fn resolve(base: &Path, file: &str) -> PathBuf {
    let path = PathBuf::from(file);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

#[derive(Debug)]
pub struct ExtractReport {
    pub obpk: PathBuf,
    pub dec: Option<PathBuf>,
    pub textures: Option<PathBuf>,
    pub texture_count: usize,
    /// Per-texture report lines from [`crate::pack::dds::extract`] for the caller
    /// to print.
    pub texture_lines: Vec<String>,
    pub bytes: usize,
}

pub fn extract_asset(media_dir: &Path, asset: &str) -> Result<ExtractReport> {
    let manifest_bytes = std::fs::read(media_dir.join("manifest.bin"))?;
    let m = manifest::parse(&manifest_bytes)?;
    let entry = m
        .find(asset)
        .ok_or_else(|| format!("asset not found in manifest: {asset}"))?;
    let carved = upak::carve(media_dir, &m, entry)?;
    let work = media_dir.join("_modkit_work");
    std::fs::create_dir_all(&work)?;
    let stem = entry.name.replace('\\', "_");
    let obpk_path = work.join(format!("{stem}.obpk"));
    std::fs::write(&obpk_path, &carved)?;

    let mut report = ExtractReport {
        obpk: obpk_path,
        dec: None,
        textures: None,
        texture_count: 0,
        texture_lines: Vec::new(),
        bytes: carved.len(),
    };
    if obpk::is_obpk(&carved) {
        let (_, payload) = obpk::decompress(&carved)?;
        let dec_path = work.join(format!("{stem}.dec"));
        std::fs::write(&dec_path, &payload)?;
        let tex_dir = work.join(format!("{stem}_textures"));
        let extracted = dds::extract(&payload, &tex_dir)?;
        report.texture_count = extracted.count;
        report.texture_lines = extracted.lines;
        report.dec = Some(dec_path);
        report.textures = Some(tex_dir);
    }
    Ok(report)
}
