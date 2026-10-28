//! Semantic stat editing: view and edit an item's `ItemModifier` stats through
//! the attribute catalog instead of raw field paths.
//!
//! Encoding (verified in IDA, see `docs/save-item-db.md` §2): a modifier is
//! `Modifier{ a = [type, attribute_enum, value], s = [attr, value_table,
//! value_lookup, displayed_override] }`. The magnitude is `(float)(int)a[2]`
//! when there is no `ValueTable`, otherwise a float-table lookup whose row is
//! `ItemModifier.fields[0]` (1..=40). [`stat_views`] exposes both encodings;
//! [`set_stat`] writes the static `a[2]` value and [`set_stat_row`] the table
//! row. Both fall back to the raw field paths understood by
//! [`super::items::set_item_field`].

use super::db::{AttrKind, AttributeCatalog};
use super::model::{Item, ItemBody, ItemModifier, Modifier};
use crate::Result;

/// `a[2]` value meaning "none" (`u32::MAX`; the engine reads it as `-1`).
pub const VALUE_NONE: i64 = -1;

/// The static value `a[2]` is read by the engine as `(float)(int)a[2]`, so the
/// largest value it can express is `i32::MAX`.
pub const VALUE_MAX: i64 = i32::MAX as i64;

/// The value-table row range the engine accepts (`ItemModifier.fields[0]`).
pub const ROW_RANGE: std::ops::RangeInclusive<u32> = 1..=40;

/// How `ItemModifier.fields[0]` reads: a table row (1..=40) or, on real
/// weapons, a **signed level offset** (the engine looks up
/// `row = level + offset`, e.g. Damage = -5; see `docs/backlog.md`).
pub fn row_display(row: u32) -> String {
    if ROW_RANGE.contains(&row) {
        return format!("row {row}");
    }
    format!("offset {} (level + offset)", row as i32)
}

/// One editable stat of a save item (one `AttributeModifier` entry).
#[derive(Debug, Clone, PartialEq)]
pub struct StatView {
    /// Attribute name from `s[0]` (or the `$Item.Attributes.` suffix when
    /// `s[0]` is empty), e.g. `Damage`.
    pub attribute: String,
    /// Attribute enum id written to `a[1]` (`Damage` = 7, `Defense` = 63).
    pub enum_id: u32,
    /// Catalog typing for `attribute` (`Unknown` when the catalog has no entry).
    pub kind: AttrKind,
    /// Static value `a[2]` as `i64`; [`VALUE_NONE`] for `u32::MAX`.
    pub value: i64,
    /// Raw field path to `a[2]`, usable with `set_item_field`.
    pub value_path: String,
    /// `s[1]` value-table asset when present.
    pub table: Option<String>,
    /// Table row (`ItemModifier.fields[0]`) of the owning `ItemModifier`, only
    /// set when the stat is table-backed.
    pub row: Option<u32>,
    /// Raw field path to the row (`mod[i].f[0]`), 00 empty when not
    /// table-backed.
    pub row_path: String,
    /// Index of the owning `ItemModifier` in `SaveItem.nested`.
    pub stat_index: usize,
    /// Index of this modifier entry inside the `ItemModifier`.
    pub mod_index: usize,
}

impl StatView {
    pub fn is_table_backed(&self) -> bool {
        self.table.is_some()
    }
}

/// The displayed name of one `AttributeModifier`: `s[0]`, else the suffix of
/// the owning `ItemModifier`'s `$Item.Attributes.*` attr.
fn attr_name(m: &ItemModifier, md: &Modifier) -> String {
    let s = md.s[0].trim();
    if !s.is_empty() {
        return s.to_string();
    }
    m.attr
        .rsplit('.')
        .next()
        .unwrap_or(m.attr.as_str())
        .trim()
        .to_string()
}

fn view_of(
    m: &ItemModifier,
    md: &Modifier,
    i: usize,
    j: usize,
    catalog: &AttributeCatalog,
) -> StatView {
    let attribute = attr_name(m, md);
    let table = {
        let t = md.s[1].trim();
        (!t.is_empty()).then(|| t.to_string())
    };
    StatView {
        kind: catalog
            .get(&attribute)
            .map(|a| a.kind)
            .unwrap_or(AttrKind::Unknown),
        enum_id: md.a[1],
        // The engine reads `(float)(int)a[2]`, so the value is a signed i32;
        // `u32::MAX` comes out as -1 = "none".
        value: i64::from(md.a[2] as i32),
        value_path: format!("mod[{i}].mods[{j}].a[2]"),
        row: table.is_some().then_some(m.fields[0]),
        row_path: if table.is_some() {
            format!("mod[{i}].f[0]")
        } else {
            String::new()
        },
        table,
        attribute,
        stat_index: i,
        mod_index: j,
    }
}

/// All stats of a save item, in `ItemModifier` order. Items without a save-item
/// body (base items, `ItemModifier` objects) have no stats.
pub fn stat_views(item: &Item, catalog: &AttributeCatalog) -> Vec<StatView> {
    let Some(s) = item.save_body() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, nested) in s.nested.iter().enumerate() {
        let ItemBody::Modifier(m) = &nested.body else {
            continue;
        };
        for (j, md) in m.mods.iter().enumerate() {
            out.push(view_of(m, md, i, j, catalog));
        }
    }
    out
}

/// Resolve `attribute_or_enum` (a name or a decimal enum id) against the
/// catalog and locate the matching stat of `item`.
fn locate(
    item: &Item,
    catalog: &AttributeCatalog,
    attribute_or_enum: &str,
) -> Result<(usize, usize, String)> {
    let (name, enum_id) = if let Ok(id) = attribute_or_enum.trim().parse::<u32>() {
        let info = catalog
            .attrs()
            .iter()
            .find(|a| a.enum_id == Some(id))
            .ok_or_else(|| format!("no catalog attribute with enum id {id}"))?;
        (info.name.clone(), Some(id))
    } else {
        let info = catalog.get(attribute_or_enum).ok_or_else(|| {
            format!("unknown attribute {attribute_or_enum:?} (not in the catalog)")
        })?;
        (info.name.clone(), info.enum_id)
    };
    let s = item
        .save_body()
        .ok_or("item has no save-item body, so it has no stats")?;
    // Prefer a name match, then fall back to the enum id.
    let mut enum_hit = None;
    for (i, nested) in s.nested.iter().enumerate() {
        let ItemBody::Modifier(m) = &nested.body else {
            continue;
        };
        for (j, md) in m.mods.iter().enumerate() {
            let a = attr_name(m, md);
            if a.eq_ignore_ascii_case(&name) {
                return Ok((i, j, a));
            }
            if enum_hit.is_none() && enum_id.is_some_and(|e| md.a[1] == e) {
                enum_hit = Some((i, j, a));
            }
        }
    }
    enum_hit.ok_or_else(|| format!("item has no {name:?} stat").into())
}

pub(crate) fn check_value(value: i64) -> Result<u32> {
    if value < VALUE_NONE {
        return Err(
            format!("{value} is not a valid stat value (minimum is {VALUE_NONE} = none)").into(),
        );
    }
    if value > VALUE_MAX {
        return Err(format!(
            "{value} is outside the engine's range: a[2] is read as (float)(int)value, \
             so values above {VALUE_MAX} come out negative"
        )
        .into());
    }
    Ok(if value == VALUE_NONE {
        u32::MAX
    } else {
        value as u32
    })
}

fn modifier_mut(item: &mut Item, stat: usize) -> Result<&mut ItemModifier> {
    match item.save_body_mut().and_then(|s| s.nested.get_mut(stat)) {
        Some(nested) => match &mut nested.body {
            ItemBody::Modifier(m) => Ok(m),
            _ => Err(format!("no stat (ItemModifier) at nested index {stat}").into()),
        },
        None => Err(format!("no stat (ItemModifier) at nested index {stat}").into()),
    }
}

/// Set the static value `a[2]` of the stat named/decimal-id'd
/// `attribute_or_enum`; `-1` writes the `u32::MAX` "none" sentinel.
///
/// For table-backed stats the engine reads the magnitude from the value table
/// (row = `ItemModifier.fields[0]`) instead — use [`set_stat_row`] for those;
/// this function still writes `a[2]` and says so in its report.
pub fn set_stat(
    item: &mut Item,
    catalog: &AttributeCatalog,
    attribute_or_enum: &str,
    value: i64,
) -> Result<String> {
    let u = check_value(value)?;
    let (i, j, attr) = locate(item, catalog, attribute_or_enum)?;
    let m = modifier_mut(item, i)?;
    let md = m
        .mods
        .get_mut(j)
        .ok_or_else(|| format!("no modifier at index {j}"))?;
    let old = md.a[2];
    md.a[2] = u;
    let note = if md.s[1].trim().is_empty() {
        String::new()
    } else {
        format!(
            " (note: {attr} is table-backed; the game reads table row {} — use save-stat-row to change the magnitude)",
            m.fields[0]
        )
    };
    Ok(format!("{attr} a[2] {old} -> {u}{note}"))
}

/// Set the value-table row (`ItemModifier.fields[0]`) of a table-backed stat.
/// Rows are validated against 1..=40.
pub fn set_stat_row(
    item: &mut Item,
    catalog: &AttributeCatalog,
    attribute_or_enum: &str,
    row: u32,
) -> Result<String> {
    if !ROW_RANGE.contains(&row) {
        return Err(format!(
            "row {row} out of range {}..={}",
            ROW_RANGE.start(),
            ROW_RANGE.end()
        )
        .into());
    }
    let (i, j, attr) = locate(item, catalog, attribute_or_enum)?;
    let m = modifier_mut(item, i)?;
    let md = m
        .mods
        .get(j)
        .ok_or_else(|| format!("no modifier at index {j}"))?;
    if md.s[1].trim().is_empty() {
        return Err(format!(
            "{attr} is not table-backed (no ValueTable); use save-stat to write a static value"
        )
        .into());
    }
    let old = m.fields[0];
    m.fields[0] = row;
    Ok(format!(
        "{attr} {} -> row {row} (table {})",
        row_display(old),
        md.s[1]
    ))
}

/// Set the **signed level offset** (`ItemModifier.fields[0]` as `i32`) of a
/// table-backed stat. Real weapons store an offset here, not a 1..=40 row: the
/// engine resolves the table row as `level + offset` (Damage = -5 on
/// Bheithir's Talons). Use [`set_stat_row`] for the synthetic 1..=40 case.
pub fn set_stat_row_offset(
    item: &mut Item,
    catalog: &AttributeCatalog,
    attribute_or_enum: &str,
    offset: i32,
) -> Result<String> {
    let (i, j, attr) = locate(item, catalog, attribute_or_enum)?;
    let m = modifier_mut(item, i)?;
    let md = m
        .mods
        .get(j)
        .ok_or_else(|| format!("no modifier at index {j}"))?;
    if md.s[1].trim().is_empty() {
        return Err(format!(
            "{attr} is not table-backed (no ValueTable); use save-stat to write a static value"
        )
        .into());
    }
    let old = m.fields[0];
    m.fields[0] = offset as u32;
    Ok(format!(
        "{attr} {} -> offset {offset} (table {})",
        row_display(old),
        md.s[1]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsave::db::{AttrInfo, Confidence};
    use crate::dsave::model::{ItemBody, Modifier, SaveItem};

    fn catalog() -> AttributeCatalog {
        AttributeCatalog::from_attrs(vec![
            AttrInfo {
                name: "Damage".into(),
                kind: AttrKind::Int,
                confidence: Confidence::High,
                enum_id: Some(7),
                table_backed: true,
            },
            AttrInfo {
                name: "CriticalDamageMod".into(),
                kind: AttrKind::Int,
                confidence: Confidence::High,
                enum_id: Some(71),
                table_backed: false,
            },
        ])
    }

    fn item() -> Item {
        let stat = Item {
            name: "ItemModifier".into(),
            body: ItemBody::Modifier(Box::new(ItemModifier {
                base: ([0, 1, 0], u32::MAX),
                attr: "$Item.Attributes.Damage".into(),
                fields: [12, 0, 0, 0, 0],
                mods: vec![
                    Modifier {
                        name: "AttributeModifier".into(),
                        a: [1, 7, u32::MAX],
                        s: [
                            "Damage".into(),
                            "base/Mod_Weapon_DoubleScythe".into(),
                            "Damage".into(),
                            String::new(),
                        ],
                        extra: None,
                    },
                    Modifier {
                        name: "AttributeModifier".into(),
                        a: [1, 71, 150],
                        s: [
                            "CriticalDamageMod".into(),
                            String::new(),
                            String::new(),
                            String::new(),
                        ],
                        extra: None,
                    },
                ],
                effects: Vec::new(),
            })),
        };
        Item {
            name: "weapons/weapon_test".into(),
            body: ItemBody::Save(Box::new(SaveItem {
                base: ([1, 1, 0], u32::MAX),
                b816: 0,
                f872: u32::MAX,
                f820: u32::MAX,
                f844: u32::MAX,
                s856: String::new(),
                s864: String::new(),
                nested: vec![stat],
                desc: None,
            })),
        }
    }

    #[test]
    fn stat_views_list_both_encodings() {
        let views = stat_views(&item(), &catalog());
        assert_eq!(views.len(), 2);
        assert_eq!(views[0].attribute, "Damage");
        assert_eq!(views[0].enum_id, 7);
        assert_eq!(views[0].kind, AttrKind::Int);
        assert_eq!(views[0].value, VALUE_NONE);
        assert!(views[0].is_table_backed());
        assert_eq!(views[0].row, Some(12));
        assert_eq!(views[0].value_path, "mod[0].mods[0].a[2]");
        assert_eq!(views[0].row_path, "mod[0].f[0]");
        assert_eq!(views[1].attribute, "CriticalDamageMod");
        assert_eq!(views[1].value, 150);
        assert_eq!(views[1].row, None);
    }

    #[test]
    fn set_stat_round_trips_by_name_and_enum() {
        let mut it = item();
        let c = catalog();
        let change = set_stat(&mut it, &c, "CriticalDamageMod", 200).unwrap();
        assert!(change.contains("150 -> 200"), "{change}");
        let views = stat_views(&it, &c);
        assert_eq!(views[1].value, 200);
        // by enum id
        set_stat(&mut it, &c, "71", 300).unwrap();
        assert_eq!(stat_views(&it, &c)[1].value, 300);
        // -1 clears the static value
        set_stat(&mut it, &c, "criticaldamagemod", -1).unwrap();
        assert_eq!(stat_views(&it, &c)[1].value, VALUE_NONE);
        // case-insensitive names
        set_stat(&mut it, &c, "damage", 42).unwrap();
        assert_eq!(stat_views(&it, &c)[0].value, 42);
    }

    #[test]
    fn set_stat_rejects_bad_input() {
        let mut it = item();
        let c = catalog();
        assert!(set_stat(&mut it, &c, "CriticalDamageMod", -2).is_err());
        assert!(set_stat(&mut it, &c, "CriticalDamageMod", VALUE_MAX + 1).is_err());
        assert!(set_stat(&mut it, &c, "NoSuchAttr", 1).is_err());
        assert!(set_stat(&mut it, &c, "9999", 1).is_err());
        assert!(set_stat(&mut it, &c, "CriticalDamageMod", VALUE_MAX).is_ok());
    }

    #[test]
    fn set_stat_row_validates_table_and_range() {
        let mut it = item();
        let c = catalog();
        assert!(set_stat_row(&mut it, &c, "Damage", 0).is_err());
        assert!(set_stat_row(&mut it, &c, "Damage", 41).is_err());
        assert!(set_stat_row(&mut it, &c, "CriticalDamageMod", 5).is_err());
        let change = set_stat_row(&mut it, &c, "Damage", 7).unwrap();
        assert!(change.contains("row 12 -> row 7"), "{change}");
        assert_eq!(stat_views(&it, &c)[0].row, Some(7));
    }

    #[test]
    fn items_without_save_body_have_no_stats() {
        let it = Item {
            name: "items/item_dust".into(),
            body: ItemBody::Base([0, 0, 0], u32::MAX),
        };
        assert!(stat_views(&it, &catalog()).is_empty());
        let mut it = it;
        assert!(set_stat(&mut it, &catalog(), "Damage", 1).is_err());
    }

    #[test]
    fn stat_values_are_sign_extended_like_the_engine() {
        let mut it = item();
        let c = catalog();
        // WrathCostMod-style penalty: the engine reads (float)(int)a[2]
        if let ItemBody::Save(s) = &mut it.body {
            if let ItemBody::Modifier(m) = &mut s.nested[0].body {
                m.mods[1].a[2] = -25i32 as u32;
            }
        }
        let views = stat_views(&it, &c);
        assert_eq!(views[1].value, -25);
        // u32::MAX sign-extends to -1, which is the "none" sentinel
        if let ItemBody::Save(s) = &mut it.body {
            if let ItemBody::Modifier(m) = &mut s.nested[0].body {
                m.mods[1].a[2] = u32::MAX;
            }
        }
        assert_eq!(stat_views(&it, &c)[1].value, VALUE_NONE);
    }

    #[test]
    fn row_display_separates_rows_from_signed_offsets() {
        assert_eq!(row_display(1), "row 1");
        assert_eq!(row_display(40), "row 40");
        assert_eq!(row_display(-5i32 as u32), "offset -5 (level + offset)");
        assert_eq!(row_display(0), "offset 0 (level + offset)");
    }

    #[test]
    fn set_stat_row_offset_writes_signed_fields0() {
        let mut it = item();
        let c = catalog();
        // Damage is table-backed in the fixture (fields[0] = 12)
        let change = set_stat_row_offset(&mut it, &c, "Damage", -5).unwrap();
        assert!(change.contains("-5"), "{change}");
        let views = stat_views(&it, &c);
        assert_eq!(views[0].row, Some(-5i32 as u32));
        assert_eq!(
            row_display(views[0].row.unwrap()),
            "offset -5 (level + offset)"
        );
        // not table-backed -> refused
        assert!(set_stat_row_offset(&mut it, &c, "CriticalDamageMod", -5).is_err());
        assert!(set_stat_row_offset(&mut it, &c, "NoSuchAttr", 1).is_err());
    }
}
