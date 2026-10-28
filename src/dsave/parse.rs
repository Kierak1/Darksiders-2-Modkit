use crate::cursor::Cursor;
use crate::Result;

use super::model::{
    Desc, Ef300, EquipSlot, Gc48, Info, Item, ItemBody, ItemList, ItemModifier, LeafValue,
    MapEntry, Mod80, Modifier, Module, ObjChild, ObjElem, ObjLeaf, Object, Record, RefBody,
    RefObject, Save, SaveItem, ScriptBase, Slot80, State96, V1Inner, V1Record, FILE_VERSION, MAGIC,
    MODULE_VERSION, RAW_BLOCK,
};

// ---------------------------------------------------------------------------
// reader

pub(crate) struct Reader<'a> {
    pub(crate) c: Cursor<'a>,
    table: Vec<String>,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self {
            c: Cursor::new(data),
            table: Vec::new(),
        }
    }

    fn str(&mut self) -> Result<String> {
        let at = self.c.pos;
        let n = self.c.u16()?;
        if n >= 0x100 {
            let idx = (n - 0x100) as usize;
            return self
                .table
                .get(idx)
                .cloned()
                .ok_or_else(|| format!("string index {idx} out of range at {at:#x}").into());
        }
        let bytes = self.c.take(n as usize)?;
        let s = String::from_utf8_lossy(bytes).into_owned();
        self.table.push(s.clone());
        Ok(s)
    }

    fn name_token_valid(&self) -> bool {
        let at = self.c.pos;
        let data = self.c.data;
        if at + 2 > data.len() {
            return false;
        }
        let n = u16::from_le_bytes([data[at], data[at + 1]]);
        if n >= 0x100 {
            return ((n - 0x100) as usize) < self.table.len();
        }
        if n == 0 || at + 2 + n as usize > data.len() {
            return false;
        }
        data[at + 2..at + 2 + n as usize]
            .iter()
            .all(|b| (9..=126).contains(b))
    }

    fn savepoint(&self) -> (usize, usize) {
        (self.c.pos, self.table.len())
    }

    fn restore(&mut self, sp: (usize, usize)) {
        self.c.pos = sp.0;
        self.table.truncate(sp.1);
    }
}

fn map_entries(r: &mut Reader, flag: bool) -> Result<Vec<MapEntry>> {
    let n = r.c.u16()? as usize;
    let mut out = Vec::with_capacity((n).min(65536));
    for _ in 0..n {
        let hash = r.c.u64()?;
        let name = r.str()?;
        let f = if flag { Some(r.c.u8()?) } else { None };
        out.push(MapEntry::new(hash, name, f));
    }
    Ok(out)
}

fn parse_object(r: &mut Reader) -> Result<Object> {
    let class = r.str()?;
    let props = map_entries(r, true)?;
    let n = r.c.u32()? as usize;
    let mut elems = Vec::with_capacity((n).min(65536));
    for _ in 0..n {
        let kind = r.c.u16()?;
        let c1 = r.c.u32()? as usize;
        let mut children = Vec::with_capacity((c1).min(65536));
        for _ in 0..c1 {
            let ck = r.c.u16()?;
            let c2 = r.c.u32()? as usize;
            let mut leaves = Vec::with_capacity((c2).min(65536));
            for _ in 0..c2 {
                let data = r.c.u32()?;
                let tag = r.c.u8()?;
                let value = match tag {
                    0 => LeafValue::None,
                    1 => LeafValue::Two(r.c.u8()?, r.c.u8()?),
                    2 => {
                        let b1 = r.c.u8()? != 0;
                        let v1 = if b1 {
                            Some([r.c.u32()?, r.c.u32()?, r.c.u32()?])
                        } else {
                            None
                        };
                        let b2 = r.c.u8()? != 0;
                        let v2 = if b2 {
                            Some([r.c.u32()?, r.c.u32()?, r.c.u32()?])
                        } else {
                            None
                        };
                        LeafValue::Pair(v1, v2)
                    }
                    other => return Err(format!("unknown object leaf tag {other}").into()),
                };
                let props = map_entries(r, false)?;
                leaves.push(ObjLeaf {
                    data,
                    tag,
                    value,
                    props,
                });
            }
            children.push(ObjChild { kind: ck, leaves });
        }
        elems.push(ObjElem { kind, children });
    }
    Ok(Object {
        class,
        props,
        elems,
    })
}

fn parse_modifier(r: &mut Reader, name: String) -> Result<Modifier> {
    let a = [r.c.u32()?, r.c.u32()?, r.c.u32()?];
    let s = [r.str()?, r.str()?, r.str()?, r.str()?];
    let extra = if name == "PossessedAttributeModifier" {
        Some(r.c.u32()?)
    } else if name == "AttributeModifier" {
        None
    } else {
        return Err(format!("unknown modifier class {name:?}").into());
    };
    Ok(Modifier { name, a, s, extra })
}

fn parse_script_base(r: &mut Reader) -> Result<ScriptBase> {
    let flag = r.c.u8()?;
    let mut refs = Vec::new();
    if flag == 1 {
        let n = r.c.u32()? as usize;
        for _ in 0..n {
            refs.push(parse_ref_object(r)?);
        }
    }
    Ok(ScriptBase { flag, refs })
}

fn parse_ref_object(r: &mut Reader) -> Result<RefObject> {
    let name = r.str()?;
    let body = match name.as_str() {
        "CombatEffect" => RefBody::Script(parse_script_base(r)?),
        "ApplyDamageCEffect" => {
            let base = parse_script_base(r)?;
            let mut fields = [0u32; 9];
            for f in &mut fields {
                *f = r.c.u32()?;
            }
            let b = r.c.u8()?;
            let n = r.c.u32()? as usize;
            let mut mods = Vec::with_capacity((n).min(65536));
            for _ in 0..n {
                let m = parse_nameless_modifier(r)?;
                mods.push(m);
            }
            RefBody::Combat(base, fields, b, mods)
        }
        "EnhancementEffect" => {
            let base = parse_script_base(r)?;
            let b = r.c.u8()?;
            let s = r.str()?;
            RefBody::Enhancement(base, b, s)
        }
        "FreezeCEffect" => {
            let base = parse_script_base(r)?;
            let v = r.c.u32()?;
            RefBody::Freeze(base, v)
        }
        other => return Err(format!("unknown ref object class {other:?}").into()),
    };
    Ok(RefObject { name, body })
}

fn parse_nameless_modifier(r: &mut Reader) -> Result<Modifier> {
    Ok(Modifier {
        name: String::new(),
        a: [r.c.u32()?, r.c.u32()?, r.c.u32()?],
        s: [r.str()?, r.str()?, r.str()?, r.str()?],
        extra: None,
    })
}

fn parse_gc48(r: &mut Reader) -> Result<Gc48> {
    let b = r.c.u8()?;
    let f = [r.c.u32()?, r.c.u32()?, r.c.u32()?, r.c.u32()?];
    let obj = parse_ref_object(r)?;
    Ok(Gc48 { b, f, obj })
}

fn parse_item_modifier(r: &mut Reader) -> Result<ItemModifier> {
    let base = (read3(r)?, r.c.u32()?);
    let attr = r.str()?;
    let mut fields = [0u32; 5];
    for f in &mut fields {
        *f = r.c.u32()?;
    }
    let n1 = r.c.u32()? as usize;
    let mut mods = Vec::with_capacity((n1).min(65536));
    for _ in 0..n1 {
        let name = r.str()?;
        mods.push(parse_modifier(r, name)?);
    }
    let n2 = r.c.u32()? as usize;
    let mut effects = Vec::with_capacity((n2).min(65536));
    for _ in 0..n2 {
        effects.push(parse_gc48(r)?);
    }
    Ok(ItemModifier {
        base,
        attr,
        fields,
        mods,
        effects,
    })
}

fn read3(r: &mut Reader) -> Result<[u8; 3]> {
    Ok([r.c.u8()?, r.c.u8()?, r.c.u8()?])
}

fn parse_save_body(r: &mut Reader) -> Result<SaveItem> {
    let base = (read3(r)?, r.c.u32()?);
    let b816 = r.c.u8()?;
    let f872 = r.c.u32()?;
    let f820 = r.c.u32()?;
    let f844 = r.c.u32()?;
    let s856 = r.str()?;
    let s864 = r.str()?;
    let n = r.c.u32()? as usize;
    let mut nested = Vec::with_capacity((n).min(65536));
    for _ in 0..n {
        nested.push(parse_item(r)?);
    }
    let has_desc = r.c.u8()?;
    let desc = if has_desc != 0 {
        let descname = r.str()?;
        match descname.as_str() {
            "PossessedItemComponent" => {
                let a = r.c.u32()?;
                let b = r.c.u32()?;
                let c = r.c.u32()?;
                let n = r.c.u32()? as usize;
                let mut entries = Vec::with_capacity((n).min(65536));
                for _ in 0..n {
                    entries.push([r.c.u32()?, r.c.u32()?, r.c.u32()?]);
                }
                Some(Desc::PossessedComponent { a, b, c, entries })
            }
            other => return Err(format!("unknown item desc class {other:?}").into()),
        }
    } else {
        None
    };
    Ok(SaveItem {
        base,
        b816,
        f872,
        f820,
        f844,
        s856,
        s864,
        nested,
        desc,
    })
}

pub(crate) fn parse_item(r: &mut Reader) -> Result<Item> {
    let sp = r.savepoint();
    let name = r.str()?;
    if name == "ItemModifier" {
        return Ok(Item {
            name,
            body: ItemBody::Modifier(Box::new(parse_item_modifier(r)?)),
        });
    }
    // Try the payload variants in order; the first that either validates or is
    // the last candidate wins (mirrors the runtime reader's class dispatch).
    let variants: [fn(&mut Reader) -> Result<ItemBody>; 4] = [
        |r| Ok(ItemBody::Save(Box::new(parse_save_body(r)?))),
        |r| {
            let b = (read3(r)?, r.c.u32()?);
            Ok(ItemBody::Base(b.0, b.1))
        },
        |r| {
            let s = parse_save_body(r)?;
            Ok(ItemBody::SaveU32(Box::new(s), r.c.u32()?))
        },
        |r| {
            let s = parse_save_body(r)?;
            Ok(ItemBody::SaveU32U8(Box::new(s), r.c.u32()?, r.c.u8()?))
        },
    ];
    let mut last_err = None;
    for (i, variant) in variants.iter().enumerate() {
        r.restore(sp);
        let n2 = r.str()?;
        debug_assert_eq!(n2, name);
        match variant(r) {
            Ok(body) => {
                if i < variants.len() - 1 && !r.name_token_valid() {
                    last_err = Some(format!("variant {i} left an invalid token"));
                    continue;
                }
                return Ok(Item { name, body });
            }
            Err(e) => last_err = Some(e.to_string()),
        }
    }
    r.restore(sp);
    Err(format!("no item variant matched {name:?}: {last_err:?}").into())
}

fn parse_item_list(r: &mut Reader) -> Result<ItemList> {
    let name = r.str()?;
    let field = r.c.u32()?;
    let n = r.c.u32()? as usize;
    let mut items = Vec::with_capacity((n).min(65536));
    for _ in 0..n {
        items.push(parse_item(r)?);
    }
    Ok(ItemList { name, field, items })
}

fn parse_equip_slot(r: &mut Reader) -> Result<EquipSlot> {
    let f1 = r.c.u32()?;
    let f2 = r.c.u32()?;
    let b24 = r.c.u8()?;
    let f32 = r.c.u64()?;
    let b40 = r.c.u8()?;
    let b41 = if f1 >= 1 { Some(r.c.u8()?) } else { None };
    let s1 = r.str()?;
    let s2 = r.str()?;
    let s3 = r.str()?;
    let n = r.c.u32()? as usize;
    let mut items = Vec::with_capacity((n).min(65536));
    for _ in 0..n {
        items.push(parse_item(r)?);
    }
    Ok(EquipSlot {
        f1,
        f2,
        b24,
        f32,
        b40,
        b41,
        s1,
        s2,
        s3,
        items,
    })
}

fn parse_ef300(r: &mut Reader) -> Result<Ef300> {
    let name = r.str()?;
    let n = r.c.u32()? as usize;
    let mut rows = Vec::with_capacity((n).min(65536));
    for _ in 0..n {
        let s = r.str()?;
        rows.push((s, r.c.u8()?, r.c.u8()?, r.c.u32()?, r.c.u32()?));
    }
    Ok(Ef300 { name, rows })
}

fn parse_slot80(r: &mut Reader) -> Result<Slot80> {
    let a = r.c.u32()?;
    let name = r.str()?;
    let n = r.c.u32()? as usize;
    let mut rows = Vec::with_capacity((n).min(65536));
    for _ in 0..n {
        rows.push((r.c.u64()?, r.c.u32()?));
    }
    Ok(Slot80 { a, name, rows })
}

fn parse_mod80(r: &mut Reader) -> Result<Mod80> {
    let version = r.c.u32()?;
    let mut a = [0u32; 27];
    for v in &mut a {
        *v = r.c.u32()?;
    }
    let v1 = if version == 1 { Some(r.c.u32()?) } else { None };
    let f1 = r.c.u32()?;
    let mut f124 = None;
    let mut f132 = None;
    if version >= 3 {
        if version >= 4 {
            f124 = Some(r.c.u32()?);
        }
        f132 = Some(r.c.u32()?);
    }
    let mut b = [0u32; 23];
    for v in &mut b {
        *v = r.c.u32()?;
    }
    let mut c1 = None;
    let mut c2 = None;
    if version < 3 {
        c1 = Some(r.c.u32()?);
    } else {
        c2 = Some((r.c.u32()?, r.c.u32()?));
    }
    let mut d = [0u32; 4];
    for v in &mut d {
        *v = r.c.u32()?;
    }
    let mut slots = Vec::with_capacity(5);
    for _ in 0..5 {
        slots.push(parse_slot80(r)?);
    }
    let u16_tail = if version >= 3 { Some(r.c.u16()?) } else { None };
    Ok(Mod80 {
        version,
        a,
        v1,
        f1,
        f124,
        f132,
        b,
        c1,
        c2,
        d,
        slots,
        u16_tail,
    })
}

fn parse_module(r: &mut Reader) -> Result<Module> {
    let version = r.c.u32()?;
    if version != MODULE_VERSION {
        return Err(
            format!("unsupported module version {version} (expected {MODULE_VERSION})").into(),
        );
    }
    let name = r.str()?;
    let n1 = r.c.u32()? as usize;
    let mut loc_records = Vec::with_capacity((n1).min(65536));
    for _ in 0..n1 {
        loc_records.push([r.str()?, r.str()?, r.str()?, r.str()?, r.str()?]);
    }
    let f32v = r.c.u32()?;
    let f40 = r.str()?;
    let f48 = r.c.u32()?;
    let f52 = r.c.u32()?;
    let f56 = r.c.u32()?;
    let f60 = r.c.u32()?;
    let f64 = r.c.u32()?;
    let f68 = r.c.u32()?;
    let f72 = r.c.u32()?;
    let _zero = r.c.u32()?;
    let n2 = r.c.u32()? as usize;
    let mut objects = Vec::with_capacity((n2).min(65536));
    for _ in 0..n2 {
        objects.push(parse_object(r)?);
    }
    let v1_first = r.c.u32()?;
    let n3 = r.c.u32()? as usize;
    let mut v1 = Vec::with_capacity((n3).min(65536));
    for _ in 0..n3 {
        let name = r.str()?;
        let inner_n = r.c.u32()? as usize;
        let mut inner = Vec::with_capacity((inner_n).min(65536));
        for _ in 0..inner_n {
            inner.push(V1Inner {
                name: r.str()?,
                a: r.c.u16()?,
                b: r.c.u16()?,
                c: r.c.u32()?,
                d: r.c.u32()?,
                e: r.c.u32()?,
                f: r.c.u32()?,
                g: r.str()?,
                h: r.c.u32()?,
                i: r.c.u32()?,
                j: r.c.u32()?,
                k: r.c.u8()?,
            });
        }
        v1.push(V1Record { name, inner });
    }
    let n4 = r.c.u32()? as usize;
    let mut n3v = Vec::with_capacity((n4).min(65536));
    for _ in 0..n4 {
        n3v.push((r.str()?, r.c.u8()?, r.c.u8()?, r.c.u8()?));
    }
    let n5 = r.c.u32()? as usize;
    let mut n4v = Vec::with_capacity((n5).min(65536));
    for _ in 0..n5 {
        n4v.push((r.c.u32()?, r.c.u8()?, r.c.u8()?));
    }
    let n6 = r.c.u32()? as usize;
    let mut n5v = Vec::with_capacity((n6).min(65536));
    for _ in 0..n6 {
        n5v.push(r.str()?);
    }
    let inv_n = r.c.u32()? as usize;
    let mut inventory = Vec::with_capacity((inv_n).min(65536));
    for _ in 0..inv_n {
        inventory.push(parse_item_list(r)?);
    }
    let equip_marker = r.c.u32()?;
    let eq_n = r.c.u32()? as usize;
    let mut equipped = Vec::with_capacity((eq_n).min(65536));
    for _ in 0..eq_n {
        equipped.push(parse_equip_slot(r)?);
    }
    let rec_n = r.c.u32()? as usize;
    let mut records = Vec::with_capacity((rec_n).min(65536));
    for _ in 0..rec_n {
        let name = r.str()?;
        let a = r.c.u32()?;
        let b = r.c.u32()?;
        let list_n = r.c.u32()? as usize;
        let mut lists = Vec::with_capacity((list_n).min(65536));
        for _ in 0..list_n {
            lists.push(parse_item_list(r)?);
        }
        records.push(Record { name, a, b, lists });
    }
    let s96name = r.str()?;
    let s96a_n = r.c.u32()? as usize;
    let mut s96a = Vec::with_capacity((s96a_n).min(65536));
    for _ in 0..s96a_n {
        s96a.push(parse_ef300(r)?);
    }
    let s96b_n = r.c.u32()? as usize;
    let mut s96b = Vec::with_capacity((s96b_n).min(65536));
    for _ in 0..s96b_n {
        s96b.push(parse_ef300(r)?);
    }
    let raw = r.c.take(RAW_BLOCK)?.to_vec();
    let s80 = parse_mod80(r)?;
    let mut tail = Vec::with_capacity(4);
    for _ in 0..4 {
        let n = r.c.u32()? as usize;
        let mut v = Vec::with_capacity((n).min(65536));
        for _ in 0..n {
            v.push(r.c.u64()?);
        }
        tail.push(v);
    }
    let last = r.c.u8()?;
    Ok(Module {
        version,
        name,
        loc_records,
        f32: f32v,
        f40,
        f48,
        f52,
        f56,
        f60,
        f64,
        f68,
        f72,
        objects,
        v1_first,
        v1,
        n3: n3v,
        n4: n4v,
        n5: n5v,
        inventory,
        equip_marker,
        equipped,
        records,
        state96: State96 {
            name: s96name,
            a: s96a,
            b: s96b,
        },
        raw,
        s80,
        tail,
        last,
    })
}

pub fn parse(data: &[u8]) -> Result<Save> {
    if data.len() < 9 || &data[..4] != MAGIC {
        return Err("not a DSAV save file".into());
    }
    let mut c = Cursor::new(data);
    c.skip(4)?;
    let endian = c.u8()?;
    let version = c.u32()?;
    if !(0x8..=FILE_VERSION).contains(&version) {
        return Err(format!("unsupported save version {version:#x}").into());
    }
    if version < 0xA {
        c.skip(4)?;
    }
    let info = Info {
        format: c.u8()?,
        endian: c.u8()?,
        b17: c.u8()?,
        b18: c.u8()?,
        b19: c.u8()?,
        zero: c.u8()?,
        f20: c.u32()?,
        f24: c.u32()?,
        f28: c.u32()?,
        f36: c.u32()?,
        f40: c.u32()?,
        f44: c.u32()?,
        f48: c.u32()?,
        counter: c.u32()?,
        global_version: c.u32()?,
        flags: c.u32()?,
        b56: c.u8()?,
    };
    let mut strings = Vec::new();
    if version >= 0xD {
        let n = c.u32()? as usize;
        for _ in 0..n {
            let tag = c.u8()?;
            if tag != 0xFF {
                return Err(format!("bad string tag {tag:#x} in save profile list").into());
            }
            let len = c.u16()? as usize;
            let bytes = c.take(len)?;
            strings.push(String::from_utf8_lossy(bytes).into_owned());
        }
    }
    let mut r = Reader::new(&data[c.pos..]);
    let module = parse_module(&mut r)?;
    let end = c.pos + r.c.pos;
    if end != data.len() {
        return Err(format!(
            "save parse ended at {end:#x}, file is {:#x} bytes",
            data.len()
        )
        .into());
    }
    Ok(Save {
        endian,
        info,
        strings,
        module,
    })
}
