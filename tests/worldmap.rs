use ds2_modkit::map::worldmap;
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

/// Spawn a clone of a real z2_world spawnpoint, verify every blob in the file
/// still parses, then delete the clone and require a byte-identical file.
#[test]
fn spawn_delete_roundtrip_real_worlds() {
    let path = media_dir().join("worlds.bin");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let original = std::fs::read(&path).unwrap();
    let worlds = worldmap::parse_worlds(&original).unwrap();
    let w = worlds
        .iter()
        .find(|w| w.name == "z2_world")
        .expect("z2_world");
    let objects = worldmap::collect_objects(&original, w).unwrap();
    let template = objects
        .iter()
        .find(|o| o.class == "spawnpoint")
        .expect("no spawnpoint in z2_world");
    let pos = template.position;
    let (boff, bsize, tpath) = (
        template.blob_offset,
        template.blob_size,
        template.path.clone(),
    );
    let want = [pos[0] + 500.0, pos[1] + 1.0, pos[2] - 500.0];

    let mut file = original.clone();
    let new_path =
        worldmap::spawn_object(&mut file, boff, bsize, &tpath, want).expect("spawn failed");
    assert!(file.len() > original.len());
    let delta = (file.len() - original.len()) as i64;

    let worlds2 = worldmap::parse_worlds(&file).unwrap();
    assert_eq!(worlds2.len(), worlds.len());
    // Every blob except the edited one must have moved intact (offset fixups).
    for (w_old, w_new) in worlds.iter().zip(&worlds2) {
        for ((off, size), (off2, size2)) in w_old.pairs1.iter().zip(&w_new.pairs1) {
            let expect = if *off as usize == boff {
                continue;
            } else if *off as usize > boff {
                (*off as i64 + delta) as usize
            } else {
                *off as usize
            };
            assert_eq!(*off2 as usize, expect, "{} pair offset", w_new.name);
            assert_eq!(size2, size, "{} pair size", w_new.name);
            assert_eq!(
                &file[expect..expect + *size as usize],
                &original[*off as usize..*off as usize + *size as usize],
                "{} blob content",
                w_new.name
            );
        }
        for ((off, size), (off2, size2)) in w_old.pairs2.iter().zip(&w_new.pairs2) {
            let expect = if *off as usize == boff {
                continue;
            } else if *off as usize > boff {
                (*off as i64 + delta) as usize
            } else {
                *off as usize
            };
            assert_eq!(*off2 as usize, expect, "{} pair offset", w_new.name);
            assert_eq!(size2, size, "{} pair size", w_new.name);
            assert_eq!(
                &file[expect..expect + *size as usize],
                &original[*off as usize..*off as usize + *size as usize],
                "{} blob content",
                w_new.name
            );
        }
    }
    let w2 = worlds2.iter().find(|w| w.name == "z2_world").unwrap();
    let objects2 = worldmap::collect_objects(&file, w2).unwrap();
    assert_eq!(objects2.len(), objects.len() + 1);
    let spawned = objects2
        .iter()
        .find(|o| o.blob_offset == boff && o.path == new_path)
        .expect("spawned object not found");
    assert_eq!(spawned.position, want);

    worldmap::delete_object(&mut file, boff, spawned.blob_size, &new_path).expect("delete failed");
    assert_eq!(
        file, original,
        "spawn+delete must round-trip byte-identical"
    );
}

/// Region table + point-in-region lookup against the real worlds.bin.
#[test]
fn nearest_region_resolves_rooms() {
    let path = media_dir().join("worlds.bin");
    if !path.exists() {
        eprintln!("skipping: {} not found", path.display());
        return;
    }
    let file = std::fs::read(&path).unwrap();
    let worlds = worldmap::parse_worlds(&file).unwrap();
    let region_name = |w: &worldmap::World, id: u32| {
        worldmap::regions(&file, w)
            .unwrap()
            .into_iter()
            .find(|r| r.id == id)
            .map(|r| r.name)
    };

    // `world`: the z1_ld_rm00 room sits at [3048, -39805, 13557].
    let w = worlds.iter().find(|w| w.name == "world").unwrap();
    let regions = worldmap::regions(&file, w).unwrap();
    assert!(regions.iter().any(|r| r.name == "Z1_LD_Rm00"));
    let id = worldmap::nearest_region(&file, w, [3048.0, -39805.0, 13557.0])
        .expect("no region at z1_ld_rm00");
    assert_eq!(region_name(w, id).as_deref(), Some("Z1_LD_Rm00"));

    // `dlc1_world`: Crypt_Rm01 cell 1 centre (from the parsed blob).
    let w = worlds.iter().find(|w| w.name == "dlc1_world").unwrap();
    let id = worldmap::nearest_region(&file, w, [48660.047, -17152.521, -2452.6926])
        .expect("no region at dlc1 crypt cell");
    assert_eq!(region_name(w, id).as_deref(), Some("Crypt_Rm01"));
}
