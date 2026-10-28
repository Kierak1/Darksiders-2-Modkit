//! Possessed-weapon wizard: level / XP and upgrade-entry editing on the
//! `PossessedItemComponent` desc.
//!
//! Field semantics verified in IDA (2026-10-11): the component reader
//! `sub_1406EB434` reads `a, b, c` + `n x {u32,u32,u32}`; the item UI
//! (`sub_1402BD550`) maps them through `sub_140673E60` (the component lookup):
//! `+24` is `possessedCurXP`, `+28` is `possessedLevel`, `+32` is
//! `possessedLevelCap`, and the entry vector is the "FedModifiers" list
//! (one record per fed weapon). `possessedLevel` falls back to the item's
//! `possessedStarterLevel` when the component is absent.
//!
//! Operations here are deliberately minimal and validated: they only touch the
//! level / XP fields and (for [`possessed_max`]) pad the fed list with clones of
//! its last entry. Upgrade *stats* are edited through
//! [`possessed_add_mod`] / [`possessed_remove_mod`] (an `AttributeModifier` on
//! the item's `ItemModifier`) and effects through
//! [`possessed_add_effect`] / [`possessed_remove_effect`].

use super::db::AttributeCatalog;
use super::model::{Desc, Item, ItemBody, ItemModifier, Modifier};
use super::stats::check_value;
use crate::Result;

/// Current state of a possessed item's `PossessedItemComponent`.
#[derive(Debug, Clone, PartialEq)]
pub struct PossessedInfo {
    /// Component `a` = accumulated XP (`possessedCurXP`).
    pub cur_xp: u32,
    /// Component `b` = current level (`possessedLevel`).
    pub level: u32,
    /// Component `c` = level cap (`possessedLevelCap`).
    pub level_cap: u32,
    /// Fed-weapon records (`FedModifiers`), each one serialized `[u32; 3]`.
    pub fed: Vec<[u32; 3]>,
}

/// The possessed component of an item, if it has one.
pub fn possessed_info(item: &Item) -> Option<PossessedInfo> {
    let Desc::PossessedComponent { a, b, c, entries } = item.save_body()?.desc.as_ref()?;
    Some(PossessedInfo {
        cur_xp: *a,
        level: *b,
        level_cap: *c,
        fed: entries.clone(),
    })
}

fn desc_mut(item: &mut Item) -> Option<&mut Desc> {
    item.save_body_mut()?.desc.as_mut()
}

/// Set the possessed level (`component.b`), validated against the component's
/// own level cap (`component.c`). XP (`component.a`) is left untouched.
pub fn possessed_set_level(item: &mut Item, level: u32) -> Result<String> {
    let cap = possessed_info(item)
        .ok_or("item has no PossessedItemComponent")?
        .level_cap;
    if level == 0 || level > cap {
        return Err(format!("level {level} out of range 1..={cap}").into());
    }
    let Some(Desc::PossessedComponent { b, .. }) = desc_mut(item) else {
        return Err("item has no PossessedItemComponent".into());
    };
    let old = *b;
    *b = level;
    Ok(format!(
        "possessed level {old} -> {level} (component.b; level cap {cap})"
    ))
}

/// Max out a possessed item: sets the level (`component.b`) to the level cap
/// (`component.c`) and resets XP (`component.a`) to 0 so it cannot level past
/// the cap. When the fed list (`entries`) is non-empty it is padded to `level_cap`
/// entries by cloning the last entry (capped at 64); with no template entry the
/// list is left empty (nothing is invented).
pub fn possessed_max(item: &mut Item) -> Result<String> {
    if possessed_info(item).is_none() {
        return Err("item has no PossessedItemComponent".into());
    }
    let Some(Desc::PossessedComponent {
        a, b, c, entries, ..
    }) = desc_mut(item)
    else {
        return Err("item has no PossessedItemComponent".into());
    };
    let cap = *c;
    if cap == 0 {
        return Err("possessed level cap is 0; refusing to max out".into());
    }
    let old_level = *b;
    *b = cap;
    *a = 0;
    let mut fed_note = "fed list left empty (no entry to clone)".to_string();
    if let Some(template) = entries.last().copied() {
        let target = (cap as usize).min(64);
        while entries.len() < target {
            entries.push(template);
        }
        fed_note = format!(
            "fed list {} -> {} entries (cloned [{}])",
            entries.len(),
            target,
            template.map(|v| v.to_string()).join(", ")
        );
    }
    Ok(format!(
        "possessed level {old_level} -> {cap}, xp -> 0; {fed_note} (component.a/b/c, entries)"
    ))
}

/// Resolve an attribute name or decimal enum id. Checks the item's own stats
/// first, then `catalog` (when given). Returns `(name, enum_id)`.
fn resolve_attr(
    item: &Item,
    catalog: Option<&AttributeCatalog>,
    attribute: &str,
) -> Result<(String, u32)> {
    let attr = attribute.trim();
    if let Ok(id) = attr.parse::<u32>() {
        let name = find_mod(item, attr)
            .and_then(|(i, j)| mod_of(item, i, j))
            .map(|m| m.s[0].clone())
            .filter(|n| !n.is_empty())
            .or_else(|| {
                catalog
                    .and_then(|c| c.attrs().iter().find(|a| a.enum_id == Some(id)))
                    .map(|a| a.name.clone())
            });
        return Ok((name.unwrap_or_else(|| attr.to_string()), id));
    }
    if let Some((i, j)) = find_mod(item, attr) {
        let m = mod_of(item, i, j).ok_or("stat disappeared while resolving")?;
        return Ok((m.s[0].clone(), m.a[1]));
    }
    if let Some(info) = catalog.and_then(|c| c.get(attr)) {
        if let Some(id) = info.enum_id {
            return Ok((info.name.clone(), id));
        }
    }
    Err(format!(
        "unknown attribute {attr:?}: not on this item and not resolvable; pass the attribute enum id or load the item DB"
    )
    .into())
}

fn mod_of(item: &Item, stat: usize, md: usize) -> Option<&Modifier> {
    match &item.save_body()?.nested.get(stat)?.body {
        ItemBody::Modifier(m) => m.mods.get(md),
        _ => None,
    }
}

/// Locate a modifier entry by attribute name or decimal enum id:
/// `(stat_index, mod_index)`.
pub fn find_mod(item: &Item, attribute_or_enum: &str) -> Option<(usize, usize)> {
    let attr = attribute_or_enum.trim();
    let enum_id = attr.parse::<u32>().ok();
    let s = item.save_body()?;
    let mut enum_hit = None;
    for (i, nested) in s.nested.iter().enumerate() {
        let ItemBody::Modifier(m) = &nested.body else {
            continue;
        };
        for (j, md) in m.mods.iter().enumerate() {
            if md.s[0].eq_ignore_ascii_case(attr) {
                return Some((i, j));
            }
            if enum_hit.is_none() && enum_id.is_some_and(|e| md.a[1] == e) {
                enum_hit = Some((i, j));
            }
        }
    }
    enum_hit
}

/// Locate an `ItemModifier` (stat) by its attribute name/enum: the first stat
/// with a matching entry, else `None`.
pub fn find_stat(item: &Item, attribute_or_enum: &str) -> Option<usize> {
    find_mod(item, attribute_or_enum).map(|(i, _)| i)
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

/// Add an `AttributeModifier` entry for `attribute` (name or decimal enum id)
/// to the stat at `stat_index`, cloning the stat's last entry as a structural
/// template. `value` is the static `a[2]` magnitude (`None` keeps the
/// template's value). Refuses duplicates and unknown stat indexes.
pub fn possessed_add_mod(
    item: &mut Item,
    catalog: Option<&AttributeCatalog>,
    stat_index: usize,
    attribute: &str,
    value: Option<i64>,
) -> Result<String> {
    if possessed_info(item).is_none() {
        return Err("item has no PossessedItemComponent".into());
    }
    let (name, enum_id) = resolve_attr(item, catalog, attribute)?;
    let u = value.map(check_value).transpose()?;
    let m = modifier_mut(item, stat_index)?;
    if m.mods.iter().any(|md| md.s[0].eq_ignore_ascii_case(&name)) {
        return Err(format!("stat {stat_index} already has a {name:?} entry").into());
    }
    let mut md = m
        .mods
        .last()
        .cloned()
        .ok_or("this stat has no modifier entry to clone as a template")?;
    md.s[0] = name.clone();
    md.a[1] = enum_id;
    if let Some(u) = u {
        md.a[2] = u;
    }
    m.mods.push(md);
    Ok(format!(
        "added {name} (enum {enum_id}, a[2] {}) to stat {stat_index} ({} entries)",
        u.map(|v| v.to_string())
            .unwrap_or_else(|| "unchanged".into()),
        m.mods.len()
    ))
}

/// Remove the modifier entry at `(stat_index, mod_index)`.
pub fn possessed_remove_mod(
    item: &mut Item,
    stat_index: usize,
    mod_index: usize,
) -> Result<String> {
    if possessed_info(item).is_none() {
        return Err("item has no PossessedItemComponent".into());
    }
    let m = modifier_mut(item, stat_index)?;
    if mod_index >= m.mods.len() {
        return Err(format!(
            "no modifier at index {mod_index} (stat {stat_index} has {})",
            m.mods.len()
        )
        .into());
    }
    let removed = m.mods.remove(mod_index);
    Ok(format!(
        "removed {} from stat {stat_index} ({} entries left)",
        if removed.s[0].is_empty() {
            removed.name.clone()
        } else {
            removed.s[0].clone()
        },
        m.mods.len()
    ))
}

/// Add an effect entry to the stat at `stat_index` by cloning its last effect
/// (the only structurally safe template; failed with no effects present).
pub fn possessed_add_effect(item: &mut Item, stat_index: usize) -> Result<String> {
    if possessed_info(item).is_none() {
        return Err("item has no PossessedItemComponent".into());
    }
    let m = modifier_mut(item, stat_index)?;
    let e = m
        .effects
        .last()
        .cloned()
        .ok_or("this stat has no effect entry to clone as a template")?;
    m.effects.push(e);
    Ok(format!(
        "added cloned {} effect to stat {stat_index} ({} effects)",
        m.effects.last().map(|e| e.obj.name.as_str()).unwrap_or("?"),
        m.effects.len()
    ))
}

/// Remove the effect entry at `(stat_index, effect_index)`.
pub fn possessed_remove_effect(
    item: &mut Item,
    stat_index: usize,
    effect_index: usize,
) -> Result<String> {
    if possessed_info(item).is_none() {
        return Err("item has no PossessedItemComponent".into());
    }
    let m = modifier_mut(item, stat_index)?;
    if effect_index >= m.effects.len() {
        return Err(format!(
            "no effect at index {effect_index} (stat {stat_index} has {})",
            m.effects.len()
        )
        .into());
    }
    let removed = m.effects.remove(effect_index);
    Ok(format!(
        "removed effect {} from stat {stat_index} ({} effects left)",
        removed.obj.name,
        m.effects.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsave::db::{AttrInfo, AttrKind, Confidence};
    use crate::dsave::model::{Gc48, RefBody, RefObject, SaveItem, ScriptBase};

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
                name: "DamageRange".into(),
                kind: AttrKind::Int,
                confidence: Confidence::High,
                enum_id: Some(19),
                table_backed: true,
            },
            AttrInfo {
                name: "FireDamage".into(),
                kind: AttrKind::Int,
                confidence: Confidence::High,
                enum_id: Some(80),
                table_backed: true,
            },
        ])
    }

    fn item(info: PossessedInfo) -> Item {
        let stat = Item {
            name: "ItemModifier".into(),
            body: ItemBody::Modifier(Box::new(ItemModifier {
                base: ([0, 1, 0], u32::MAX),
                attr: "$Item.Attributes.Damage".into(),
                fields: [4, 0, 0, 0, 0],
                mods: vec![
                    Modifier {
                        name: "PossessedAttributeModifier".into(),
                        a: [1, 7, u32::MAX],
                        s: [
                            "Damage".into(),
                            "base/Mod_Weapon_DoubleScythe".into(),
                            "Damage".into(),
                            String::new(),
                        ],
                        extra: Some(0),
                    },
                    Modifier {
                        name: "PossessedAttributeModifier".into(),
                        a: [0, 63, u32::MAX],
                        s: [
                            "Defense".into(),
                            "base/Mod_Weapon_DoubleScythe".into(),
                            "Defense".into(),
                            String::new(),
                        ],
                        extra: Some(1),
                    },
                ],
                effects: vec![Gc48 {
                    b: 0,
                    f: [100, 0, 1, 0],
                    obj: RefObject {
                        name: "EnhancementEffect".into(),
                        body: RefBody::Enhancement(
                            ScriptBase {
                                flag: 0,
                                refs: Vec::new(),
                            },
                            1,
                            "items/enhancement_test".into(),
                        ),
                    },
                }],
            })),
        };
        Item {
            name: "weapons/weapon_possessed_test".into(),
            body: ItemBody::Save(Box::new(SaveItem {
                base: ([1, 1, 0], u32::MAX),
                b816: 0,
                f872: u32::MAX,
                f820: u32::MAX,
                f844: u32::MAX,
                s856: String::new(),
                s864: String::new(),
                nested: vec![stat],
                desc: Some(Desc::PossessedComponent {
                    a: info.cur_xp,
                    b: info.level,
                    c: info.level_cap,
                    entries: info.fed,
                }),
            })),
        }
    }

    fn base_info() -> PossessedInfo {
        PossessedInfo {
            cur_xp: 11850,
            level: 8,
            level_cap: 11,
            fed: vec![[7, 1, 0], [63, 2, 0], [7, 3, 0]],
        }
    }

    #[test]
    fn info_and_set_level() {
        let mut it = item(base_info());
        assert_eq!(possessed_info(&it).unwrap().level, 8);
        let change = possessed_set_level(&mut it, 10).unwrap();
        assert!(change.contains("8 -> 10"), "{change}");
        assert_eq!(possessed_info(&it).unwrap().level, 10);
        assert!(possessed_set_level(&mut it, 0).is_err());
        assert!(possessed_set_level(&mut it, 12).is_err());
    }

    #[test]
    fn max_sets_cap_and_pads_fed() {
        let mut it = item(base_info());
        let change = possessed_max(&mut it).unwrap();
        let info = possessed_info(&it).unwrap();
        assert_eq!(info.level, 11);
        assert_eq!(info.cur_xp, 0);
        assert_eq!(info.fed.len(), 11);
        assert_eq!(info.fed[10], [7, 3, 0]);
        assert!(change.contains("fed list"), "{change}");
    }

    #[test]
    fn max_without_fed_template_leaves_list_empty() {
        let mut it = item(PossessedInfo {
            fed: vec![],
            ..base_info()
        });
        possessed_max(&mut it).unwrap();
        assert!(possessed_info(&it).unwrap().fed.is_empty());
    }

    #[test]
    fn add_remove_mods_and_effects() {
        let mut it = item(base_info());
        let c = catalog();
        let change = possessed_add_mod(&mut it, Some(&c), 0, "FireDamage", Some(80)).unwrap();
        assert!(change.contains("FireDamage"), "{change}");
        {
            let ItemBody::Modifier(m) = &it.save_body().unwrap().nested[0].body else {
                panic!("expected modifier");
            };
            assert_eq!(m.mods.len(), 3);
            assert_eq!(m.mods[2].s[0], "FireDamage");
            assert_eq!(m.mods[2].a[2], 80);
            assert_eq!(m.mods[2].extra, Some(1), "clone keeps the possessed shape");
        }
        assert!(possessed_add_mod(&mut it, Some(&c), 0, "FireDamage", Some(1)).is_err());

        possessed_remove_mod(&mut it, 0, 2).unwrap();
        assert!(possessed_remove_mod(&mut it, 0, 2).is_err());

        let before = match &it.save_body().unwrap().nested[0].body {
            ItemBody::Modifier(m) => m.effects.len(),
            _ => panic!("expected modifier"),
        };
        possessed_add_effect(&mut it, 0).unwrap();
        let ItemBody::Modifier(m) = &it.save_body().unwrap().nested[0].body else {
            panic!("expected modifier");
        };
        assert_eq!(m.effects.len(), before + 1);
        possessed_remove_effect(&mut it, 0, 0).unwrap();
        let ItemBody::Modifier(m) = &it.save_body().unwrap().nested[0].body else {
            panic!("expected modifier");
        };
        assert_eq!(m.effects.len(), before);
        assert!(possessed_remove_effect(&mut it, 0, 9).is_err());
    }

    #[test]
    fn enum_ids_resolve_from_existing_stats() {
        let mut it = item(base_info());
        let c = catalog();
        // the enum id of a stat already on the item resolves to its name, so
        // adding it again is a duplicate
        assert!(possessed_add_mod(&mut it, None, 0, "63", Some(5)).is_err());
        // an enum id only in the catalog names the new entry from the catalog
        possessed_add_mod(&mut it, Some(&c), 0, "19", Some(5)).unwrap();
        let ItemBody::Modifier(m) = &it.save_body().unwrap().nested[0].body else {
            panic!("expected modifier");
        };
        assert_eq!(m.mods.last().unwrap().s[0], "DamageRange");
        assert_eq!(m.mods.last().unwrap().a[1], 19);
        // unknown names need a catalog
        assert!(possessed_add_mod(&mut it, None, 0, "NotAnAttr", Some(5)).is_err());
        assert!(possessed_add_mod(&mut it, Some(&c), 0, "NotAnAttr", Some(5)).is_err());
        // numeric-only names still get a bare enum-id label
        possessed_add_mod(&mut it, None, 0, "999", Some(5)).unwrap();
        let ItemBody::Modifier(m) = &it.save_body().unwrap().nested[0].body else {
            panic!("expected modifier");
        };
        assert_eq!(m.mods.last().unwrap().s[0], "999");
        assert_eq!(m.mods.last().unwrap().a[1], 999);
    }
}
