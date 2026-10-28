//! Real-file save-editing tests (add / stat / possessed / move / equip / diff).
//!
//! Every test copies the user's save to a fresh `%TEMP%` directory first; the
//! originals are never touched. Tests skip when the save or game files are
//! missing.

use ds2_modkit::dsave::{self, ItemDb, ItemSource};
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

/// Copy `dssave0.dsav` to a fresh temp folder and return the copy's path.
fn temp_copy(tag: &str) -> Option<(PathBuf, Vec<u8>)> {
    let src = save_dir().join("dssave0.dsav");
    if !src.exists() {
        eprintln!("skipping: {} not found", src.display());
        return None;
    }
    let data = std::fs::read(&src).ok()?;
    let dir = std::env::temp_dir().join(format!("ds2_save_edit_{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("dssave0.dsav");
    std::fs::write(&path, &data).ok()?;
    Some((path, data))
}

fn index_of(save: &dsave::Save, list: &str, item_idx: usize) -> usize {
    save.items()
        .into_iter()
        .find(|r| {
            r.source == ItemSource::Inventory
                && save.module.inventory[r.list_idx].name == list
                && r.item_idx == item_idx
        })
        .unwrap()
        .index
}

fn item_in(save: &dsave::Save, list: &str, name: &str) -> Option<usize> {
    save.items()
        .into_iter()
        .find(|r| {
            r.source == ItemSource::Inventory
                && save.module.inventory[r.list_idx].name == list
                && save.item(r.index).is_some_and(|i| i.name == name)
        })
        .map(|r| r.index)
}

fn flags(save: &dsave::Save, list: &str) -> Vec<(String, u8)> {
    save.module
        .inventory
        .iter()
        .find(|l| l.name == list)
        .map(|l| {
            l.items
                .iter()
                .filter_map(|i| i.save_body().map(|s| (i.name.clone(), s.b816)))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn add_stat_diff_remove_cycle() {
    let Some((db, obsp, data)) = load_db() else {
        return;
    };
    let Some((path, original)) = temp_copy("cycle") else {
        return;
    };
    let mut save = dsave::parse(&original).unwrap();
    let before = save.items().len();
    let idx = dsave::add_item(
        &mut save,
        Some(&db),
        "PrimaryWeapon",
        "weapon_scythe_bheithir_talons",
        Some(4),
        None,
    )
    .unwrap();
    assert_eq!(save.items().len(), before + 1);
    assert_eq!(
        save.item(idx).unwrap().name,
        "weapons/weapon_scythe_bheithir_talons"
    );
    println!("added bheithir as item {idx}");

    let catalog = dsave::AttributeCatalog::build(&obsp, &data, &[&save]);
    let change = dsave::set_stat(save.item_mut(idx).unwrap(), &catalog, "Damage", 123).unwrap();
    assert!(change.contains("123"), "{change}");
    println!("stat edit: {change}");

    let report = dsave::write_file(&save, &path).unwrap();
    assert!(report.backup.is_some());
    assert!(report.timestamped_backup.as_ref().unwrap().exists());
    assert!(
        report.diff.iter().any(|l| l.contains("PrimaryWeapon")),
        "diff should mention the container: {:?}",
        report.diff
    );
    println!("write diff: {:?}", report.diff);

    let reparsed = dsave::parse(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(reparsed.items().len(), before + 1);
    // dssave0 already has a Bheithir's Talons; the added clone is the one with
    // the patched Damage (deduplicated by scanning the two candidates).
    let patched = reparsed
        .items()
        .into_iter()
        .filter(|r| {
            reparsed.item(r.index).is_some_and(|i| {
                i.name == "weapons/weapon_scythe_bheithir_talons"
                    && r.source == ItemSource::Inventory
            })
        })
        .filter(|r| {
            let views = dsave::stat_views(reparsed.item(r.index).unwrap(), &catalog);
            views
                .iter()
                .any(|v| v.attribute == "Damage" && v.value == 123)
        })
        .count();
    assert_eq!(
        patched, 1,
        "the added clone with Damage=123 survives the round trip"
    );

    // diff against the one-time backup (the original content)
    let bak = format!("{}.bak", path.display());
    let old = dsave::parse(&std::fs::read(&bak).unwrap()).unwrap();
    let diff = reparsed.diff(&old);
    assert!(
        diff.iter().any(|l| l.contains("bheithir")) || diff.iter().any(|l| l.contains("Bheithir")),
        "{diff:?}"
    );
    assert_eq!(reparsed.diff(&reparsed), Vec::<String>::new());

    // remove again: the file returns to the original bytes
    let mut save2 = reparsed;
    save2.remove_item(idx);
    dsave::write_file(&save2, &path).unwrap();
    let final_bytes = std::fs::read(&path).unwrap();
    assert_eq!(final_bytes.len(), original.len());
    assert!(
        final_bytes == original,
        "add+remove must restore the original file byte for byte"
    );
}

#[test]
fn move_between_lists_validates_containers() {
    let Some((db, _obsp, _data)) = load_db() else {
        return;
    };
    let Some((path, _original)) = temp_copy("move") else {
        return;
    };
    let mut save = dsave::parse(&std::fs::read(&path).unwrap()).unwrap();

    // a known Boot def refuses a move into another equipment list (the
    // `*_base` items have no def, so they are not usable for this check)
    let Some(boot_idx) = save
        .items()
        .into_iter()
        .find(|r| {
            r.source == ItemSource::Inventory
                && save.module.inventory[r.list_idx].name == "Boot"
                && save.item(r.index).is_some_and(|i| {
                    db.get(&i.name)
                        .is_some_and(|d| d.container.as_deref() == Some("Boot"))
                })
        })
        .map(|r| r.index)
    else {
        eprintln!("skipping: no def-backed Boot item");
        return;
    };
    let err = dsave::move_item(&mut save, boot_idx, "PrimaryWeapon", Some(&db)).unwrap_err();
    assert!(err.to_string().contains("belongs in"), "{err}");

    // an item with no def container moves freely between lists
    let Some((global, name, src)) = save.items().into_iter().find_map(|r| {
        if r.source != ItemSource::Inventory {
            return None;
        }
        let item = save.item(r.index)?;
        item.save_body()?;
        db.get(&item.name)
            .is_none_or(|d| d.container.is_none())
            .then(|| {
                (
                    r.index,
                    item.name.clone(),
                    save.module.inventory[r.list_idx].name.clone(),
                )
            })
    }) else {
        eprintln!("skipping: no container-less item");
        return;
    };
    let target = if src == "MiscItem" {
        "Buyback"
    } else {
        "MiscItem"
    };
    let moved = dsave::move_item(&mut save, global, target, Some(&db)).unwrap();
    assert_eq!(save.item(moved).unwrap().name, name);
    let back = dsave::move_item(&mut save, moved, &src, Some(&db)).unwrap();
    assert_eq!(save.item(back).unwrap().name, name);
    dsave::write_file(&save, &path).unwrap();
    let re = dsave::parse(&std::fs::read(&path).unwrap()).unwrap();
    assert!(item_in(&re, &src, &name).is_some());
    assert!(item_in(&re, target, &name).is_none());
}

#[test]
fn equip_and_unequip_flip_the_b816_flag() {
    let Some((db, _obsp, _data)) = load_db() else {
        return;
    };
    let Some((path, _original)) = temp_copy("equip") else {
        return;
    };
    let mut save = dsave::parse(&std::fs::read(&path).unwrap()).unwrap();

    let li = save
        .module
        .inventory
        .iter()
        .position(|l| l.name == "Boot")
        .unwrap();
    let equipped = save.module.inventory[li]
        .items
        .iter()
        .position(|i| i.save_body().is_some_and(|s| s.b816 == 1));
    let candidate = save.module.inventory[li]
        .items
        .iter()
        .position(|i| i.save_body().is_some_and(|s| s.b816 == 0));
    let (Some(old_equipped), Some(candidate)) = (equipped, candidate) else {
        eprintln!("skipping: save has no two Boot items to swap");
        return;
    };

    let candidate_global = index_of(&save, "Boot", candidate);
    let equipped_global = index_of(&save, "Boot", old_equipped);
    dsave::equip_item(&mut save, candidate_global, Some(&db)).unwrap();
    let f = flags(&save, "Boot");
    assert_eq!(f.iter().filter(|(_, b)| *b == 1).count(), 1);
    assert_eq!(
        save.item(candidate_global)
            .unwrap()
            .save_body()
            .unwrap()
            .b816,
        1
    );
    assert_eq!(
        save.item(equipped_global)
            .unwrap()
            .save_body()
            .unwrap()
            .b816,
        0
    );

    dsave::write_file(&save, &path).unwrap();
    let mut re = dsave::parse(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        re.item(candidate_global).unwrap().save_body().unwrap().b816,
        1
    );

    // unequip, then the list flag count drops to zero
    dsave::unequip_item(&mut re, candidate_global).unwrap();
    assert_eq!(
        flags(&re, "Boot").iter().filter(|(_, b)| *b == 1).count(),
        0
    );
    dsave::write_file(&re, &path).unwrap();

    // a non-equipment list refuses equipping (item_dust in `None` is a base
    // item with no save body, so it errors either way)
    let none_idx = index_of(&re, "None", 0);
    assert!(dsave::equip_item(&mut re, none_idx, Some(&db)).is_err());
}

#[test]
fn possessed_max_round_trips_through_file() {
    let Some((path, _original)) = temp_copy("possess") else {
        return;
    };
    let mut save = dsave::parse(&std::fs::read(&path).unwrap()).unwrap();
    let Some(idx) = save
        .items()
        .into_iter()
        .find(|r| {
            save.item(r.index)
                .is_some_and(|i| dsave::possessed_info(i).is_some())
        })
        .map(|r| r.index)
    else {
        eprintln!("skipping: no possessed item in dssave0");
        return;
    };
    let before = dsave::possessed_info(save.item(idx).unwrap()).unwrap();
    println!("possessed item {idx}: {before:?}");
    let change = dsave::possessed_max(save.item_mut(idx).unwrap()).unwrap();
    println!("{change}");
    let after = dsave::possessed_info(save.item(idx).unwrap()).unwrap();
    assert_eq!(after.level, after.level_cap);
    assert_eq!(after.cur_xp, 0);

    // add/remove a stat entry, then round trip
    let added = dsave::possessed_add_mod(save.item_mut(idx).unwrap(), None, 0, "92", Some(7));
    if let Ok(change) = added {
        println!("{change}");
        let (stat, md) = dsave::find_mod(save.item(idx).unwrap(), "92").unwrap();
        dsave::possessed_remove_mod(save.item_mut(idx).unwrap(), stat, md).unwrap();
    }

    dsave::write_file(&save, &path).unwrap();
    let re = dsave::parse(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        dsave::possessed_info(re.item(idx).unwrap()).unwrap().level,
        after.level
    );
}

#[test]
fn unknown_defs_and_bad_arity_are_rejected() {
    let Some((db, _obsp, _data)) = load_db() else {
        return;
    };
    let Some((path, _original)) = temp_copy("reject") else {
        return;
    };
    let mut save = dsave::parse(&std::fs::read(&path).unwrap()).unwrap();
    let items = save.items().len();
    assert!(dsave::add_item(&mut save, Some(&db), "Boot", "no_such_def", None, None).is_err());
    assert!(dsave::add_item(
        &mut save,
        Some(&db),
        "NoSuchList",
        "armor_feet_wanderer_d01",
        None,
        None
    )
    .is_err());
    assert_eq!(
        save.items().len(),
        items,
        "refused adds must not change the save"
    );
    // no save-item template at all -> refused
    let mut empty = dsave::parse(&std::fs::read(&path).unwrap()).unwrap();
    empty.module.inventory.clear();
    empty.module.equipped.clear();
    empty.module.records.clear();
    assert!(dsave::add_item(
        &mut empty,
        Some(&db),
        "Boot",
        "armor_feet_wanderer_d01",
        None,
        None
    )
    .is_err());
}

#[test]
fn write_file_makes_timestamped_backups() {
    let Some((path, original)) = temp_copy("backup") else {
        return;
    };
    let save = dsave::parse(&original).unwrap();
    let report = dsave::write_file(&save, &path).unwrap();
    let stamped = report.timestamped_backup.as_ref().unwrap();
    assert!(stamped.exists(), "{}", stamped.display());
    let name = stamped.file_name().unwrap().to_string_lossy().to_string();
    assert!(Path::new(&name).extension().is_some_and(|e| e == "bak"));
    assert_eq!(std::fs::read(stamped).unwrap(), original);
    assert_eq!(
        report.diff,
        Vec::<String>::new(),
        "an unchanged write has no diff"
    );
    // a second write makes a second stamp without clobbering the first
    let report2 = dsave::write_file(&save, &path).unwrap();
    assert_ne!(
        report.timestamped_backup, report2.timestamped_backup,
        "two writes must produce two distinct timestamped backups"
    );
}
