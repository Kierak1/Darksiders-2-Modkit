use ds2_modkit::pack::{manifest, upak};
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

fn setup(name: &str) -> (PathBuf, Vec<u8>) {
    let dir = std::env::temp_dir().join(format!("ds2_upak_repack_test_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let manifest_bytes = build_manifest(&[("a", 0), ("b", 10), ("c", 30)]);
    let mut pak = Vec::new();
    pak.extend(std::iter::repeat_n(0xAAu8, 10));
    pak.extend(std::iter::repeat_n(0xBBu8, 20));
    pak.extend(std::iter::repeat_n(0xCCu8, 30));
    std::fs::write(dir.join("manifest.bin"), &manifest_bytes).unwrap();
    std::fs::write(dir.join("test.upak"), &pak).unwrap();
    (dir, manifest_bytes)
}

#[test]
fn replace_same_size_in_place() {
    let (dir, manifest_bytes) = setup("same_size");
    let m = manifest::parse(&manifest_bytes).unwrap();
    let entry = m.find("b").unwrap().clone();
    let mut mb = manifest_bytes.clone();
    let new_b = vec![0x11u8; 20];
    let report = upak::replace_asset(&dir, &m, &mut mb, &entry, &new_b).unwrap();
    assert!(report.in_place);
    assert_eq!(report.old_size, 20);
    assert_eq!(upak::carve(&dir, &m, &entry).unwrap(), new_b);
    let mb2 = std::fs::read(dir.join("manifest.bin")).unwrap();
    assert_eq!(
        mb2, manifest_bytes,
        "same-size replace must not touch the manifest"
    );
}

#[test]
fn replace_grow_shifts_following_assets_and_manifest() {
    let (dir, manifest_bytes) = setup("grow");
    let m = manifest::parse(&manifest_bytes).unwrap();
    let entry = m.find("b").unwrap().clone();
    let mut mb = manifest_bytes.clone();
    let new_b = vec![0x22u8; 35];
    let report = upak::replace_asset(&dir, &m, &mut mb, &entry, &new_b).unwrap();
    assert!(!report.in_place);
    assert_eq!(report.shifted_entries, 1);

    let mb2 = std::fs::read(dir.join("manifest.bin")).unwrap();
    let m2 = manifest::parse(&mb2).unwrap();
    let a = m2.find("a").unwrap();
    let b = m2.find("b").unwrap();
    let c = m2.find("c").unwrap();
    assert_eq!((a.offset, b.offset, c.offset), (0, 10, 45));
    assert_eq!(upak::entry_size(&m2, 75, b), 35);
    assert_eq!(upak::entry_size(&m2, 75, c), 30);

    assert_eq!(upak::carve(&dir, &m2, b).unwrap(), new_b);
    assert_eq!(upak::carve(&dir, &m2, a).unwrap(), vec![0xAAu8; 10]);
    assert_eq!(upak::carve(&dir, &m2, c).unwrap(), vec![0xCCu8; 30]);
    assert!(dir.join("test.upak.bak").exists());
    assert!(dir.join("manifest.bin.bak").exists());
}

#[test]
fn replace_last_asset_grows_without_shifts() {
    let (dir, manifest_bytes) = setup("last");
    let m = manifest::parse(&manifest_bytes).unwrap();
    let entry = m.find("c").unwrap().clone();
    let mut mb = manifest_bytes.clone();
    let new_c = vec![0x33u8; 40];
    let report = upak::replace_asset(&dir, &m, &mut mb, &entry, &new_c).unwrap();
    assert_eq!(report.shifted_entries, 0);
    let m2 = manifest::parse(&std::fs::read(dir.join("manifest.bin")).unwrap()).unwrap();
    let c = m2.find("c").unwrap();
    assert_eq!(c.offset, 30);
    assert_eq!(upak::carve(&dir, &m2, c).unwrap(), new_c);
}

#[test]
fn replace_multiple_in_one_pass() {
    let (dir, manifest_bytes) = setup("batch");
    let m = manifest::parse(&manifest_bytes).unwrap();
    let mut mb = manifest_bytes.clone();
    let new_a = vec![0x11u8; 15];
    let new_c = vec![0x33u8; 20];
    let idx_a = m.find_index("a").unwrap();
    let idx_c = m.find_index("c").unwrap();
    let results = upak::replace_assets(
        &dir,
        &m,
        &mut mb,
        &[(idx_a, new_a.clone()), (idx_c, new_c.clone())],
    )
    .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].replaced.len(), 2);
    assert!(!results[0].in_place);

    let m2 = manifest::parse(&std::fs::read(dir.join("manifest.bin")).unwrap()).unwrap();
    let a = m2.find("a").unwrap();
    let b = m2.find("b").unwrap();
    let c = m2.find("c").unwrap();
    assert_eq!((a.offset, b.offset, c.offset), (0, 15, 35));
    assert_eq!(std::fs::metadata(dir.join("test.upak")).unwrap().len(), 55);
    assert_eq!(upak::carve(&dir, &m2, a).unwrap(), new_a);
    assert_eq!(upak::carve(&dir, &m2, b).unwrap(), vec![0xBBu8; 20]);
    assert_eq!(upak::carve(&dir, &m2, c).unwrap(), new_c);
    assert!(dir.join("test.upak.bak").exists());
    assert!(dir.join("manifest.bin.bak").exists());
}

#[test]
fn replace_reports_progress() {
    let (dir, manifest_bytes) = setup("progress");
    let m = manifest::parse(&manifest_bytes).unwrap();
    let mut mb = manifest_bytes.clone();
    let idx = m.find_index("b").unwrap();
    let mut events: Vec<(String, u64, u64)> = Vec::new();
    let results = upak::replace_assets_with_progress(
        &dir,
        &m,
        &mut mb,
        &[(idx, vec![0x22u8; 35])],
        &mut |label, done, total| events.push((label.to_string(), done, total)),
    )
    .unwrap();
    assert!(!results[0].in_place);
    assert!(!events.is_empty(), "no progress reported");
    assert!(events.iter().all(|(label, _, _)| label == "test"));
    assert!(events.iter().all(|(_, _, total)| *total == 60));
    assert!(
        events.windows(2).all(|w| w[0].1 <= w[1].1),
        "progress went backwards: {events:?}"
    );
    assert_eq!(
        events.last().unwrap().1,
        60,
        "last event should reach total"
    );
}
