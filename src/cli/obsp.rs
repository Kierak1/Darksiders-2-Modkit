use std::path::Path;

use ds2_modkit::obsp::{self, payload as obsp_payload};

pub(crate) fn hash(args: &[String]) -> ds2_modkit::Result<()> {
    let name = args.get(1).ok_or("usage: ds2-modkit hash <name>")?;
    println!("{:#018X}", ds2_modkit::hash::crc64(name.as_bytes()));
    Ok(())
}

pub(crate) fn run(cmd: &str, args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let mut data = std::fs::read(media.join("scripts.obsp"))?;
    let obsp = obsp::parse(&data)?;
    if cmd == "obsp-list" {
        let filter = args.get(1).map(|s| s.to_lowercase());
        for entry in &obsp.entries {
            let names = obsp.entry_names(entry).join(" | ");
            if filter
                .as_ref()
                .is_none_or(|f| names.to_lowercase().contains(f))
            {
                println!(
                    "0x{:08X} {:6} {}",
                    entry.payload_offset, entry.payload_size, names
                );
            }
        }
        return Ok(());
    }
    let name = args
        .get(1)
        .ok_or("usage: ds2-modkit obsp-<list|dump|show|set|import> <entry> [property] [value]")?;
    let entry = obsp
        .find(name)
        .ok_or_else(|| format!("no OBSP entry named {name}"))?;
    let entry_label = obsp.entry_names(entry).join(" | ");
    let start = entry.payload_offset as usize;
    let size = entry.payload_size as usize;

    if cmd == "obsp-dump" {
        let payload = &data[start..start + size];
        let work = media.join("_modkit_work");
        std::fs::create_dir_all(&work)?;
        let out = work.join(format!("obsp_{}.bin", name.replace(['\\', '/'], "_")));
        std::fs::write(&out, payload)?;
        println!(
            "{entry_label} -> {} ({} bytes)",
            out.display(),
            payload.len()
        );
        return Ok(());
    }

    if cmd == "obsp-import" {
        let file = args
            .get(2)
            .ok_or("usage: ds2-modkit obsp-import <entry> <payload-file>")?;
        let new_payload = std::fs::read(file)?;
        let index = obsp
            .find_index(name)
            .ok_or_else(|| format!("no OBSP entry named {name}"))?;
        let mut replacements = std::collections::HashMap::new();
        replacements.insert(index, new_payload.clone());
        let rebuilt = obsp::rebuild(&data, &obsp, &replacements)?;
        let path = media.join("scripts.obsp");
        let backup = media.join("scripts.obsp.bak");
        if !backup.exists() {
            std::fs::copy(&path, &backup)?;
        }
        std::fs::write(&path, &rebuilt)?;
        let status = match obsp_payload::parse_payload(&new_payload) {
            Ok(p) => format!(
                "{} properties{}",
                p.properties.len(),
                if p.complete { "" } else { ", partial" }
            ),
            Err(_) => "unparsed".to_string(),
        };
        println!(
            "{entry_label}: payload {size} -> {} bytes, file {} -> {} bytes ({status})",
            new_payload.len(),
            data.len(),
            rebuilt.len()
        );
        return Ok(());
    }

    let payload = obsp_payload::parse_payload(&data[start..start + size])?;
    if cmd == "obsp-show" {
        println!(
            "{entry_label} ({} properties{})",
            payload.properties.len(),
            if payload.complete { "" } else { ", partial" }
        );
        for p in &payload.properties {
            println!("  {} = {}", p.name, p.value);
        }
        return Ok(());
    }

    if cmd == "obsp-set" {
        let prop_path = args
            .get(2)
            .ok_or("usage: ds2-modkit obsp-set <entry> <property-path> <value>")?;
        let repr = args
            .get(3)
            .ok_or("usage: ds2-modkit obsp-set <entry> <property-path> <value>")?;
        let target = obsp_payload::resolve_path(&data[start..start + size], &payload, prop_path)
            .ok_or_else(|| format!("no property path {prop_path} on {entry_label}"))?;
        let old = target.value.clone();
        let mut buf = data[start..start + size].to_vec();
        obsp_payload::patch_target(&mut buf, &target, repr)?;
        data[start..start + size].copy_from_slice(&buf);
        let path = media.join("scripts.obsp");
        let backup = media.join("scripts.obsp.bak");
        if !backup.exists() {
            std::fs::copy(&path, &backup)?;
        }
        std::fs::write(&path, &data)?;
        println!("{entry_label} {prop_path}: {old} -> {repr}");
        return Ok(());
    }

    if cmd == "obsp-add-prop" {
        let prop_name = args
            .get(2)
            .ok_or("usage: ds2-modkit obsp-add-prop <entry> <property> <value>")?;
        let repr = args
            .get(3)
            .ok_or("usage: ds2-modkit obsp-add-prop <entry> <property> <value>")?;
        if payload.get(prop_name).is_some() {
            return Err(
                format!("{prop_name} already exists on {entry_label}; use obsp-set").into(),
            );
        }
        let kind = ds2_modkit::modkit::infer_kind(&obsp, &data, prop_name)?;
        let index = obsp
            .find_index(name)
            .ok_or_else(|| format!("no OBSP entry named {name}"))?;
        let mut buf = data[start..start + size].to_vec();
        obsp_payload::add_property(&mut buf, &payload, prop_name, kind, repr)?;
        let mut replacements = std::collections::HashMap::new();
        let new_len = buf.len();
        replacements.insert(index, buf);
        let rebuilt = obsp::rebuild(&data, &obsp, &replacements)?;
        let path = media.join("scripts.obsp");
        let backup = media.join("scripts.obsp.bak");
        if !backup.exists() {
            std::fs::copy(&path, &backup)?;
        }
        std::fs::write(&path, &rebuilt)?;
        println!("{entry_label} {prop_name} added = {repr} (payload {size} -> {new_len} bytes)");
        return Ok(());
    }

    Err(format!("unknown command: {cmd}").into())
}
