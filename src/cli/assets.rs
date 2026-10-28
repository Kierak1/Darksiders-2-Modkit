use std::path::Path;

use ds2_modkit::pack::{dds, manifest, obpk, upak};

pub(crate) fn run(cmd: &str, args: &[String], media: &Path) -> Option<ds2_modkit::Result<()>> {
    match cmd {
        "list" => Some(list(args, media)),
        "asset-list" => Some(asset_list(args, media)),
        "extract" | "repack" => Some(extract_repack(cmd, args, media)),
        "upak-replace" => Some(upak_replace(args, media)),
        "dds-extract" => Some(dds_extract(args)),
        "dds-inject" => Some(dds_inject(args)),
        "tex-bake" => Some(tex_bake(args)),
        "obpk-probe" => Some(obpk_probe(args)),
        "obpk-unchunk" => Some(obpk_unchunk(args)),
        "obpk-repack" => Some(obpk_repack(args)),
        _ => None,
    }
}

fn obpk_probe(args: &[String]) -> ds2_modkit::Result<()> {
    use flate2::{Decompress, FlushDecompress, Status};

    let path = Path::new(
        args.get(1)
            .ok_or("usage: ds2-modkit obpk-probe <obpk-file>")?,
    );
    let data = std::fs::read(path)?;
    if !obpk::is_obpk(&data) {
        return Err("not an OBPK file".into());
    }
    let ds = u32::from_le_bytes(data[25..29].try_into().unwrap()) as usize;
    let mut pos = ds;
    let mut chunk = 0usize;
    let mut total_out = 0u64;
    let mut dirty = Vec::new();
    let mut out = Vec::new();
    let mut scratch = vec![0u8; 64 * 1024];
    while pos + 4 < data.len() {
        let usize_ = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        if usize_ == 0 || usize_ > 1 << 30 {
            break;
        }
        let input = &data[pos + 4..];
        out.clear();
        out.resize(usize_, 0);
        let mut d = Decompress::new(true);
        let status = d.decompress(input, &mut out, FlushDecompress::None)?;
        let produced = d.total_out() as usize;
        let clean = status == Status::StreamEnd && produced == usize_;
        let mut s = status;
        while s != Status::StreamEnd {
            let consumed = d.total_in() as usize;
            s = d.decompress(&input[consumed..], &mut scratch, FlushDecompress::None)?;
            if s == Status::BufError {
                return Err(format!("chunk {chunk}: inflate stuck").into());
            }
        }
        let consumed = d.total_in() as usize;
        if !clean {
            dirty.push((chunk, usize_, consumed));
        }
        total_out += produced as u64;
        pos += 4 + consumed;
        chunk += 1;
    }
    println!(
        "{}: {} chunks, {} payload bytes, {} chunks where the stream does not end at output completion",
        path.display(),
        chunk,
        total_out,
        dirty.len()
    );
    for (i, usize_, csize) in dirty.iter().take(40) {
        println!("  chunk {i} usize {usize_} compressed {csize}");
    }
    Ok(())
}

fn list(_args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let data = std::fs::read(media.join("manifest.bin"))?;
    let m = manifest::parse(&data)?;
    for f in m.files.iter().filter(|f| f.pkg_idx != 0xFF) {
        println!("off=0x{:09X} {}", f.offset, m.full_name(f));
    }
    Ok(())
}

fn asset_list(args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let data = std::fs::read(media.join("manifest.bin"))?;
    let m = manifest::parse(&data)?;
    let filter = args.get(1).map(|s| s.to_lowercase()).unwrap_or_default();
    let limit: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let mut n = 0usize;
    for f in m.files.iter().filter(|f| f.pkg_idx != 0xFF) {
        let full = m.full_name(f);
        if !filter.is_empty() && !full.to_lowercase().contains(&filter) {
            continue;
        }
        let pkg = m
            .packages
            .get(f.pkg_idx as usize)
            .map(String::as_str)
            .unwrap_or("?");
        println!("{full} [{pkg}] id {} off {:#X}", f.package_id, f.offset);
        n += 1;
        if n >= limit {
            break;
        }
    }
    println!("{n} shown");
    Ok(())
}

fn extract_repack(cmd: &str, args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let data = std::fs::read(media.join("manifest.bin"))?;
    let m = manifest::parse(&data)?;
    let asset = args
        .get(1)
        .ok_or("usage: ds2-modkit <extract|repack> <asset>")?;
    let entry = m
        .find(asset)
        .ok_or_else(|| format!("asset not found in manifest: {asset}"))?;
    let stem = entry.name.replace('\\', "_");
    let work = media.join("_modkit_work");
    std::fs::create_dir_all(&work)?;
    let obpk_path = work.join(format!("{stem}.obpk"));
    let dec_path = work.join(format!("{stem}.dec"));
    let tex_dir = work.join(format!("{stem}_textures"));

    if cmd == "extract" {
        let report = ds2_modkit::modkit::extract_asset(media, asset)?;
        for line in &report.texture_lines {
            println!("{line}");
        }
        let mut line = format!(
            "{} -> {} ({} bytes",
            m.full_name(entry),
            report.obpk.display(),
            report.bytes
        );
        if report.texture_count > 0 {
            line.push_str(&format!(
                ", {} textures -> {}",
                report.texture_count,
                report.textures.as_ref().unwrap().display()
            ));
        }
        line.push(')');
        println!("{line}");
    } else {
        let dec_data = std::fs::read(&dec_path)?;
        let payload = if tex_dir.is_dir() {
            dds::inject(&dec_data, &tex_dir)?
        } else {
            dec_data
        };
        let original = std::fs::read(&obpk_path)?;
        let out = obpk::compress(&obpk::header(&original)?, &payload)?;
        let out_path = work.join(format!("{stem}_repacked.obpk"));
        std::fs::write(&out_path, &out)?;
        println!("repacked -> {} ({} bytes)", out_path.display(), out.len());
    }
    Ok(())
}

fn upak_replace(args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let mut data = std::fs::read(media.join("manifest.bin"))?;
    let m = manifest::parse(&data)?;
    let asset = args
        .get(1)
        .ok_or("usage: ds2-modkit upak-replace <asset> <file>")?;
    let file = args
        .get(2)
        .ok_or("usage: ds2-modkit upak-replace <asset> <file>")?;
    let entry = m
        .find(asset)
        .ok_or_else(|| format!("asset not found in manifest: {asset}"))?;
    let new_bytes = std::fs::read(file)?;
    let report = upak::replace_asset(media, &m, &mut data, entry, &new_bytes)?;
    println!(
        "{} <- {} [{}] ({} -> {} bytes, {} following assets shifted{})",
        m.full_name(entry),
        file,
        report.package,
        report.old_size,
        report.new_size,
        report.shifted_entries,
        if report.in_place { ", in place" } else { "" }
    );
    Ok(())
}

fn dds_extract(args: &[String]) -> ds2_modkit::Result<()> {
    let dec = args
        .get(1)
        .ok_or("usage: ds2-modkit dds-extract <payload.dec> <out-dir>")?;
    let dir = args
        .get(2)
        .ok_or("usage: ds2-modkit dds-extract <payload.dec> <out-dir>")?;
    let payload = std::fs::read(dec)?;
    let report = dds::extract(&payload, Path::new(dir))?;
    for line in &report.lines {
        println!("{line}");
    }
    println!("extracted {} textures", report.count);
    Ok(())
}

fn dds_inject(args: &[String]) -> ds2_modkit::Result<()> {
    let dec = args
        .get(1)
        .ok_or("usage: ds2-modkit dds-inject <payload.dec> <tex-dir>")?;
    let dir = args
        .get(2)
        .ok_or("usage: ds2-modkit dds-inject <payload.dec> <tex-dir>")?;
    let payload = std::fs::read(dec)?;
    let out = dds::inject(&payload, Path::new(dir))?;
    std::fs::write(dec, &out)?;
    println!(
        "injected textures: {} -> {} bytes",
        payload.len(),
        out.len()
    );
    Ok(())
}

fn tex_bake(args: &[String]) -> ds2_modkit::Result<()> {
    let stock = args
        .get(1)
        .ok_or("usage: ds2-modkit tex-bake <stock.dds> <out.dds> <raw.rgba|--fill r,g,b>")?;
    let out_path = args
        .get(2)
        .ok_or("usage: ds2-modkit tex-bake <stock.dds> <out.dds> <raw.rgba|--fill r,g,b>")?;
    let stock_bytes = std::fs::read(stock)?;
    let arg = args
        .get(3)
        .ok_or("usage: ds2-modkit tex-bake <stock.dds> <out.dds> <raw.rgba|--fill r,g,b>")?;
    let baked = if let Some(rgb) = arg.strip_prefix("--fill") {
        let v: Vec<u8> = rgb
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        if v.len() != 3 {
            return Err("usage: --fill r,g,b".into());
        }
        ds2_modkit::pack::texbake::bake(&[], &stock_bytes, Some([v[0], v[1], v[2], 255]))?
    } else {
        let rgba = std::fs::read(arg)?;
        ds2_modkit::pack::texbake::bake(&rgba, &stock_bytes, None)?
    };
    std::fs::write(out_path, &baked)?;
    println!("baked {out_path} ({} bytes)", baked.len());
    Ok(())
}

fn obpk_unchunk(args: &[String]) -> ds2_modkit::Result<()> {
    let stock = args
        .get(1)
        .ok_or("usage: ds2-modkit obpk-unchunk <stock.obpk> <out.dec>")?;
    let out_path = args
        .get(2)
        .ok_or("usage: ds2-modkit obpk-unchunk <stock.obpk> <out.dec>")?;
    let payload = obpk::unchunk(&std::fs::read(stock)?)?;
    std::fs::write(out_path, &payload)?;
    println!("wrote {out_path} ({} bytes)", payload.len());
    Ok(())
}

fn obpk_repack(args: &[String]) -> ds2_modkit::Result<()> {
    let stock = args
        .get(1)
        .ok_or("usage: ds2-modkit obpk-repack <stock.obpk> <payload.dec> <out.obpk> [--exact]")?;
    let payload_path = args
        .get(2)
        .ok_or("usage: ds2-modkit obpk-repack <stock.obpk> <payload.dec> <out.obpk> [--exact]")?;
    let out_path = args
        .get(3)
        .ok_or("usage: ds2-modkit obpk-repack <stock.obpk> <payload.dec> <out.obpk> [--exact]")?;
    let exact = args.iter().any(|a| a == "--exact");
    let stock = std::fs::read(stock)?;
    let payload = std::fs::read(payload_path)?;
    let report = obpk::repack_chunks(&stock, &payload, exact)?;
    eprintln!(
        "repack: {} chunks, {} payload bytes, {} changed, {} -> {} bytes",
        report.chunks,
        report.payload_bytes,
        report.changed,
        report.stock_bytes,
        report.data.len()
    );
    let (chunks, bytes) = obpk::verify_chunks(&report.data, &payload)?;
    println!("verified {chunks} chunks / {bytes} payload bytes");
    std::fs::write(out_path, &report.data)?;
    println!("wrote {out_path} ({} bytes)", report.data.len());
    Ok(())
}
