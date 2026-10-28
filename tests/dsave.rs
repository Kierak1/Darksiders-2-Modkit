use ds2_modkit::dsave::{self, ItemBody};
use std::path::PathBuf;

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

fn real_saves() -> Vec<(String, Vec<u8>)> {
    let dir = save_dir();
    let mut out = Vec::new();
    for name in ["dssave0.dsav", "dssave1.dsav", "dssave2.dsav"] {
        let p = dir.join(name);
        if p.exists() {
            out.push((name.to_string(), std::fs::read(&p).unwrap()));
        }
    }
    out
}

#[test]
fn real_save_roundtrip_is_byte_identical() {
    let saves = real_saves();
    if saves.is_empty() {
        eprintln!("skipping: no save files found in {}", save_dir().display());
        return;
    }
    for (name, data) in &saves {
        let save = dsave::parse(data).unwrap();
        let out = save.write();
        assert_eq!(out.len(), data.len(), "{name}: length mismatch");
        assert!(out == *data, "{name}: round-trip mismatch");
        println!(
            "{name}: {} lists, {} top-level items, {} objects",
            save.module.inventory.len(),
            save.items().len(),
            save.module.objects.len()
        );
    }
}

#[test]
fn stat_patch_roundtrips_through_file() {
    let saves = real_saves();
    let Some((name, data)) = saves.first() else {
        eprintln!("skipping: no save files");
        return;
    };
    let mut save = dsave::parse(data).unwrap();

    // find the first save-item item with a modifier stat and patch a value
    let mut patched = false;
    for r in save.items() {
        let Some(item) = save.item(r.index) else {
            continue;
        };
        let ItemBody::Save(body) = &item.body else {
            continue;
        };
        for nested in &body.nested {
            if let ItemBody::Modifier(m) = &nested.body {
                if let Some(_md) = m.mods.first() {
                    if !patched {
                        println!("{name}: patching {}", item.name);
                        patched = true;
                    }
                }
            }
        }
    }
    assert!(patched, "no modifier stats found in {name}");

    // mutate the first modifier's first value via model edit
    'outer: for r in save.items() {
        let Some(item) = save.item_mut(r.index) else {
            continue;
        };
        if let ItemBody::Save(body) = &mut item.body {
            for nested in &mut body.nested {
                if let ItemBody::Modifier(m) = &mut nested.body {
                    if let Some(md) = m.mods.first_mut() {
                        md.a[1] = md.a[1].wrapping_add(1);
                        break 'outer;
                    }
                }
            }
        }
    }
    let out = save.write();
    assert!(out != *data, "patched save should differ");
    let re = dsave::parse(&out).unwrap();
    let found = re.items().iter().any(|r| {
        let Some(item) = re.item(r.index) else {
            return false;
        };
        if let ItemBody::Save(body) = &item.body {
            for nested in &body.nested {
                if let ItemBody::Modifier(m) = &nested.body {
                    if let Some(_md) = m.mods.first() {
                        return true;
                    }
                }
            }
        }
        false
    });
    assert!(found, "patched save lost its modifiers");
}

#[test]
fn write_file_creates_backup_and_verifies() {
    let saves = real_saves();
    let Some((_name, data)) = saves.first() else {
        eprintln!("skipping: no save files");
        return;
    };
    let dir = std::env::temp_dir().join("ds2_modkit_save_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("dssave0.dsav");
    std::fs::write(&target, data).unwrap();

    let mut save = dsave::parse(data).unwrap();
    let report = dsave::write_file(&save, &target).unwrap();
    assert_eq!(report.bytes, data.len());
    assert!(
        report.backup.is_some(),
        "expected a .bak next to the target"
    );
    assert_eq!(
        std::fs::read(target.with_file_name("dssave0.dsav.bak")).unwrap(),
        *data
    );

    // second write must not clobber the original backup
    if let Some(first) = save.item_mut(0) {
        first.name = "armor/armor_feet_base".to_string();
    }
    dsave::write_file(&save, &target).unwrap();
    assert_eq!(
        std::fs::read(target.with_file_name("dssave0.dsav.bak")).unwrap(),
        *data
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_cli_is_available() {
    // smoke check that the CLI binary answers save-list on a copy
    let saves = real_saves();
    let Some((_name, data)) = saves.first() else {
        return;
    };
    let dir = std::env::temp_dir().join("ds2_modkit_save_cli");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("dssave0.dsav");
    std::fs::write(&target, data).unwrap();
    let exe = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("release")
        .join("ds2-modkit.exe");
    if !exe.exists() {
        return;
    }
    let out = std::process::Command::new(&exe)
        .arg("save-list")
        .arg(&target)
        .output()
        .unwrap();
    assert!(out.status.success(), "save-list failed: {out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("items shown"), "unexpected output: {text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn structural_add_remove_roundtrip() {
    let saves = real_saves();
    let Some((name, data)) = saves.first() else {
        eprintln!("skipping: no save files");
        return;
    };
    let mut save = dsave::parse(data).unwrap();
    let list_idx = save
        .module
        .inventory
        .iter()
        .position(|l| !l.items.is_empty())
        .expect("no non-empty inventory list");
    let before = save.module.inventory[list_idx].items.len();
    let template = save.module.inventory[list_idx].items[0].clone();

    save.module.inventory[list_idx].items.push(template);
    assert_eq!(save.module.inventory[list_idx].items.len(), before + 1);
    let out = save.write();
    let mut re = dsave::parse(&out).unwrap();
    assert_eq!(
        re.module.inventory[list_idx].items.len(),
        before + 1,
        "{name}: added item not in re-parsed save"
    );

    re.module.inventory[list_idx].items.pop();
    let out2 = re.write();
    assert_eq!(out2, *data, "{name}: add+remove must restore the original");
}
