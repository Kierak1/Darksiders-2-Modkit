//! Derived stat totals and item ordering.
//!
//! [`totals`] sums the static `a[2]` magnitudes of the stats of a set of items
//! (equipped items, one inventory container, or the whole save);
//! [`equipped_totals`] is the common equipped-gear case. [`sorted_items`]
//! orders item indices by one attribute's value.
//!
//! Caveat inherited from [`super::stats`]: a table-backed stat (`s[1]`
//! non-empty) stores its magnitude in a float value table selected by
//! `ItemModifier.fields[0]` (a level offset on real weapons), and its `a[2]`
//! is usually the `none` sentinel. Such stats contribute nothing to the sums
//! until a value-table reader exists; the raw `/ -1` view is exposed by
//! [`super::stats::stat_views`].

use super::db::{AttrKind, AttributeCatalog};
use super::items::ItemSource;
use super::model::Save;
use super::stats::{stat_views, VALUE_NONE};
use std::collections::HashMap;

/// Which items [`totals`] sums over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TotalScope<'a> {
    /// Items flagged equipped (`SaveItem.b816 == 1`).
    Equipped,
    /// Every item in the inventory list `container`.
    Container(&'a str),
    /// Every item in the save.
    All,
}

/// One summed attribute.
#[derive(Debug, Clone, PartialEq)]
pub struct Total {
    /// Attribute name (catalog spelling of the first item that carried it).
    pub attribute: String,
    /// Sum of the contributing `a[2]` values.
    pub value: i64,
    /// Catalog typing (`Int`/`Float`/`Unknown`).
    pub kind: AttrKind,
    /// Number of stats that contributed (`none` values are skipped).
    pub items: usize,
}

/// Sum the stats of the items in `scope`, sorted by value descending.
pub fn totals(save: &Save, catalog: &AttributeCatalog, scope: TotalScope<'_>) -> Vec<Total> {
    let mut out: Vec<Total> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for r in save.items() {
        let Some(item) = save.item(r.index) else {
            continue;
        };
        let in_scope = match scope {
            TotalScope::All => true,
            TotalScope::Equipped => item.save_body().is_some_and(|s| s.b816 == 1),
            TotalScope::Container(name) => {
                r.source == ItemSource::Inventory
                    && save.module.inventory[r.list_idx]
                        .name
                        .eq_ignore_ascii_case(name)
            }
        };
        if !in_scope {
            continue;
        }
        for v in stat_views(item, catalog) {
            if v.value == VALUE_NONE {
                continue;
            }
            match index.get(&v.attribute.to_lowercase()) {
                Some(i) => {
                    out[*i].value += v.value;
                    out[*i].items += 1;
                }
                None => {
                    index.insert(v.attribute.to_lowercase(), out.len());
                    out.push(Total {
                        attribute: v.attribute,
                        value: v.value,
                        kind: v.kind,
                        items: 1,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| {
        b.value
            .cmp(&a.value)
            .then_with(|| a.attribute.cmp(&b.attribute))
    });
    out
}

/// Totals over the equipped items (`SaveItem.b816 == 1`).
pub fn equipped_totals(save: &Save, catalog: &AttributeCatalog) -> Vec<Total> {
    totals(save, catalog, TotalScope::Equipped)
}

/// Item indices carrying `attribute` (name or decimal enum id, case
/// insensitive), ordered by value descending. Items whose stat is the `none`
/// sentinel are omitted.
pub fn sorted_items(save: &Save, catalog: &AttributeCatalog, attribute: &str) -> Vec<usize> {
    let want = attribute.trim();
    let want_id: Option<u32> = want.parse().ok();
    let mut hits: Vec<(usize, i64)> = Vec::new();
    for r in save.items() {
        let Some(item) = save.item(r.index) else {
            continue;
        };
        for v in stat_views(item, catalog) {
            if v.value == VALUE_NONE {
                continue;
            }
            if v.attribute.eq_ignore_ascii_case(want) || want_id == Some(v.enum_id) {
                hits.push((r.index, v.value));
                break;
            }
        }
    }
    hits.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    hits.into_iter().map(|(i, _)| i).collect()
}

/// Format a stat magnitude for display (`Float` kinds get a decimal form).
pub fn format_value(kind: AttrKind, value: i64) -> String {
    if kind == AttrKind::Float {
        format!("{}", value as f32)
    } else {
        value.to_string()
    }
}

/// One display line for a total: `label: value (N items)`.
pub fn total_line(label: &str, t: &Total) -> String {
    let suffix = if t.items > 1 {
        format!(" ({} items)", t.items)
    } else {
        String::new()
    };
    format!("{label}: {}{suffix}", format_value(t.kind, t.value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsave::db::{AttrInfo, Confidence};
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
        // Boot: one equipped + one not; Gear: one item with Damage only.
        synthetic_save(vec![
            (
                "Boot",
                vec![
                    synthetic_item(
                        "armor/armor_feet_a",
                        &[("Damage", 7, 100), ("Health", 30, 50)],
                        1,
                    ),
                    synthetic_item("armor/armor_feet_b", &[("Damage", 7, 10)], 0),
                ],
            ),
            (
                "Gear",
                vec![synthetic_item("items/gear_a", &[("Damage", 7, 7)], 0)],
            ),
        ])
    }

    #[test]
    fn equipped_totals_only_sum_flagged_items() {
        let t = equipped_totals(&save(), &catalog());
        assert_eq!(t.len(), 2);
        let damage = t.iter().find(|t| t.attribute == "Damage").unwrap();
        assert_eq!(damage.value, 100);
        assert_eq!(damage.items, 1);
        let health = t.iter().find(|t| t.attribute == "Health").unwrap();
        assert_eq!(health.value, 50);
    }

    #[test]
    fn totals_scope_selects_container_or_all() {
        let save = save();
        let c = catalog();
        let gear = totals(&save, &c, TotalScope::Container("gear"));
        assert_eq!(gear.len(), 1);
        assert_eq!(gear[0].value, 7);
        let all = totals(&save, &c, TotalScope::All);
        let damage = all.iter().find(|t| t.attribute == "Damage").unwrap();
        assert_eq!(damage.value, 117);
        assert_eq!(damage.items, 3);
        assert!(totals(&save, &c, TotalScope::Container("NoSuch")).is_empty());
    }

    #[test]
    fn sorted_items_orders_by_value_desc() {
        let save = save();
        let order = sorted_items(&save, &catalog(), "Damage");
        assert_eq!(order, vec![0, 1, 2]); // 100, 10, 7
                                          // by enum id
        assert_eq!(sorted_items(&save, &catalog(), "7"), order);
        // items without the stat are omitted
        assert!(sorted_items(&save, &catalog(), "Arcane").is_empty());
    }

    #[test]
    fn value_and_line_formatting() {
        assert_eq!(format_value(AttrKind::Int, 12), "12");
        assert_eq!(format_value(AttrKind::Float, 3), "3");
        let t = Total {
            attribute: "Damage".into(),
            value: 117,
            kind: AttrKind::Int,
            items: 3,
        };
        assert_eq!(total_line("Damage", &t), "Damage: 117 (3 items)");
        let one = Total { items: 1, ..t };
        assert_eq!(total_line("Damage", &one), "Damage: 117");
    }
}
