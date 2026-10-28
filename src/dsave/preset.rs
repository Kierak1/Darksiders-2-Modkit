//! `save.toml` presets: a list of save edits applied all-or-nothing.
//!
//! # Schema
//!
//! Every key is a TOML array of tables; the final schema is this file.
//!
//! ```toml
//! [[set_stat]]               # per-item stat edit (index or def+container)
//! item = 75                  # global index as printed by `save-list`
//! attr = "Damage"            # attribute name or decimal enum id
//! value = 250                # static a[2] value, or:
//! # row = -5                 # value-table row (1..=40) / signed level offset
//!
//! [[bulk.set_stat]]          # bulk attribute edit across a scope
//! attr = "Health"
//! value = 5000
//! filter = "equipped"        # equipped | possessed | all | <container list>
//!
//! [[bulk.scale]]             # multiply the static values by percent/100
//! attr = "Damage"
//! percent = 150
//! filter = "PrimaryWeapon"
//!
//! [[bulk.clear]]             # empty an inventory list
//! container = "Buyback"
//!
//! [[bulk.set_level]]         # f872 item level, or the possessed component level
//! level = 20
//! possessed = false
//! filter = "all"
//!
//! [[add_item]]               # add a def as a new item (needs the item DB)
//! container = "PrimaryWeapon"
//! def = "weapon_scythe_bheithir_talons"
//! rarity = 4                 # optional (validated, not stored)
//! level = 15                 # optional (f872)
//!
//! [[possess]]                # possessed wizard
//! item = 87                  # or def = "..." [container = "..."]
//! max = true                 # optional; and/or level = 18, effect = true,
//! # add_mod = { attr = "HealthRegen", value = 5 }
//!
//! [[equip]]     item = 12    # set the b816 equipped flag
//! [[unequip]]   item = 12    # clear the b816 equipped flag
//! [[duplicate]] item = 12    # clone the item (the copy's b816 is cleared)
//! [[remove]]    item = 3     # delete the item
//! ```
//!
//! # Semantics
//!
//! All `item` indices refer to the save **as loaded** (the indices `save-list`
//! prints). `def` lookups are resolved against the loaded state too; when a
//! def matches several items, add `container` or use an index.
//!
//! Operations apply in this fixed order: `set_stat`, `possess`, `equip`,
//! `unequip`, `bulk.set_stat`/`bulk.scale`/`bulk.set_level`, `duplicate`,
//! `add_item`, `remove`, `bulk.clear`. Resolution happens before any mutation,
//! and the whole preset is applied to a clone of the save: an error anywhere
//! leaves the save untouched and names the failing op.

use serde::Deserialize;

use super::bulk::{apply_bulk, BulkOp, BulkScope};
use super::db::{AttributeCatalog, ItemDb};
use super::items::{add_item, equip_item, find_items, unequip_item, ItemSource};
use super::model::Save;
use super::possessed::{
    find_stat, possessed_add_effect, possessed_add_mod, possessed_max, possessed_set_level,
};
use super::stats::{set_stat, set_stat_row, set_stat_row_offset, ROW_RANGE};
use crate::Result;
use std::path::Path;

/// A parsed `save.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    #[serde(default)]
    pub set_stat: Vec<SetStatOp>,
    #[serde(default)]
    pub bulk: BulkSection,
    #[serde(default)]
    pub add_item: Vec<AddItemOp>,
    #[serde(default)]
    pub possess: Vec<PossessOp>,
    #[serde(default)]
    pub equip: Vec<SelectOp>,
    #[serde(default)]
    pub unequip: Vec<SelectOp>,
    #[serde(default)]
    pub duplicate: Vec<SelectOp>,
    #[serde(default)]
    pub remove: Vec<SelectOp>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BulkSection {
    #[serde(default)]
    pub set_stat: Vec<BulkSetStatOp>,
    #[serde(default)]
    pub scale: Vec<BulkScaleOp>,
    #[serde(default)]
    pub clear: Vec<BulkClearOp>,
    #[serde(default)]
    pub set_level: Vec<BulkSetLevelOp>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetStatOp {
    #[serde(default)]
    pub item: Option<usize>,
    #[serde(default)]
    pub def: Option<String>,
    #[serde(default)]
    pub container: Option<String>,
    pub attr: String,
    #[serde(default)]
    pub value: Option<i64>,
    #[serde(default)]
    pub row: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BulkSetStatOp {
    pub attr: String,
    pub value: i64,
    #[serde(default)]
    pub filter: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BulkScaleOp {
    pub attr: String,
    pub percent: i64,
    #[serde(default)]
    pub filter: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BulkClearOp {
    pub container: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BulkSetLevelOp {
    pub level: u32,
    #[serde(default)]
    pub possessed: bool,
    #[serde(default)]
    pub filter: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddItemOp {
    pub container: String,
    pub def: String,
    #[serde(default)]
    pub rarity: Option<u32>,
    #[serde(default)]
    pub level: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddModOp {
    pub attr: String,
    pub value: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PossessOp {
    #[serde(default)]
    pub item: Option<usize>,
    #[serde(default)]
    pub def: Option<String>,
    #[serde(default)]
    pub container: Option<String>,
    #[serde(default)]
    pub max: bool,
    #[serde(default)]
    pub level: Option<u32>,
    #[serde(default)]
    pub effect: bool,
    #[serde(default)]
    pub add_mod: Option<AddModOp>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectOp {
    #[serde(default)]
    pub item: Option<usize>,
    #[serde(default)]
    pub def: Option<String>,
    #[serde(default)]
    pub container: Option<String>,
}

impl Preset {
    /// True when the preset has no operations.
    pub fn is_empty(&self) -> bool {
        self.set_stat.is_empty()
            && self.equip.is_empty()
            && self.unequip.is_empty()
            && self.duplicate.is_empty()
            && self.remove.is_empty()
            && self.add_item.is_empty()
            && self.possess.is_empty()
            && self.bulk.set_stat.is_empty()
            && self.bulk.scale.is_empty()
            && self.bulk.clear.is_empty()
            && self.bulk.set_level.is_empty()
    }
}

/// Parse a preset from TOML text.
pub fn parse_preset(text: &str) -> Result<Preset> {
    let preset: Preset = toml::from_str(text)?;
    if preset.is_empty() {
        return Err("preset contains no operations".into());
    }
    Ok(preset)
}

/// Read and parse a preset file.
pub fn load_preset(path: &Path) -> Result<Preset> {
    let text = std::fs::read_to_string(path)?;
    parse_preset(&text).map_err(|e| format!("{}: {e}", path.display()).into())
}

/// A resolved item target: the structural position, stable across item
/// insertions (duplicates/adds) until the removal phase.
#[derive(Debug, Clone, PartialEq)]
struct Target {
    source: ItemSource,
    /// Record index (module.records) for `ItemSource::Record`.
    record_idx: usize,
    list_idx: usize,
    item_idx: usize,
    global: usize,
    name: String,
}

fn resolve_selector(
    save: &Save,
    item: Option<usize>,
    def: Option<&str>,
    container: Option<&str>,
    what: &str,
) -> Result<Target> {
    let r = match (item, def) {
        (Some(i), _) => save
            .items()
            .into_iter()
            .find(|r| r.index == i)
            .ok_or_else(|| format!("{what}: no item at index {i}"))?,
        (None, Some(d)) => {
            let hits = find_items(save, d, container);
            match hits.len() {
                0 => {
                    return Err(format!("{what}: no item matching def {d:?}").into());
                }
                1 => save
                    .items()
                    .into_iter()
                    .find(|r| r.index == hits[0])
                    .ok_or_else(|| format!("{what}: item index {} vanished", hits[0]))?,
                n => {
                    return Err(format!(
                        "{what}: def {d:?} matches {n} items ({}); add container or use item = <index>",
                        hits.iter()
                            .take(8)
                            .map(usize::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                    .into());
                }
            }
        }
        (None, None) => {
            return Err(format!("{what}: needs item = <index> or def = \"...\"").into());
        }
    };
    let name = save
        .item(r.index)
        .map(|i| i.name.clone())
        .unwrap_or_default();
    Ok(Target {
        source: r.source,
        record_idx: r.source_name_idx,
        list_idx: r.list_idx,
        item_idx: r.item_idx,
        global: r.index,
        name,
    })
}

enum StepOp {
    SetStat {
        target: Target,
        attr: String,
        value: Option<i64>,
        row: Option<i32>,
    },
    Bulk(BulkOp),
    AddItem {
        container: String,
        def: String,
        rarity: Option<u32>,
        level: Option<i64>,
    },
    Possess {
        target: Target,
        max: bool,
        level: Option<u32>,
        effect: bool,
        add_mod: Option<(String, i64)>,
    },
    Equip(Target),
    Unequip(Target),
    Duplicate(Target),
    Remove(Target),
    Clear(String),
}

struct Step {
    label: String,
    op: StepOp,
}

/// Apply a parsed preset. On success every op was applied and `save` holds the
/// result; on error `save` is untouched and the message names the failing op.
/// Returns one report line per op (plus bulk detail lines).
pub fn apply_preset(
    save: &mut Save,
    db: Option<&ItemDb>,
    catalog: Option<&AttributeCatalog>,
    preset: &Preset,
) -> Result<Vec<String>> {
    if preset.is_empty() {
        return Err("preset contains no operations".into());
    }
    // ---- resolve every op against the save as loaded
    let mut steps: Vec<Step> = Vec::new();
    for (i, op) in preset.set_stat.iter().enumerate() {
        let label = format!("[[set_stat]] #{}", i + 1);
        if op.value.is_none() && op.row.is_none() {
            return Err(format!("{label}: needs value = N or row = N").into());
        }
        if op.value.is_some() && op.row.is_some() {
            return Err(format!("{label}: use either value or row, not both").into());
        }
        let target = resolve_selector(
            save,
            op.item,
            op.def.as_deref(),
            op.container.as_deref(),
            &label,
        )?;
        steps.push(Step {
            label: format!("{label} (item {}, {})", target.global, target.name),
            op: StepOp::SetStat {
                target,
                attr: op.attr.clone(),
                value: op.value,
                row: op.row,
            },
        });
    }
    for (i, op) in preset.possess.iter().enumerate() {
        let label = format!("[[possess]] #{}", i + 1);
        if !op.max && op.level.is_none() && !op.effect && op.add_mod.is_none() {
            return Err(format!("{label}: needs max, level, effect or add_mod").into());
        }
        let target = resolve_selector(
            save,
            op.item,
            op.def.as_deref(),
            op.container.as_deref(),
            &label,
        )?;
        steps.push(Step {
            label: format!("{label} (item {}, {})", target.global, target.name),
            op: StepOp::Possess {
                target,
                max: op.max,
                level: op.level,
                effect: op.effect,
                add_mod: op.add_mod.as_ref().map(|m| (m.attr.clone(), m.value)),
            },
        });
    }
    for (i, op) in preset.equip.iter().enumerate() {
        let label = format!("[[equip]] #{}", i + 1);
        let target = resolve_selector(
            save,
            op.item,
            op.def.as_deref(),
            op.container.as_deref(),
            &label,
        )?;
        steps.push(Step {
            label: format!("{label} (item {}, {})", target.global, target.name),
            op: StepOp::Equip(target),
        });
    }
    for (i, op) in preset.unequip.iter().enumerate() {
        let label = format!("[[unequip]] #{}", i + 1);
        let target = resolve_selector(
            save,
            op.item,
            op.def.as_deref(),
            op.container.as_deref(),
            &label,
        )?;
        steps.push(Step {
            label: format!("{label} (item {}, {})", target.global, target.name),
            op: StepOp::Unequip(target),
        });
    }
    for (i, op) in preset.bulk.set_stat.iter().enumerate() {
        let scope = BulkScope::parse(op.filter.as_deref().unwrap_or("all"));
        steps.push(Step {
            label: format!("[[bulk.set_stat]] #{} (attr {})", i + 1, op.attr),
            op: StepOp::Bulk(BulkOp::SetStat {
                attribute: op.attr.clone(),
                value: op.value,
                scope,
            }),
        });
    }
    for (i, op) in preset.bulk.scale.iter().enumerate() {
        let scope = BulkScope::parse(op.filter.as_deref().unwrap_or("all"));
        steps.push(Step {
            label: format!("[[bulk.scale]] #{} (attr {})", i + 1, op.attr),
            op: StepOp::Bulk(BulkOp::Scale {
                attribute: op.attr.clone(),
                percent: op.percent,
                scope,
            }),
        });
    }
    for (i, op) in preset.bulk.set_level.iter().enumerate() {
        let scope = if op.possessed && op.filter.is_none() {
            BulkScope::Possessed
        } else {
            BulkScope::parse(op.filter.as_deref().unwrap_or("all"))
        };
        steps.push(Step {
            label: format!("[[bulk.set_level]] #{}", i + 1),
            op: StepOp::Bulk(BulkOp::SetLevel {
                level: op.level,
                possessed: op.possessed,
                scope,
            }),
        });
    }
    for (i, op) in preset.duplicate.iter().enumerate() {
        let label = format!("[[duplicate]] #{}", i + 1);
        let target = resolve_selector(
            save,
            op.item,
            op.def.as_deref(),
            op.container.as_deref(),
            &label,
        )?;
        steps.push(Step {
            label: format!("{label} (item {}, {})", target.global, target.name),
            op: StepOp::Duplicate(target),
        });
    }
    for (i, op) in preset.add_item.iter().enumerate() {
        steps.push(Step {
            label: format!("[[add_item]] #{} (def {})", i + 1, op.def),
            op: StepOp::AddItem {
                container: op.container.clone(),
                def: op.def.clone(),
                rarity: op.rarity,
                level: op.level,
            },
        });
    }
    for (i, op) in preset.remove.iter().enumerate() {
        let label = format!("[[remove]] #{}", i + 1);
        let target = resolve_selector(
            save,
            op.item,
            op.def.as_deref(),
            op.container.as_deref(),
            &label,
        )?;
        steps.push(Step {
            label: format!("{label} (item {}, {})", target.global, target.name),
            op: StepOp::Remove(target),
        });
    }
    for (i, op) in preset.bulk.clear.iter().enumerate() {
        steps.push(Step {
            label: format!("[[bulk.clear]] #{} ({})", i + 1, op.container),
            op: StepOp::Clear(op.container.clone()),
        });
    }

    // Shift fixups for `remove`: a duplicate at position q inserts at q + 1,
    // so removal targets later in that list move by the number of duplicates
    // before them.
    let duplicates: Vec<Target> = steps
        .iter()
        .filter_map(|s| match &s.op {
            StepOp::Duplicate(t) => Some(t.clone()),
            _ => None,
        })
        .collect();

    // ---- apply on a clone; any failure leaves `save` untouched
    let mut work = save.clone();
    let mut report: Vec<String> = Vec::new();
    let apply = |step: &Step, work: &mut Save| -> Result<Option<String>> {
        let detail = match &step.op {
            StepOp::SetStat {
                target,
                attr,
                value,
                row,
            } => {
                let catalog = catalog
                    .ok_or("setting stats needs the attribute catalog (scripts.obsp is missing)")?;
                let item = work
                    .item_mut(target.global)
                    .ok_or_else(|| format!("no item at index {}", target.global))?;
                match row {
                    Some(r) if ROW_RANGE.contains(&(*r as u32)) => {
                        Some(set_stat_row(item, catalog, attr, *r as u32)?)
                    }
                    Some(r) => Some(set_stat_row_offset(item, catalog, attr, *r)?),
                    None => Some(set_stat(
                        item,
                        catalog,
                        attr,
                        value.ok_or("missing value")?,
                    )?),
                }
            }
            StepOp::Possess {
                target,
                max,
                level,
                effect,
                add_mod,
            } => {
                let item = work
                    .item_mut(target.global)
                    .ok_or_else(|| format!("no item at index {}", target.global))?;
                let mut changes: Vec<String> = Vec::new();
                if let Some(level) = level {
                    changes.push(possessed_set_level(item, *level)?);
                }
                if *max {
                    changes.push(possessed_max(item)?);
                }
                if let Some((attr, value)) = add_mod {
                    let stat = find_stat(item, attr)
                        .or_else(|| first_mod_stat(item))
                        .ok_or(format!(
                            "item {} has no ItemModifier to add to",
                            target.global
                        ))?;
                    changes.push(possessed_add_mod(item, catalog, stat, attr, Some(*value))?);
                }
                if *effect {
                    let stat = first_mod_stat(item).ok_or(format!(
                        "item {} has no ItemModifier to add to",
                        target.global
                    ))?;
                    changes.push(possessed_add_effect(item, stat)?);
                }
                Some(changes.join("; "))
            }
            StepOp::Equip(target) => {
                equip_item(work, target.global, db)?;
                Some(format!("equipped item {}", target.global))
            }
            StepOp::Unequip(target) => {
                unequip_item(work, target.global)?;
                Some(format!("unequipped item {}", target.global))
            }
            StepOp::Bulk(op) => {
                let r = apply_bulk(work, catalog, op)?;
                let mut lines = vec![format!("{} changed, {} skipped", r.changed, r.skipped)];
                lines.extend(r.lines.iter().take(10).cloned());
                Some(lines.join("; "))
            }
            StepOp::Duplicate(target) => {
                let report = apply_bulk(
                    work,
                    None,
                    &BulkOp::Duplicate {
                        item_index: target.global,
                    },
                )?;
                Some(report.lines.join("; "))
            }
            StepOp::AddItem {
                container,
                def,
                rarity,
                level,
            } => {
                let index = add_item(work, db, container, def, *rarity, *level)?;
                Some(format!("added as item {index} in {container}"))
            }
            StepOp::Remove(target) => {
                let shift = duplicates
                    .iter()
                    .filter(|d| {
                        d.source == target.source
                            && d.list_idx == target.list_idx
                            && d.item_idx < target.item_idx
                    })
                    .count();
                let pos = target.item_idx + shift;
                let items = match target.source {
                    ItemSource::Inventory => &mut work.module.inventory[target.list_idx].items,
                    ItemSource::Equipped => &mut work.module.equipped[target.list_idx].items,
                    ItemSource::Record => {
                        &mut work.module.records[target.record_idx].lists[target.list_idx].items
                    }
                };
                if pos >= items.len() {
                    return Err(format!(
                        "item moved out of range while applying (position {pos} of {})",
                        items.len()
                    )
                    .into());
                }
                let removed = items.remove(pos);
                Some(format!(
                    "removed {pos} (<- item {} {})",
                    target.global, removed.name
                ))
            }
            StepOp::Clear(container) => {
                let r = apply_bulk(
                    work,
                    None,
                    &BulkOp::Clear {
                        container: container.clone(),
                    },
                )?;
                Some(r.lines.join("; "))
            }
        };
        Ok(detail)
    };

    for step in &steps {
        match apply(step, &mut work) {
            Ok(detail) => {
                let line = match detail {
                    Some(d) if !d.is_empty() => format!("{}: {d}", step.label),
                    _ => step.label.clone(),
                };
                report.push(line);
            }
            Err(e) => return Err(format!("{}: {e}", step.label).into()),
        }
    }
    *save = work;
    Ok(report)
}

fn first_mod_stat(item: &super::model::Item) -> Option<usize> {
    let body = item.save_body()?;
    body.nested
        .iter()
        .position(|n| matches!(n.body, super::model::ItemBody::Modifier(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsave::db::{AttrInfo, AttrKind, Confidence};
    use crate::dsave::tests::{synthetic_item, synthetic_save};

    fn catalog() -> AttributeCatalog {
        AttributeCatalog::from_attrs(vec![AttrInfo {
            name: "Damage".into(),
            kind: AttrKind::Int,
            confidence: Confidence::High,
            enum_id: Some(7),
            table_backed: false,
        }])
    }

    fn save() -> Save {
        synthetic_save(vec![
            (
                "Boot",
                vec![
                    synthetic_item("armor/armor_feet_a", &[("Damage", 7, 100)], 0),
                    synthetic_item("armor/armor_feet_a", &[("Damage", 7, 20)], 0),
                    synthetic_item("armor/armor_feet_b", &[("Damage", 7, 5)], 0),
                ],
            ),
            (
                "Gear",
                vec![synthetic_item("items/gear_a", &[("Damage", 7, 7)], 0)],
            ),
        ])
    }

    #[test]
    fn parses_the_documented_schema() {
        let text = r#"
[[set_stat]]
item = 75
attr = "Damage"
value = 250

[[bulk.set_stat]]
attr = "Health"
value = 5000
filter = "equipped"

[[add_item]]
container = "PrimaryWeapon"
def = "weapon_scythe_bheithir_talons"
rarity = 4

[[possess]]
item = 87
max = true

[[equip]]
item = 12

[[remove]]
item = 3
"#;
        let p = parse_preset(text).unwrap();
        assert_eq!(p.set_stat.len(), 1);
        assert_eq!(p.set_stat[0].item, Some(75));
        assert_eq!(p.set_stat[0].value, Some(250));
        assert_eq!(p.bulk.set_stat.len(), 1);
        assert_eq!(p.bulk.set_stat[0].filter.as_deref(), Some("equipped"));
        assert_eq!(p.add_item[0].def, "weapon_scythe_bheithir_talons");
        assert_eq!(p.add_item[0].rarity, Some(4));
        assert!(p.possess[0].max);
        assert_eq!(p.equip[0].item, Some(12));
        assert_eq!(p.remove[0].item, Some(3));
        assert!(!p.is_empty());
    }

    #[test]
    fn rejects_empty_and_unknown_keys() {
        assert!(parse_preset("").is_err());
        assert!(parse_preset("[[set_stat]]\nitem = 1\nwat = 2\n").is_err());
    }

    #[test]
    fn apply_set_stat_by_index_and_def_and_remove_shift() {
        let mut s = save();
        let c = catalog();
        let text = r#"
[[set_stat]]
item = 0
attr = "Damage"
value = 250

[[set_stat]]
def = "armor_feet_b"
attr = "Damage"
value = 1

[[duplicate]]
item = 0

[[remove]]
item = 1
"#;
        let p = parse_preset(text).unwrap();
        let report = apply_preset(&mut s, None, Some(&c), &p).unwrap();
        assert_eq!(report.len(), 4);
        assert!(report[0].starts_with("[[set_stat]] #1"));
        let boot = s
            .module
            .inventory
            .iter()
            .find(|l| l.name == "Boot")
            .unwrap();
        // set item 0 -> 250, set def armor_feet_b -> 1, duplicate item 0 (the
        // copy's b816 is cleared), then remove global item 1 (the 20-value a,
        // shifted one slot by the duplicate).
        assert_eq!(boot.items.len(), 3);
        let values: Vec<i64> = boot
            .items
            .iter()
            .map(|i| crate::dsave::stat_views(i, &c)[0].value)
            .collect();
        assert_eq!(values, vec![250, 250, 1]);
        let gear = s
            .module
            .inventory
            .iter()
            .find(|l| l.name == "Gear")
            .unwrap();
        assert_eq!(crate::dsave::stat_views(&gear.items[0], &c)[0].value, 7);
    }

    #[test]
    fn error_names_the_op_and_leaves_the_save_untouched() {
        let mut s = save();
        let text = "[[set_stat]]\nitem = 99\nattr = \"Damage\"\nvalue = 1\n";
        let p = parse_preset(text).unwrap();
        let err = apply_preset(&mut s, None, Some(&catalog()), &p).unwrap_err();
        assert!(err.to_string().contains("[[set_stat]] #1"), "{err}");
        assert!(err.to_string().contains("99"), "{err}");
        assert_eq!(s, save());

        // a failure in the middle of the list also rolls back everything
        let text = "[[set_stat]]\nitem = 0\nattr = \"Damage\"\nvalue = 1\n\
                    [[set_stat]]\nitem = 99\nattr = \"Damage\"\nvalue = 1\n";
        let p = parse_preset(text).unwrap();
        assert!(apply_preset(&mut s, None, Some(&catalog()), &p).is_err());
        assert_eq!(s, save());
    }

    #[test]
    fn bulk_and_possess_run_in_order() {
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
        let mut s = synthetic_save(vec![("PrimaryWeapon", vec![item])]);
        let c = catalog();
        let text = r#"
[[set_stat]]
item = 0
attr = "Damage"
value = 100

[[bulk.scale]]
attr = "Damage"
percent = 200
filter = "all"

[[possess]]
item = 0
level = 8
max = true
"#;
        let p = parse_preset(text).unwrap();
        let report = apply_preset(&mut s, None, Some(&c), &p).unwrap();
        assert_eq!(report.len(), 3);
        assert_eq!(
            crate::dsave::stat_views(s.item(0).unwrap(), &c)[0].value,
            200
        );
        let info = crate::dsave::possessed_info(s.item(0).unwrap()).unwrap();
        assert_eq!(info.level, 10);
        assert_eq!(info.level_cap, 10);
    }
}
