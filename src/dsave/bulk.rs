//! Bulk operations across a save's items.
//!
//! Every operation is conservative: items without the attribute are skipped
//! (counted, not an error) and a [`BulkReport`] says how many items changed.
//! `set-stat`/`scale` write the static `a[2]` value; table-backed stats (see
//! [`super::stats`]) keep a `none` value on real weapons and are therefore
//! skipped unless they already carry a static value.
//!
//! [`parse_bulk_op`] parses the CLI/GUI argument surface
//! (`set-stat <attr> <value> [--container X|--equipped|--possessed|--all]`,
//! `scale <attr> <percent> [scope]`, `clear <container>`,
//! `duplicate <item-index>`, `set-level <level> [--possessed|scope]`).

use super::db::AttributeCatalog;
use super::items::{index_of, ItemRef, ItemSource};
use super::model::{Item, Save};
use super::possessed::{possessed_info, possessed_set_level};
use super::stats::{check_value, set_stat, stat_views, VALUE_MAX, VALUE_NONE};
use crate::Result;

/// Which items a bulk op touches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkScope {
    All,
    Equipped,
    Possessed,
    Container(String),
}

impl BulkScope {
    /// Parse `equipped` / `possessed` / `all` / `<container name>`.
    pub fn parse(s: &str) -> BulkScope {
        match s.to_lowercase().as_str() {
            "" | "all" => BulkScope::All,
            "equipped" => BulkScope::Equipped,
            "possessed" => BulkScope::Possessed,
            _ => BulkScope::Container(s.to_string()),
        }
    }

    /// The inverse of [`BulkScope::parse`] for round-tripping.
    pub fn as_filter(&self) -> String {
        match self {
            BulkScope::All => "all".into(),
            BulkScope::Equipped => "equipped".into(),
            BulkScope::Possessed => "possessed".into(),
            BulkScope::Container(c) => c.clone(),
        }
    }
}

/// One bulk operation.
#[derive(Debug, Clone, PartialEq)]
pub enum BulkOp {
    /// Set the static `a[2]` value on every item in scope that has the stat.
    SetStat {
        attribute: String,
        value: i64,
        scope: BulkScope,
    },
    /// Multiply the static `a[2]` value by `percent / 100`.
    Scale {
        attribute: String,
        percent: i64,
        scope: BulkScope,
    },
    /// Remove every item from the inventory list `container`.
    Clear { container: String },
    /// Clone `item_index` after itself (the copy's `b816` flag is cleared).
    Duplicate { item_index: usize },
    /// Set `f872` (item level) on every item in scope, or — with `possessed` —
    /// the `PossessedItemComponent` level of the possessed items in scope.
    SetLevel {
        level: u32,
        possessed: bool,
        scope: BulkScope,
    },
}

impl BulkOp {
    pub fn name(&self) -> &'static str {
        match self {
            BulkOp::SetStat { .. } => "set-stat",
            BulkOp::Scale { .. } => "scale",
            BulkOp::Clear { .. } => "clear",
            BulkOp::Duplicate { .. } => "duplicate",
            BulkOp::SetLevel { .. } => "set-level",
        }
    }

    /// True when applying the op needs the attribute catalog.
    pub fn needs_catalog(&self) -> bool {
        matches!(self, BulkOp::SetStat { .. } | BulkOp::Scale { .. })
    }
}

/// What one [`apply_bulk`] did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct BulkReport {
    /// Items actually changed.
    pub changed: usize,
    /// Items skipped (no such stat, or the operation did not apply).
    pub skipped: usize,
    /// Per-change report lines.
    pub lines: Vec<String>,
}

/// Parse the argument surface shared by `save-bulk` and the GUI's `savebulk`.
pub fn parse_bulk_op(args: &[String]) -> Result<BulkOp> {
    let usage = "usage: <set-stat <attr|enum> <value>|scale <attr|enum> <percent>|\
                 clear <container>|duplicate <item-index>|set-level <level>> \
                 [--container X|--equipped|--possessed|--all]";
    let op = args.first().map(String::as_str).unwrap_or("");
    match op {
        "set-stat" => {
            let attribute = args.get(1).ok_or(format!("no attribute; {usage}"))?.clone();
            let value: i64 = args
                .get(2)
                .ok_or(format!("no value; {usage}"))?
                .parse()
                .map_err(|_| "bad set-stat value")?;
            check_value(value)?;
            let scope = scope_from(&args[3..])?;
            Ok(BulkOp::SetStat {
                attribute,
                value,
                scope,
            })
        }
        "scale" => {
            let attribute = args.get(1).ok_or(format!("no attribute; {usage}"))?.clone();
            let percent: i64 = args
                .get(2)
                .ok_or(format!("no percent; {usage}"))?
                .parse()
                .map_err(|_| "bad scale percent")?;
            let scope = scope_from(&args[3..])?;
            Ok(BulkOp::Scale {
                attribute,
                percent,
                scope,
            })
        }
        "clear" => {
            let container = args.get(1).ok_or(format!("no container; {usage}"))?.clone();
            Ok(BulkOp::Clear { container })
        }
        "duplicate" => {
            let item_index: usize = args
                .get(1)
                .ok_or(format!("no item index; {usage}"))?
                .parse()
                .map_err(|_| "bad item index")?;
            Ok(BulkOp::Duplicate { item_index })
        }
        "set-level" => {
            let level: u32 = args
                .get(1)
                .ok_or(format!("no level; {usage}"))?
                .parse()
                .map_err(|_| "bad level")?;
            let mut possessed = false;
            let mut scope_args: Vec<String> = Vec::new();
            for a in &args[2..] {
                if a == "--possessed" {
                    possessed = true;
                } else {
                    scope_args.push(a.clone());
                }
            }
            let scope = if scope_args.is_empty() && possessed {
                BulkScope::Possessed
            } else {
                scope_from(&scope_args)?
            };
            Ok(BulkOp::SetLevel {
                level,
                possessed,
                scope,
            })
        }
        other => Err(format!("unknown bulk op {other:?}; {usage}").into()),
    }
}

fn scope_from(args: &[String]) -> Result<BulkScope> {
    let mut scope = BulkScope::All;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--equipped" => scope = BulkScope::Equipped,
            "--possessed" => scope = BulkScope::Possessed,
            "--all" => scope = BulkScope::All,
            "--container" => {
                let name = args
                    .get(i + 1)
                    .ok_or("--container needs a list name")?
                    .clone();
                scope = BulkScope::Container(name);
                i += 1;
            }
            other => return Err(format!("unknown bulk option {other:?}").into()),
        }
        i += 1;
    }
    Ok(scope)
}

fn in_scope(save: &Save, r: &ItemRef, scope: &BulkScope) -> bool {
    match scope {
        BulkScope::All => true,
        BulkScope::Equipped => save
            .item(r.index)
            .and_then(|i| i.save_body())
            .is_some_and(|s| s.b816 == 1),
        BulkScope::Possessed => save.item(r.index).and_then(possessed_info).is_some(),
        BulkScope::Container(name) => {
            r.source == ItemSource::Inventory
                && save.module.inventory[r.list_idx]
                    .name
                    .eq_ignore_ascii_case(name)
        }
    }
}

/// Apply one bulk op. Returns per-change lines and the changed/skipped counts.
pub fn apply_bulk(
    save: &mut Save,
    catalog: Option<&AttributeCatalog>,
    op: &BulkOp,
) -> Result<BulkReport> {
    match op {
        BulkOp::SetStat {
            attribute,
            value,
            scope,
        } => bulk_set_stat(save, catalog, attribute, *value, scope),
        BulkOp::Scale {
            attribute,
            percent,
            scope,
        } => bulk_scale(save, catalog, attribute, *percent, scope),
        BulkOp::Clear { container } => bulk_clear(save, container),
        BulkOp::Duplicate { item_index } => bulk_duplicate(save, *item_index),
        BulkOp::SetLevel {
            level,
            possessed,
            scope,
        } => bulk_set_level(save, *level, *possessed, scope),
    }
}

/// Resolve a name or decimal enum id against the catalog once, so a typo is a
/// hard error instead of "0 items changed, 400 skipped".
fn check_attribute(catalog: &AttributeCatalog, attribute: &str) -> Result<()> {
    if let Ok(id) = attribute.trim().parse::<u32>() {
        if catalog.attrs().iter().any(|a| a.enum_id == Some(id)) {
            return Ok(());
        }
        return Err(format!("no catalog attribute with enum id {id}").into());
    }
    if catalog.get(attribute).is_some() {
        return Ok(());
    }
    Err(format!("unknown attribute {attribute:?} (not in the catalog)").into())
}

fn view_value(item: &Item, catalog: &AttributeCatalog, attribute: &str) -> Option<i64> {
    let want_id: Option<u32> = attribute.trim().parse().ok();
    stat_views(item, catalog)
        .into_iter()
        .find(|v| v.attribute.eq_ignore_ascii_case(attribute) || want_id == Some(v.enum_id))
        .map(|v| v.value)
}

fn bulk_set_stat(
    save: &mut Save,
    catalog: Option<&AttributeCatalog>,
    attribute: &str,
    value: i64,
    scope: &BulkScope,
) -> Result<BulkReport> {
    let catalog =
        catalog.ok_or("setting stats needs the attribute catalog (scripts.obsp is missing)")?;
    check_attribute(catalog, attribute)?;
    let mut report = BulkReport::default();
    for r in save.items() {
        if !in_scope(save, &r, scope) {
            continue;
        }
        let Some(item) = save.item(r.index) else {
            continue;
        };
        let Some(old) = view_value(item, catalog, attribute) else {
            report.skipped += 1;
            continue;
        };
        let Some(item) = save.item_mut(r.index) else {
            continue;
        };
        match set_stat(item, catalog, attribute, value) {
            Ok(_) => {
                report.changed += 1;
                report
                    .lines
                    .push(format!("item {} {attribute}: {old} -> {value}", r.index));
            }
            Err(_) => report.skipped += 1,
        }
    }
    Ok(report)
}

fn bulk_scale(
    save: &mut Save,
    catalog: Option<&AttributeCatalog>,
    attribute: &str,
    percent: i64,
    scope: &BulkScope,
) -> Result<BulkReport> {
    let empty = AttributeCatalog::default();
    let catalog = catalog.unwrap_or(&empty);
    if catalog.get(attribute).is_none() && attribute.trim().parse::<u32>().is_err() {
        return Err(format!("unknown attribute {attribute:?} (not in the catalog)").into());
    }
    let mut report = BulkReport::default();
    let mut table_backed = 0usize;
    let mut out_of_range = 0usize;
    for r in save.items() {
        if !in_scope(save, &r, scope) {
            continue;
        }
        let Some(item) = save.item(r.index) else {
            continue;
        };
        let want_id: Option<u32> = attribute.trim().parse().ok();
        let Some(view) = stat_views(item, catalog)
            .into_iter()
            .find(|v| v.attribute.eq_ignore_ascii_case(attribute) || want_id == Some(v.enum_id))
        else {
            report.skipped += 1;
            continue;
        };
        if view.value == VALUE_NONE {
            report.skipped += 1;
            continue;
        }
        if view.is_table_backed() {
            table_backed += 1;
        }
        let scaled = i128::from(view.value) * i128::from(percent) / 100;
        if scaled < i128::from(VALUE_NONE) || scaled > i128::from(VALUE_MAX) {
            out_of_range += 1;
            report.skipped += 1;
            continue;
        }
        let new = scaled as i64;
        let old = view.value;
        let attr = view.attribute.clone();
        let Some(item) = save.item_mut(r.index) else {
            continue;
        };
        match set_stat(item, catalog, &attr, new) {
            Ok(_) => {
                report.changed += 1;
                report
                    .lines
                    .push(format!("item {} {attr}: {old} -> {new}", r.index));
            }
            Err(_) => report.skipped += 1,
        }
    }
    if table_backed > 0 {
        report.lines.push(format!(
            "note: {table_backed} changed stat(s) are table-backed; the game reads the value table (row/offset fields[0]), not a[2]"
        ));
    }
    if out_of_range > 0 {
        report.lines.push(format!(
            "note: {out_of_range} stat(s) skipped: scaled value out of range"
        ));
    }
    Ok(report)
}

fn bulk_clear(save: &mut Save, container: &str) -> Result<BulkReport> {
    let li = save
        .module
        .inventory
        .iter()
        .position(|l| l.name.eq_ignore_ascii_case(container))
        .ok_or_else(|| format!("no inventory list named {container}"))?;
    let n = save.module.inventory[li].items.len();
    save.module.inventory[li].items.clear();
    Ok(BulkReport {
        changed: n,
        skipped: 0,
        lines: vec![format!("cleared {container}: {n} item(s) removed")],
    })
}

fn bulk_duplicate(save: &mut Save, item_index: usize) -> Result<BulkReport> {
    let r = save
        .items()
        .into_iter()
        .find(|r| r.index == item_index)
        .ok_or_else(|| format!("no item at index {item_index}"))?;
    let mut item = save
        .item(item_index)
        .cloned()
        .ok_or_else(|| format!("no item at index {item_index}"))?;
    if let Some(s) = item.save_body_mut() {
        s.b816 = 0; // a copy must not carry the equipped flag
    }
    let name = item.name.clone();
    let item_idx = r.item_idx + 1;
    match r.source {
        ItemSource::Inventory => {
            save.module.inventory[r.list_idx]
                .items
                .insert(item_idx, item);
        }
        ItemSource::Equipped => {
            save.module.equipped[r.list_idx]
                .items
                .insert(item_idx, item);
        }
        ItemSource::Record => {
            save.module.records[r.source_name_idx].lists[r.list_idx]
                .items
                .insert(item_idx, item);
        }
    }
    let new_index = index_of(save, r.source, r.list_idx, item_idx)
        .ok_or_else(|| "duplicated item is not visible in the item index".to_string())?;
    Ok(BulkReport {
        changed: 1,
        skipped: 0,
        lines: vec![format!(
            "duplicated item {item_index} ({name}) as item {new_index} (b816 cleared)"
        )],
    })
}

fn bulk_set_level(
    save: &mut Save,
    level: u32,
    possessed: bool,
    scope: &BulkScope,
) -> Result<BulkReport> {
    let mut report = BulkReport::default();
    for r in save.items() {
        if !in_scope(save, &r, scope) {
            continue;
        }
        let Some(item) = save.item_mut(r.index) else {
            continue;
        };
        if possessed {
            match possessed_set_level(item, level) {
                Ok(_) => {
                    report.changed += 1;
                    report
                        .lines
                        .push(format!("item {} possessed level -> {level}", r.index));
                }
                Err(_) => report.skipped += 1,
            }
        } else {
            match item.save_body_mut() {
                Some(s) => {
                    s.f872 = level;
                    report.changed += 1;
                    report
                        .lines
                        .push(format!("item {} f872 -> {level}", r.index));
                }
                None => report.skipped += 1,
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsave::db::{AttrInfo, AttrKind, AttributeCatalog, Confidence};
    use crate::dsave::tests::{synthetic_item, synthetic_save};

    fn catalog() -> AttributeCatalog {
        AttributeCatalog::from_attrs(vec![
            AttrInfo {
                name: "Damage".into(),
                kind: AttrKind::Int,
                confidence: Confidence::High,
                enum_id: Some(7),
                table_backed: false,
            },
            AttrInfo {
                name: "Health".into(),
                kind: AttrKind::Int,
                confidence: Confidence::High,
                enum_id: Some(30),
                table_backed: false,
            },
        ])
    }

    fn save() -> Save {
        synthetic_save(vec![
            (
                "Boot",
                vec![
                    synthetic_item("armor/armor_feet_a", &[("Damage", 7, 100)], 1),
                    synthetic_item("armor/armor_feet_b", &[("Health", 30, 50)], 0),
                    synthetic_item("armor/armor_feet_c", &[], 0),
                ],
            ),
            (
                "Gear",
                vec![synthetic_item("items/gear_a", &[("Damage", 7, 7)], 0)],
            ),
        ])
    }

    #[test]
    fn parse_surface() {
        let op = parse_bulk_op(&["set-stat".into(), "Damage".into(), "250".into()]).unwrap();
        assert_eq!(
            op,
            BulkOp::SetStat {
                attribute: "Damage".into(),
                value: 250,
                scope: BulkScope::All
            }
        );
        let op = parse_bulk_op(&[
            "scale".into(),
            "7".into(),
            "150".into(),
            "--container".into(),
            "Boot".into(),
        ])
        .unwrap();
        assert_eq!(
            op,
            BulkOp::Scale {
                attribute: "7".into(),
                percent: 150,
                scope: BulkScope::Container("Boot".into())
            }
        );
        assert_eq!(
            parse_bulk_op(&["duplicate".into(), "3".into()]).unwrap(),
            BulkOp::Duplicate { item_index: 3 }
        );
        let level =
            parse_bulk_op(&["set-level".into(), "20".into(), "--possessed".into()]).unwrap();
        assert_eq!(
            level,
            BulkOp::SetLevel {
                level: 20,
                possessed: true,
                scope: BulkScope::Possessed
            }
        );
        assert!(parse_bulk_op(&["clear".into()]).is_err());
        assert!(parse_bulk_op(&[
            "set-stat".into(),
            "Damage".into(),
            "1".into(),
            "--wat".into()
        ])
        .is_err());
        assert!(parse_bulk_op(&["nope".into()]).is_err());
    }

    #[test]
    fn set_stat_counts_changed_and_skipped() {
        let mut save = save();
        let c = catalog();
        let op = BulkOp::SetStat {
            attribute: "Damage".into(),
            value: 250,
            scope: BulkScope::All,
        };
        let report = apply_bulk(&mut save, Some(&c), &op).unwrap();
        assert_eq!(report.changed, 2);
        assert_eq!(report.skipped, 2); // Health item + no-stat item
        assert_eq!(stat_views(save.item(0).unwrap(), &c)[0].value, 250);
        assert_eq!(stat_views(save.item(3).unwrap(), &c)[0].value, 250);

        // equipped scope only touches the flagged one
        let eq = BulkOp::SetStat {
            attribute: "Damage".into(),
            value: 99,
            scope: BulkScope::Equipped,
        };
        let report = apply_bulk(&mut save, Some(&c), &eq).unwrap();
        assert_eq!(report.changed, 1);
        assert_eq!(report.skipped, 0);
        assert_eq!(stat_views(save.item(0).unwrap(), &c)[0].value, 99);
        // the all-scope op still reaches both Damage items
        let report = apply_bulk(&mut save, Some(&c), &op).unwrap();
        assert_eq!(report.changed, 2);
        assert_eq!(report.skipped, 2);
        // a typo is a hard error
        let bad = BulkOp::SetStat {
            attribute: "Nope".into(),
            value: 1,
            scope: BulkScope::All,
        };
        assert!(apply_bulk(&mut save, Some(&c), &bad).is_err());
    }

    #[test]
    fn scale_multiplies_and_skips_none() {
        let mut save = save();
        let c = catalog();
        let op = BulkOp::Scale {
            attribute: "Damage".into(),
            percent: 200,
            scope: BulkScope::All,
        };
        let report = apply_bulk(&mut save, Some(&c), &op).unwrap();
        assert_eq!(report.changed, 2);
        assert_eq!(report.skipped, 2);
        assert_eq!(stat_views(save.item(0).unwrap(), &c)[0].value, 200);
        assert_eq!(stat_views(save.item(3).unwrap(), &c)[0].value, 14);
        // scale down again restores the values
        let op = BulkOp::Scale {
            attribute: "Damage".into(),
            percent: 50,
            scope: BulkScope::All,
        };
        apply_bulk(&mut save, Some(&c), &op).unwrap();
        assert_eq!(stat_views(save.item(0).unwrap(), &c)[0].value, 100);
        assert_eq!(stat_views(save.item(3).unwrap(), &c)[0].value, 7);
    }

    #[test]
    fn clear_removes_the_list() {
        let mut save = save();
        let report = apply_bulk(
            &mut save,
            None,
            &BulkOp::Clear {
                container: "boot".into(),
            },
        )
        .unwrap();
        assert_eq!(report.changed, 3);
        assert!(save
            .module
            .inventory
            .iter()
            .find(|l| l.name == "Boot")
            .unwrap()
            .items
            .is_empty());
        assert!(apply_bulk(
            &mut save,
            None,
            &BulkOp::Clear {
                container: "Nope".into(),
            }
        )
        .is_err());
    }

    #[test]
    fn duplicate_clears_the_equip_flag() {
        let mut save = save();
        let report = apply_bulk(&mut save, None, &BulkOp::Duplicate { item_index: 0 }).unwrap();
        assert_eq!(report.changed, 1);
        let boot = save
            .module
            .inventory
            .iter()
            .find(|l| l.name == "Boot")
            .unwrap();
        assert_eq!(boot.items.len(), 4);
        assert_eq!(boot.items[0].save_body().unwrap().b816, 1);
        assert_eq!(boot.items[1].save_body().unwrap().b816, 0);
        assert_eq!(boot.items[0].name, boot.items[1].name);
    }

    #[test]
    fn set_level_writes_f872_and_possessed_component() {
        use crate::dsave::model::{Desc, ItemBody};
        let mut item = synthetic_item("weapons/w", &[("Damage", 7, 5)], 0);
        if let ItemBody::Save(s) = &mut item.body {
            s.desc = Some(Desc::PossessedComponent {
                a: 0,
                b: 3,
                c: 10,
                entries: vec![],
            });
        }
        let mut save = synthetic_save(vec![("PrimaryWeapon", vec![item])]);
        let op = BulkOp::SetLevel {
            level: 5,
            possessed: true,
            scope: BulkScope::Possessed,
        };
        let report = apply_bulk(&mut save, None, &op).unwrap();
        assert_eq!(report.changed, 1);
        assert_eq!(possessed_info(save.item(0).unwrap()).unwrap().level, 5);
        // level above the cap is skipped, not an error
        let op = BulkOp::SetLevel {
            level: 50,
            possessed: true,
            scope: BulkScope::All,
        };
        let report = apply_bulk(&mut save, None, &op).unwrap();
        assert_eq!(report.changed, 0);
        assert_eq!(report.skipped, 1);
        // non-possessed path writes f872 on save-body items
        let op = BulkOp::SetLevel {
            level: 7,
            possessed: false,
            scope: BulkScope::All,
        };
        let report = apply_bulk(&mut save, None, &op).unwrap();
        assert_eq!(report.changed, 1);
        assert_eq!(save.item(0).unwrap().save_body().unwrap().f872, 7);
    }
}
