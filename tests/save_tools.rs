//! Integration tests for the tools wave: totals/sorting, bulk ops, presets,
//! slot listing and cross-save item copying.
//!
//! Every test copies the user's saves to a fresh `%TEMP%` directory first and
//! never touches the originals; tests skip when their inputs are missing.

use ds2_modkit::dsave::{self, AttributeCatalog, ItemDb};
use std::path::{Path, PathBuf};

fn media_dir() -> PathBuf {
    std::env::var_os("DS2_MEDIA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("Game")
                .join("Darksiders II Deathinitive Edition")
                .join("media")
        })
}

fn save_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DS2_SAVE_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join("Documents")
        .join("My Games")
        .join("Darksiders2")
        .join("218190165")
}

fn load_db() -> Option<(ItemDb, ds2_modkit::obsp::Obsp, Vec<u8>)> {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return None;
    }
    let data = std::fs::read(&path).ok()?;
    let obsp = ds2_modkit::obsp::parse(&data).ok()?;
    let db = ItemDb::build(&obsp, &data);
    Some((db, obsp, data))
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ds2_save_tools_{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Copy `name` from the user's save dir into `dst`, returning the copy's path
/// and the original bytes.
fn copy_save(name: &str, dst: &Path) -> Option<(PathBuf, Vec<u8>)> {
    let src = save_dir().join(name);
    if !src.exists() {
        eprintln!("skipping: {} not found", src.display());
        return None;
    }
    let data = std::fs::read(&src).ok()?;
    let path = dst.join(name);
    std::fs::write(&path, &data).ok()?;
    Some((path, data))
}

fn item_index(save: &dsave::Save, def: &str) -> Option<usize> {
    dsave::find_items(save, def, None).into_iter().next()
}

#[test]
fn totals_and_sort_on_real_items() {
    let Some((_db, obsp, data)) = load_db() else {
        return;
    };
    let dir = temp_dir("totals");
    let Some((path, _original)) = copy_save("dssave0.dsav", &dir) else {
        return;
    };
    let save = dsave::parse(&std::fs::read(&path).unwrap()).unwrap();
    let catalog = AttributeCatalog::build(&obsp, &data, &[&save]);
    let totals = dsave::equipped_totals(&save, &catalog);
    assert!(!totals.is_empty(), "dssave0 has equipped static stats");
    println!(
        "equipped totals: {}",
        totals
            .iter()
            .map(|t| dsave::total_line(&t.attribute, t))
            .collect::<Vec<_>>()
            .join(", ")
    );
    // sorted_items returns indices in descending value order
    let sorted = dsave::sorted_items(&save, &catalog, "CriticalDamageMod");
    assert!(!sorted.is_empty(), "dssave0 has CriticalDamageMod stats");
    let values: Vec<i64> = sorted
        .iter()
        .map(|i| {
            dsave::stat_views(save.item(*i).unwrap(), &catalog)
                .into_iter()
                .find(|v| v.attribute == "CriticalDamageMod")
                .map(|v| v.value)
                .unwrap()
        })
        .collect();
    assert!(
        values.windows(2).all(|w| w[0] >= w[1]),
        "values must be descending: {values:?}"
    );
    println!("sorted by CriticalDamageMod: {sorted:?} / {values:?}");
}

#[test]
fn bulk_scale_round_trips_the_file() {
    let Some((_db, obsp, data)) = load_db() else {
        return;
    };
    let dir = temp_dir("bulk");
    let Some((path, original)) = copy_save("dssave0.dsav", &dir) else {
        return;
    };
    let mut save = dsave::parse(&original).unwrap();
    let catalog = AttributeCatalog::build(&obsp, &data, &[&save]);
    let Some(idx) = item_index(&save, "weapon_scythe_bheithir_talons") else {
        eprintln!("skipping: no bheithir in dssave0");
        return;
    };
    let before: Vec<i64> = dsave::stat_views(save.item(idx).unwrap(), &catalog)
        .iter()
        .map(|v| v.value)
        .collect();
    assert!(before.contains(&150), "CriticalDamageMod = 150: {before:?}");

    let op = dsave::BulkOp::Scale {
        attribute: "CriticalDamageMod".into(),
        percent: 200,
        scope: dsave::BulkScope::All,
    };
    let report = dsave::apply_bulk(&mut save, Some(&catalog), &op).unwrap();
    assert!(report.changed >= 1, "{report:?}");
    dsave::write_file(&save, &path).unwrap();
    let scaled = dsave::parse(&std::fs::read(&path).unwrap()).unwrap();
    let after: Vec<i64> = dsave::stat_views(scaled.item(idx).unwrap(), &catalog)
        .iter()
        .map(|v| v.value)
        .collect();
    assert!(after.contains(&300), "150 -> 300: {after:?}");

    let back = dsave::BulkOp::Scale {
        attribute: "CriticalDamageMod".into(),
        percent: 50,
        scope: dsave::BulkScope::All,
    };
    let mut scaled = scaled;
    dsave::apply_bulk(&mut scaled, Some(&catalog), &back).unwrap();
    dsave::write_file(&scaled, &path).unwrap();
    let final_bytes = std::fs::read(&path).unwrap();
    assert!(
        final_bytes == original,
        "scale x2 then /2 must restore the original bytes"
    );
}

#[test]
fn preset_apply_diff_and_restore() {
    let Some((_db, obsp, data)) = load_db() else {
        return;
    };
    let dir = temp_dir("preset");
    let Some((path, original)) = copy_save("dssave0.dsav", &dir) else {
        return;
    };
    let save = dsave::parse(&original).unwrap();
    let Some(idx) = item_index(&save, "weapon_scythe_bheithir_talons") else {
        eprintln!("skipping: no bheithir in dssave0");
        return;
    };
    let catalog = AttributeCatalog::build(&obsp, &data, &[&save]);
    let preset_path = dir.join("save.toml");
    std::fs::write(
        &preset_path,
        format!(
            "[[set_stat]]\nitem = {idx}\nattr = \"CriticalDamageMod\"\nvalue = 250\n\
             [[duplicate]]\nitem = {idx}\n"
        ),
    )
    .unwrap();

    let preset = dsave::load_preset(&preset_path).unwrap();
    let mut save = save;
    let lines = dsave::apply_preset(&mut save, None, Some(&catalog), &preset).unwrap();
    assert_eq!(lines.len(), 2, "{lines:?}");
    let report = dsave::write_file(&save, &path).unwrap();
    assert!(
        report.diff.iter().any(|l| l.contains("item(s)")),
        "duplicate shows in the diff: {:?}",
        report.diff
    );
    let written = dsave::parse(&std::fs::read(&path).unwrap()).unwrap();
    let crit = dsave::stat_views(written.item(idx).unwrap(), &catalog)
        .into_iter()
        .find(|v| v.attribute == "CriticalDamageMod")
        .unwrap();
    assert_eq!(crit.value, 250);
    assert_eq!(written.items().len(), save.items().len());
    let diff = written.diff(&dsave::parse(&original).unwrap());
    assert!(diff.len() >= 2, "set_stat change + list growth: {diff:?}");

    // restore the original bytes (the test owns this copy)
    std::fs::write(&path, &original).unwrap();
    let restored = dsave::parse(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(restored, dsave::parse(&original).unwrap());
}

#[test]
fn copy_item_between_two_save_copies() {
    let Some((db, _obsp, _data)) = load_db() else {
        return;
    };
    let dir = temp_dir("copy");
    let Some((src_path, src_bytes)) = copy_save("dssave0.dsav", &dir) else {
        return;
    };
    let Some((dst_path, _dst_bytes)) = copy_save("dssave1.dsav", &dir) else {
        return;
    };
    let src = dsave::parse(&src_bytes).unwrap();
    let mut dst = dsave::parse(&std::fs::read(&dst_path).unwrap()).unwrap();

    // pick a def-backed source item whose container exists in the destination
    let found = src.items().into_iter().find(|r| {
        let Some(item) = src.item(r.index) else {
            return false;
        };
        let Some(container) = db.get(&item.name).and_then(|d| d.container.clone()) else {
            return false;
        };
        item.save_body().is_some()
            && dst
                .module
                .inventory
                .iter()
                .any(|l| l.name.eq_ignore_ascii_case(&container))
    });
    let Some(r) = found else {
        eprintln!("skipping: no shared def container between dssave0/dssave1");
        return;
    };
    let name = src.item(r.index).unwrap().name.clone();
    let container = db
        .get(&name)
        .and_then(|d| d.container.clone())
        .unwrap_or_else(|| "PrimaryWeapon".to_string());
    let before = dst.items().len();
    let new_index = dsave::copy_item(&src, r.index, &mut dst, &container, Some(&db)).unwrap();
    assert_eq!(dst.items().len(), before + 1);
    let copied = dst.item(new_index).unwrap();
    assert_eq!(copied.name, name);
    assert_eq!(
        copied.save_body().unwrap().b816,
        0,
        "copies are not equipped"
    );
    dsave::write_file(&dst, &dst_path).unwrap();
    let re = dsave::parse(&std::fs::read(&dst_path).unwrap()).unwrap();
    assert_eq!(re.items().len(), before + 1);
    assert!(!dsave::find_items(&re, &name, Some(&container)).is_empty());

    // the source file is untouched
    assert_eq!(std::fs::read(&src_path).unwrap(), src_bytes);

    // def/container mismatch is refused
    if let Some(boot) = src.items().into_iter().find(|r| {
        src.item(r.index)
            .and_then(|i| db.get(&i.name))
            .and_then(|d| d.container.as_deref())
            == Some("Boot")
    }) {
        assert!(dsave::copy_item(&src, boot.index, &mut dst, "PrimaryWeapon", Some(&db)).is_err());
    }
}

#[test]
fn slots_listing_summarizes_copies() {
    let dir = temp_dir("slots");
    let Some((path0, _)) = copy_save("dssave0.dsav", &dir) else {
        return;
    };
    let Some((path1, _)) = copy_save("dssave1.dsav", &dir) else {
        return;
    };
    std::fs::write(dir.join("not-a-save.txt"), b"junk").unwrap();
    let slots = dsave::list_slots(&dir).unwrap();
    assert_eq!(slots.len(), 2, "only dssave*.dsav files");
    assert_eq!(slots[0].path, path0);
    assert_eq!(slots[1].path, path1);
    for s in &slots {
        let save = dsave::parse(&std::fs::read(&s.path).unwrap()).unwrap();
        assert_eq!(s.items, save.items().len());
        assert_eq!(s.counter, save.info.counter);
        assert_eq!(s.global_version, save.info.global_version);
        assert_eq!(s.flags, save.info.flags);
        assert_eq!(s.bytes, std::fs::metadata(&s.path).unwrap().len());
        assert!(s.modified.is_some());
        println!(
            "{}: {} B, {} items, {} equipped, {} location record(s), counter {}",
            s.path.file_name().unwrap().to_string_lossy(),
            s.bytes,
            s.items,
            s.equipped,
            s.locations.len(),
            s.counter
        );
    }
    assert!(slots[0].items > slots[1].items, "dssave0 is the big save");
    assert!(slots[0].equipped >= 1);
}
