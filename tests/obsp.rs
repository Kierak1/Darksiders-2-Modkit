use ds2_modkit::obsp;
use ds2_modkit::obsp::payload::{self as obsp_payload, Value};
use std::collections::HashMap;
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

#[test]
fn scripts_obsp_container_layout() {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let o = obsp::parse(&data).unwrap();

    assert!(obsp::is_obsp(&data));
    assert_eq!(o.marker, 10);
    assert!(o.names_present);
    assert_eq!(o.names.len(), 15520);
    assert_eq!(o.entries.len(), 7862);
    assert_eq!(o.entries_offset as usize, 639206);

    let entries_end = o.entries_offset as usize + o.entries.len() * obsp::ENTRY_SIZE;
    assert!(entries_end <= data.len());

    let e0 = &o.entries[0];
    assert_eq!(o.name_of(e0.refs[0]), Some("plinth_animations"));
    assert_eq!(o.name_of(e0.refs[2]), Some("AnimationList"));
    assert_eq!(e0.payload_offset as usize, entries_end);

    let mut prev_end = None;
    for entry in &o.entries {
        let start = entry.payload_offset as usize;
        let end = start + entry.payload_size as usize;
        assert!(end <= data.len(), "payload out of range");
        if let Some(pe) = prev_end {
            assert_eq!(start, pe, "payloads not contiguous");
        }
        prev_end = Some(end);
    }
    assert_eq!(prev_end.unwrap(), data.len());
}

#[test]
fn scripts_obsp_find_and_payload() {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let o = obsp::parse(&data).unwrap();

    let brute = o.find("Brute").expect("Brute entry");
    assert_eq!(o.name_of(brute.name_hash), Some("base/brute"));
    assert_eq!(o.name_of(brute.refs[0]), Some("Brute"));
    assert_eq!(o.name_of(brute.refs[1]), Some("internal\\characterclasses"));
    let payload = o.payload(&data, brute);
    assert!(
        payload.windows(9).any(|w| w == b"StatTable"),
        "Brute payload should contain StatTable"
    );
    assert!(
        payload.windows(18).any(|w| w == b"ExperienceValueInc"),
        "Brute payload should contain ExperienceValueInc"
    );

    let hit = o.find("HitPointMultiplier");
    assert!(
        hit.is_none(),
        "HitPointMultiplier is a property, not an entry"
    );
}

#[test]
fn brute_payload_properties() {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let o = obsp::parse(&data).unwrap();
    let brute = o.find("Brute").expect("Brute entry");
    let payload = obsp_payload::parse_payload(o.payload(&data, brute)).unwrap();
    assert!(payload.complete, "Brute payload should parse completely");

    let val = |name: &str| payload.get(name).unwrap().value.clone();
    assert_eq!(val("ExperienceValue"), Value::Int(30));
    assert_eq!(val("ExperienceValueInc"), Value::Int(3));
    assert_eq!(val("CombatWeight"), Value::Int(8));
    assert_eq!(val("CombatPressure"), Value::Int(7));
    assert_eq!(val("StatTable"), Value::Str("base/char_brute".into()));
    assert_eq!(
        val("DifficultyLevelDamageScales"),
        Value::Array(vec![
            Value::Float(0.75),
            Value::Float(1.0),
            Value::Float(1.5),
            Value::Float(2.0)
        ])
    );
}

#[test]
fn patch_scalar_and_string_in_memory() {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let o = obsp::parse(&data).unwrap();
    let brute = o.find("Brute").expect("Brute entry");
    let original = o.payload(&data, brute).to_vec();
    let mut payload = original.clone();

    let parsed = obsp_payload::parse_payload(&payload).unwrap();
    let exp = parsed.get("ExperienceValue").unwrap().clone();
    obsp_payload::patch_value(&mut payload, &exp, "999").unwrap();

    let parsed2 = obsp_payload::parse_payload(&payload).unwrap();
    assert_eq!(
        parsed2.get("ExperienceValue").unwrap().value,
        Value::Int(999)
    );
    assert_eq!(
        parsed2.get("ExperienceValueInc").unwrap().value,
        Value::Int(3)
    );

    let item = parsed2.get("ItemGeneratorID").unwrap().clone();
    obsp_payload::patch_value(&mut payload, &item, "Tiny").unwrap();
    let parsed3 = obsp_payload::parse_payload(&payload).unwrap();
    assert_eq!(
        parsed3.get("ItemGeneratorID").unwrap().value,
        Value::Str("Tiny".into())
    );

    let parsed4 = obsp_payload::parse_payload(&original).unwrap();
    assert_eq!(
        parsed4.get("ExperienceValue").unwrap().value,
        Value::Int(30)
    );
    assert_eq!(
        parsed4.get("ItemGeneratorID").unwrap().value,
        Value::Str("Loot_Enemy_Brute".into())
    );
}

#[test]
fn encounter_spawn_tree() {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let o = obsp::parse(&data).unwrap();
    let entry = o.find("Z2_LD_EncounterList").expect("encounter entry");
    let payload = obsp_payload::parse_payload(o.payload(&data, entry)).unwrap();
    assert!(
        payload.complete,
        "encounter payload should parse completely"
    );

    let path_val = |p: &str| payload.find_path(p).unwrap().value.clone();
    assert_eq!(
        path_val("Encounters[0].Waves[0].Spawns[0].SpawnCountMin"),
        Value::Int(20)
    );
    assert_eq!(
        path_val("Encounters[0].Waves[0].Spawns[0].SpawnCountMax"),
        Value::Int(20)
    );
    assert_eq!(
        path_val("Encounters[0].Waves[0].Spawns[0].SpawnDelayMin"),
        Value::Float(0.5)
    );
    assert_eq!(
        path_val("Encounters[0].Waves[0].Spawns[0].EnemyType"),
        Value::Ref("WH_Skeletons".into())
    );

    let z1 = o.find("Z1_OS_EnemyList").expect("enemy list entry");
    let z1_payload = obsp_payload::parse_payload(o.payload(&data, z1)).unwrap();
    assert_eq!(
        z1_payload
            .find_path("EnemySets[0].Enemies[0].EnemyType")
            .unwrap()
            .value,
        Value::Ref("Prowler".into())
    );
    assert_eq!(
        z1_payload
            .find_path("EnemySets[0].Enemies[0].LeashRadius")
            .unwrap()
            .value,
        Value::Float(2000.0)
    );
}

#[test]
fn patch_nested_spawn_count_in_memory() {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let o = obsp::parse(&data).unwrap();
    let entry = o.find("Z2_LD_EncounterList").expect("encounter entry");
    let original = o.payload(&data, entry).to_vec();
    let mut payload = original.clone();

    let parsed = obsp_payload::parse_payload(&payload).unwrap();
    let prop = parsed
        .find_path("Encounters[0].Waves[0].Spawns[0].SpawnCountMin")
        .unwrap()
        .clone();
    obsp_payload::patch_value(&mut payload, &prop, "3").unwrap();

    let parsed2 = obsp_payload::parse_payload(&payload).unwrap();
    assert_eq!(
        parsed2
            .find_path("Encounters[0].Waves[0].Spawns[0].SpawnCountMin")
            .unwrap()
            .value,
        Value::Int(3)
    );
    assert_eq!(
        parsed2
            .find_path("Encounters[0].Waves[0].Spawns[0].SpawnCountMax")
            .unwrap()
            .value,
        Value::Int(20)
    );

    let untouched = obsp_payload::parse_payload(&original).unwrap();
    assert_eq!(
        untouched
            .find_path("Encounters[0].Waves[0].Spawns[0].SpawnCountMin")
            .unwrap()
            .value,
        Value::Int(20)
    );
}

#[test]
fn bod_payload_properties() {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let o = obsp::parse(&data).unwrap();
    let entry = o
        .find("DLC2_ChaosOre_BR_desc")
        .expect("BOD entry DLC2_ChaosOre_BR_desc");
    let payload = obsp_payload::parse_payload(o.payload(&data, entry)).unwrap();
    assert!(payload.complete, "BOD payload should parse completely");
    assert_eq!(payload.class_ref.as_deref(), Some("GrabbableActorDesc"));

    let val = |name: &str| payload.get(name).unwrap().value.clone();
    assert_eq!(val("GrabbableType"), Value::Int(1));
    assert_eq!(val("HitPointMultiplier"), Value::Float(1.5));
    assert_eq!(val("MoveWeight"), Value::Float(5.0));
    assert_eq!(val("CanUsePortal"), Value::Bool(false));
    assert_eq!(
        payload
            .find_path("Visuals.Visuals[0].VisualName")
            .unwrap()
            .value,
        Value::Ref("DLC2_ChaosOre_BR".into())
    );
    assert_eq!(
        payload
            .find_path("SoundsArray.SoundsLists[0].Sounds[0].Name")
            .unwrap()
            .value,
        Value::Ref("Break".into())
    );
}

#[test]
fn patch_bod_float_in_memory() {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let o = obsp::parse(&data).unwrap();
    let entry = o
        .find("DLC2_ChaosOre_BR_desc")
        .expect("BOD entry DLC2_ChaosOre_BR_desc");
    let original = o.payload(&data, entry).to_vec();
    let mut payload = original.clone();

    let parsed = obsp_payload::parse_payload(&payload).unwrap();
    let hp = parsed.get("HitPointMultiplier").unwrap().clone();
    obsp_payload::patch_value(&mut payload, &hp, "2.5").unwrap();

    let parsed2 = obsp_payload::parse_payload(&payload).unwrap();
    assert_eq!(
        parsed2.get("HitPointMultiplier").unwrap().value,
        Value::Float(2.5)
    );
    assert_eq!(parsed2.get("MoveWeight").unwrap().value, Value::Float(5.0));
}

#[test]
fn patch_array_element_in_memory() {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let o = obsp::parse(&data).unwrap();
    let brute = o.find("Brute").expect("Brute entry");
    let original = o.payload(&data, brute).to_vec();
    let mut payload = original.clone();

    let parsed = obsp_payload::parse_payload(&payload).unwrap();
    let target = obsp_payload::resolve_path(&payload, &parsed, "DifficultyLevelDamageScales[0]")
        .expect("array element target");
    assert_eq!(target.value, Value::Float(0.75));
    obsp_payload::patch_target(&mut payload, &target, "2.5").unwrap();

    let parsed2 = obsp_payload::parse_payload(&payload).unwrap();
    match &parsed2.get("DifficultyLevelDamageScales").unwrap().value {
        Value::Array(items) => {
            assert_eq!(items[0], Value::Float(2.5));
            assert_eq!(items[1], Value::Float(1.0));
            assert_eq!(items[2], Value::Float(1.5));
            assert_eq!(items[3], Value::Float(2.0));
        }
        other => panic!("expected array, got {other}"),
    }

    assert!(
        obsp_payload::resolve_path(&payload, &parsed2, "DifficultyLevelDamageScales[9]").is_none()
    );
}

#[test]
fn patch_map_element_in_memory() {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let o = obsp::parse(&data).unwrap();

    let possessed = o
        .find("PossessedBehavior")
        .expect("PossessedBehavior entry");
    let payload = o.payload(&data, possessed).to_vec();
    let parsed = obsp_payload::parse_payload(&payload).unwrap();
    let target = obsp_payload::resolve_path(&payload, &parsed, "XPNeededByLevel[2]")
        .expect("int-keyed map element");
    assert_eq!(target.value, Value::Int(14200));
    assert!(obsp_payload::resolve_path(&payload, &parsed, "XPNeededByLevel[999]").is_none());
    assert!(obsp_payload::resolve_path(&payload, &parsed, "XPNeededByLevel[abc]").is_none());

    let mut patched = payload.clone();
    obsp_payload::patch_target(&mut patched, &target, "99999").unwrap();
    let parsed2 = obsp_payload::parse_payload(&patched).unwrap();
    let reparsed = obsp_payload::resolve_path(&patched, &parsed2, "XPNeededByLevel[2]").unwrap();
    assert_eq!(reparsed.value, Value::Int(99999));
    assert_eq!(
        obsp_payload::resolve_path(&patched, &parsed2, "XPNeededByLevel[1]")
            .unwrap()
            .value,
        Value::Int(10000)
    );

    let categories = o.find("ItemGen_LevelRange_Rarity").expect("rarity entry");
    let payload = o.payload(&data, categories).to_vec();
    let parsed = obsp_payload::parse_payload(&payload).unwrap();
    assert_eq!(
        obsp_payload::resolve_path(&payload, &parsed, "Categories[Enchanted]")
            .unwrap()
            .value,
        Value::Ref("base/levelrange_rarity_enchanted".into())
    );

    let map_manager = o.find("MapManager").expect("MapManager entry");
    let payload = o.payload(&data, map_manager).to_vec();
    let parsed = obsp_payload::parse_payload(&payload).unwrap();
    assert_eq!(
        obsp_payload::resolve_path(&payload, &parsed, "MapParams[Fullmap].ViewWidth")
            .unwrap()
            .value,
        Value::Float(30000.0)
    );

    let names = o
        .find("NameGenerator_WeaponNameKeyMap")
        .expect("name map entry");
    let payload = o.payload(&data, names).to_vec();
    let parsed = obsp_payload::parse_payload(&payload).unwrap();
    assert_eq!(
        obsp_payload::resolve_path(&payload, &parsed, "NameKeyMap[\"Axe\"]")
            .unwrap()
            .value,
        Value::Str("Axe".into())
    );
}

#[test]
fn script_module_def_values() {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let o = obsp::parse(&data).unwrap();
    let entry = o
        .find("setallowfasttravelmodule")
        .expect("script module entry");
    let payload = obsp_payload::parse_payload(o.payload(&data, entry)).unwrap();
    assert!(payload.complete);
    assert_eq!(
        payload.get("Label").unwrap().value,
        Value::Str("Enable / Disable Fast Travel (Manually)".into())
    );
    assert_eq!(payload.get("Category").unwrap().value, Value::Int(14));
    match &payload.get("Actions").unwrap().value {
        Value::Array(items) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], Value::Str("In".into()));
            assert_eq!(items[1], Value::Str("Enable".into()));
            assert_eq!(items[2], Value::Str("Disable".into()));
        }
        other => panic!("Actions should be an array, got {other}"),
    }
}

#[test]
fn rebuild_without_edits_is_byte_identical() {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let o = obsp::parse(&data).unwrap();
    let rebuilt = obsp::rebuild(&data, &o, &HashMap::new()).unwrap();
    assert_eq!(rebuilt, data, "no-op rebuild must be byte-identical");
}

#[test]
fn rebuild_resizes_payload_and_updates_offsets() {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let o = obsp::parse(&data).unwrap();

    let idx = o.find_index("Brute").expect("Brute entry");
    let old = o.payload(&data, &o.entries[idx]).to_vec();
    let mut new_payload = old.clone();
    new_payload.extend_from_slice(&[0u8; 16]);

    let mut replacements = HashMap::new();
    replacements.insert(idx, new_payload.clone());
    let rebuilt = obsp::rebuild(&data, &o, &replacements).unwrap();
    assert_eq!(rebuilt.len(), data.len() + 16);

    let o2 = obsp::parse(&rebuilt).unwrap();
    assert_eq!(o2.entries.len(), o.entries.len());
    let brute2 = &o2.entries[idx];
    assert_eq!(brute2.payload_size as usize, new_payload.len());
    assert_eq!(o2.payload(&rebuilt, brute2), new_payload.as_slice());

    let z1 = o.find_index("Z1_OS_EnemyList").expect("enemy list entry");
    assert_eq!(
        o2.payload(&rebuilt, &o2.entries[z1]),
        o.payload(&data, &o.entries[z1]),
        "unrelated payload must be unchanged"
    );

    let mut order: Vec<usize> = (0..o2.entries.len()).collect();
    order.sort_by_key(|&i| o2.entries[i].payload_offset);
    let mut expect = o2.entries[order[0]].payload_offset as usize;
    for &i in &order {
        assert_eq!(o2.entries[i].payload_offset as usize, expect, "contiguity");
        expect += o2.entries[i].payload_size as usize;
    }
    assert_eq!(expect, rebuilt.len(), "payloads must reach EOF");

    let parsed = obsp_payload::parse_payload(o2.payload(&rebuilt, brute2)).unwrap();
    assert_eq!(parsed.get("ExperienceValue").unwrap().value, Value::Int(30));
}

#[test]
fn name_table_hashes_match_crc64() {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let o = obsp::parse(&data).unwrap();
    let mut checked = 0;
    for (hash, name) in &o.names {
        if *hash == 0 {
            continue;
        }
        assert_eq!(
            ds2_modkit::hash::crc64(name.as_bytes()),
            *hash,
            "crc64 mismatch for {name}"
        );
        checked += 1;
    }
    assert!(checked > 15_000, "expected the whole name table");
}

#[test]
fn add_property_in_memory() {
    let path = media_dir().join("scripts.obsp");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let o = obsp::parse(&data).unwrap();
    let brute = o.find("Brute").expect("Brute entry");
    let original = o.payload(&data, brute).to_vec();
    let parsed = obsp_payload::parse_payload(&original).unwrap();
    assert!(parsed.get("MaxSimultaneousSpawns").is_none());

    let mut payload = original.clone();
    obsp_payload::add_property(&mut payload, &parsed, "MaxSimultaneousSpawns", 0x02, "6").unwrap();

    let parsed2 = obsp_payload::parse_payload(&payload).unwrap();
    assert!(parsed2.complete);
    assert_eq!(parsed2.defs.len(), parsed.defs.len() + 1);
    let added = parsed2
        .get("MaxSimultaneousSpawns")
        .expect("added property");
    assert_eq!(added.value, Value::Int(6));
    assert_eq!(
        added.hash,
        ds2_modkit::hash::crc64(b"MaxSimultaneousSpawns")
    );
    assert_eq!(
        parsed2.get("ExperienceValue").unwrap().value,
        Value::Int(30)
    );
}
