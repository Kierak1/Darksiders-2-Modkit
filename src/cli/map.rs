use std::path::Path;

use ds2_modkit::obsp::payload as obsp_payload;
use ds2_modkit::pack::{manifest, obpk, upak};

use super::parse_num;

pub(crate) fn run(cmd: &str, args: &[String], media: &Path) -> Option<ds2_modkit::Result<()>> {
    match cmd {
        "world-dump" => Some(world_dump(args, media)),
        "world-blob" => Some(world_blob(args, media)),
        "world-show" => Some(world_show(args, media)),
        "world-objects" => Some(world_objects(args, media)),
        "world-get" => Some(world_get(args, media)),
        "world-move" => Some(world_move(args, media)),
        "world-spawn" => Some(world_spawn(args, media)),
        "world-delete" => Some(world_delete(args, media)),
        "room-mesh" => Some(room_mesh(args, media)),
        "room-materials" => Some(room_materials(args, media)),
        "prop-mesh" => Some(prop_mesh(args, media)),
        _ => None,
    }
}

fn world_dump(_args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let (worlds, nodes) = ds2_modkit::map::worldmap::load(media)?;
    println!("{} worlds, {} node sets", worlds.len(), nodes.len());
    let by_hash: std::collections::HashMap<u64, &ds2_modkit::map::worldmap::NodeSet> =
        nodes.iter().map(|n| (n.hash, n)).collect();
    for w in &worlds {
        let (e1, e2) = by_hash
            .get(&w.hash)
            .map(|n| (n.entries1.len(), n.entries2.len()))
            .unwrap_or((0, 0));
        println!(
            "{:20} {:#018X} a {} b {} pairs1 {} pairs2 {} flag {} | nodes {}/{}",
            w.name,
            w.hash,
            w.a,
            w.b,
            w.pairs1.len(),
            w.pairs2.len(),
            w.flag,
            e1,
            e2
        );
    }
    Ok(())
}

fn world_blob(args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let off_arg = args
        .get(1)
        .ok_or("usage: ds2-modkit world-blob <offset> <size> [depth]")?;
    let size_arg = args
        .get(2)
        .ok_or("usage: ds2-modkit world-blob <offset> <size> [depth]")?;
    let depth: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);
    let o = parse_num(off_arg)?;
    let n = parse_num(size_arg)?;
    let blob = std::fs::read(media.join("worlds.bin"))?;
    let slice = blob.get(o..o + n).ok_or("world-blob range out of bounds")?;
    let payload = obsp_payload::parse_payload(slice)?;
    println!(
        "blob {o:#X}..{:#X} ({} B): {} properties, complete {}",
        o + n,
        slice.len(),
        payload.properties.len(),
        payload.complete
    );
    for p in &payload.properties {
        dump_value(&p.name, &p.value, 1, depth);
    }
    Ok(())
}

fn world_show(args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let name = args.get(1).ok_or("usage: ds2-modkit world-show <world>")?;
    let file = std::fs::read(media.join("worlds.bin"))?;
    let worlds = ds2_modkit::map::worldmap::parse_worlds(&file)?;
    let w = worlds
        .iter()
        .find(|w| w.name.eq_ignore_ascii_case(name))
        .or_else(|| {
            worlds
                .iter()
                .find(|w| w.name.to_lowercase().contains(&name.to_lowercase()))
        })
        .ok_or_else(|| format!("no world named {name}"))?;
    let blob = ds2_modkit::map::worldmap::world_blob(&file, w)?;
    let payload = obsp_payload::parse_payload(blob)?;
    println!(
        "{} {:#018X}: blob {:#X}..{:#X} ({} B), {} properties, complete {}",
        w.name,
        w.hash,
        w.a,
        w.a as usize + w.b as usize,
        blob.len(),
        payload.properties.len(),
        payload.complete
    );
    for p in &payload.properties {
        println!("  {} = {}", p.name, p.value);
    }
    Ok(())
}

fn world_objects(args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let name = args
        .get(1)
        .ok_or("usage: ds2-modkit world-objects <world> [filter] [limit]")?;
    let filter = args.get(2).map(|s| s.to_lowercase());
    let limit: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(50);
    let file = std::fs::read(media.join("worlds.bin"))?;
    let worlds = ds2_modkit::map::worldmap::parse_worlds(&file)?;
    let w = worlds
        .iter()
        .find(|w| w.name.eq_ignore_ascii_case(name))
        .or_else(|| {
            worlds
                .iter()
                .find(|w| w.name.to_lowercase().contains(&name.to_lowercase()))
        })
        .ok_or_else(|| format!("no world named {name}"))?;
    let objects = ds2_modkit::map::worldmap::collect_objects(&file, w)?;
    println!("{} objects with positions in {}", objects.len(), w.name);
    let mut by_class: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for o in &objects {
        *by_class.entry(o.class.as_str()).or_default() += 1;
    }
    let mut classes: Vec<_> = by_class.into_iter().collect();
    classes.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (c, n) in classes.iter().take(20) {
        println!("  {n:5} {c}");
    }
    let mut shown = 0;
    for o in &objects {
        if let Some(f) = &filter {
            if !o.class.to_lowercase().contains(f) && !o.name.to_lowercase().contains(f) {
                continue;
            }
        }
        println!(
            "  [{}] {} {} @ ({:.1}, {:.1}, {:.1}){} blob {:#X}+{:#X}",
            o.class,
            o.name,
            o.path,
            o.position[0],
            o.position[1],
            o.position[2],
            o.rotation
                .map(|r| format!(" rot ({:.1}, {:.1}, {:.1})", r[0], r[1], r[2]))
                .unwrap_or_default(),
            o.blob_offset,
            o.blob_size
        );
        shown += 1;
        if shown >= limit {
            break;
        }
    }
    Ok(())
}

fn world_get(args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let usage = "usage: ds2-modkit world-get <blob_offset> <blob_size> <path>";
    let boff = parse_num(args.get(1).ok_or(usage)?)?;
    let bsize = parse_num(args.get(2).ok_or(usage)?)?;
    let path = args.get(3).ok_or(usage)?;
    let data = std::fs::read(media.join("worlds.bin"))?;
    let blob = data
        .get(boff..boff + bsize)
        .ok_or("blob range out of bounds")?;
    let parsed = obsp_payload::parse_payload(blob)?;
    match obsp_payload::resolve_path(blob, &parsed, path) {
        Some(t) => println!(
            "{path}: offset {:#X} size {} = {}",
            t.offset, t.size, t.value
        ),
        None => println!("{path}: NOT FOUND (complete {})", parsed.complete),
    }
    Ok(())
}

fn world_move(args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let usage =
        "usage: ds2-modkit world-move <blob_offset> <blob_size> <path> <x> <y> <z> [--write]";
    let boff = parse_num(args.get(1).ok_or(usage)?)?;
    let bsize = parse_num(args.get(2).ok_or(usage)?)?;
    let path = args.get(3).ok_or(usage)?.clone();
    let vals: Vec<f32> = args
        .get(4..7)
        .ok_or(usage)?
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect();
    if vals.len() != 3 {
        return Err(usage.into());
    }
    let write = args.iter().any(|a| a == "--write");
    let path_file = media.join("worlds.bin");
    let mut data = std::fs::read(&path_file)?;
    let mut blob = data
        .get(boff..boff + bsize)
        .ok_or("blob range out of bounds")?
        .to_vec();
    let parsed = obsp_payload::parse_payload(&blob)?;
    println!("blob parsed, complete {}", parsed.complete);
    for (k, v) in vals.iter().enumerate() {
        let p = format!("{path}.Position[{k}]");
        match obsp_payload::resolve_path(&blob, &parsed, &p) {
            Some(target) => {
                println!(
                    "{p}: offset {:#X} size {} value {}",
                    target.offset, target.size, target.value
                );
                obsp_payload::patch_target(&mut blob, &target, &v.to_string())?;
            }
            None => println!("{p}: NOT FOUND"),
        }
    }
    if write {
        let bak = media.join("worlds.bin.bak");
        if !bak.exists() {
            std::fs::copy(&path_file, &bak)?;
        }
        data[boff..boff + bsize].copy_from_slice(&blob);
        std::fs::write(&path_file, &data)?;
        println!("wrote {}", path_file.display());
    }
    Ok(())
}

fn world_spawn(args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let usage =
        "usage: ds2-modkit world-spawn <blob_offset> <blob_size> <path> <x> <y> <z> [--write]";
    let boff = parse_num(args.get(1).ok_or(usage)?)?;
    let bsize = parse_num(args.get(2).ok_or(usage)?)?;
    let path = args.get(3).ok_or(usage)?.clone();
    let vals: Vec<f32> = args
        .get(4..7)
        .ok_or(usage)?
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect();
    if vals.len() != 3 {
        return Err(usage.into());
    }
    let write = args.iter().any(|a| a == "--write");
    let path_file = media.join("worlds.bin");
    let mut data = std::fs::read(&path_file)?;
    let before = data.len();
    let new_path = ds2_modkit::map::worldmap::spawn_object(
        &mut data,
        boff,
        bsize,
        &path,
        [vals[0], vals[1], vals[2]],
    )?;
    println!(
        "spawned {new_path} at ({}, {}, {}): file {before} -> {} B",
        vals[0],
        vals[1],
        vals[2],
        data.len()
    );
    if write {
        let bak = media.join("worlds.bin.bak");
        if !bak.exists() {
            std::fs::copy(&path_file, &bak)?;
        }
        std::fs::write(&path_file, &data)?;
        println!("wrote {}", path_file.display());
    }
    Ok(())
}

fn world_delete(args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let usage = "usage: ds2-modkit world-delete <blob_offset> <blob_size> <path> [--write]";
    let boff = parse_num(args.get(1).ok_or(usage)?)?;
    let bsize = parse_num(args.get(2).ok_or(usage)?)?;
    let path = args.get(3).ok_or(usage)?;
    let write = args.iter().any(|a| a == "--write");
    let path_file = media.join("worlds.bin");
    let mut data = std::fs::read(&path_file)?;
    let before = data.len();
    ds2_modkit::map::worldmap::delete_object(&mut data, boff, bsize, path)?;
    println!(
        "deleted {path}: blob {bsize} B, file {before} -> {} B",
        data.len()
    );
    if write {
        let bak = media.join("worlds.bin.bak");
        if !bak.exists() {
            std::fs::copy(&path_file, &bak)?;
        }
        std::fs::write(&path_file, &data)?;
        println!("wrote {}", path_file.display());
    }
    Ok(())
}

fn room_mesh(args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let data = std::fs::read(media.join("manifest.bin"))?;
    let m = manifest::parse(&data)?;
    use std::fmt::Write;
    let asset = args
        .get(1)
        .ok_or("usage: ds2-modkit room-mesh <asset> [out.obj]")?;
    let entry = m
        .find(asset)
        .ok_or_else(|| format!("asset not found in manifest: {asset}"))?;
    let carved = upak::carve(media, &m, entry)?;
    let (_, payload) = obpk::decompress(&carved)?;
    let room = ds2_modkit::map::roommesh::parse_room(&payload)?;
    let tris = room.triangles();
    let out = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| format!("{}.obj", entry.name));
    let mut obj = String::new();
    for (a, b, c) in &tris {
        for p in [a, b, c] {
            writeln!(obj, "v {} {} {}", p[0], p[1], p[2]).unwrap();
        }
    }
    for i in 0..tris.len() {
        writeln!(obj, "f {} {} {}", i * 3 + 1, i * 3 + 2, i * 3 + 3).unwrap();
    }
    std::fs::write(&out, obj)?;
    println!(
        "{} batches, {} triangles, origin {:?} scale {} -> {out}",
        room.batches.len(),
        tris.len(),
        room.origin,
        room.scale
    );
    Ok(())
}

fn room_materials(args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let data = std::fs::read(media.join("manifest.bin"))?;
    let m = manifest::parse(&data)?;
    use std::fmt::Write;
    let asset = args
        .get(1)
        .ok_or("usage: ds2-modkit room-materials <asset> [out.obj]")?;
    let entry = m
        .find(asset)
        .ok_or_else(|| format!("asset not found in manifest: {asset}"))?;
    let folder = ds2_modkit::map::roommat::zone_folder(&m, entry)
        .ok_or("no media zone folder matches this room")?;
    let carved = upak::carve(media, &m, entry)?;
    let (_, payload) = obpk::decompress(&carved)?;
    let room = ds2_modkit::map::roommesh::parse_room(&payload)?;
    let defs = ds2_modkit::map::propmesh::material_defs(&payload);
    let wanted: std::collections::HashSet<String> = defs
        .keys()
        .map(|n| ds2_modkit::map::roommat::material_key(n))
        .collect();
    eprintln!("scanning world/environments/{folder} ...");
    let db = ds2_modkit::map::roommat::build(
        media,
        &m,
        &folder,
        if wanted.is_empty() {
            None
        } else {
            Some(&wanted)
        },
    )?;
    eprintln!(
        "{} assets, {} source meshes, {} materials",
        db.assets,
        db.meshes,
        db.fingerprints.len()
    );
    let classes = ds2_modkit::map::roommat::classify(&room, &db);
    let out = args.get(2).cloned();
    // Diffuse binding per material key (room material name -> texture).
    let mut defs_by_key: std::collections::HashMap<
        String,
        (&String, &ds2_modkit::map::propmesh::MaterialTex),
    > = std::collections::HashMap::new();
    for (name, def) in &defs {
        defs_by_key
            .entry(ds2_modkit::map::roommat::material_key(name))
            .or_insert((name, def));
    }
    let mut used: Vec<String> = Vec::new();
    for class in classes.iter().flatten() {
        if let Some((_, def)) = defs_by_key.get(&ds2_modkit::map::roommat::material_key(&class.0)) {
            if !def.diffuse.is_empty() && !used.contains(&def.diffuse) {
                used.push(def.diffuse.clone());
            }
        }
    }
    let mut loader = ds2_modkit::map::texmap::TextureLoader::default();
    let raw = loader.raw_dds_many(media, &m, &used);
    eprintln!(
        "textures: {} of {} diffuse names resolved from the packages",
        raw.len(),
        used.len()
    );
    let tex_dir = out.as_ref().map(|o| format!("{o}_textures"));
    if let Some(dir) = &tex_dir {
        std::fs::create_dir_all(dir)?;
    }
    let mut tex_written = 0usize;
    let mut obj = String::new();
    let mut mtl = String::new();
    let mut mtl_names: Vec<String> = Vec::new();
    let mut base = 0usize;
    let mut classified = 0usize;
    let mut face_total = 0usize;
    for (batch, class) in room.batches.iter().zip(&classes) {
        let (mat, mtl_name) = match class {
            Some((name, _)) => {
                classified += 1;
                (name.clone(), sanitize_mtl(name))
            }
            None => ("-".to_string(), "unmatched".to_string()),
        };
        if out.is_some() && !mtl_names.contains(&mtl_name) {
            let c = if class.is_some() {
                ds2_modkit::map::roommat::material_color(&db, &mat)
            } else {
                [110, 110, 110]
            };
            let mut kd = String::new();
            if let Some((_, def)) = defs_by_key.get(&ds2_modkit::map::roommat::material_key(&mat)) {
                if let Some(bytes) = raw.get(&def.diffuse) {
                    if let Some(dir) = &tex_dir {
                        let dds_path = format!("{dir}/{mtl_name}.dds");
                        std::fs::write(&dds_path, bytes)?;
                        tex_written += 1;
                        let dir_name = Path::new(dir)
                            .file_name()
                            .map(|f| f.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        kd = format!("map_Kd {dir_name}/{mtl_name}.dds\n");
                    }
                }
            }
            mtl.push_str(&format!(
                "newmtl {mtl_name}\nKd {:.4} {:.4} {:.4}\n{kd}\n",
                c[0] as f32 / 255.0,
                c[1] as f32 / 255.0,
                c[2] as f32 / 255.0
            ));
            mtl_names.push(mtl_name.clone());
        }
        let mut faces = batch.indices.len() / 3;
        if out.is_some() {
            obj.push_str(&format!("usemtl {mtl_name}\n"));
            faces = 0;
            for t in batch.indices.as_chunks::<3>().0 {
                let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
                if a >= batch.positions.len()
                    || b >= batch.positions.len()
                    || c >= batch.positions.len()
                {
                    continue;
                }
                for v in [a, b, c] {
                    let p = room.world_position(batch.positions[v]);
                    writeln!(obj, "v {} {} {}", p[0], p[1], p[2]).unwrap();
                }
                writeln!(obj, "f {} {} {}", base + 1, base + 2, base + 3).unwrap();
                base += 3;
                faces += 1;
            }
        }
        face_total += faces;
        match class {
            Some((name, cov)) => println!(
                "{:<16} vc={:<6} tris={:<6} {name} ({cov:.2})",
                batch.name,
                batch.positions.len(),
                faces
            ),
            None => println!(
                "{:<16} vc={:<6} tris={:<6} no material match (grey)",
                batch.name,
                batch.positions.len(),
                faces
            ),
        }
    }
    if let Some(out) = out {
        let mtl_path = format!("{out}.mtl");
        let mtl_file = Path::new(&mtl_path).file_name().unwrap().to_string_lossy();
        obj = format!("mtllib {mtl_file}\n{obj}");
        std::fs::write(&out, obj)?;
        std::fs::write(&mtl_path, mtl)?;
        println!("-> {out} (+ {mtl_path}, {tex_written} dds)");
    }
    println!(
        "{classified}/{} batches classified, {face_total} triangles",
        room.batches.len()
    );
    Ok(())
}

fn prop_mesh(args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let data = std::fs::read(media.join("manifest.bin"))?;
    let m = manifest::parse(&data)?;
    use std::fmt::Write;
    let asset = args
        .get(1)
        .ok_or("usage: ds2-modkit prop-mesh <asset> [out.obj]")?;
    let entry = m
        .find(asset)
        .ok_or_else(|| format!("asset not found in manifest: {asset}"))?;
    let carved = upak::carve(media, &m, entry)?;
    let (_, payload) = obpk::decompress(&carved)?;
    let props = ds2_modkit::map::propmesh::parse_props(&payload);
    let tris: usize = props.iter().map(|p| p.indices.len() / 3).sum();
    let out = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| format!("{}.props.obj", entry.name));
    let mut obj = String::new();
    for p in &props {
        for (t, c) in p.triangles() {
            for v in t {
                writeln!(obj, "v {} {} {}", v[0], v[1], v[2]).unwrap();
            }
            let _ = c;
        }
    }
    let mut base = 0usize;
    for p in &props {
        for _ in p.triangles() {
            writeln!(obj, "f {} {} {}", base + 1, base + 2, base + 3).unwrap();
            base += 3;
        }
    }
    std::fs::write(&out, obj)?;
    println!("{} props, {tris} triangles:", props.len());
    for p in &props {
        println!(
            "  {} vc {} tris {} color {:?}",
            p.name,
            p.positions.len(),
            p.indices.len() / 3,
            p.color
        );
    }
    println!("-> {out}");
    Ok(())
}

fn dump_value(name: &str, v: &obsp_payload::Value, indent: usize, max: usize) {
    let pad = "  ".repeat(indent);
    match v {
        obsp_payload::Value::Object(o) => {
            let class = o.class_ref.clone().unwrap_or_default();
            if indent >= max || o.properties.is_empty() {
                println!("{pad}{name} = obj({class})");
                return;
            }
            println!("{pad}{name} = obj({class} {{");
            for p in &o.properties {
                dump_value(&p.name, &p.value, indent + 1, max);
            }
            println!("{pad}}})");
        }
        obsp_payload::Value::Array(items) => {
            if indent >= max || items.len() > 16 {
                println!("{pad}{name} = [{} items]", items.len());
                return;
            }
            println!("{pad}{name} = [");
            for (i, it) in items.iter().enumerate() {
                dump_value(&format!("[{i}]"), it, indent + 1, max);
            }
            println!("{pad}]");
        }
        obsp_payload::Value::Map(pairs) => {
            if indent >= max || pairs.len() > 16 {
                println!("{pad}{name} = {{map {} items}}", pairs.len());
                return;
            }
            println!("{pad}{name} = {{");
            for (k, val) in pairs {
                dump_value(&format!("{k}"), val, indent + 1, max);
            }
            println!("{pad}}}");
        }
        other => println!("{pad}{name} = {other}"),
    }
}

fn sanitize_mtl(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
