use std::sync::mpsc;

use eframe::egui;

use ds2_modkit::dsave;

use crate::app::{App, Tab};
use crate::save::{first_mod_stat, SaveEdit};

impl App {
    /// Execute one line from the control socket; returns the reply text.
    pub(crate) fn handle_command(&mut self, line: &str) -> String {
        let mut parts = line.split_whitespace();
        let cmd = parts.next().unwrap_or("").to_lowercase();
        let rest: Vec<String> = parts.map(|s| s.to_string()).collect();
        match cmd.as_str() {
            "ping" => "pong".into(),
            "quit" => std::process::exit(0),
            "tab" => {
                let tab = rest.first().map(|s| s.to_lowercase()).unwrap_or_default();
                self.tab = match tab.as_str() {
                    "map" => Tab::Map,
                    "assets" => Tab::Assets,
                    "status" => Tab::Status,
                    "save" => Tab::Save,
                    "mods" => Tab::Mods,
                    _ => return format!("err: unknown tab {tab}"),
                };
                format!("tab {tab}")
            }
            "saveload" => {
                if let Some(path) = rest.first() {
                    self.save_path = path.clone();
                }
                if self.save_path.is_empty() {
                    return "err: saveload <path.dsav>".into();
                }
                self.save_load();
                self.tab = Tab::Save;
                self.save_status.to_string()
            }
            "savelist" => {
                let Some(save) = &self.save_file else {
                    return "err: no save loaded".into();
                };
                let filter = rest.join(" ").to_lowercase();
                let sort_attr = self.save_sort_attr.clone();
                let sort_positions: std::collections::HashMap<usize, usize> =
                    match (&self.save_db, sort_attr.is_empty()) {
                        (Some(db), false) => dsave::sorted_items(save, &db.catalog, &sort_attr)
                            .iter()
                            .enumerate()
                            .map(|(p, idx)| (*idx, p))
                            .collect(),
                        _ => std::collections::HashMap::new(),
                    };
                let mut rows: Vec<(usize, String, Option<u32>, String)> = Vec::new();
                for r in save.items() {
                    let Some(item) = save.item(r.index) else {
                        continue;
                    };
                    let origin = save.origin(&r);
                    let rarity = self
                        .save_db
                        .as_ref()
                        .and_then(|db| db.item_db.get(&item.name))
                        .and_then(|d| d.rarity);
                    let label = match self.save_db.as_ref() {
                        Some(db) => dsave::display_name(&item.name, rarity, &db.loc, &db.item_db),
                        None => dsave::pretty_name(&item.name),
                    };
                    if !filter.is_empty()
                        && !item.name.to_lowercase().contains(&filter)
                        && !label.to_lowercase().contains(&filter)
                    {
                        continue;
                    }
                    rows.push((r.index, label, rarity, origin));
                }
                if !sort_attr.is_empty() {
                    rows.sort_by_key(|(idx, ..)| {
                        sort_positions.get(idx).copied().unwrap_or(usize::MAX)
                    });
                }
                rows.iter()
                    .map(|(idx, label, rarity, origin)| {
                        let tag = rarity.map(|r| format!(" [R{r}]")).unwrap_or_default();
                        format!("[{idx}] {label}{tag} ({origin})")
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            "saveselect" => {
                let Some(idx) = rest.first().and_then(|s| s.parse::<usize>().ok()) else {
                    return "err: saveselect <index>".into();
                };
                self.save_selected = Some(idx);
                format!("selected {idx}")
            }
            "saveshow" => {
                let Some(save) = &self.save_file else {
                    return "err: no save loaded".into();
                };
                let idx = rest
                    .first()
                    .and_then(|s| s.parse::<usize>().ok())
                    .or(self.save_selected);
                let Some(idx) = idx else {
                    return "err: saveshow <index>".into();
                };
                match save.item(idx) {
                    Some(item) => dsave::item_lines(item, 0).join("\n"),
                    None => format!("err: no item at index {idx}"),
                }
            }
            "saveset" => {
                let Some(save) = &mut self.save_file else {
                    return "err: no save loaded".into();
                };
                let (Some(idx), Some(field), Some(value)) =
                    (rest.first(), rest.get(1), rest.get(2))
                else {
                    return "err: saveset <index> <field> <value>".into();
                };
                let Ok(idx) = idx.parse::<usize>() else {
                    return "err: saveset <index> <field> <value>".into();
                };
                match save.item_mut(idx) {
                    Some(item) => match dsave::set_item_field(item, field, value) {
                        Ok(change) => format!("item {idx}: {change}"),
                        Err(e) => format!("err: {e}"),
                    },
                    None => format!("err: no item at index {idx}"),
                }
            }
            "saveaddclone" => {
                let Some(idx) = rest.first().and_then(|s| s.parse::<usize>().ok()) else {
                    return "err: saveaddclone <index>".into();
                };
                self.save_add_clone(idx);
                self.save_status.clone()
            }
            "saveremove" => {
                let Some(idx) = rest.first().and_then(|s| s.parse::<usize>().ok()) else {
                    return "err: saveremove <index>".into();
                };
                self.save_remove_item(idx);
                self.save_status.clone()
            }
            "savewrite" => {
                self.save_write(true);
                self.save_status.clone()
            }
            "savediff" => {
                let diff = self.save_diff_lines();
                if diff.is_empty() {
                    "no changes vs the loaded file".into()
                } else {
                    format!("{} change(s):\n{}", diff.len(), diff.join("\n"))
                }
            }
            "savedb" => match &self.save_db {
                Some(db) => format!(
                    "item db ready: {} defs, {} attrs, {} loc entries",
                    db.item_db.len(),
                    db.catalog.len(),
                    db.loc.len()
                ),
                None => "item db loading".into(),
            },
            "saveslots" => {
                let arg = rest.first().cloned();
                self.save_refresh_slots_with(arg.as_deref());
                if self.save_slots.is_empty() {
                    "no dssave*.dsav slots found".into()
                } else {
                    self.save_slots
                        .iter()
                        .map(|s| {
                            let name = s
                                .path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            let mtime = s
                                .modified
                                .map(dsave::format_time)
                                .unwrap_or_else(|| "-".to_string());
                            let mut line = format!(
                                "{name} {} B {mtime} counter={} global_version={:#010X} flags={:#x} items={} equipped={}",
                                s.bytes, s.counter, s.global_version, s.flags, s.items, s.equipped
                            );
                            for rec in &s.locations {
                                line += &format!(
                                    "\n  last saved: {} | {} | {} | {} | {}",
                                    rec[0], rec[1], rec[2], rec[3], rec[4]
                                );
                            }
                            line
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
            "savetotals" => {
                let (Some(save), Some(db)) = (&self.save_file, &self.save_db) else {
                    return "err: save or item db not loaded".into();
                };
                let arg = rest.first().cloned().unwrap_or_default();
                let scope = match arg.as_str() {
                    "" | "equipped" => dsave::TotalScope::Equipped,
                    "all" => dsave::TotalScope::All,
                    c => dsave::TotalScope::Container(c),
                };
                let totals = dsave::totals(save, &db.catalog, scope);
                if totals.is_empty() {
                    "no static stat values in scope".into()
                } else {
                    totals
                        .iter()
                        .map(|t| dsave::total_line(&db.label(&t.attribute), t))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
            "savesort" => {
                let attr = rest.first().cloned().unwrap_or_default();
                self.save_sort_attr = if attr == "none" {
                    String::new()
                } else {
                    attr.clone()
                };
                format!("sort {attr}")
            }
            "savebulk" => {
                let op = match dsave::parse_bulk_op(&rest) {
                    Ok(op) => op,
                    Err(e) => return format!("err: {e}"),
                };
                self.save_apply(SaveEdit::Bulk(Box::new(op)));
                self.save_status.clone()
            }
            "saveapply" => {
                let Some(path) = rest.first() else {
                    return "err: saveapply <preset.toml>".into();
                };
                self.save_apply(SaveEdit::Preset(path.clone()));
                self.save_status.clone()
            }
            "savecopyslot" => {
                let (Some(dst), Some(idx)) = (
                    rest.first(),
                    rest.get(1).and_then(|s| s.parse::<usize>().ok()),
                ) else {
                    return "err: savecopyslot <dst.dsav> <item-index>".into();
                };
                self.save_apply(SaveEdit::CopyToSlot(dst.clone(), idx));
                self.save_status.clone()
            }
            "savestats" => {
                let (Some(save), Some(db)) = (&self.save_file, &self.save_db) else {
                    return "err: save or item db not loaded".into();
                };
                let idx = rest
                    .first()
                    .and_then(|s| s.parse::<usize>().ok())
                    .or(self.save_selected);
                let Some(idx) = idx else {
                    return "err: savestats <index>".into();
                };
                let Some(item) = save.item(idx) else {
                    return format!("err: no item at index {idx}");
                };
                let views = dsave::stat_views(item, &db.catalog);
                views
                    .iter()
                    .map(|v| {
                        format!(
                            "{} enum={} value={} table={} row={} path={}",
                            db.label(&v.attribute),
                            v.enum_id,
                            v.value,
                            v.table.as_deref().unwrap_or("-"),
                            v.row.map(|r| r.to_string()).unwrap_or_else(|| "-".into()),
                            v.value_path
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            "savestat" | "saverow" => {
                let (Some(idx), Some(attr), Some(value)) = (rest.first(), rest.get(1), rest.get(2))
                else {
                    return "err: savestat <index> <attr|enum> <value>".into();
                };
                let Ok(idx) = idx.parse::<usize>() else {
                    return "err: savestat <index> <attr|enum> <value>".into();
                };
                if cmd == "savestat" {
                    let Ok(value) = value.parse::<i64>() else {
                        return "err: savestat <index> <attr|enum> <value>".into();
                    };
                    self.save_apply(SaveEdit::SetStat(idx, attr.clone(), value));
                } else {
                    let Ok(row) = value.parse::<u32>() else {
                        return "err: saverow <index> <attr|enum> <row>".into();
                    };
                    self.save_apply(SaveEdit::SetRow(idx, attr.clone(), row));
                }
                self.save_status.clone()
            }
            "saveadditem" => {
                let (Some(container), Some(def)) = (rest.first(), rest.get(1)) else {
                    return "err: saveadditem <container> <def-name>".into();
                };
                self.save_apply(SaveEdit::AddDef(def.clone(), container.clone()));
                self.save_status.clone()
            }
            "saveitems" => {
                let Some(db) = &self.save_db else {
                    return "err: item db not loaded".into();
                };
                let filter = rest.first().map(|s| s.to_lowercase()).unwrap_or_default();
                let limit: usize = rest.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
                let mut out = Vec::new();
                for def in db.item_db.defs() {
                    let name = dsave::display_name(&def.name, None, &db.loc, &db.item_db);
                    if !filter.is_empty()
                        && !def.name.to_lowercase().contains(&filter)
                        && !name.to_lowercase().contains(&filter)
                    {
                        continue;
                    }
                    out.push(format!(
                        "{:<42} {:<14} {:<3} {}",
                        def.name,
                        def.container.as_deref().unwrap_or("-"),
                        def.rarity
                            .map(|r| r.to_string())
                            .unwrap_or_else(|| "-".into()),
                        name
                    ));
                    if out.len() >= limit {
                        break;
                    }
                }
                if out.is_empty() {
                    "none".into()
                } else {
                    out.join("\n")
                }
            }
            "savepossess" => {
                let Some(idx) = rest.first().and_then(|s| s.parse::<usize>().ok()) else {
                    return "err: savepossess <index> [--level N|--max|--add-mod <attr> <value>|--del-mod <attr>|--add-effect|--del-effect <stat> <effect>]".into();
                };
                let mut edits: Vec<SaveEdit> = Vec::new();
                let mut i = 1usize;
                while i < rest.len() {
                    match rest[i].as_str() {
                        "--level" => {
                            let Some(n) = rest.get(i + 1).and_then(|s| s.parse::<u32>().ok())
                            else {
                                return "err: --level <n>".into();
                            };
                            edits.push(SaveEdit::SetLevel(idx, n));
                            i += 2;
                        }
                        "--max" => {
                            edits.push(SaveEdit::MaxPossess(idx));
                            i += 1;
                        }
                        "--add-mod" => {
                            let (Some(attr), Some(value)) = (
                                rest.get(i + 1),
                                rest.get(i + 2).and_then(|s| s.parse::<i64>().ok()),
                            ) else {
                                return "err: --add-mod <attr> <value>".into();
                            };
                            let stat = self
                                .save_file
                                .as_ref()
                                .and_then(|s| s.item(idx))
                                .and_then(|it| {
                                    dsave::find_stat(it, attr).or_else(|| first_mod_stat(it))
                                })
                                .unwrap_or(0);
                            edits.push(SaveEdit::AddMod(idx, stat, attr.clone(), value));
                            i += 3;
                        }
                        "--del-mod" => {
                            let Some(attr) = rest.get(i + 1) else {
                                return "err: --del-mod <attr>".into();
                            };
                            let found = self
                                .save_file
                                .as_ref()
                                .and_then(|s| s.item(idx))
                                .and_then(|it| dsave::find_mod(it, attr));
                            match found {
                                Some((stat, md)) => edits.push(SaveEdit::DelMod(idx, stat, md)),
                                None => return format!("err: no modifier named {attr:?}"),
                            }
                            i += 2;
                        }
                        "--add-effect" => {
                            edits.push(SaveEdit::AddEffect(idx, 0));
                            i += 1;
                        }
                        "--del-effect" => {
                            let (Some(stat), Some(effect)) = (
                                rest.get(i + 1).and_then(|s| s.parse::<usize>().ok()),
                                rest.get(i + 2).and_then(|s| s.parse::<usize>().ok()),
                            ) else {
                                return "err: --del-effect <stat> <effect>".into();
                            };
                            edits.push(SaveEdit::DelEffect(idx, stat, effect));
                            i += 3;
                        }
                        other => return format!("err: unknown savepossess op {other:?}"),
                    }
                }
                let mut out = Vec::new();
                for edit in edits {
                    self.save_apply(edit);
                    out.push(self.save_status.clone());
                }
                if out.is_empty() {
                    let info = self
                        .save_file
                        .as_ref()
                        .and_then(|s| s.item(idx))
                        .and_then(dsave::possessed_info);
                    match info {
                        Some(info) => format!(
                            "possessed xp={} level={} cap={} fed={}",
                            info.cur_xp,
                            info.level,
                            info.level_cap,
                            info.fed.len()
                        ),
                        None => format!("item {idx} is not a possessed item"),
                    }
                } else {
                    out.join("\n")
                }
            }
            "savemove" => {
                let (Some(idx), Some(target)) = (
                    rest.first().and_then(|s| s.parse::<usize>().ok()),
                    rest.get(1),
                ) else {
                    return "err: savemove <index> <container>".into();
                };
                self.save_apply(SaveEdit::Move(idx, target.clone()));
                self.save_status.clone()
            }
            "saveequip" => {
                let Some(idx) = rest.first().and_then(|s| s.parse::<usize>().ok()) else {
                    return "err: saveequip <index>".into();
                };
                self.save_apply(SaveEdit::Equip(idx));
                self.save_status.clone()
            }
            "saveunequip" => {
                let Some(idx) = rest.first().and_then(|s| s.parse::<usize>().ok()) else {
                    return "err: saveunequip <item-index>".into();
                };
                self.save_apply(SaveEdit::Unequip(idx));
                self.save_status.clone()
            }
            "world" => {
                let want = rest.join(" ").to_lowercase();
                let Some(i) = self
                    .map_world_names
                    .iter()
                    .position(|n| n.to_lowercase() == want)
                    .or_else(|| {
                        self.map_world_names
                            .iter()
                            .position(|n| n.to_lowercase().contains(&want))
                    })
                else {
                    return format!("err: no world matching {want}");
                };
                self.map_world = Some(i);
                self.tab = Tab::Map;
                self.load_map_objects(i);
                self.select_first_object();
                let queued = self.queue_world_rooms();
                self.log(format!("map: {queued}"));
                format!(
                    "world {} objects {} classes {}",
                    self.map_world_names[i],
                    self.map_objects.len(),
                    self.map_classes.len()
                )
            }
            "classes" => self
                .map_classes
                .iter()
                .map(|(c, n)| {
                    format!(
                        "{c} {n} {}",
                        if self.class_enabled(c) { "on" } else { "off" }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            "class" => {
                let Some(name) = rest.first() else {
                    return "err: class <name> [on|off]".into();
                };
                let on = match rest.get(1).map(|s| s.as_str()) {
                    Some("on") => true,
                    Some("off") => false,
                    Some("toggle") | None => !self.class_enabled(name),
                    Some(other) => return format!("err: bad mode {other}"),
                };
                self.map_class_on.insert(name.clone(), on);
                format!("class {name} {}", if on { "on" } else { "off" })
            }
            "only" => {
                if rest.is_empty() {
                    return "err: only <class...>".into();
                }
                for (c, _) in &self.map_classes {
                    self.map_class_on.insert(c.clone(), false);
                }
                for want in &rest {
                    for (c, _) in &self.map_classes {
                        if c.to_lowercase().contains(&want.to_lowercase()) {
                            self.map_class_on.insert(c.clone(), true);
                        }
                    }
                }
                format!("only {}", rest.join(" "))
            }
            "all" => {
                self.map_class_on.clear();
                "all classes on".into()
            }
            "list" => {
                let filt = rest.first().map(|s| s.to_lowercase()).unwrap_or_default();
                let limit: usize = rest.get(1).and_then(|s| s.parse().ok()).unwrap_or(50);
                let mut out = Vec::new();
                for (i, o) in self.map_objects.iter().enumerate() {
                    if !filt.is_empty()
                        && !o.class.to_lowercase().contains(&filt)
                        && !o.name.to_lowercase().contains(&filt)
                    {
                        continue;
                    }
                    let p = self.object_position(i);
                    out.push(format!(
                        "{i} {} {} {:.1} {:.1} {:.1}",
                        o.class, o.name, p[0], p[1], p[2]
                    ));
                    if out.len() >= limit {
                        break;
                    }
                }
                if out.is_empty() {
                    "none".into()
                } else {
                    out.join("\n")
                }
            }
            "select" => {
                let Some(i) = rest.first().and_then(|s| s.parse::<usize>().ok()) else {
                    return "err: select <index>".into();
                };
                if i >= self.map_objects.len() {
                    return format!("err: index {} of {}", i, self.map_objects.len());
                }
                self.map_selected = Some(i);
                self.describe_selected()
            }
            "move" => {
                let vals: Vec<f32> = rest.iter().filter_map(|s| s.parse().ok()).collect();
                if vals.len() != 3 {
                    return "err: move <x> <y> <z>".into();
                }
                let Some(i) = self.map_selected else {
                    return "err: no selection".into();
                };
                let p = [vals[0], vals[1], vals[2]];
                self.set_position_edit(i, p);
                self.cam_target = p;
                format!(
                    "pending {} pos {:.1} {:.1} {:.1}",
                    self.map_edits.len(),
                    p[0],
                    p[1],
                    p[2]
                )
            }
            "moveby" => {
                let vals: Vec<f32> = rest.iter().filter_map(|s| s.parse().ok()).collect();
                if vals.len() != 3 {
                    return "err: moveby <dx> <dy> <dz>".into();
                }
                let Some(i) = self.map_selected else {
                    return "err: no selection".into();
                };
                let cur = self.object_position(i);
                let p = [cur[0] + vals[0], cur[1] + vals[1], cur[2] + vals[2]];
                self.set_position_edit(i, p);
                self.cam_target = p;
                format!(
                    "pending {} pos {:.1} {:.1} {:.1}",
                    self.map_edits.len(),
                    p[0],
                    p[1],
                    p[2]
                )
            }
            "rotate" => {
                let vals: Vec<f32> = rest.iter().filter_map(|s| s.parse().ok()).collect();
                if vals.len() != 3 {
                    return "err: rotate <x> <y> <z>".into();
                }
                let Some(i) = self.map_selected else {
                    return "err: no selection".into();
                };
                let rot = [vals[0], vals[1], vals[2]];
                self.set_rotation_edit(i, rot);
                format!("rotation set {:.2} {:.2} {:.2}", rot[0], rot[1], rot[2])
            }
            "save" => self.save_map(),
            "spawn" => {
                let vals: Vec<f32> = rest.iter().filter_map(|s| s.parse().ok()).collect();
                let pos = if vals.len() == 3 {
                    [vals[0], vals[1], vals[2]]
                } else {
                    self.cam_target
                };
                self.map_struct_edit(Some(pos))
            }
            "delete" => self.map_struct_edit(None),
            "meshload" => {
                let want = rest.join(" ");
                if want.is_empty() {
                    return "err: meshload <filter>".into();
                }
                self.map_mesh_filter = want.clone();
                self.find_mesh_assets();
                let Some(name) = self.map_mesh_matches.first().cloned() else {
                    return format!("err: no mesh asset matching {want}");
                };
                self.map_mesh_matches.clear();
                self.load_room_mesh(&name)
            }
            "meshauto" => self.load_nearest_room(),
            "meshprops" => {
                let msg = self.load_props();
                self.log(format!("map: {msg}"));
                msg
            }
            "meshmats" => {
                let msg = self.load_materials();
                self.log(format!("map: {msg}"));
                msg
            }
            "meshworld" => {
                let msg = self.queue_world_rooms();
                if msg.starts_with("err") {
                    msg
                } else {
                    self.log(format!("map: {msg}"));
                    let loaded = self.flush_room_queue();
                    self.log(format!("map: {loaded}"));
                    let props = self.load_props();
                    self.log(format!("map: {props}"));
                    format!("{loaded}; {props}")
                }
            }
            "meshclear" => {
                let nrooms = self.map_rooms.len();
                let n: usize = self.map_rooms.iter().map(|r| r.tris.len()).sum();
                self.clear_rooms();
                format!("mesh cleared ({nrooms} rooms, {n} tris)")
            }
            "meshtoggle" => {
                self.map_show_meshes = !self.map_show_meshes;
                format!("meshes {}", if self.map_show_meshes { "on" } else { "off" })
            }
            "meshinfo" => {
                let tris: usize = self.map_rooms.iter().map(|r| r.tris.len()).sum();
                let prop_tris: usize = self.map_props.iter().map(|p| p.tris.len()).sum();
                let pending = self.map_room_loading;
                format!(
                    "mesh rooms {} tris {tris} props {} ptris {prop_tris} pending {pending} shown {} tex {:?} diffuses {}",
                    self.map_rooms.len(),
                    self.map_props.len(),
                    self.map_show_meshes,
                    self.map_tex_override
                        .as_ref()
                        .map(|(p, t)| format!("{p} {}x{}", t.w, t.h)),
                    self.map_tex_status
                )
            }
            "meshtex" => {
                let arg = rest.join(" ");
                if arg.is_empty() {
                    return self
                        .map_tex_override
                        .as_ref()
                        .map(|(p, t)| format!("{p} {}x{}", t.w, t.h))
                        .unwrap_or_else(|| "none".into());
                }
                if arg == "none" || arg == "clear" {
                    self.map_tex_override = None;
                    self.map_room_cam = None;
                    "texture off".into()
                } else {
                    self.load_room_texture(&arg)
                }
            }
            "fit" => {
                self.fit_view();
                format!(
                    "fit target {:.0} {:.0} {:.0} dist {:.0}",
                    self.cam_target[0], self.cam_target[1], self.cam_target[2], self.cam_dist
                )
            }
            "flip" => {
                self.cam_yaw += std::f32::consts::PI;
                self.map_room_cam = None;
                format!("flip view yaw {:.2}", self.cam_yaw)
            }
            "camera" => {
                let v: Vec<f32> = rest.iter().filter_map(|s| s.parse().ok()).collect();
                if v.len() == 3 {
                    self.cam_yaw = v[0];
                    self.cam_pitch = v[1];
                    self.cam_dist = v[2];
                }
                let eye = self.camera_eye();
                format!(
                    "camera yaw {:.2} pitch {:.2} dist {:.0} eye ({:.0}, {:.0}, {:.0}) target ({:.0}, {:.0}, {:.0})",
                    self.cam_yaw, self.cam_pitch, self.cam_dist,
                    eye[0], eye[1], eye[2],
                    self.cam_target[0], self.cam_target[1], self.cam_target[2]
                )
            }
            "target" => {
                let v: Vec<f32> = rest.iter().filter_map(|s| s.parse().ok()).collect();
                if v.len() == 3 {
                    self.cam_target = [v[0], v[1], v[2]];
                }
                format!(
                    "target {:.0} {:.0} {:.0}",
                    self.cam_target[0], self.cam_target[1], self.cam_target[2]
                )
            }
            "selected" => self.describe_selected(),
            "props" => {
                if self.selected_props.is_empty() {
                    "none".into()
                } else {
                    self.selected_props
                        .iter()
                        .map(|(k, v)| format!("{k} = {v}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
            "revert" => match self.map_selected {
                Some(i) => {
                    self.map_edits.remove(&i);
                    format!("reverted {i}")
                }
                None => "err: no selection".into(),
            },
            "revertall" => {
                let n = self.map_edits.len();
                self.map_edits.clear();
                format!("reverted {n} edits")
            }
            "shot" => {
                let Some(path) = rest.first() else {
                    return "err: shot <path>".into();
                };
                self.shot_path = Some(path.clone());
                if let Some(ctx) = &self.ctx {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
                format!("shot {path}")
            }
            "info" => format!(
                "tab {:?} world {:?} objects {} classes {} enabled {} pending {} log {}",
                self.tab,
                self.map_world.map(|i| self.map_world_names[i].clone()),
                self.map_objects.len(),
                self.map_classes.len(),
                self.map_classes
                    .iter()
                    .filter(|(c, _)| self.class_enabled(c))
                    .count(),
                self.map_edits.len(),
                self.log.len()
            ),
            "log" => self
                .log
                .iter()
                .rev()
                .take(30)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"),
            other => format!("err: unknown command {other}"),
        }
    }
}

pub(crate) fn control_server(tx: mpsc::Sender<(String, mpsc::Sender<String>)>) {
    use std::io::{BufRead, BufReader, Write};
    let port: u16 = std::env::var("DS2_GUI_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(47777);
    let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("control server: bind 127.0.0.1:{port} failed: {e}");
            return;
        }
    };
    eprintln!("control server on 127.0.0.1:{port}");
    for stream in listener.incoming() {
        let Ok(mut s) = stream else { continue };
        let tx = tx.clone();
        std::thread::spawn(move || {
            let Ok(clone) = s.try_clone() else { return };
            let mut reader = BufReader::new(clone);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let cmd = line.trim().to_string();
                if cmd.is_empty() {
                    continue;
                }
                let (rtx, rrx) = mpsc::channel();
                if tx.send((cmd, rtx)).is_err() {
                    break;
                }
                let resp = rrx.recv().unwrap_or_else(|_| "err: app closed".into());
                if s.write_all(resp.as_bytes()).is_err() || s.write_all(b"\n.\n").is_err() {
                    break;
                }
            }
        });
    }
}
