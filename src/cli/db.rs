//! `save-db` commands: item definition DB, attribute catalog, save coverage.

use std::collections::BTreeSet;
use std::path::Path;

use ds2_modkit::dsave::{self, db, LocTable};

pub(crate) fn run(args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let sub = args
        .get(1)
        .map(String::as_str)
        .ok_or(
            "usage: ds2-modkit save-db items [filter] | save-db attrs [filter] [save.dsav] | save-db coverage <save.dsav>",
        )?;
    match sub {
        "items" => items(args, media),
        "attrs" => attrs(args, media),
        "coverage" => coverage(args, media),
        other => Err(format!("unknown save-db command: {other}").into()),
    }
}

fn load_db(media: &Path) -> ds2_modkit::Result<(db::ItemDb, ds2_modkit::obsp::Obsp, Vec<u8>)> {
    let data = std::fs::read(media.join("scripts.obsp"))?;
    let obsp = ds2_modkit::obsp::parse(&data)?;
    let db = db::ItemDb::build(&obsp, &data);
    Ok((db, obsp, data))
}

fn items(args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let (db, _obsp, _data) = load_db(media)?;
    // A missing/modded ui_core only degrades names to the prettified fallback.
    let loc = LocTable::load_from_media(media).unwrap_or_default();
    let filter = args.get(2).map(|s| s.to_lowercase()).unwrap_or_default();
    let cov = db.coverage();
    println!(
        "{} defs ({} entries, {} parsed payloads, {} with container, {} with rarity, {} with pickup title)",
        cov.defs,
        cov.entries_scanned,
        cov.payloads_parsed,
        cov.with_container,
        cov.with_rarity,
        cov.with_pickup_title
    );
    let mut shown = 0usize;
    for def in db.defs() {
        let name = db::display_name(&def.name, None, &loc, &db);
        if !filter.is_empty()
            && !def.name.to_lowercase().contains(&filter)
            && !name.to_lowercase().contains(&filter)
        {
            continue;
        }
        println!(
            "{:<42} {:<14} {:<4} {}",
            def.name,
            def.container.as_deref().unwrap_or("-"),
            def.rarity
                .map(|r| r.to_string())
                .unwrap_or_else(|| "-".into()),
            name
        );
        shown += 1;
    }
    println!("{shown} shown (loc entries: {})", loc.len());
    Ok(())
}

fn attrs(args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let (_db, obsp, data) = load_db(media)?;
    // `save-db attrs [filter] [save.dsav]`: a save adds the observed enum ids
    // and the save-side typing evidence.
    let saves: Vec<dsave::Save> = match args.get(3) {
        Some(path) => vec![dsave::parse(&std::fs::read(path)?)?],
        None => Vec::new(),
    };
    let refs: Vec<&dsave::Save> = saves.iter().collect();
    let catalog = db::AttributeCatalog::build(&obsp, &data, &refs);
    let filter = args.get(2).map(|s| s.to_lowercase()).unwrap_or_default();
    println!(
        "{} attributes (AttributeNames + def modifiers + value-table columns)",
        catalog.len()
    );
    let mut shown = 0usize;
    for a in catalog.attrs() {
        if !filter.is_empty() && !a.name.to_lowercase().contains(&filter) {
            continue;
        }
        println!(
            "{:<22} {:<8} {:<7} enum={:<4} table={}",
            a.name,
            a.kind.as_str(),
            a.confidence.as_str(),
            a.enum_id
                .map(|e| e.to_string())
                .unwrap_or_else(|| "-".into()),
            a.table_backed
        );
        shown += 1;
    }
    println!("{shown} shown");
    Ok(())
}

fn coverage(args: &[String], media: &Path) -> ds2_modkit::Result<()> {
    let path = args
        .get(2)
        .ok_or("usage: ds2-modkit save-db coverage <save.dsav>")?;
    let save = dsave::parse(&std::fs::read(path)?)?;
    let (db, _obsp, _data) = load_db(media)?;
    let loc = LocTable::load_from_media(media)?;

    let mut total = 0usize;
    let mut found = 0usize;
    let mut named = 0usize;
    let mut distinct = BTreeSet::new();
    let mut missing = Vec::new();
    for r in save.items() {
        let Some(item) = save.item(r.index) else {
            continue;
        };
        total += 1;
        distinct.insert(item.name.clone());
        match db.get(&item.name) {
            Some(def) => {
                found += 1;
                if db::localized_name(&def.name, None, &loc, &db).is_some() {
                    named += 1;
                }
            }
            None => missing.push(item.name.clone()),
        }
    }
    let pct = |n: usize| {
        if total == 0 {
            0.0
        } else {
            100.0 * n as f64 / total as f64
        }
    };
    println!("{}: {} items ({} distinct)", path, total, distinct.len());
    println!("  in item DB:       {found} ({:.1}%)", pct(found));
    println!(
        "  localized names:  {named} ({:.1}% of all, {:.1}% of known)",
        pct(named),
        if found == 0 {
            0.0
        } else {
            100.0 * named as f64 / found as f64
        }
    );
    if !missing.is_empty() {
        missing.sort();
        missing.dedup();
        println!(
            "  not in DB:        {} distinct, e.g. {}",
            missing.len(),
            missing
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(())
}
