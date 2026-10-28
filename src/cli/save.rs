//! `save-*` commands: browse/edit `.dsav` saves.

use std::path::{Path, PathBuf};

use ds2_modkit::dsave::{self, ItemDb, LocTable};
use ds2_modkit::obsp;

pub(crate) fn run(cmd: &str, args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    if cmd == "save-slots" {
        return save_slots(args);
    }
    let path = args
        .get(1)
        .ok_or("usage: ds2-modkit save-<cmd> <file.dsav> ...")?;
    match cmd {
        "save-list" => {
            let data = std::fs::read(path)?;
            let save = dsave::parse(&data)?;
            // Filters: a plain positional argument is a text filter (legacy),
            // plus --container/--rarity/--name and --sort <attr>.
            let mut filter = String::new();
            let mut container = String::new();
            let mut rarity: Option<u32> = None;
            let mut sort: Option<String> = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--container" => {
                        i += 1;
                        container = args.get(i).ok_or("--container needs a list name")?.clone();
                    }
                    "--rarity" => {
                        i += 1;
                        rarity = Some(
                            args.get(i)
                                .ok_or("--rarity needs a number")?
                                .parse()
                                .map_err(|_| "--rarity needs a number")?,
                        );
                    }
                    "--name" => {
                        i += 1;
                        filter = args.get(i).ok_or("--name needs text")?.to_lowercase();
                    }
                    "--sort" => {
                        i += 1;
                        sort = Some(args.get(i).ok_or("--sort needs an attribute")?.clone());
                    }
                    other if !other.starts_with("--") => filter = other.to_lowercase(),
                    other => return Err(format!("unknown save-list option {other:?}").into()),
                }
                i += 1;
            }
            // The DB/loc are optional: without them the list falls back to
            // prettified def names.
            let db_hold = try_load_db(media);
            let db = db_hold.as_ref().map(|(db, _, _)| db);
            let loc = try_load_loc(media);
            let catalog = db_hold
                .as_ref()
                .map(|(_, obsp, data)| dsave::AttributeCatalog::build(obsp, data, &[&save]));
            let sort_values: std::collections::HashMap<usize, i64> = match (&sort, &catalog) {
                (Some(attr), Some(catalog)) => {
                    let want_id: Option<u32> = attr.trim().parse().ok();
                    dsave::sorted_items(&save, catalog, attr)
                        .into_iter()
                        .filter_map(|idx| {
                            let item = save.item(idx)?;
                            let v = dsave::stat_views(item, catalog).into_iter().find(|v| {
                                v.attribute.eq_ignore_ascii_case(attr) || want_id == Some(v.enum_id)
                            })?;
                            Some((idx, v.value))
                        })
                        .collect()
                }
                (Some(_), None) => {
                    return Err("--sort needs the item DB (scripts.obsp); set DS2_MEDIA".into());
                }
                (None, _) => std::collections::HashMap::new(),
            };
            println!(
                "{}: module v{} format {} ({} objects, {} inventory lists)",
                path,
                save.module.version,
                save.info.format,
                save.module.objects.len(),
                save.module.inventory.len()
            );
            let mut rows: Vec<(usize, String, Option<u32>, String)> = Vec::new();
            for r in &save.items() {
                let Some(item) = save.item(r.index) else {
                    continue;
                };
                let origin = save.origin(r);
                let def = db.and_then(|db| db.get(&item.name));
                let item_rarity = def.and_then(|d| d.rarity);
                let label = match (db, &loc) {
                    (Some(db), Some(loc)) => dsave::display_name(&item.name, item_rarity, loc, db),
                    _ => dsave::pretty_name(&item.name),
                };
                if !container.is_empty() {
                    let origin_match = origin.eq_ignore_ascii_case(&container);
                    let def_match = def
                        .and_then(|d| d.container.as_deref())
                        .is_some_and(|c| c.eq_ignore_ascii_case(&container));
                    if !origin_match && !def_match {
                        continue;
                    }
                }
                if let Some(want) = rarity {
                    if item_rarity != Some(want) {
                        continue;
                    }
                }
                if !filter.is_empty()
                    && !item.name.to_lowercase().contains(&filter)
                    && !label.to_lowercase().contains(&filter)
                {
                    continue;
                }
                rows.push((r.index, label, item_rarity, origin));
            }
            if let Some(attr) = &sort {
                // Sorted items first (value desc), then the rest in index order.
                let catalog = catalog
                    .as_ref()
                    .ok_or("--sort needs the item DB (scripts.obsp)")?;
                let sorted = dsave::sorted_items(&save, catalog, attr);
                let pos: std::collections::HashMap<usize, usize> = sorted
                    .iter()
                    .enumerate()
                    .map(|(p, idx)| (*idx, p))
                    .collect();
                rows.sort_by_key(|(idx, ..)| pos.get(idx).copied().unwrap_or(usize::MAX));
            }
            let mut shown = 0usize;
            for (idx, label, item_rarity, origin) in &rows {
                let tag = item_rarity.map(|r| format!(" [R{r}]")).unwrap_or_default();
                let stat = sort
                    .as_deref()
                    .and_then(|attr| sort_values.get(idx).map(|v| format!("  {attr}={v}")))
                    .unwrap_or_default();
                println!("[{idx}] {label}{tag} ({origin}){stat}");
                shown += 1;
            }
            println!("{shown} items shown");
            Ok(())
        }
        "save-show" => {
            let data = std::fs::read(path)?;
            let save = dsave::parse(&data)?;
            let index: usize = args
                .get(2)
                .ok_or("usage: ds2-modkit save-show <file.dsav> <item-index>")?
                .parse()?;
            let item = save
                .item(index)
                .ok_or_else(|| format!("no item at index {index}"))?;
            for line in dsave::item_lines(item, 0) {
                println!("{line}");
            }
            Ok(())
        }
        "save-set" => {
            let mut save = load_save(path)?;
            let index: usize = args
                .get(2)
                .ok_or("usage: ds2-modkit save-set <file.dsav> <item-index> <field-path> <value>")?
                .parse()?;
            let field = args.get(3).ok_or(
                "usage: ds2-modkit save-set <file.dsav> <item-index> <field-path> <value>",
            )?;
            let value = args.get(4).ok_or(
                "usage: ds2-modkit save-set <file.dsav> <item-index> <field-path> <value>",
            )?;
            let item = save
                .item_mut(index)
                .ok_or_else(|| format!("no item at index {index}"))?;
            let change = dsave::set_item_field(item, field, value)?;
            write_and_report(&save, path, &format!("item {index} {field}: {change}"))
        }
        "save-add" => {
            let mut save = load_save(path)?;
            let container = args
                .get(2)
                .ok_or("usage: ds2-modkit save-add <file.dsav> <container-list> <template-index> [new-name]")?;
            let template: usize = args
                .get(3)
                .ok_or("usage: ds2-modkit save-add <file.dsav> <container-list> <template-index> [new-name]")?
                .parse()?;
            let mut item = save
                .item(template)
                .ok_or_else(|| format!("no item at index {template}"))?
                .clone();
            if let Some(name) = args.get(4) {
                item.name = name.clone();
            }
            let list = save
                .inventory_list(container)
                .ok_or_else(|| format!("no inventory list named {container}"))?;
            let name = list.name.clone();
            let new_len = list.items.len() + 1;
            list.items.push(item);
            write_and_report(
                &save,
                path,
                &format!("added clone of item {template} to {name} ({new_len} items)"),
            )
        }
        "save-remove" => {
            let mut save = load_save(path)?;
            let index: usize = args
                .get(2)
                .ok_or("usage: ds2-modkit save-remove <file.dsav> <item-index>")?
                .parse()?;
            let r = save
                .items()
                .into_iter()
                .find(|r| r.index == index)
                .ok_or_else(|| format!("no item at index {index}"))?;
            let origin = save.origin(&r);
            let removed = save
                .remove_item(index)
                .ok_or_else(|| format!("no item at index {index}"))?;
            write_and_report(
                &save,
                path,
                &format!("removed {:?} from {origin}", removed.name),
            )
        }
        "save-stats" => {
            let save = load_save(path)?;
            let index: usize = args
                .get(2)
                .ok_or("usage: ds2-modkit save-stats <file.dsav> <item-index>")?
                .parse()?;
            let item = save
                .item(index)
                .ok_or_else(|| format!("no item at index {index}"))?;
            let catalog = load_catalog(media, &save)?;
            for (k, v) in dsave::stat_views(item, &catalog).iter().enumerate() {
                println!(
                    "[{k}] {:<20} enum={:<4} value={:<5} table={:<32} row={:<3} path={}",
                    v.attribute,
                    v.enum_id,
                    v.value,
                    v.table.as_deref().unwrap_or("-"),
                    v.row.map(|r| r.to_string()).unwrap_or_else(|| "-".into()),
                    v.value_path
                );
            }
            Ok(())
        }
        "save-stat" | "save-stat-row" => {
            let mut save = load_save(path)?;
            let index: usize = args
                .get(2)
                .ok_or(
                    "usage: ds2-modkit save-stat <file.dsav> <item-index> <attribute|enum> <value>",
                )?
                .parse()?;
            let attr = args.get(3).ok_or(
                "usage: ds2-modkit save-stat <file.dsav> <item-index> <attribute|enum> <value>",
            )?;
            let value = args.get(4).ok_or(
                "usage: ds2-modkit save-stat <file.dsav> <item-index> <attribute|enum> <value>",
            )?;
            let catalog = load_catalog(media, &save)?;
            let item = save
                .item_mut(index)
                .ok_or_else(|| format!("no item at index {index}"))?;
            let change = if cmd == "save-stat" {
                let value: i64 = value
                    .parse()
                    .map_err(|_| format!("bad stat value {value:?}"))?;
                dsave::set_stat(item, &catalog, attr, value)?
            } else {
                // 1..=40 = synthetic table row; anything else = the signed
                // level offset real weapons store (`row = level + offset`).
                let job: i64 = value
                    .parse()
                    .map_err(|_| format!("bad table row {value:?}"))?;
                if (1..=40).contains(&job) {
                    dsave::set_stat_row(item, &catalog, attr, job as u32)?
                } else if (i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&job) {
                    dsave::set_stat_row_offset(item, &catalog, attr, job as i32)?
                } else {
                    return Err(format!("table offset {job} does not fit in i32").into());
                }
            };
            write_and_report(&save, path, &format!("item {index} {change}"))
        }
        "save-add-item" => {
            let mut save = load_save(path)?;
            let container = args.get(2).ok_or(
                "usage: ds2-modkit save-add-item <file.dsav> <container-list> <def-name> [rarity] [level]",
            )?;
            let def = args.get(3).ok_or(
                "usage: ds2-modkit save-add-item <file.dsav> <container-list> <def-name> [rarity] [level]",
            )?;
            let rarity: Option<u32> = match args.get(4) {
                Some(s) => Some(s.parse().map_err(|_| format!("bad rarity {s:?}"))?),
                None => None,
            };
            let level: Option<i64> = match args.get(5) {
                Some(s) => Some(s.parse().map_err(|_| format!("bad level {s:?}"))?),
                None => None,
            };
            let (db, _obsp, _data) = load_db(media)?;
            let index = dsave::add_item(&mut save, Some(&db), container, def, rarity, level)?;
            let name = save
                .item(index)
                .map(|i| i.name.clone())
                .unwrap_or_else(|| def.clone());
            write_and_report(
                &save,
                path,
                &format!("added {name} as item {index} in {container}"),
            )
        }
        "save-possess" => {
            let mut save = load_save(path)?;
            let index: usize = args
                .get(2)
                .ok_or("usage: ds2-modkit save-possess <file.dsav> <item-index> [--level N|--max|--add-mod <attr> <value>|--del-mod <attr>|--add-effect|--del-effect <stat> <effect>]")?
                .parse()?;
            if args.len() <= 3 {
                let item = save
                    .item(index)
                    .ok_or_else(|| format!("no item at index {index}"))?;
                match dsave::possessed_info(item) {
                    Some(info) => println!(
                        "item {index}: possessed xp={} level={} cap={} fed={}",
                        info.cur_xp,
                        info.level,
                        info.level_cap,
                        info.fed.len()
                    ),
                    None => println!("item {index} is not a possessed item"),
                }
                return Ok(());
            }
            let ops = &args[3..];
            // A catalog resolves new attribute names for `--add-mod`; without
            // the game files only item-local names and enum ids work.
            let catalog = if ops.iter().any(|o| o == "--add-mod") {
                // A missing/invalid scripts.obsp degrades to enum-id-only
                // resolution, which `possessed_add_mod` reports clearly.
                load_catalog(media, &save).ok()
            } else {
                None
            };
            let mut changes: Vec<String> = Vec::new();
            let mut i = 0usize;
            while i < ops.len() {
                let (change, used) = possess_step(&mut save, index, &ops[i..], catalog.as_ref())?;
                changes.push(change);
                i += used;
            }
            write_and_report(
                &save,
                path,
                &format!("item {index}: {}", changes.join("; ")),
            )
        }
        "save-move" => {
            let mut save = load_save(path)?;
            let index: usize = args
                .get(2)
                .ok_or("usage: ds2-modkit save-move <file.dsav> <item-index> <container>")?
                .parse()?;
            let container = args
                .get(3)
                .ok_or("usage: ds2-modkit save-move <file.dsav> <item-index> <container>")?;
            let db_hold = try_load_db(media);
            let db = db_hold.as_ref().map(|(db, _, _)| db);
            let new_index = dsave::move_item(&mut save, index, container, db)?;
            write_and_report(
                &save,
                path,
                &format!("moved item {index} to {container} (now item {new_index})"),
            )
        }
        "save-equip" => {
            let mut save = load_save(path)?;
            let index: usize = args
                .get(2)
                .ok_or("usage: ds2-modkit save-equip <file.dsav> <item-index>")?
                .parse()?;
            let db_hold = try_load_db(media);
            let db = db_hold.as_ref().map(|(db, _, _)| db);
            let new_index = dsave::equip_item(&mut save, index, db)?;
            write_and_report(
                &save,
                path,
                &format!("equipped item {index} (now item {new_index})"),
            )
        }
        "save-unequip" => {
            let mut save = load_save(path)?;
            let index: usize = args
                .get(2)
                .ok_or("usage: ds2-modkit save-unequip <file.dsav> <item-index>")?
                .parse()?;
            let new_index = dsave::unequip_item(&mut save, index)?;
            write_and_report(
                &save,
                path,
                &format!("unequipped item {index} (still item {new_index})"),
            )
        }
        "save-diff" => {
            let save = load_save(path)?;
            let bak = format!("{path}.bak");
            if !std::path::Path::new(&bak).exists() {
                return Err(format!("no backup at {bak}; nothing to diff against").into());
            }
            let old = load_save(&bak)?;
            let lines = save.diff(&old);
            if lines.is_empty() {
                println!("{path}: no differences vs {bak}");
            } else {
                println!("{path} vs {bak}: {} change(s)", lines.len());
                for line in &lines {
                    println!("  {line}");
                }
            }
            Ok(())
        }
        "save-totals" => save_totals(path, args, media),
        "save-bulk" => save_bulk(path, args, media),
        "save-apply" => save_apply(path, args, media),
        "save-copy-item" => save_copy_item(path, args, media),
        _ => Err(format!("unknown save command: {cmd}").into()),
    }
}

/// `save-totals <file.dsav> [container|all]` — per-attribute sums over the
/// equipped items (default), one container, or the whole save.
fn save_totals(path: &str, args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let save = load_save(path)?;
    let catalog = load_catalog(media, &save)?;
    let (scope, label) = match args.get(2).map(String::as_str) {
        None | Some("equipped") => (dsave::TotalScope::Equipped, "equipped".to_string()),
        Some(s) if s.eq_ignore_ascii_case("all") => {
            (dsave::TotalScope::All, "all items".to_string())
        }
        Some(s) => (dsave::TotalScope::Container(s), s.to_string()),
    };
    let totals = dsave::totals(&save, &catalog, scope);
    println!("{path}: {label} totals ({} attribute(s))", totals.len());
    for t in &totals {
        println!("  {}", dsave::total_line(&t.attribute, t));
    }
    Ok(())
}

/// `save-bulk <file.dsav> <op> ...` — one conservative bulk operation.
fn save_bulk(path: &str, args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let op = dsave::parse_bulk_op(&args[2..])?;
    let mut save = load_save(path)?;
    let catalog = if op.needs_catalog() {
        Some(load_catalog(media, &save)?)
    } else {
        None
    };
    let report = dsave::apply_bulk(&mut save, catalog.as_ref(), &op)?;
    println!(
        "bulk {}: {} item(s) changed, {} skipped",
        op.name(),
        report.changed,
        report.skipped
    );
    for line in report.lines.iter().take(20) {
        println!("  {line}");
    }
    if report.lines.len() > 20 {
        println!("  ... ({} more lines)", report.lines.len() - 20);
    }
    write_and_report(&save, path, &format!("bulk {}", op.name()))
}

/// `save-apply <file.dsav> <preset.toml>` — all-or-nothing preset.
fn save_apply(path: &str, args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let preset_path = args
        .get(2)
        .ok_or("usage: ds2-modkit save-apply <file.dsav> <preset.toml>")?;
    let mut save = load_save(path)?;
    let preset = dsave::load_preset(Path::new(preset_path))?;
    let db_hold = try_load_db(media);
    let db = db_hold.as_ref().map(|(db, _, _)| db);
    let catalog = db_hold
        .as_ref()
        .map(|(_, obsp, data)| dsave::AttributeCatalog::build(obsp, data, &[&save]));
    let lines = dsave::apply_preset(&mut save, db, catalog.as_ref(), &preset)?;
    for line in &lines {
        println!("  {line}");
    }
    write_and_report(
        &save,
        path,
        &format!("applied {} preset operation(s)", lines.len()),
    )
}

/// `save-slots [dir|save.dsav]` — list the `dssave*.dsav` files with a summary.
fn save_slots(args: &[String]) -> ds2_modkit::Result<()> {
    let dir = match args.get(1) {
        Some(d) => {
            let p = PathBuf::from(d);
            if p.is_file() {
                // a save file lists its own directory ("the save's parent dir")
                p.parent().map(Path::to_path_buf).unwrap_or(p)
            } else {
                p
            }
        }
        None => dsave::default_save_dir()
            .ok_or("no save directory found; pass one: ds2-modkit save-slots <dir>")?,
    };
    let slots = dsave::list_slots(&dir)?;
    if slots.is_empty() {
        println!("{}: no dssave*.dsav files", dir.display());
        return Ok(());
    }
    for s in &slots {
        println!(
            "{}  {} B  {}  counter={} global_version={:#010X} flags={:#x}  items={} equipped={}",
            s.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            s.bytes,
            s.modified
                .map(dsave::format_time)
                .unwrap_or_else(|| "-".to_string()),
            s.counter,
            s.global_version,
            s.flags,
            s.items,
            s.equipped
        );
        for rec in &s.locations {
            println!(
                "  last saved: {} | {} | {} | {} | {}",
                rec[0], rec[1], rec[2], rec[3], rec[4]
            );
        }
    }
    Ok(())
}

/// `save-copy-item <src.dsav> <item-index-or-def> <dst.dsav> <container>` —
/// clone an item from one save into another (never the `b816` equip flag).
fn save_copy_item(src_path: &str, args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let usage =
        "usage: ds2-modkit save-copy-item <src.dsav> <item-index-or-def> <dst.dsav> <container>";
    let selector = args.get(2).ok_or(usage)?;
    let dst_path = args.get(3).ok_or(usage)?;
    let container = args.get(4).ok_or(usage)?;
    let src = load_save(src_path)?;
    let idx = resolve_item_arg(&src, selector)?;
    let name = src
        .item(idx)
        .map(|i| i.name.clone())
        .ok_or_else(|| format!("no item at index {idx}"))?;
    let mut dst = load_save(dst_path)?;
    let db_hold = try_load_db(media);
    let db = db_hold.as_ref().map(|(db, _, _)| db);
    let new_index = dsave::copy_item(&src, idx, &mut dst, container, db)?;
    write_and_report(
        &dst,
        dst_path,
        &format!(
            "copied {name} (item {idx} of {src_path}) to {dst_path} as item {new_index} in {container}"
        ),
    )
}

/// Resolve the `<item-index-or-def>` argument of `save-copy-item`.
fn resolve_item_arg(save: &dsave::Save, arg: &str) -> ds2_modkit::Result<usize> {
    if let Ok(i) = arg.parse::<usize>() {
        return match save.item(i) {
            Some(_) => Ok(i),
            None => Err(format!("no item at index {i}").into()),
        };
    }
    let hits = dsave::find_items(save, arg, None);
    match hits.len() {
        0 => Err(format!("no item matching def {arg:?}").into()),
        1 => Ok(hits[0]),
        n => Err(format!(
            "def {arg:?} matches {n} items ({}); pass an index",
            hits.iter()
                .take(8)
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into()),
    }
}

/// Apply one `--flag` op of `save-possess`; returns the report text and the
/// number of argv entries consumed.
fn possess_step(
    save: &mut dsave::Save,
    index: usize,
    ops: &[String],
    catalog: Option<&dsave::AttributeCatalog>,
) -> ds2_modkit::Result<(String, usize)> {
    let flag = ops.first().map(String::as_str).unwrap_or("");
    let need = |n: usize| -> ds2_modkit::Result<()> {
        if ops.len() < n {
            Err(format!("{flag} expects {} argument(s)", n - 1).into())
        } else {
            Ok(())
        }
    };
    let item = item_mut_at(save, index)?;
    match flag {
        "--level" => {
            need(2)?;
            let n: u32 = ops[1]
                .parse()
                .map_err(|_| format!("bad level {:?}", ops[1]))?;
            Ok((dsave::possessed_set_level(item, n)?, 2))
        }
        "--max" => Ok((dsave::possessed_max(item)?, 1)),
        "--add-mod" => {
            need(3)?;
            let attr = ops[1].clone();
            let value: i64 = ops[2]
                .parse()
                .map_err(|_| format!("bad value {:?}", ops[2]))?;
            let stat = dsave::find_stat(item, &attr)
                .or_else(|| first_stat(item))
                .ok_or("item has no ItemModifier to add to")?;
            Ok((
                dsave::possessed_add_mod(item, catalog, stat, &attr, Some(value))?,
                3,
            ))
        }
        "--del-mod" => {
            need(2)?;
            let (stat, md) = dsave::find_mod(item, &ops[1])
                .ok_or_else(|| format!("no modifier named {:?}", ops[1]))?;
            Ok((dsave::possessed_remove_mod(item, stat, md)?, 2))
        }
        "--add-effect" => {
            let stat = first_stat(item).ok_or("item has no ItemModifier to add an effect to")?;
            Ok((dsave::possessed_add_effect(item, stat)?, 1))
        }
        "--del-effect" => {
            need(3)?;
            let stat: usize = ops[1]
                .parse()
                .map_err(|_| format!("bad stat {:?}", ops[1]))?;
            let effect: usize = ops[2]
                .parse()
                .map_err(|_| format!("bad effect {:?}", ops[2]))?;
            Ok((dsave::possessed_remove_effect(item, stat, effect)?, 3))
        }
        other => Err(format!("unknown save-possess op {other:?}").into()),
    }
}

fn item_mut_at(save: &mut dsave::Save, index: usize) -> ds2_modkit::Result<&mut dsave::Item> {
    save.item_mut(index)
        .ok_or_else(|| format!("no item at index {index}").into())
}

fn first_stat(item: &dsave::Item) -> Option<usize> {
    let s = item.save_body()?;
    s.nested
        .iter()
        .position(|n| matches!(n.body, dsave::ItemBody::Modifier(_)))
}

fn load_save(path: &str) -> ds2_modkit::Result<dsave::Save> {
    let data = std::fs::read(path)?;
    dsave::parse(&data)
}

fn load_db(media: &Path) -> ds2_modkit::Result<(ItemDb, obsp::Obsp, Vec<u8>)> {
    let data = std::fs::read(media.join("scripts.obsp")).map_err(|e| {
        format!(
            "cannot read {}: {e} (set DS2_MEDIA to the game media folder)",
            media.join("scripts.obsp").display()
        )
    })?;
    let obsp = obsp::parse(&data)?;
    let db = ItemDb::build(&obsp, &data);
    Ok((db, obsp, data))
}

/// The item DB is optional for move/equip (container validation); a missing
/// `scripts.obsp` only disables the def check.
fn try_load_db(media: &Path) -> Option<(ItemDb, obsp::Obsp, Vec<u8>)> {
    let data = std::fs::read(media.join("scripts.obsp")).ok()?;
    let obsp = obsp::parse(&data).ok()?;
    let db = ItemDb::build(&obsp, &data);
    Some((db, obsp, data))
}

/// The loc table is optional too: without it display names fall back to the
/// prettified def name.
fn try_load_loc(media: &Path) -> Option<LocTable> {
    LocTable::load_from_media(media).ok()
}

fn load_catalog(media: &Path, save: &dsave::Save) -> ds2_modkit::Result<dsave::AttributeCatalog> {
    let (_db, obsp, data) = load_db(media)?;
    Ok(dsave::AttributeCatalog::build(&obsp, &data, &[save]))
}

fn write_and_report(save: &dsave::Save, path: &str, what: &str) -> ds2_modkit::Result<()> {
    let report = dsave::write_file(save, Path::new(path))?;
    println!(
        "{what} ({} bytes written{}, {} diff line(s){})",
        report.bytes,
        report
            .backup
            .as_ref()
            .map(|b| format!(", backup {}", b.display()))
            .unwrap_or_default(),
        report.diff.len(),
        report
            .timestamped_backup
            .as_ref()
            .map(|b| format!(", timestamped {}", b.display()))
            .unwrap_or_default()
    );
    for line in report.diff.iter().take(10) {
        println!("  {line}");
    }
    if report.diff.len() > 10 {
        println!(
            "  ... ({} more diff lines; run save-diff for the full list)",
            report.diff.len() - 10
        );
    }
    Ok(())
}
