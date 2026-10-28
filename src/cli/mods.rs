use std::path::Path;

pub(crate) fn apply(media: &Path, args: &[String]) -> ds2_modkit::Result<()> {
    let manifest = args.get(1).ok_or("usage: ds2-modkit apply <mod.toml>")?;
    let report = ds2_modkit::modkit::apply(media, Path::new(manifest))?;
    for line in &report.patched {
        println!("patch: {line}");
    }
    for line in &report.added {
        println!("add: {line}");
    }
    for line in &report.imported {
        println!("import: {line}");
    }
    for line in &report.assets {
        println!("asset: {line}");
    }
    println!(
        "applied {} ({} patches, {} adds, {} imports, {} assets)",
        report.name,
        report.patched.len(),
        report.added.len(),
        report.imported.len(),
        report.assets.len()
    );
    if let (Some(size), Some(backup)) = (report.file_size, &report.backup) {
        println!(
            "scripts.obsp -> {size} bytes (backup: {})",
            backup.display()
        );
    }
    Ok(())
}
