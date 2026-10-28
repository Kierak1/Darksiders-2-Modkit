use ds2_modkit::map::roommesh;
use ds2_modkit::pack::{manifest, obpk, upak};
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

/// Carve a real room asset, decode its glomm batches, and check the room lands
/// at the world placement its BOD nodes declare.
#[test]
fn decode_real_room_mesh() {
    let media = media_dir();
    let manifest_path = media.join("manifest.bin");
    if !manifest_path.exists() {
        eprintln!("skipping: {} not found", manifest_path.display());
        return;
    }
    let data = std::fs::read(&manifest_path).unwrap();
    let m = manifest::parse(&data).unwrap();
    let entry = m
        .find("environments/world_z1_ld_rm00")
        .expect("maps asset missing from manifest");
    let carved = upak::carve(&media, &m, entry).unwrap();
    let (_, payload) = obpk::decompress(&carved).unwrap();

    let room = roommesh::parse_room(&payload).unwrap();
    let tris = room.triangles();
    let with_uv = room
        .triangles_with_uv()
        .iter()
        .filter(|(_, uv)| uv.is_some())
        .count();
    println!(
        "{} batches, {} triangles ({} with UVs), origin {:?} scale {}",
        room.batches.len(),
        tris.len(),
        with_uv,
        room.origin,
        room.scale
    );
    assert!(room.batches.len() >= 20, "too few batches");
    assert!(tris.len() >= 30_000, "too few triangles: {}", tris.len());
    assert!(with_uv > 0, "no UV-bearing triangles decoded");
    assert_eq!(room.scale, 10.0);
    // The room's declared origin is the world position of its glomm nodes.
    assert!((room.origin[0] - 3048.0).abs() < 1.0);
    assert!((room.origin[1] + 39805.0).abs() < 1.0);
    assert!((room.origin[2] - 13557.0).abs() < 1.0);

    // Local geometry must sit inside the room's declared bboxes (world ~300
    // units around the origin).
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for (a, b, c) in &tris {
        for p in [a, b, c] {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    let span = (0..3).map(|k| hi[k] - lo[k]).fold(0.0f32, f32::max);
    println!("world bbox {lo:?} .. {hi:?} span {span:.1}");
    assert!(span > 1000.0 && span < 10_000.0, "implausible room span");
}
