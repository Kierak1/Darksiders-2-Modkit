//! Item DB / attribute catalog / localization integration tests against the
//! real game files. Each test skips when its inputs are missing.

use ds2_modkit::dsave::{self, db, LocTable};
use std::path::PathBuf;

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

fn load_db() -> Option<(db::ItemDb, ds2_modkit::obsp::Obsp, Vec<u8>)> {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return None;
    }
    let data = std::fs::read(&path).unwrap();
    let obsp = ds2_modkit::obsp::parse(&data).unwrap();
    let db = db::ItemDb::build(&obsp, &data);
    Some((db, obsp, data))
}

fn real_saves() -> Vec<dsave::Save> {
    let dir = save_dir();
    let mut out = Vec::new();
    for name in ["dssave0.dsav", "dssave1.dsav", "dssave2.dsav"] {
        let p = dir.join(name);
        if p.exists() {
            out.push(dsave::parse(&std::fs::read(&p).unwrap()).unwrap());
        }
    }
    out
}

#[test]
fn item_db_builds_real_defs() {
    let Some((db, _obsp, _data)) = load_db() else {
        return;
    };
    let cov = db.coverage();
    assert!(cov.defs > 1000, "expected >1000 defs, got {}", cov.defs);
    assert!(
        cov.with_container > 400,
        "containers: {}",
        cov.with_container
    );
    assert!(cov.with_rarity > 50, "rarities: {}", cov.with_rarity);
    println!(
        "defs {} / container {} / rarity {} / titles {} / namegen {} / modifiers {}",
        cov.defs,
        cov.with_container,
        cov.with_rarity,
        cov.with_pickup_title,
        cov.with_name_generator,
        cov.with_modifiers
    );

    let wanderer = db.get("armor_feet_wanderer_d01").expect("wanderer def");
    assert_eq!(wanderer.container.as_deref(), Some("Boot"));
    assert_eq!(wanderer.desc_class, "EquippableItemDesc");
    assert_eq!(wanderer.rarity_levels, 6);
    assert_eq!(
        wanderer.name_generator.as_deref(),
        Some("base/NameGenerator_ArmorFeet_Combination")
    );
    assert_eq!(wanderer.pickup_icon.as_deref(), Some("Wanderer_D01Boots"));
    assert!(db.get("armor/armor_feet_wanderer_d02").is_some());

    let bheithir = db
        .get("weapon_scythe_bheithir_talons")
        .expect("bheithir def");
    assert_eq!(bheithir.container.as_deref(), Some("PrimaryWeapon"));
    assert_eq!(bheithir.desc_class, "WeaponDesc");
    assert_eq!(bheithir.rarity, Some(4));
    assert_eq!(bheithir.level, Some(15));
    assert_eq!(bheithir.rarity_titles.len(), 6);
    assert!(bheithir.rarity_titles[4].contains("BheithirsTalons"));
    assert_eq!(bheithir.modifiers.len(), 5);

    let damage = bheithir
        .modifiers
        .iter()
        .flat_map(|m| m.attrs.iter())
        .find(|a| a.attribute == "Damage")
        .expect("damage attr");
    assert_eq!(
        damage.value_table.as_deref(),
        Some("base/Mod_Weapon_DoubleScythe")
    );
    assert_eq!(damage.value_lookup.as_deref(), Some("Damage"));
    assert_eq!(damage.value, None);
    let crit = bheithir
        .modifiers
        .iter()
        .flat_map(|m| m.attrs.iter())
        .find(|a| a.attribute == "CriticalDamageMod")
        .expect("crit attr");
    assert_eq!(crit.value, Some(db::DefValue::Int(150)));
}

#[test]
fn loc_table_resolves_real_item_text() {
    let media = media_dir();
    if !media.join("media.upak").exists() {
        eprintln!("skipping: media.upak not found");
        return;
    }
    let loc = LocTable::load_from_media(&media).unwrap();
    assert!(
        loc.len() > 3000,
        "expected a large table, got {}",
        loc.len()
    );
    println!("ui_core loc entries: {}", loc.len());

    assert_eq!(
        loc.resolve("$Item.Weapons.Legendary.Scythe.BheithirsTalons.PickupTitle"),
        Some("Bheithir's Talons")
    );
    let details = loc
        .resolve("$Item.Weapons.Legendary.Scythe.BheithirsTalons.Description")
        .expect("details");
    assert!(details.starts_with("Found within the very belly of the beast"));
    assert_eq!(
        loc.resolve("$Item.Armor.Boots.PickupDetails"),
        Some("This armor protects the wearer's feet and legs from attack.")
    );
    assert_eq!(
        loc.resolve("$Item.Attributes.FireDamage"),
        Some("Fire Damage")
    );
    assert_eq!(loc.resolve("$Item.Attributes.Damage"), Some("Damage"));
}

#[test]
fn attribute_catalog_types_real_saves() {
    let saves = real_saves();
    if saves.is_empty() {
        eprintln!("skipping: no save files found in {}", save_dir().display());
        return;
    }
    let Some((_db, obsp, data)) = load_db() else {
        return;
    };
    let refs: Vec<&dsave::Save> = saves.iter().collect();
    let catalog = db::AttributeCatalog::build(&obsp, &data, &refs);
    assert!(
        catalog.len() >= 42,
        "expected >=42 attrs, got {}",
        catalog.len()
    );
    println!("catalog attributes: {}", catalog.len());

    let damage = catalog.get("Damage").expect("Damage attr");
    assert_eq!(damage.kind, db::AttrKind::Int);
    assert_eq!(damage.enum_id, Some(7));
    assert!(damage.table_backed);

    let defense = catalog.get("Defense").expect("Defense attr");
    assert_eq!(defense.kind, db::AttrKind::Int);
    assert_eq!(defense.enum_id, Some(63));

    let crit = catalog.get("CriticalDamageMod").expect("crit attr");
    assert_eq!(crit.kind, db::AttrKind::Int);
    assert_eq!(
        crit.confidence,
        db::Confidence::High,
        "static Value=150 in defs"
    );
    assert_eq!(crit.enum_id, Some(71));

    let ints = catalog
        .attrs()
        .iter()
        .filter(|a| a.kind == db::AttrKind::Int)
        .count();
    let floats = catalog
        .attrs()
        .iter()
        .filter(|a| a.kind == db::AttrKind::Float)
        .count();
    let unknown = catalog
        .attrs()
        .iter()
        .filter(|a| a.kind == db::AttrKind::Unknown)
        .count();
    println!("typed: {ints} int, {floats} float, {unknown} unknown");
    assert_eq!(
        floats, 0,
        "no float-encoded value was observed in these files"
    );
}

#[test]
fn display_names_resolve_for_examples() {
    let Some((db, _obsp, _data)) = load_db() else {
        return;
    };
    let media = media_dir();
    if !media.join("media.upak").exists() {
        eprintln!("skipping: media.upak not found");
        return;
    }
    let loc = LocTable::load_from_media(&media).unwrap();

    assert_eq!(
        db::display_name("weapon_scythe_bheithir_talons", None, &loc, &db),
        "Bheithir's Talons"
    );
    assert_eq!(
        db::localized_name("weapon_scythe_bheithir_talons", Some(4), &loc, &db).as_deref(),
        Some("Bheithir's Talons")
    );
    // armor defs only carry a name generator (not resolvable offline)
    assert_eq!(
        db::display_name("armor_feet_wanderer_d01", None, &loc, &db),
        "Armor Feet Wanderer D01"
    );
    assert_eq!(
        db::localized_name("armor_feet_wanderer_d01", None, &loc, &db),
        None
    );

    // coverage over the real save items
    let saves = real_saves();
    if saves.is_empty() {
        return;
    }
    let mut total = 0usize;
    let mut known = 0usize;
    let mut named = 0usize;
    for save in &saves {
        for r in save.items() {
            let Some(item) = save.item(r.index) else {
                continue;
            };
            total += 1;
            if let Some(def) = db.get(&item.name) {
                known += 1;
                if db::localized_name(&def.name, None, &loc, &db).is_some() {
                    named += 1;
                }
            }
        }
    }
    println!(
        "save items: {total}, in DB: {known} ({:.1}%), localized: {named} ({:.1}%)",
        100.0 * known as f64 / total.max(1) as f64,
        100.0 * named as f64 / total.max(1) as f64
    );
    assert!(known > total / 2, "most save items should be in the def DB");
}
