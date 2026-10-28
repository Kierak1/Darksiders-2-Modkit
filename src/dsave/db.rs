//! Item definition database, attribute catalog and display-name resolution.
//!
//! # Where the data comes from
//!
//! Item definitions are `scripts.obsp` entries whose payload has a
//! `Description` object (class `EquippableItemDesc`, `WeaponDesc`, `ItemDesc`,
//! `ConsumableDesc`, ...). Save item names are `<asset>/<def name>` — e.g.
//! `armor/armor_feet_wanderer_d02` is the def `armor_feet_wanderer_d02`.
//! [`ItemDb::build`] parses every payload once and indexes the defs by name.
//!
//! Attribute names come from two places, both collected by
//! [`AttributeCatalog::build`]:
//! * `ItemCategoryAttributeList` entries (`AttributeNames = [...]`), and
//! * every `AttributeModifier.Attribute` used by an item def.
//!
//! # Value encoding (verified against the engine, IDA `sub_1406CE7C4`)
//!
//! A save modifier is `Modifier{ a=[type, attr_enum, value], s=[attr,
//! value_table, value_lookup, displayed_override] }`:
//! * `a[1]` is the **attribute enum id** (`Damage` = 7, `Defense` = 63, ...),
//! * `a[2]` is the static **Value** (`u32::MAX` = none); the engine widens it
//!   with `(float)(int)value` when there is no `ValueTable`,
//! * when a `ValueTable` exists the magnitude is looked up from a
//!   `FloatTableRow` table using `ItemModifier.fields[0]` as the row (level).
//!
//! So every stat magnitude is an `f32` at runtime, while the authored values
//! are integer-encoded; [`AttrInfo::kind`] follows the *authored* encoding
//! (the task's empirical rule: a raw > 0x10000 whose f32 bit pattern is a
//! finite non-zero number in `[1e-4, 1e7)` is `Float`, everything else `Int`),
//! and [`AttrInfo::table_backed`] marks attributes whose magnitudes are
//! actually authored in float value tables.

use super::loc::LocTable;
use super::model::{ItemBody, Save};
use crate::obsp::{payload as obsp_payload, Entry, Obsp};
use std::collections::{BTreeMap, HashMap};

use obsp_payload::{ObjectValue, Payload, Property, Value};

// ---------------------------------------------------------------------------
// item definitions

/// One item definition from `scripts.obsp`.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemDef {
    /// Def name (the part of a save item name after the last `/`).
    pub name: String,
    /// Payload/entry name when available, e.g. `armor/armor_feet_wanderer_d01`.
    pub asset: Option<String>,
    /// Module reference, e.g. `oc\Feet\wanderer`.
    pub module: Option<String>,
    /// `Description` object class, e.g. `EquippableItemDesc`, `WeaponDesc`.
    pub desc_class: String,
    /// `Container` (equip slot: `Boot`, `PrimaryWeapon`, ...).
    pub container: Option<String>,
    /// `Rarity` (only some desc classes carry it).
    pub rarity: Option<u32>,
    /// Number of `RarityLevels` entries.
    pub rarity_levels: usize,
    pub hud_icon: Option<String>,
    pub pickup_icon: Option<String>,
    /// Top-level `PickupTitle` loc key (`$Item.Weapons...`).
    pub pickup_title: Option<String>,
    /// Per-rarity `PickupTitle` overrides from `RarityLevels[i]`.
    pub rarity_titles: Vec<String>,
    /// `Description` loc key (the details text).
    pub details: Option<String>,
    pub visual: Option<String>,
    pub name_generator: Option<String>,
    /// `ItemType` from the desc (or top-level `ItemTypeOverride`).
    pub item_type: Option<u32>,
    /// Top-level `Level` (legendary items have one).
    pub level: Option<i32>,
    /// Top-level `Modifiers` (`ItemModifier` objects).
    pub modifiers: Vec<DefModifier>,
}

/// One `ItemModifier` from an item def.
#[derive(Debug, Clone, PartialEq)]
pub struct DefModifier {
    /// `ModifierLabel` loc key (`$Item.Attributes.Damage`).
    pub label: Option<String>,
    /// `LevelOffset` (signed; stored as u32 in the payload).
    pub level_offset: Option<i32>,
    pub attrs: Vec<DefAttr>,
}

/// A statically authored `Value` in a def, keeping the serialized type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DefValue {
    Int(i64),
    Float(f32),
}

/// One `AttributeModifier` from a `ItemModifier`.
#[derive(Debug, Clone, PartialEq)]
pub struct DefAttr {
    /// `Attribute` name (`Damage`, `CriticalDamageMod`, ...).
    pub attribute: String,
    /// `Type` (0/1/2 observed; the engine's modifier operation).
    pub type_code: Option<u32>,
    /// Static `Value` when authored, `None` otherwise.
    pub value: Option<DefValue>,
    /// `ValueTable` asset (`base/Mod_Weapon_DoubleScythe`).
    pub value_table: Option<String>,
    /// `ValueLookup` column name inside the table.
    pub value_lookup: Option<String>,
}

/// Def counts recorded while building the database.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Coverage {
    pub entries_scanned: usize,
    /// Entries whose payload parsed (any property).
    pub payloads_parsed: usize,
    /// Entries with a `Description` object.
    pub defs: usize,
    pub with_container: usize,
    pub with_rarity: usize,
    pub with_pickup_title: usize,
    pub with_name_generator: usize,
    pub with_modifiers: usize,
    /// `(desc class, count)` sorted by count descending.
    pub desc_classes: Vec<(String, usize)>,
}

/// Item definition database built from the stock `scripts.obsp`.
#[derive(Debug, Default)]
pub struct ItemDb {
    defs: Vec<ItemDef>,
    index: HashMap<String, usize>,
    coverage: Coverage,
}

impl ItemDb {
    /// Parse every OBSP payload once and collect the defs that carry a
    /// `Description` object. Unparseable payloads are skipped (counted in
    /// [`Coverage`]).
    pub fn build(obsp: &Obsp, data: &[u8]) -> ItemDb {
        let mut db = ItemDb::default();
        let mut classes: BTreeMap<String, usize> = BTreeMap::new();
        for entry in &obsp.entries {
            db.coverage.entries_scanned += 1;
            let payload = obsp.payload(data, entry);
            let Ok(parsed) = obsp_payload::parse_payload(payload) else {
                continue;
            };
            db.coverage.payloads_parsed += 1;
            let Some(desc) = parsed.get("Description") else {
                continue;
            };
            let Value::Object(desc_obj) = &desc.value else {
                continue;
            };
            let Some(def) = def_from_entry(obsp, entry, &parsed, desc_obj) else {
                continue;
            };
            db.coverage.defs += 1;
            db.coverage.with_container += usize::from(def.container.is_some());
            db.coverage.with_rarity += usize::from(def.rarity.is_some());
            db.coverage.with_pickup_title += usize::from(def.pickup_title.is_some());
            db.coverage.with_name_generator += usize::from(def.name_generator.is_some());
            db.coverage.with_modifiers += usize::from(!def.modifiers.is_empty());
            *classes.entry(def.desc_class.clone()).or_default() += 1;
            db.index
                .entry(def.name.to_lowercase())
                .or_insert(db.defs.len());
            db.defs.push(def);
        }
        let mut classes: Vec<(String, usize)> = classes.into_iter().collect();
        classes.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        db.coverage.desc_classes = classes;
        db
    }

    /// Def by name. A save item name (`armor/armor_feet_wanderer_d02`) is
    /// accepted as well; only the part after the last `/` is significant.
    pub fn get(&self, name: &str) -> Option<&ItemDef> {
        let key = name.rsplit('/').next().unwrap_or(name).to_lowercase();
        self.index.get(&key).map(|i| &self.defs[*i])
    }

    pub fn defs(&self) -> &[ItemDef] {
        &self.defs
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    pub fn coverage(&self) -> &Coverage {
        &self.coverage
    }
}

fn def_from_entry(
    obsp: &Obsp,
    entry: &Entry,
    parsed: &Payload,
    desc: &ObjectValue,
) -> Option<ItemDef> {
    let refs = obsp.entry_names(entry);
    let name = refs
        .first()
        .map(|s| (*s).to_owned())
        .or_else(|| obsp.name_of(entry.name_hash).map(str::to_owned))?;
    let module = refs.get(1).map(|s| (*s).to_owned());
    let asset = obsp.name_of(entry.name_hash).map(str::to_owned);
    let desc_class = desc.class_ref.clone().unwrap_or_default();

    let prop = |n: &str| value_of(&desc.properties, n);
    let prop_ref = |n: &str| prop(n).and_then(as_ref_str).map(str::to_owned);

    let rarity_levels: &[Value] = prop("RarityLevels").and_then(as_array).unwrap_or(&[]);
    let rarity_titles: Vec<String> = rarity_levels
        .iter()
        .filter_map(|v| match v {
            Value::Object(o) => value_of(&o.properties, "PickupTitle")
                .and_then(as_ref_str)
                .map(str::to_owned),
            _ => None,
        })
        .collect();

    // EquippableItemDesc carries the icons only on its RarityLevels entries.
    let first_level = rarity_levels.iter().find_map(|v| match v {
        Value::Object(o) => Some(o),
        _ => None,
    });
    let level_ref = |n: &str| {
        first_level
            .and_then(|o| value_of(&o.properties, n))
            .and_then(as_ref_str)
            .map(str::to_owned)
    };
    let item_type = prop("ItemType").and_then(as_u32).or_else(|| {
        parsed
            .get("ItemTypeOverride")
            .and_then(|p| as_u32(&p.value))
    });
    let level = parsed
        .get("Level")
        .and_then(|p| as_u32(&p.value))
        .map(|v| v as i32);

    Some(ItemDef {
        name,
        asset,
        module,
        desc_class,
        container: prop_ref("Container"),
        rarity: prop("Rarity").and_then(as_u32),
        rarity_levels: rarity_levels.len(),
        hud_icon: prop_ref("HudIcon").or_else(|| level_ref("HudIcon")),
        pickup_icon: prop_ref("PickupIcon").or_else(|| level_ref("PickupIcon")),
        pickup_title: prop_ref("PickupTitle"),
        rarity_titles,
        details: prop_ref("Description"),
        visual: prop_ref("Visual"),
        name_generator: prop_ref("NameGenerator"),
        item_type,
        level,
        modifiers: def_modifiers(parsed),
    })
}

fn def_modifiers(parsed: &Payload) -> Vec<DefModifier> {
    let Some(arr) = parsed.get("Modifiers").and_then(|p| as_array(&p.value)) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| {
            let Value::Object(m) = v else { return None };
            let attrs = value_of(&m.properties, "AttributeModifiers")
                .and_then(as_array)
                .map(|items| items.iter().filter_map(def_attr).collect())
                .unwrap_or_default();
            Some(DefModifier {
                label: value_of(&m.properties, "ModifierLabel")
                    .and_then(as_ref_str)
                    .map(str::to_owned),
                level_offset: value_of(&m.properties, "LevelOffset")
                    .and_then(as_u32)
                    .map(|v| v as i32),
                attrs,
            })
        })
        .collect()
}

fn def_attr(v: &Value) -> Option<DefAttr> {
    let Value::Object(o) = v else { return None };
    let attribute = value_of(&o.properties, "Attribute")
        .and_then(as_ref_str)?
        .to_owned();
    let value = match value_of(&o.properties, "Value") {
        Some(Value::Int(v)) => Some(DefValue::Int(i64::from(*v))),
        Some(Value::Float(v)) => Some(DefValue::Float(*v)),
        _ => None,
    };
    Some(DefAttr {
        attribute,
        type_code: value_of(&o.properties, "Type").and_then(as_u32),
        value,
        value_table: value_of(&o.properties, "ValueTable")
            .and_then(as_ref_str)
            .map(str::to_owned),
        value_lookup: value_of(&o.properties, "ValueLookup")
            .and_then(as_ref_str)
            .map(str::to_owned),
    })
}

// ---------------------------------------------------------------------------
// attribute catalog

/// How an attribute's authored magnitude is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrKind {
    /// Integer-encoded (the engine widens with `(float)(int)value`).
    Int,
    /// A real `f32` (float `Value` in a def, or an f32 bit pattern in a save).
    Float,
    /// Seen by name, no value evidence.
    Unknown,
}

impl AttrKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AttrKind::Int => "int",
            AttrKind::Float => "float",
            AttrKind::Unknown => "unknown",
        }
    }
}

/// Confidence in [`AttrInfo::kind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// Decided by a static `Value` authored in an item def.
    High,
    /// Decided only by values observed in saves.
    Medium,
    /// No value evidence (name only).
    Low,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::High => "high",
            Confidence::Medium => "medium",
            Confidence::Low => "low",
        }
    }
}

/// One attribute of the catalog.
#[derive(Debug, Clone, PartialEq)]
pub struct AttrInfo {
    /// Authored name (`Damage`, `CriticalDamageMod`, ...) as used by
    /// `AttributeNames` and `AttributeModifier.Attribute`.
    pub name: String,
    pub kind: AttrKind,
    pub confidence: Confidence,
    /// Attribute enum id observed in saves (`mod.mods[m].a[1]`), e.g.
    /// `Damage` = 7, `Defense` = 63.
    pub enum_id: Option<u32>,
    /// True when the attribute has a (float) `Mod_*` value-table column, i.e.
    /// its magnitudes come from `FloatTableRow` cells.
    pub table_backed: bool,
}

#[derive(Debug, Default)]
pub struct AttributeCatalog {
    attrs: Vec<AttrInfo>,
    index: HashMap<String, usize>,
}

impl AttributeCatalog {
    /// Collect the attribute union from `AttributeNames` lists, item-def
    /// `AttributeModifier.Attribute`s and save modifier names, then type them
    /// with the def/save values. `Mod_*` value-table `ColumnNames` only mark
    /// existing entries as [`AttrInfo::table_backed`].
    pub fn build(obsp: &Obsp, data: &[u8], saves: &[&Save]) -> AttributeCatalog {
        let mut acc: BTreeMap<String, AttrAcc> = BTreeMap::new();
        // Column names of `Mod_*` value tables (marked by `<AttributeName>`);
        // they only flag existing catalog members as table-backed, never add
        // new attributes (skill/ability tables use the same marker).
        let mut table_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for entry in &obsp.entries {
            let Ok(parsed) = obsp_payload::parse_payload(obsp.payload(data, entry)) else {
                continue;
            };
            if let Some(arr) = parsed
                .get("AttributeNames")
                .and_then(|p| as_array(&p.value))
            {
                for v in arr {
                    if let Some(n) = as_ref_str(v) {
                        acc.entry(n.to_owned()).or_default();
                    }
                }
            }
            if let Some(arr) = parsed.get("ColumnNames").and_then(|p| as_array(&p.value)) {
                let is_attribute_table =
                    arr.iter().any(|v| as_ref_str(v) == Some("<AttributeName>"));
                if is_attribute_table {
                    for v in arr {
                        if let Some(n) = as_ref_str(v) {
                            if !n.starts_with('<') {
                                table_names.insert(n.to_owned());
                            }
                        }
                    }
                }
            }
            for m in def_modifiers(&parsed) {
                for a in m.attrs {
                    let e = acc.entry(a.attribute).or_default();
                    if a.value_table.is_some() {
                        e.table_backed = true;
                    }
                    match a.value {
                        Some(DefValue::Float(_)) => e.def_float += 1,
                        Some(DefValue::Int(_)) => e.def_int += 1,
                        None => {}
                    }
                }
            }
        }
        for save in saves {
            for r in save.items() {
                let Some(item) = save.item(r.index) else {
                    continue;
                };
                let body = match &item.body {
                    ItemBody::Save(s) | ItemBody::SaveU32(s, _) | ItemBody::SaveU32U8(s, _, _) => s,
                    _ => continue,
                };
                for nested in &body.nested {
                    let ItemBody::Modifier(m) = &nested.body else {
                        continue;
                    };
                    for md in &m.mods {
                        let name = md.s[0].trim();
                        if name.is_empty() {
                            continue;
                        }
                        let e = acc.entry(name.to_owned()).or_default();
                        e.enum_id.get_or_insert(md.a[1]);
                        match classify_raw(md.a[2]) {
                            AttrKind::Float => e.save_float += 1,
                            _ => e.save_int += 1,
                        }
                    }
                }
            }
        }
        for (name, a) in acc.iter_mut() {
            if table_names.contains(name) {
                a.table_backed = true;
            }
        }
        let mut attrs: Vec<AttrInfo> = acc.into_iter().map(|(name, a)| a.finish(name)).collect();
        attrs.sort_by(|a, b| a.name.cmp(&b.name));
        let index = attrs
            .iter()
            .enumerate()
            .map(|(i, a)| (a.name.to_lowercase(), i))
            .collect();
        AttributeCatalog { attrs, index }
    }

    /// Build a catalog directly from attribute entries (sorted by name and
    /// indexed case-insensitively). Useful for synthetic setups and tests.
    pub fn from_attrs(mut attrs: Vec<AttrInfo>) -> AttributeCatalog {
        attrs.sort_by(|a, b| a.name.cmp(&b.name));
        let index = attrs
            .iter()
            .enumerate()
            .map(|(i, a)| (a.name.to_lowercase(), i))
            .collect();
        AttributeCatalog { attrs, index }
    }

    pub fn get(&self, name: &str) -> Option<&AttrInfo> {
        self.index
            .get(&name.to_lowercase())
            .map(|i| &self.attrs[*i])
    }

    pub fn attrs(&self) -> &[AttrInfo] {
        &self.attrs
    }

    pub fn len(&self) -> usize {
        self.attrs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.attrs.is_empty()
    }
}

#[derive(Debug, Default)]
struct AttrAcc {
    table_backed: bool,
    def_int: usize,
    def_float: usize,
    save_int: usize,
    save_float: usize,
    enum_id: Option<u32>,
}

impl AttrAcc {
    fn finish(self, name: String) -> AttrInfo {
        let (kind, confidence) = if self.def_float + self.save_float > 0 {
            if self.def_float > 0 {
                (AttrKind::Float, Confidence::High)
            } else {
                (AttrKind::Float, Confidence::Medium)
            }
        } else if self.def_int > 0 {
            (AttrKind::Int, Confidence::High)
        } else if self.save_int > 0 {
            (AttrKind::Int, Confidence::Medium)
        } else {
            (AttrKind::Unknown, Confidence::Low)
        };
        AttrInfo {
            name,
            kind,
            confidence,
            enum_id: self.enum_id,
            table_backed: self.table_backed,
        }
    }
}

/// The task's empirical rule, applied to a raw `a[2]`/def value: a value above
/// `0x10000` whose `f32` bit pattern is a finite non-zero number in a sane
/// range is a float; everything else (small raw, NaN/denormal patterns such as
/// the `u32::MAX` sentinel) is integer-encoded.
pub fn classify_raw(raw: u32) -> AttrKind {
    if raw > 0x1_0000 {
        let f = f32::from_bits(raw);
        if f.is_finite() && f != 0.0 && (1e-4..1e7).contains(&f.abs()) {
            return AttrKind::Float;
        }
    }
    AttrKind::Int
}

// ---------------------------------------------------------------------------
// display names

/// Localized display name for a def, when one can be resolved.
///
/// Tries the per-rarity `PickupTitle` first (falling back to the desc-level
/// `PickupTitle`) through `loc`. Armor name generators (`ItemNameCombineSelector`
/// chains) are not resolvable offline — the referenced `ItemNameLeafSelector`
/// objects live in the `base` package and their keys are not in the loc table —
/// so armor defs return `None` here and callers fall back to
/// [`pretty_name`].
pub fn localized_name(
    def_name: &str,
    rarity: Option<u32>,
    loc: &LocTable,
    db: &ItemDb,
) -> Option<String> {
    let def = db.get(def_name)?;
    let r = rarity.or(def.rarity);
    let title = r
        .and_then(|r| def.rarity_titles.get(r as usize))
        .map(String::as_str)
        .or(def.pickup_title.as_deref())?;
    loc.resolve(title).map(str::to_owned)
}

/// Best available name: localized title when resolvable, otherwise a
/// prettified def name.
pub fn display_name(def_name: &str, rarity: Option<u32>, loc: &LocTable, db: &ItemDb) -> String {
    localized_name(def_name, rarity, loc, db).unwrap_or_else(|| pretty_name(def_name))
}

/// Last-resort human-readable name from the def name
/// (`armor_feet_wanderer_d02` -> `Armor Feet Wanderer D02`).
pub fn pretty_name(def_name: &str) -> String {
    let base = def_name.rsplit('/').next().unwrap_or(def_name);
    base.split('_')
        .filter(|t| !t.is_empty())
        .map(|t| {
            let mut chars = t.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// payload helpers

fn value_of<'a>(props: &'a [Property], name: &str) -> Option<&'a Value> {
    props
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .map(|p| &p.value)
}

fn as_ref_str(v: &Value) -> Option<&str> {
    match v {
        Value::Ref(s) | Value::Str(s) => Some(s),
        _ => None,
    }
}

fn as_u32(v: &Value) -> Option<u32> {
    match v {
        Value::Int(v) => Some(*v),
        _ => None,
    }
}

fn as_array(v: &Value) -> Option<&[Value]> {
    match v {
        Value::Array(items) => Some(items),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_raw_follows_the_empirical_rule() {
        assert_eq!(classify_raw(7), AttrKind::Int);
        assert_eq!(classify_raw(150), AttrKind::Int);
        assert_eq!(classify_raw(0), AttrKind::Int);
        assert_eq!(classify_raw(u32::MAX), AttrKind::Int); // sentinel / NaN f32
        assert_eq!(classify_raw(1.0f32.to_bits()), AttrKind::Float);
        assert_eq!(classify_raw(0.5f32.to_bits()), AttrKind::Float);
        assert_eq!(classify_raw(-25i32 as u32), AttrKind::Int); // tiny denormal
    }

    #[test]
    fn pretty_name_titlecases() {
        assert_eq!(
            pretty_name("armor/armor_feet_wanderer_d02"),
            "Armor Feet Wanderer D02"
        );
        assert_eq!(pretty_name("weapon_scythe_x"), "Weapon Scythe X");
    }
}
