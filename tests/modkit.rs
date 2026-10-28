use std::path::PathBuf;

fn build_manifest(files: &[(&str, u64)]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&13u32.to_le_bytes());
    b.extend_from_slice(&[0u8; 40]);
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&[0u8; 4]);
    b.extend_from_slice(&[0u8; 8]);
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.push(0);
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(b"test");
    b.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (name, offset) in files {
        b.push(name.len() as u8);
        b.extend_from_slice(name.as_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b.push(0);
        b.push(0);
        b.push(0);
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&offset.to_le_bytes());
    }
    b
}

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

fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join("ds2_modkit_apply_test")
}

#[test]
fn apply_manifest_patches_and_imports() {
    let media = media_dir();
    let source = media.join("scripts.obsp");
    if !source.exists() {
        eprintln!("skipping: {} not found", source.display());
        return;
    }

    let work = scratch_dir();
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    std::fs::copy(&source, work.join("scripts.obsp")).unwrap();

    let payload = {
        let data = std::fs::read(&source).unwrap();
        let o = ds2_modkit::obsp::parse(&data).unwrap();
        let entry = o.find("Brute_NoXP").expect("Brute_NoXP entry");
        o.payload(&data, entry).to_vec()
    };
    let mut resized = payload.clone();
    resized.extend_from_slice(&[0u8; 8]);
    std::fs::write(work.join("brute_noxp.bin"), &resized).unwrap();

    let manifest = r#"
name = "test mod"

[[patch]]
entry = "Brute"
property = "ExperienceValue"
value = "4242"

[[patch]]
entry = "Brute"
property = "DifficultyLevelDamageScales[1]"
value = "2.25"

[[patch]]
entry = "Z2_LD_EncounterList"
property = "Encounters[0].Waves[0].Spawns[0].SpawnCountMin"
value = "7"

[[import]]
entry = "Brute_NoXP"
file = "brute_noxp.bin"

[[add]]
entry = "Brute"
property = "MaxSimultaneousSpawns"
value = "4"
"#;
    std::fs::write(work.join("mod.toml"), manifest).unwrap();

    let report = ds2_modkit::modkit::apply(&work, &work.join("mod.toml")).unwrap();
    assert_eq!(report.patched.len(), 3);
    assert_eq!(report.added.len(), 1);
    assert_eq!(report.imported.len(), 1);
    assert!(work.join("scripts.obsp.bak").exists());
    assert_eq!(
        report.file_size,
        Some(std::fs::metadata(&source).unwrap().len() as usize + 8 + 56)
    );

    let data = std::fs::read(work.join("scripts.obsp")).unwrap();
    let o = ds2_modkit::obsp::parse(&data).unwrap();

    let brute = o.find("Brute").unwrap();
    let payload = ds2_modkit::obsp::payload::parse_payload(o.payload(&data, brute)).unwrap();
    assert_eq!(
        payload.get("ExperienceValue").unwrap().value,
        ds2_modkit::obsp::payload::Value::Int(4242)
    );
    match &payload.get("DifficultyLevelDamageScales").unwrap().value {
        ds2_modkit::obsp::payload::Value::Array(items) => {
            assert_eq!(items[0], ds2_modkit::obsp::payload::Value::Float(0.75));
            assert_eq!(items[1], ds2_modkit::obsp::payload::Value::Float(2.25));
            assert_eq!(items[2], ds2_modkit::obsp::payload::Value::Float(1.5));
            assert_eq!(items[3], ds2_modkit::obsp::payload::Value::Float(2.0));
        }
        other => panic!("expected array, got {other}"),
    }
    assert_eq!(
        payload.get("MaxSimultaneousSpawns").unwrap().value,
        ds2_modkit::obsp::payload::Value::Int(4)
    );

    let enc = o.find("Z2_LD_EncounterList").unwrap();
    let enc_payload = ds2_modkit::obsp::payload::parse_payload(o.payload(&data, enc)).unwrap();
    assert_eq!(
        enc_payload
            .find_path("Encounters[0].Waves[0].Spawns[0].SpawnCountMin")
            .unwrap()
            .value,
        ds2_modkit::obsp::payload::Value::Int(7)
    );

    let noxp = o.find("Brute_NoXP").unwrap();
    assert_eq!(noxp.payload_size as usize, resized.len());

    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn apply_rejects_bad_property_without_writing() {
    let media = media_dir();
    let source = media.join("scripts.obsp");
    if !source.exists() {
        eprintln!("skipping: {} not found", source.display());
        return;
    }

    let work = std::env::temp_dir().join("ds2_modkit_apply_reject");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    std::fs::copy(&source, work.join("scripts.obsp")).unwrap();
    let before = std::fs::read(work.join("scripts.obsp")).unwrap();

    let manifest = r#"
[[patch]]
entry = "Brute"
property = "NoSuchProperty"
value = "1"
"#;
    std::fs::write(work.join("mod.toml"), manifest).unwrap();
    let result = ds2_modkit::modkit::apply(&work, &work.join("mod.toml"));
    assert!(result.is_err());

    let after = std::fs::read(work.join("scripts.obsp")).unwrap();
    assert_eq!(before, after, "failed apply must not modify the file");
    assert!(!work.join("scripts.obsp.bak").exists());

    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn apply_asset_entry() {
    let dir = std::env::temp_dir().join("ds2_modkit_apply_asset");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let manifest_bytes = build_manifest(&[("a", 0), ("b", 10), ("c", 30)]);
    let mut pak = Vec::new();
    pak.extend(std::iter::repeat_n(0xAAu8, 10));
    pak.extend(std::iter::repeat_n(0xBBu8, 20));
    pak.extend(std::iter::repeat_n(0xCCu8, 30));
    std::fs::write(dir.join("manifest.bin"), &manifest_bytes).unwrap();
    std::fs::write(dir.join("test.upak"), &pak).unwrap();
    std::fs::write(dir.join("new_b.bin"), vec![0x77u8; 25]).unwrap();
    std::fs::write(dir.join("new_c.bin"), vec![0x66u8; 20]).unwrap();
    std::fs::write(
        dir.join("mod.toml"),
        "[[asset]]\nasset = \"b\"\nfile = \"new_b.bin\"\n\n[[asset]]\nasset = \"c\"\nfile = \"new_c.bin\"\n",
    )
    .unwrap();

    let report = ds2_modkit::modkit::apply(&dir, &dir.join("mod.toml")).unwrap();
    assert_eq!(report.assets.len(), 3);
    assert!(report.backup.is_none());
    assert!(report.patched.is_empty());

    let mb = std::fs::read(dir.join("manifest.bin")).unwrap();
    let m = ds2_modkit::pack::manifest::parse(&mb).unwrap();
    let b = m.find("b").unwrap();
    let c = m.find("c").unwrap();
    assert_eq!(b.offset, 10);
    assert_eq!(c.offset, 35);
    assert_eq!(
        ds2_modkit::pack::upak::carve(&dir, &m, b).unwrap(),
        vec![0x77u8; 25]
    );
    assert_eq!(
        ds2_modkit::pack::upak::carve(&dir, &m, c).unwrap(),
        vec![0x66u8; 20]
    );
    let _ = std::fs::remove_dir_all(&dir);
}
