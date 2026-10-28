use super::model::{
    Desc, Ef300, Gc48, Item, ItemBody, ItemList, ItemModifier, LeafValue, MapEntry, Mod80,
    Modifier, Object, RefBody, RefObject, Save, SaveItem, ScriptBase, Slot80, FILE_VERSION, MAGIC,
    RAW_BLOCK,
};
use super::parse::parse;
use crate::Result;
use std::path::Path;

pub(crate) struct Writer {
    pub(crate) out: Vec<u8>,
    table: Vec<String>,
}

impl Writer {
    pub(crate) fn new() -> Self {
        Self {
            out: Vec::new(),
            table: Vec::new(),
        }
    }

    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }

    fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn bytes(&mut self, v: &[u8]) {
        self.out.extend_from_slice(v);
    }

    fn str(&mut self, s: &str) {
        if let Some(idx) = self.table.iter().position(|t| t == s) {
            self.u16((idx + 0x100) as u16);
            return;
        }
        let b = s.as_bytes();
        self.u16(b.len() as u16);
        self.bytes(b);
        self.table.push(s.to_string());
    }
}

fn write_map(w: &mut Writer, props: &[MapEntry], flag: bool) {
    w.u16(props.len() as u16);
    for p in props {
        w.u64(p.hash);
        w.str(&p.name);
        if flag {
            w.u8(p.flag.unwrap_or(0));
        }
    }
}

fn write_object(w: &mut Writer, o: &Object) {
    w.str(&o.class);
    write_map(w, &o.props, true);
    w.u32(o.elems.len() as u32);
    for e in &o.elems {
        w.u16(e.kind);
        w.u32(e.children.len() as u32);
        for c in &e.children {
            w.u16(c.kind);
            w.u32(c.leaves.len() as u32);
            for l in &c.leaves {
                w.u32(l.data);
                w.u8(l.tag);
                match &l.value {
                    LeafValue::None => {}
                    LeafValue::Two(a, b) => {
                        w.u8(*a);
                        w.u8(*b);
                    }
                    LeafValue::Pair(v1, v2) => {
                        w.u8(v1.is_some() as u8);
                        if let Some(v) = v1 {
                            for x in v {
                                w.u32(*x);
                            }
                        }
                        w.u8(v2.is_some() as u8);
                        if let Some(v) = v2 {
                            for x in v {
                                w.u32(*x);
                            }
                        }
                    }
                }
                write_map(w, &l.props, false);
            }
        }
    }
}

fn write_modifier(w: &mut Writer, m: &Modifier) {
    for v in m.a {
        w.u32(v);
    }
    for s in &m.s {
        w.str(s);
    }
    if let Some(x) = m.extra {
        w.u32(x);
    }
}

fn write_ref_object(w: &mut Writer, o: &RefObject) {
    w.str(&o.name);
    match &o.body {
        RefBody::Script(b) => write_script_base(w, b),
        RefBody::Combat(b, fields, flag, mods) => {
            write_script_base(w, b);
            for f in fields {
                w.u32(*f);
            }
            w.u8(*flag);
            w.u32(mods.len() as u32);
            for m in mods {
                write_modifier(w, m);
            }
        }
        RefBody::Enhancement(b, flag, s) => {
            write_script_base(w, b);
            w.u8(*flag);
            w.str(s);
        }
        RefBody::Freeze(b, v) => {
            write_script_base(w, b);
            w.u32(*v);
        }
    }
}

fn write_script_base(w: &mut Writer, b: &ScriptBase) {
    w.u8(b.flag);
    if b.flag == 1 {
        w.u32(b.refs.len() as u32);
        for r in &b.refs {
            write_ref_object(w, r);
        }
    }
}

fn write_gc48(w: &mut Writer, g: &Gc48) {
    w.u8(g.b);
    for f in g.f {
        w.u32(f);
    }
    write_ref_object(w, &g.obj);
}

fn write_item_modifier(w: &mut Writer, m: &ItemModifier) {
    w.bytes(&m.base.0);
    w.u32(m.base.1);
    w.str(&m.attr);
    for f in m.fields {
        w.u32(f);
    }
    w.u32(m.mods.len() as u32);
    for md in &m.mods {
        w.str(&md.name);
        write_modifier(w, md);
    }
    w.u32(m.effects.len() as u32);
    for g in &m.effects {
        write_gc48(w, g);
    }
}

fn write_save_body(w: &mut Writer, s: &SaveItem) {
    w.bytes(&s.base.0);
    w.u32(s.base.1);
    w.u8(s.b816);
    w.u32(s.f872);
    w.u32(s.f820);
    w.u32(s.f844);
    w.str(&s.s856);
    w.str(&s.s864);
    w.u32(s.nested.len() as u32);
    for i in &s.nested {
        write_item(w, i);
    }
    match &s.desc {
        None => w.u8(0),
        Some(Desc::PossessedComponent { a, b, c, entries }) => {
            w.u8(1);
            w.str("PossessedItemComponent");
            w.u32(*a);
            w.u32(*b);
            w.u32(*c);
            w.u32(entries.len() as u32);
            for e in entries {
                for v in e {
                    w.u32(*v);
                }
            }
        }
    }
}

pub(crate) fn write_item(w: &mut Writer, i: &Item) {
    w.str(&i.name);
    match &i.body {
        ItemBody::Base(b, v) => {
            w.bytes(b);
            w.u32(*v);
        }
        ItemBody::Save(s) => write_save_body(w, s),
        ItemBody::SaveU32(s, v) => {
            write_save_body(w, s);
            w.u32(*v);
        }
        ItemBody::SaveU32U8(s, v, b) => {
            write_save_body(w, s);
            w.u32(*v);
            w.u8(*b);
        }
        ItemBody::Modifier(m) => write_item_modifier(w, m),
    }
}

fn write_item_list(w: &mut Writer, l: &ItemList) {
    w.str(&l.name);
    w.u32(l.field);
    w.u32(l.items.len() as u32);
    for i in &l.items {
        write_item(w, i);
    }
}

fn write_ef300(w: &mut Writer, e: &Ef300) {
    w.str(&e.name);
    w.u32(e.rows.len() as u32);
    for (s, a, b, c, d) in &e.rows {
        w.str(s);
        w.u8(*a);
        w.u8(*b);
        w.u32(*c);
        w.u32(*d);
    }
}

fn write_slot80(w: &mut Writer, s: &Slot80) {
    w.u32(s.a);
    w.str(&s.name);
    w.u32(s.rows.len() as u32);
    for (h, v) in &s.rows {
        w.u64(*h);
        w.u32(*v);
    }
}

fn write_mod80(w: &mut Writer, m: &Mod80) {
    w.u32(m.version);
    for v in m.a {
        w.u32(v);
    }
    if let Some(v) = m.v1 {
        w.u32(v);
    }
    w.u32(m.f1);
    if m.version >= 3 {
        if let Some(v) = m.f124 {
            w.u32(v);
        }
        w.u32(m.f132.unwrap_or(0));
    }
    for v in m.b {
        w.u32(v);
    }
    if let Some(v) = m.c1 {
        w.u32(v);
    }
    if let Some((x, y)) = m.c2 {
        w.u32(x);
        w.u32(y);
    }
    for v in m.d {
        w.u32(v);
    }
    for s in &m.slots {
        write_slot80(w, s);
    }
    if let Some(v) = m.u16_tail {
        w.u16(v);
    }
}

impl Save {
    pub fn write(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.bytes(MAGIC);
        w.u8(self.endian);
        w.u32(FILE_VERSION);
        let i = &self.info;
        w.u8(i.format);
        w.u8(i.endian);
        w.u8(i.b17);
        w.u8(i.b18);
        w.u8(i.b19);
        w.u8(i.zero);
        w.u32(i.f20);
        w.u32(i.f24);
        w.u32(i.f28);
        w.u32(i.f36);
        w.u32(i.f40);
        w.u32(i.f44);
        w.u32(i.f48);
        w.u32(i.counter);
        w.u32(i.global_version);
        w.u32(i.flags);
        w.u8(i.b56);
        w.u32(self.strings.len() as u32);
        for s in &self.strings {
            w.u8(0xFF);
            w.u16(s.len() as u16);
            w.bytes(s.as_bytes());
        }
        let m = &self.module;
        w.u32(m.version);
        w.str(&m.name);
        w.u32(m.loc_records.len() as u32);
        for rec in &m.loc_records {
            for s in rec {
                w.str(s);
            }
        }
        w.u32(m.f32);
        w.str(&m.f40);
        w.u32(m.f48);
        w.u32(m.f52);
        w.u32(m.f56);
        w.u32(m.f60);
        w.u32(m.f64);
        w.u32(m.f68);
        w.u32(m.f72);
        w.u32(0);
        w.u32(m.objects.len() as u32);
        for o in &m.objects {
            write_object(&mut w, o);
        }
        w.u32(m.v1_first);
        w.u32(m.v1.len() as u32);
        for rec in &m.v1 {
            w.str(&rec.name);
            w.u32(rec.inner.len() as u32);
            for x in &rec.inner {
                w.str(&x.name);
                w.u16(x.a);
                w.u16(x.b);
                w.u32(x.c);
                w.u32(x.d);
                w.u32(x.e);
                w.u32(x.f);
                w.str(&x.g);
                w.u32(x.h);
                w.u32(x.i);
                w.u32(x.j);
                w.u8(x.k);
            }
        }
        w.u32(m.n3.len() as u32);
        for (s, a, b, c) in &m.n3 {
            w.str(s);
            w.u8(*a);
            w.u8(*b);
            w.u8(*c);
        }
        w.u32(m.n4.len() as u32);
        for (a, b, c) in &m.n4 {
            w.u32(*a);
            w.u8(*b);
            w.u8(*c);
        }
        w.u32(m.n5.len() as u32);
        for s in &m.n5 {
            w.str(s);
        }
        w.u32(m.inventory.len() as u32);
        for l in &m.inventory {
            write_item_list(&mut w, l);
        }
        w.u32(m.equip_marker);
        w.u32(m.equipped.len() as u32);
        for e in &m.equipped {
            w.u32(e.f1);
            w.u32(e.f2);
            w.u8(e.b24);
            w.u64(e.f32);
            w.u8(e.b40);
            if e.f1 >= 1 {
                w.u8(e.b41.unwrap_or(0));
            }
            w.str(&e.s1);
            w.str(&e.s2);
            w.str(&e.s3);
            w.u32(e.items.len() as u32);
            for i in &e.items {
                write_item(&mut w, i);
            }
        }
        w.u32(m.records.len() as u32);
        for rec in &m.records {
            w.str(&rec.name);
            w.u32(rec.a);
            w.u32(rec.b);
            w.u32(rec.lists.len() as u32);
            for l in &rec.lists {
                write_item_list(&mut w, l);
            }
        }
        w.str(&m.state96.name);
        w.u32(m.state96.a.len() as u32);
        for e in &m.state96.a {
            write_ef300(&mut w, e);
        }
        w.u32(m.state96.b.len() as u32);
        for e in &m.state96.b {
            write_ef300(&mut w, e);
        }
        debug_assert_eq!(m.raw.len(), RAW_BLOCK);
        w.bytes(&m.raw);
        write_mod80(&mut w, &m.s80);
        for t in &m.tail {
            w.u32(t.len() as u32);
            for v in t {
                w.u64(*v);
            }
        }
        w.u8(m.last);
        w.out
    }
}

// ---------------------------------------------------------------------------
// safe write

#[derive(Debug)]
pub struct WriteReport {
    pub bytes: usize,
    /// The one-time `<file>.bak` backup (created before the first overwrite).
    pub backup: Option<std::path::PathBuf>,
    /// A timestamped copy of the previous file content, made on every write
    /// (`<file>.<YYYYMMDD-HHMMSS>.bak`, UTC).
    pub timestamped_backup: Option<std::path::PathBuf>,
    /// Item-granularity diff of the written save against the file's previous
    /// content (empty when the file is new or did not parse).
    pub diff: Vec<String>,
}

pub fn write_file(save: &Save, path: &Path) -> Result<WriteReport> {
    let bytes = save.write();
    // Refuse to write a file that does not parse back cleanly.
    let reparsed = parse(&bytes)
        .map_err(|e| format!("refusing to write: patched save does not re-parse: {e}"))?;
    if reparsed.items().len() != save.items().len() {
        return Err("refusing to write: item count changed across re-parse".into());
    }
    // Diff against what the file holds right now (the state being replaced).
    let diff = std::fs::read(path)
        .ok()
        .and_then(|old| parse(&old).ok())
        .map(|old| save.diff(&old))
        .unwrap_or_default();
    let mut backup = None;
    let mut timestamped_backup = None;
    if path.exists() {
        let bak = path_with_bak(path);
        if !bak.exists() {
            std::fs::copy(path, &bak)?;
        }
        backup = Some(bak);
        let stamped = timestamped_backup_path(path);
        std::fs::copy(path, &stamped)?;
        timestamped_backup = Some(stamped);
    }
    std::fs::write(path, &bytes)?;
    Ok(WriteReport {
        bytes: bytes.len(),
        backup,
        timestamped_backup,
        diff,
    })
}

pub(crate) fn path_with_bak(path: &Path) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".bak");
    std::path::PathBuf::from(name)
}

/// `<file>.<YYYYMMDD-HHMMSS>.bak`, with a `-N` suffix when two writes land in
/// the same second.
fn timestamped_backup_path(path: &Path) -> std::path::PathBuf {
    let stamp = utc_timestamp();
    let mut last = std::path::PathBuf::new();
    for n in 0..1000u32 {
        let mut name = path.as_os_str().to_owned();
        if n == 0 {
            name.push(format!(".{stamp}.bak"));
        } else {
            name.push(format!(".{stamp}-{n}.bak"));
        }
        last = std::path::PathBuf::from(name);
        if !last.exists() {
            break;
        }
    }
    last
}

/// UTC `YYYYMMDD-HHMMSS` from the wall clock (epoch when the clock is before
/// 1970).
fn utc_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        // Clock before the epoch: timestamp 1970 rather than fail the write.
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 -> `(year, month, day)` (Howard Hinnant's
/// `civil_from_days`). Also used by [`super::slots`] for mtime display.
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::civil_from_days;

    #[test]
    fn civil_dates_are_correct() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(59), (1970, 3, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(20_737), (2026, 10, 11));
    }
}
