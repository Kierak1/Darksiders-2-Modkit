use ds2_modkit::pack::{dds, manifest, obpk, upak};
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
fn death_asset_roundtrip() {
    let media = media_dir();
    let manifest_path = media.join("manifest.bin");
    if !manifest_path.exists() {
        eprintln!("skipping: {} not found", manifest_path.display());
        return;
    }

    let data = std::fs::read(&manifest_path).unwrap();
    let m = manifest::parse(&data).unwrap();
    assert_eq!(m.version, 13, "expected manifest v13");

    let entry = m
        .find("characters/death")
        .expect("characters/death not found in manifest");
    assert_ne!(entry.pkg_idx, 0xFF);

    let carved = upak::carve(&media, &m, entry).unwrap();
    assert_eq!(&carved[0..4], b"OBPK", "carved asset is not an OBPK");

    let (header, payload) = obpk::decompress(&carved).unwrap();
    assert!(payload.len() > 1_000_000, "payload suspiciously small");

    let textures = dds::find_textures(&payload);
    assert!(!textures.is_empty(), "no DDS textures in payload");

    let recompressed = obpk::compress(&header, &payload).unwrap();
    let (_, payload2) = obpk::decompress(&recompressed).unwrap();
    assert_eq!(payload, payload2, "OBPK round-trip mismatch");

    let out_dir = std::env::temp_dir().join("ds2_modkit_test_textures");
    let _ = std::fs::remove_dir_all(&out_dir);
    let report = dds::extract(&payload, &out_dir).unwrap();
    assert_eq!(report.count, textures.len());
    for file in std::fs::read_dir(&out_dir).unwrap() {
        let bytes = std::fs::read(file.unwrap().path()).unwrap();
        assert_eq!(&bytes[0..4], b"DDS ");
        assert_eq!(dds::dds_size_at(&bytes, 0) as usize, bytes.len());
    }
    let _ = std::fs::remove_dir_all(&out_dir);
}

#[test]
fn texture_injection_keeps_size_and_content() {
    let media = media_dir();
    let manifest_path = media.join("manifest.bin");
    if !manifest_path.exists() {
        eprintln!("skipping: {} not found", manifest_path.display());
        return;
    }
    let data = std::fs::read(&manifest_path).unwrap();
    let m = manifest::parse(&data).unwrap();
    let entry = m
        .find("characters/death")
        .expect("characters/death not found in manifest");
    let carved = upak::carve(&media, &m, entry).unwrap();
    let (_, payload) = obpk::decompress(&carved).unwrap();

    let out_dir = std::env::temp_dir().join("ds2_modkit_test_inject");
    let _ = std::fs::remove_dir_all(&out_dir);
    dds::extract(&payload, &out_dir).unwrap();

    let first = std::fs::read_dir(&out_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "dds"))
        .expect("no extracted dds");
    let mut bytes = std::fs::read(&first).unwrap();
    bytes[128] ^= 0xFF;

    let inject_dir = out_dir.join("modified");
    std::fs::create_dir_all(&inject_dir).unwrap();
    let name = first.file_name().unwrap().to_string_lossy().into_owned();
    std::fs::write(inject_dir.join(&name), &bytes).unwrap();
    std::fs::remove_file(&first).unwrap();

    let modified = dds::inject(&payload, &inject_dir).unwrap();
    assert_eq!(
        modified.len(),
        payload.len(),
        "injection changed payload size"
    );
    assert_ne!(modified, payload, "modified texture had no effect");

    let recompressed = obpk::compress(&obpk::header(&carved).unwrap(), &modified).unwrap();
    let (_, decoded) = obpk::decompress(&recompressed).unwrap();
    assert_eq!(decoded, modified);

    let _ = std::fs::remove_dir_all(&out_dir);
}
