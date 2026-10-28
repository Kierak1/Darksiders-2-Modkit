use super::db::ItemDb;
use super::model::{
    Desc, Item, ItemBody, ItemList, ItemModifier, Modifier, RefBody, RefObject, Save, SaveItem,
};
use crate::Result;

// ---------------------------------------------------------------------------
// item access / editing

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ItemSource {
    Inventory,
    Equipped,
    Record,
}

#[derive(Debug, Clone, Copy)]
pub struct ItemRef {
    pub index: usize,
    pub source: ItemSource,
    pub source_name_idx: usize,
    pub list_idx: usize,
    pub item_idx: usize,
}

impl Save {
    pub fn all_lists(&self) -> Vec<(ItemSource, String, usize)> {
        let mut out = Vec::new();
        for l in &self.module.inventory {
            out.push((ItemSource::Inventory, l.name.clone(), l.items.len()));
        }
        for e in &self.module.equipped {
            out.push((
                ItemSource::Equipped,
                if e.s1.is_empty() {
                    format!("equip:{}", e.s2)
                } else {
                    format!("equip:{}", e.s1)
                },
                e.items.len(),
            ));
        }
        for r in &self.module.records {
            for l in &r.lists {
                out.push((
                    ItemSource::Record,
                    format!("{}:{}", r.name, l.name),
                    l.items.len(),
                ));
            }
        }
        out
    }

    pub fn items(&self) -> Vec<ItemRef> {
        let mut out = Vec::new();
        let mut idx = 0usize;
        for (li, l) in self.module.inventory.iter().enumerate() {
            for (ii, _) in l.items.iter().enumerate() {
                out.push(ItemRef {
                    index: idx,
                    source: ItemSource::Inventory,
                    source_name_idx: li,
                    list_idx: li,
                    item_idx: ii,
                });
                idx += 1;
            }
        }
        for (ei, e) in self.module.equipped.iter().enumerate() {
            for (ii, _) in e.items.iter().enumerate() {
                out.push(ItemRef {
                    index: idx,
                    source: ItemSource::Equipped,
                    source_name_idx: ei,
                    list_idx: ei,
                    item_idx: ii,
                });
                idx += 1;
            }
        }
        for (ri, r) in self.module.records.iter().enumerate() {
            for (li, l) in r.lists.iter().enumerate() {
                for (ii, _) in l.items.iter().enumerate() {
                    out.push(ItemRef {
                        index: idx,
                        source: ItemSource::Record,
                        source_name_idx: ri,
                        list_idx: li,
                        item_idx: ii,
                    });
                    idx += 1;
                }
            }
        }
        out
    }

    pub fn item(&self, index: usize) -> Option<&Item> {
        let r = self.items().into_iter().find(|r| r.index == index)?;
        match r.source {
            ItemSource::Inventory => Some(&self.module.inventory[r.list_idx].items[r.item_idx]),
            ItemSource::Equipped => Some(&self.module.equipped[r.list_idx].items[r.item_idx]),
            ItemSource::Record => {
                Some(&self.module.records[r.source_name_idx].lists[r.list_idx].items[r.item_idx])
            }
        }
    }

    pub fn item_mut(&mut self, index: usize) -> Option<&mut Item> {
        let r = self.items().into_iter().find(|r| r.index == index)?;
        match r.source {
            ItemSource::Inventory => Some(&mut self.module.inventory[r.list_idx].items[r.item_idx]),
            ItemSource::Equipped => Some(&mut self.module.equipped[r.list_idx].items[r.item_idx]),
            ItemSource::Record => Some(
                &mut self.module.records[r.source_name_idx].lists[r.list_idx].items[r.item_idx],
            ),
        }
    }

    /// Find the inventory (module+88) list with the given name.
    pub fn inventory_list(&mut self, name: &str) -> Option<&mut ItemList> {
        self.module
            .inventory
            .iter_mut()
            .find(|l| l.name.eq_ignore_ascii_case(name))
    }

    pub fn origin(&self, r: &ItemRef) -> String {
        match r.source {
            ItemSource::Inventory => self.module.inventory[r.list_idx].name.clone(),
            ItemSource::Equipped => {
                let e = &self.module.equipped[r.list_idx];
                format!("equip:{}", if e.s1.is_empty() { &e.s2 } else { &e.s1 })
            }
            ItemSource::Record => {
                let rec = &self.module.records[r.source_name_idx];
                format!("{}:{}", rec.name, rec.lists[r.list_idx].name)
            }
        }
    }

    pub fn remove_item(&mut self, index: usize) -> Option<Item> {
        let r = self.items().into_iter().find(|r| r.index == index)?;
        match r.source {
            ItemSource::Inventory => {
                Some(self.module.inventory[r.list_idx].items.remove(r.item_idx))
            }
            ItemSource::Equipped => Some(self.module.equipped[r.list_idx].items.remove(r.item_idx)),
            ItemSource::Record => Some(
                self.module.records[r.source_name_idx].lists[r.list_idx]
                    .items
                    .remove(r.item_idx),
            ),
        }
    }

    /// Item-granularity difference against another parse of the same save
    /// (e.g. its `.bak`). Lists are matched by name, their items by position
    /// inside the list, so inserting one item only diffs that list.
    pub fn diff(&self, other: &Save) -> Vec<String> {
        let mut out = Vec::new();
        for a in &self.module.inventory {
            match other.module.inventory.iter().find(|l| l.name == a.name) {
                Some(b) => list_diff(&mut out, &a.name, &a.items, &b.items),
                None => out.push(format!(
                    "list {}: {} item(s) here, list missing in the other save",
                    a.name,
                    a.items.len()
                )),
            }
        }
        for b in &other.module.inventory {
            if !self.module.inventory.iter().any(|l| l.name == b.name) {
                out.push(format!(
                    "list {}: missing here ({} item(s) in the other save)",
                    b.name,
                    b.items.len()
                ));
            }
        }
        for (i, (a, b)) in self
            .module
            .equipped
            .iter()
            .zip(other.module.equipped.iter())
            .enumerate()
        {
            list_diff(&mut out, &format!("equip #{i}"), &a.items, &b.items);
        }
        for a in &self.module.records {
            let Some(b) = other.module.records.iter().find(|r| r.name == a.name) else {
                continue;
            };
            for la in &a.lists {
                if let Some(lb) = b.lists.iter().find(|l| l.name == la.name) {
                    list_diff(
                        &mut out,
                        &format!("{}:{}", a.name, la.name),
                        &la.items,
                        &lb.items,
                    );
                }
            }
        }
        out
    }
}

fn list_diff(out: &mut Vec<String>, label: &str, a: &[Item], b: &[Item]) {
    if a.len() != b.len() {
        out.push(format!("{label}: {} -> {} item(s)", b.len(), a.len()));
    }
    for i in 0..a.len().max(b.len()) {
        match (a.get(i), b.get(i)) {
            (Some(x), Some(y)) if x == y => {}
            (Some(x), Some(y)) if x.name != y.name => out.push(format!(
                "{label}[{i}] {} -> {}",
                display_item_name(&y.name),
                display_item_name(&x.name)
            )),
            (Some(x), Some(_)) => out.push(format!("{label}[{i}] {}: fields changed", x.name)),
            (Some(x), None) => out.push(format!("{label}[{i}] + {} added", x.name)),
            (None, Some(y)) => out.push(format!("{label}[{i}] - {} removed", y.name)),
            (None, None) => {}
        }
    }
}

fn display_item_name(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

// ---------------------------------------------------------------------------
// add / move / equip
//
// `add_item` builds a new item body from an existing save item of the same def
// class family (verified variants: `EquippableItemDesc`/`WeaponDesc` -> save
// item, `ConsumableDesc` -> save item + u32 + u8). When the save holds no item
// of the family it falls back to the closest save-item body, and when it holds
// no save-item body at all it refuses.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyFamily {
    Save,
    SaveU32,
    SaveU32U8,
    Base,
    Modifier,
}

fn body_family(item: &Item) -> BodyFamily {
    match &item.body {
        ItemBody::Save(_) => BodyFamily::Save,
        ItemBody::SaveU32(_, _) => BodyFamily::SaveU32,
        ItemBody::SaveU32U8(_, _, _) => BodyFamily::SaveU32U8,
        ItemBody::Base(_, _) => BodyFamily::Base,
        ItemBody::Modifier(_) => BodyFamily::Modifier,
    }
}

fn desired_family(desc_class: &str) -> BodyFamily {
    match desc_class {
        "ConsumableDesc" => BodyFamily::SaveU32U8,
        _ => BodyFamily::Save,
    }
}

/// True when `item_name` may live in `container`, or when no def for the item
/// is known (nothing to validate against). Uses `ItemDef::container`.
pub fn container_fit(db: &ItemDb, item_name: &str, container: &str) -> bool {
    match db.get(item_name) {
        Some(def) => def
            .container
            .as_deref()
            .is_none_or(|c| c.eq_ignore_ascii_case(container)),
        None => true,
    }
}

/// Add an item for the def `def_name` to the inventory list `container_list`,
/// built from a template of the def's body family already present in the save
/// (an item with the exact same name is preferred, keeping its desc/f844/...).
/// Returns the new item's index in [`Save::items`].
///
/// `rarity` is validated against the def but cannot be stored: the save body
/// has no rarity field (the game derives rarity from the def). `level` is
/// written to `f872`, which only the level-bodied families (gear, ...) use.
pub fn add_item(
    save: &mut Save,
    db: Option<&ItemDb>,
    container_list: &str,
    def_name: &str,
    rarity: Option<u32>,
    level: Option<i64>,
) -> Result<usize> {
    let def = db.and_then(|d| d.get(def_name));
    let full_name = match &def {
        Some(d) => d
            .asset
            .as_deref()
            .map(|a| {
                if a.contains('/') {
                    a.to_string()
                } else {
                    format!("{a}/{}", d.name)
                }
            })
            .unwrap_or_else(|| d.name.clone()),
        None => {
            if def_name.contains('/') {
                def_name.to_string()
            } else {
                return Err(format!(
                    "unknown item def {def_name:?}: not in the item DB; pass a full <asset>/<name>"
                )
                .into());
            }
        }
    };
    if let Some(lv) = level {
        if !(0..=i64::from(u32::MAX)).contains(&lv) {
            return Err(format!("level {lv} does not fit in u32").into());
        }
    }
    if let (Some(d), Some(r)) = (&def, rarity) {
        if let Some(dr) = d.rarity {
            if dr != r {
                return Err(format!(
                    "def {} has rarity {dr}; the save body has no rarity field, so {r} cannot be stored",
                    d.name
                )
                .into());
            }
        }
    }
    let exact = save.items().into_iter().find_map(|r| {
        let item = save.item(r.index)?;
        (item.name == full_name).then(|| item.clone())
    });
    let want = def.as_ref().map(|d| desired_family(&d.desc_class));
    let mut best: Option<(u8, Item)> = None;
    if exact.is_none() {
        for r in save.items() {
            let Some(item) = save.item(r.index) else {
                continue;
            };
            let fam = body_family(item);
            if fam == BodyFamily::Modifier {
                continue;
            }
            // 0 = same def class, 1 = same body variant, 2 = plain save item,
            // 3 = any other variant.
            let rank = match (&def, want) {
                (Some(d), Some(w)) => {
                    let class_match = db
                        .and_then(|db| db.get(&item.name))
                        .is_some_and(|x| x.desc_class == d.desc_class);
                    if class_match {
                        0
                    } else if fam == w {
                        1
                    } else if fam == BodyFamily::Save {
                        2
                    } else {
                        3
                    }
                }
                _ => u8::from(fam != BodyFamily::Save),
            };
            match &best {
                Some((b, _)) if *b <= rank => {}
                _ => best = Some((rank, item.clone())),
            }
        }
    }
    let mut item = match exact {
        Some(item) => item,
        None => best
            .map(|(_, item)| item)
            .ok_or("save has no save-item body to use as a template")?,
    };
    let exact_hit = item.name == full_name;
    if !exact_hit {
        item.name = full_name;
        let visual = def.as_ref().and_then(|d| d.visual.clone());
        if let Some(s) = item.save_body_mut() {
            s.s864 = visual.unwrap_or_default();
            s.desc = None; // never inherit another def's possessed component
        }
    }
    if let Some(lv) = level {
        if let Some(s) = item.save_body_mut() {
            s.f872 = lv as u32;
        }
    }
    let li = save
        .module
        .inventory
        .iter()
        .position(|l| l.name.eq_ignore_ascii_case(container_list))
        .ok_or_else(|| {
            format!(
                "no inventory list named {container_list}; available: {}",
                list_names(save)
            )
        })?;
    let list = &mut save.module.inventory[li];
    list.items.push(item);
    let item_idx = list.items.len() - 1;
    index_of(save, ItemSource::Inventory, li, item_idx)
        .ok_or_else(|| "added item is not visible in the item index".into())
}

fn list_names(save: &Save) -> String {
    save.module
        .inventory
        .iter()
        .map(|l| l.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Find items by def name — `armor/armor_feet_wanderer_d02` or the bare
/// `armor_feet_wanderer_d02`, case-insensitive — optionally restricted to an
/// inventory list. Returns global indices in [`Save::items`] order.
pub fn find_items(save: &Save, def: &str, container: Option<&str>) -> Vec<usize> {
    let want = def.rsplit('/').next().unwrap_or(def).to_lowercase();
    save.items()
        .into_iter()
        .filter(|r| {
            let Some(item) = save.item(r.index) else {
                return false;
            };
            let name = item.name.rsplit('/').next().unwrap_or(&item.name);
            if name.to_lowercase() != want {
                return false;
            }
            match container {
                Some(c) => {
                    r.source == ItemSource::Inventory
                        && save.module.inventory[r.list_idx]
                            .name
                            .eq_ignore_ascii_case(c)
                }
                None => true,
            }
        })
        .map(|r| r.index)
        .collect()
}

/// Copy `item_index` from `src` into `dst`'s inventory list `container` as a
/// deep clone. The clone's `b816` flag is cleared (a copy is never equipped).
/// With a DB the def's `Container` is validated against the target list.
/// Returns the new item's index in `dst`.
pub fn copy_item(
    src: &Save,
    item_index: usize,
    dst: &mut Save,
    container_list: &str,
    db: Option<&ItemDb>,
) -> Result<usize> {
    let item = src
        .item(item_index)
        .ok_or_else(|| format!("no item at index {item_index}"))?;
    let name = item.name.clone();
    if let Some(db) = db {
        if !container_fit(db, &name, container_list) {
            let want = db
                .get(&name)
                .and_then(|d| d.container.clone())
                .unwrap_or_default();
            return Err(format!(
                "{name} belongs in {want:?}, not {container_list:?} (refusing the copy)"
            )
            .into());
        }
    }
    let mut item = item.clone();
    if let Some(s) = item.save_body_mut() {
        s.b816 = 0;
    }
    let li = dst
        .module
        .inventory
        .iter()
        .position(|l| l.name.eq_ignore_ascii_case(container_list))
        .ok_or_else(|| {
            format!(
                "no inventory list named {container_list}; available: {}",
                list_names(dst)
            )
        })?;
    let list = &mut dst.module.inventory[li];
    list.items.push(item);
    let item_idx = list.items.len() - 1;
    index_of(dst, ItemSource::Inventory, li, item_idx)
        .ok_or_else(|| "copied item is not visible in the item index".into())
}

pub(crate) fn index_of(
    save: &Save,
    source: ItemSource,
    list_idx: usize,
    item_idx: usize,
) -> Option<usize> {
    save.items()
        .into_iter()
        .find(|r| r.source == source && r.list_idx == list_idx && r.item_idx == item_idx)
        .map(|r| r.index)
}

/// Move an inventory item into another inventory list (`Boot` -> `BodyArmor`,
/// ...) and return its new index. With `db` the def's `Container` is enforced
/// (`container_fit`); without it only the list names are validated.
pub fn move_item(
    save: &mut Save,
    item_index: usize,
    target_container: &str,
    db: Option<&ItemDb>,
) -> Result<usize> {
    let r = save
        .items()
        .into_iter()
        .find(|r| r.index == item_index)
        .ok_or_else(|| format!("no item at index {item_index}"))?;
    if r.source != ItemSource::Inventory {
        return Err(
            "move_item only moves inventory items (use equip_item for equipped slots)".into(),
        );
    }
    let src_name = save.module.inventory[r.list_idx].name.clone();
    if src_name.eq_ignore_ascii_case(target_container) {
        return Err(format!("item {item_index} is already in {src_name}").into());
    }
    let item_name = save.module.inventory[r.list_idx].items[r.item_idx]
        .name
        .clone();
    if let Some(db) = db {
        if !container_fit(db, &item_name, target_container) {
            let def_c = db
                .get(&item_name)
                .and_then(|d| d.container.clone())
                .unwrap_or_default();
            return Err(format!(
                "{item_name} belongs in {def_c:?}, not {target_container:?} (refusing the move)"
            )
            .into());
        }
    }
    let ti = save
        .module
        .inventory
        .iter()
        .position(|l| l.name.eq_ignore_ascii_case(target_container))
        .ok_or_else(|| {
            format!(
                "no inventory list named {target_container}; available: {}",
                list_names(save)
            )
        })?;
    let item = save.module.inventory[r.list_idx].items.remove(r.item_idx);
    let list = &mut save.module.inventory[ti];
    list.items.push(item);
    let item_idx = list.items.len() - 1;
    index_of(save, ItemSource::Inventory, ti, item_idx)
        .ok_or_else(|| "moved item is not visible in the item index".into())
}

/// Equip an inventory item by flagging it in its container list.
///
/// Verified 2026-10-11 against all three saves: the equipped item of an
/// equipment container list (`Boot`, `Gauntlet`, `BodyArmor`, `Shoulder`,
/// `PrimaryWeapon`, `SecondaryWeapon`, `Talisman`, `Gear`, ...) is the single
/// item with `SaveItem.b816 == 1`; the `module+104` structure is the DLC Tome
/// delivery list, not gear. This clears `b816` on the list's other flagged
/// items and sets it on the target. The def's `Container` is validated against
/// the list (`container_fit`) when a DB is given. Returns the item's index.
pub fn equip_item(save: &mut Save, item_index: usize, db: Option<&ItemDb>) -> Result<usize> {
    let r = save
        .items()
        .into_iter()
        .find(|r| r.index == item_index)
        .ok_or_else(|| format!("no item at index {item_index}"))?;
    if r.source != ItemSource::Inventory {
        return Err("item is not in an inventory list".into());
    }
    let list_name = save.module.inventory[r.list_idx].name.clone();
    let list_field = save.module.inventory[r.list_idx].field;
    let item_name = save.module.inventory[r.list_idx].items[r.item_idx]
        .name
        .clone();
    if save.module.inventory[r.list_idx].items[r.item_idx]
        .save_body()
        .is_none()
    {
        return Err(format!("{item_name} has no save-item body to equip").into());
    }
    let def_container = db
        .and_then(|d| d.get(&item_name))
        .and_then(|def| def.container.clone());
    if let Some(db) = db {
        if !container_fit(db, &item_name, &list_name) {
            let want = def_container.clone().unwrap_or_default();
            return Err(format!(
                "{item_name} sits in {list_name} but its def wants {want:?} (refusing to equip)"
            )
            .into());
        }
    }
    let equipment_list = matches!(list_field, 21 | 22) || def_container.is_some();
    if !equipment_list {
        return Err(format!(
            "{list_name} is not an equipment container (list field {list_field}, def container {def_container:?})"
        )
        .into());
    }
    for (i, item) in save.module.inventory[r.list_idx]
        .items
        .iter_mut()
        .enumerate()
    {
        if let Some(s) = item.save_body_mut() {
            s.b816 = u8::from(i == r.item_idx);
        }
    }
    Ok(item_index)
}

/// Unequip an inventory item by clearing its `b816` equipped flag. The item is
/// named by its [`Save::items`] index; it must currently be equipped.
pub fn unequip_item(save: &mut Save, item_index: usize) -> Result<usize> {
    let r = save
        .items()
        .into_iter()
        .find(|r| r.index == item_index)
        .ok_or_else(|| format!("no item at index {item_index}"))?;
    if r.source != ItemSource::Inventory {
        return Err("item is not in an inventory list".into());
    }
    let item = &mut save.module.inventory[r.list_idx].items[r.item_idx];
    let name = item.name.clone();
    let body = item
        .save_body_mut()
        .ok_or_else(|| format!("{name} has no save-item body"))?;
    if body.b816 == 0 {
        return Err(format!("item {item_index} is not equipped (b816 already 0)").into());
    }
    body.b816 = 0;
    Ok(item_index)
}

// ---------------------------------------------------------------------------
// summaries and field editing

fn fmt_num(v: u32) -> String {
    let f = f32::from_bits(v);
    if v != 0 && v != u32::MAX && f.is_finite() && f.abs() >= 1e-4 && f.abs() < 1e7 {
        format!("{v} ({f})")
    } else {
        format!("{v}")
    }
}

pub fn modifier_summary(m: &Modifier) -> String {
    let extra = m.extra.map(|e| format!(" extra={e}")).unwrap_or_default();
    format!(
        "{} a=[{}] s=[{}]{}",
        if m.name.is_empty() {
            "(default)"
        } else {
            m.name.as_str()
        },
        m.a.iter()
            .map(|v| fmt_num(*v))
            .collect::<Vec<_>>()
            .join(", "),
        m.s.iter()
            .map(|s| if s.is_empty() {
                "\"\"".to_string()
            } else {
                s.clone()
            })
            .collect::<Vec<_>>()
            .join(", "),
        extra
    )
}

pub fn item_lines(item: &Item, indent: usize) -> Vec<String> {
    let pad = "  ".repeat(indent);
    let mut out = Vec::new();
    out.push(format!("{pad}{}", item.name));
    match &item.body {
        ItemBody::Base(b, v) => {
            out.push(format!("{pad}  base item {:?} {}", b, fmt_num(*v)));
        }
        ItemBody::Save(s) | ItemBody::SaveU32(s, _) | ItemBody::SaveU32U8(s, _, _) => {
            let tail = match &item.body {
                ItemBody::SaveU32(_, v) => format!(" u32={v}"),
                ItemBody::SaveU32U8(_, v, b) => format!(" u32={v} u8={b}"),
                _ => String::new(),
            };
            out.push(format!(
                "{pad}  b816={} f872={} f820={} f844={}{tail}",
                s.b816,
                fmt_num(s.f872),
                fmt_num(s.f820),
                fmt_num(s.f844)
            ));
            if !s.s856.is_empty() || !s.s864.is_empty() {
                out.push(format!("{pad}  s856={:?} s864={:?}", s.s856, s.s864));
            }
            if let Some(desc) = &s.desc {
                match desc {
                    Desc::PossessedComponent { a, b, c, entries } => out.push(format!(
                        "{pad}  desc PossessedItemComponent a={a} b={b} c={c} entries={}",
                        entries.len()
                    )),
                }
            }
            for nested in &s.nested {
                if let ItemBody::Modifier(m) = &nested.body {
                    out.push(format!(
                        "{pad}  stat {} ({} mods, {} effects)",
                        m.attr,
                        m.mods.len(),
                        m.effects.len()
                    ));
                    for md in &m.mods {
                        out.push(format!("{pad}    {}", modifier_summary(md)));
                    }
                    for (i, e) in m.effects.iter().enumerate() {
                        out.push(format!(
                            "{pad}    effect[{i}] {} {}",
                            e.obj.name,
                            fmt_num(e.f[0])
                        ));
                    }
                } else {
                    out.extend(item_lines(nested, indent + 2));
                }
            }
        }
        ItemBody::Modifier(m) => {
            out.push(format!(
                "{pad}  ItemModifier attr={} fields={:?}",
                m.attr,
                m.fields.map(fmt_num)
            ));
            for md in &m.mods {
                out.push(format!("{pad}    {}", modifier_summary(md)));
            }
            for (i, e) in m.effects.iter().enumerate() {
                out.push(format!("{pad}    effect[{i}] {}", e.obj.name));
            }
        }
    }
    out
}

fn parse_num(s: &str) -> Result<u32> {
    if let Some(hex) = s.strip_prefix("0x") {
        return Ok(u32::from_str_radix(hex, 16)?);
    }
    if let Some(f) = s.strip_prefix("f:") {
        let v = f.parse::<f32>()?;
        if !v.is_finite() {
            return Err(format!("{s:?} is not a finite float").into());
        }
        return Ok(v.to_bits());
    }
    if let Ok(v) = s.parse::<u32>() {
        return Ok(v);
    }
    if let Ok(v) = s.parse::<i32>() {
        return Ok(v as u32);
    }
    Err(format!("bad numeric value {s:?}").into())
}

fn parse_u8(s: &str) -> Result<u8> {
    let v = parse_num(s)?;
    u8::try_from(v).map_err(|_| format!("{v} does not fit in a byte").into())
}

fn seg(part: &str) -> (&str, Option<usize>) {
    match part.find('[') {
        Some(open) => {
            let close = part.rfind(']').unwrap_or(part.len());
            let idx = part[open + 1..close].parse().ok();
            (&part[..open], idx)
        }
        None => (part, None),
    }
}

/// Edit one field of an item addressed by a dotted path
/// (`f872`, `s864`, `mod[0].mods[1].a[2]`, `nested[0].f872`, ...).
/// Returns a human-readable description of the change.
pub fn set_item_field(item: &mut Item, path: &str, value: &str) -> Result<String> {
    let parts: Vec<&str> = path.split('.').filter(|p| !p.is_empty()).collect();
    let (head, tail) = parts.split_first().ok_or("empty field path")?;
    let (name, idx) = seg(head);
    match name {
        "name" => {
            let old = item.name.clone();
            item.name = value.to_string();
            Ok(format!("name {old:?} -> {:?}", item.name))
        }
        "nested" => {
            let i = idx.ok_or("nested[<i>] requires an index")?;
            let (_, rest) = parts.split_first().unwrap();
            let child = item
                .nested_item_mut(i)
                .ok_or_else(|| format!("no nested item at index {i}"))?;
            set_item_field(child, &rest.join("."), value)
        }
        _ => match &mut item.body {
            ItemBody::Save(s) | ItemBody::SaveU32(s, _) | ItemBody::SaveU32U8(s, _, _) => {
                set_save_field(s, name, idx, tail, value)
            }
            ItemBody::Base(b, v) => match name {
                "a" => {
                    let i = idx.ok_or("a[<i>] requires an index")?;
                    if i > 2 {
                        return Err("base byte index out of range".into());
                    }
                    let old = b[i];
                    b[i] = parse_u8(value)?;
                    Ok(format!("a[{i}] {old} -> {}", b[i]))
                }
                "v" => {
                    let old = *v;
                    *v = parse_num(value)?;
                    Ok(format!("v {old} -> {v}"))
                }
                other => Err(format!("unknown base-item field {other:?}").into()),
            },
            ItemBody::Modifier(m) => set_modifier_field(m, name, idx, tail, value),
        },
    }
}

impl Item {
    pub fn nested_item_mut(&mut self, i: usize) -> Option<&mut Item> {
        match &mut self.body {
            ItemBody::Save(s) | ItemBody::SaveU32(s, _) | ItemBody::SaveU32U8(s, _, _) => {
                s.nested.get_mut(i)
            }
            _ => None,
        }
    }
}

fn set_save_field(
    s: &mut SaveItem,
    name: &str,
    idx: Option<usize>,
    tail: &[&str],
    value: &str,
) -> Result<String> {
    match name {
        "b816" => {
            let old = s.b816;
            s.b816 = parse_u8(value)?;
            Ok(format!("b816 {old} -> {}", s.b816))
        }
        "f872" | "f820" | "f844" => {
            let slot = match name {
                "f872" => &mut s.f872,
                "f820" => &mut s.f820,
                _ => &mut s.f844,
            };
            let old = *slot;
            *slot = parse_num(value)?;
            Ok(format!("{name} {old} -> {slot}"))
        }
        "s856" | "s864" => {
            let slot = if name == "s856" {
                &mut s.s856
            } else {
                &mut s.s864
            };
            let old = std::mem::replace(slot, value.to_string());
            Ok(format!("{name} {old:?} -> {:?}", slot))
        }
        "desc" => match &mut s.desc {
            Some(Desc::PossessedComponent { a, b, c, .. }) => {
                let slot = match tail.first().copied() {
                    Some("a") => a,
                    Some("b") => b,
                    Some("c") => c,
                    other => {
                        return Err(format!("unknown desc field {other:?}").into());
                    }
                };
                let old = *slot;
                *slot = parse_num(value)?;
                Ok(format!("desc.{} {old} -> {slot}", tail[0]))
            }
            None => Err("item has no desc component".into()),
        },
        "mod" | "stat" => {
            let i = idx.ok_or("mod[<i>] requires an index")?;
            let mut iter = s.nested.iter_mut().filter_map(|n| match &mut n.body {
                ItemBody::Modifier(m) => Some(m),
                _ => None,
            });
            let m = iter
                .nth(i)
                .ok_or_else(|| format!("no stat (ItemModifier) at index {i}"))?;
            let (head, tail) = tail.split_first().ok_or("missing field after mod[i]")?;
            let (fname, fidx) = seg(head);
            set_modifier_field(m, fname, fidx, tail, value)
        }
        other => Err(format!("unknown save-item field {other:?}").into()),
    }
}

fn set_modifier_field(
    m: &mut ItemModifier,
    name: &str,
    idx: Option<usize>,
    tail: &[&str],
    value: &str,
) -> Result<String> {
    match name {
        "attr" => {
            let old = std::mem::replace(&mut m.attr, value.to_string());
            Ok(format!("attr {old:?} -> {:?}", m.attr))
        }
        "f" => {
            let i = idx.ok_or("f[<i>] requires an index")?;
            let slot = m
                .fields
                .get_mut(i)
                .ok_or_else(|| format!("field index {i} out of range"))?;
            let old = *slot;
            *slot = parse_num(value)?;
            Ok(format!("f[{i}] {old} -> {slot}"))
        }
        "mods" => {
            let i = idx.ok_or("mods[<i>] requires an index")?;
            let md = m
                .mods
                .get_mut(i)
                .ok_or_else(|| format!("no modifier at index {i}"))?;
            let (head, rest) = tail.split_first().ok_or("missing field after mods[i]")?;
            let (fname, fidx) = seg(head);
            set_plain_modifier(md, fname, fidx, rest, value)
        }
        "effects" => {
            let i = idx.ok_or("effects[<i>] requires an index")?;
            let e = m
                .effects
                .get_mut(i)
                .ok_or_else(|| format!("no effect at index {i}"))?;
            let (head, rest) = tail.split_first().ok_or("missing field after effects[i]")?;
            let (fname, fidx) = seg(head);
            match fname {
                "b" => {
                    let old = e.b;
                    e.b = parse_u8(value)?;
                    Ok(format!("effect.b {old} -> {}", e.b))
                }
                "f" => {
                    let k = fidx.ok_or("f[<i>] requires an index")?;
                    let slot =
                        e.f.get_mut(k)
                            .ok_or_else(|| format!("effect field index {k} out of range"))?;
                    let old = *slot;
                    *slot = parse_num(value)?;
                    Ok(format!("effect.f[{k}] {old} -> {slot}"))
                }
                "obj" => set_ref_object_field(&mut e.obj, rest, value),
                other => Err(format!("unknown effect field {other:?}").into()),
            }
        }
        other => Err(format!("unknown modifier field {other:?}").into()),
    }
}

fn set_plain_modifier(
    md: &mut Modifier,
    name: &str,
    idx: Option<usize>,
    tail: &[&str],
    value: &str,
) -> Result<String> {
    match name {
        "a" => {
            let i = idx.ok_or("a[<i>] requires an index")?;
            let slot =
                md.a.get_mut(i)
                    .ok_or_else(|| format!("a index {i} out of range"))?;
            let old = *slot;
            *slot = parse_num(value)?;
            Ok(format!("a[{i}] {old} -> {slot}"))
        }
        "s" => {
            let i = idx.ok_or("s[<i>] requires an index")?;
            let slot =
                md.s.get_mut(i)
                    .ok_or_else(|| format!("s index {i} out of range"))?;
            let old = std::mem::replace(slot, value.to_string());
            Ok(format!("s[{i}] {old:?} -> {value:?}"))
        }
        "extra" => {
            let old = md.extra;
            md.extra = Some(parse_num(value)?);
            Ok(format!("extra {old:?} -> {:?}", md.extra))
        }
        "name" => {
            let old = std::mem::replace(&mut md.name, value.to_string());
            Ok(format!("name {old:?} -> {value:?}"))
        }
        _ => {
            let _ = tail;
            Err(format!("unknown modifier field {name:?}").into())
        }
    }
}

fn set_ref_object_field(obj: &mut RefObject, tail: &[&str], value: &str) -> Result<String> {
    let (head, rest) = tail.split_first().ok_or("missing ref-object field")?;
    let (name, idx) = seg(head);
    match name {
        "fields" => match &mut obj.body {
            RefBody::Combat(_, fields, _, _) => {
                let i = idx.ok_or("fields[<i>] requires an index")?;
                let slot = fields
                    .get_mut(i)
                    .ok_or_else(|| format!("fields index {i} out of range"))?;
                let old = *slot;
                *slot = parse_num(value)?;
                Ok(format!("obj.fields[{i}] {old} -> {slot}"))
            }
            _ => Err("this effect has no fields".into()),
        },
        "b" => match &mut obj.body {
            RefBody::Combat(_, _, b, _) | RefBody::Enhancement(_, b, _) => {
                let old = *b;
                *b = parse_u8(value)?;
                Ok(format!("obj.b {old} -> {b}"))
            }
            _ => Err("this effect has no b flag".into()),
        },
        "s" => match &mut obj.body {
            RefBody::Enhancement(_, _, s) => {
                let old = std::mem::replace(s, value.to_string());
                Ok(format!("obj.s {old:?} -> {value:?}"))
            }
            _ => Err("this effect has no string".into()),
        },
        "v" => match &mut obj.body {
            RefBody::Freeze(_, v) => {
                let old = *v;
                *v = parse_num(value)?;
                Ok(format!("obj.v {old} -> {v}"))
            }
            _ => Err("this effect has no v".into()),
        },
        "mods" => match &mut obj.body {
            RefBody::Combat(_, _, _, mods) => {
                let i = idx.ok_or("mods[<i>] requires an index")?;
                let md = mods
                    .get_mut(i)
                    .ok_or_else(|| format!("no modifier at index {i}"))?;
                let (head, rest2) = rest.split_first().ok_or("missing field after mods[i]")?;
                let (fname, fidx) = seg(head);
                set_plain_modifier(md, fname, fidx, rest2, value)
            }
            _ => Err("this effect has no modifier list".into()),
        },
        "refs" => match &mut obj.body {
            RefBody::Script(base)
            | RefBody::Combat(base, _, _, _)
            | RefBody::Enhancement(base, _, _)
            | RefBody::Freeze(base, _) => {
                let i = idx.ok_or("refs[<i>] requires an index")?;
                let child = base
                    .refs
                    .get_mut(i)
                    .ok_or_else(|| format!("no ref at index {i}"))?;
                set_ref_object_field(child, rest, value)
            }
        },
        "flag" => match &mut obj.body {
            RefBody::Script(base)
            | RefBody::Combat(base, _, _, _)
            | RefBody::Enhancement(base, _, _)
            | RefBody::Freeze(base, _) => {
                let old = base.flag;
                base.flag = parse_u8(value)?;
                Ok(format!("obj.flag {old} -> {}", base.flag))
            }
        },
        other => Err(format!("unknown ref-object field {other:?}").into()),
    }
}
