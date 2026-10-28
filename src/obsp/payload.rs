use crate::cursor::Cursor;
use crate::Result;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectValue {
    pub kind: u8,
    pub reserved: u32,
    pub class_ref: Option<String>,
    pub properties: Vec<Property>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(u32),
    Float(f32),
    Bool(bool),
    Str(String),
    U64(u64),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Ref(String),
    Object(Box<ObjectValue>),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Int(v) => write!(f, "{v}"),
            Value::Float(v) => write!(f, "{v}"),
            Value::Bool(v) => write!(f, "{v}"),
            Value::Str(v) => write!(f, "{v:?}"),
            Value::U64(v) => write!(f, "0x{v:016X}"),
            Value::Ref(v) => write!(f, "{v}"),
            Value::Array(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, "]")
            }
            Value::Map(pairs) => {
                write!(f, "{{")?;
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                write!(f, "}}")
            }
            Value::Object(o) => {
                write!(f, "obj(")?;
                if let Some(class) = &o.class_ref {
                    write!(f, "{class}")?;
                }
                write!(f, "{{")?;
                for (i, p) in o.properties.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}={}", p.name, p.value)?;
                }
                write!(f, "}})")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub name: String,
    pub hash: u64,
    pub value: Value,
    pub offset: usize,
    pub size: usize,
}

#[derive(Debug, Clone)]
pub struct Def {
    pub name: String,
    pub hash: u64,
    pub flags: u8,
    pub kind: u8,
}

#[derive(Debug)]
pub struct Payload {
    pub names: Vec<(u64, String)>,
    pub name_list: Vec<String>,
    pub properties: Vec<Property>,
    pub defs: Vec<Def>,
    pub class_ref: Option<String>,
    pub complete: bool,
    pub def_count_at: usize,
    pub defs_end: usize,
    pub values_count_at: usize,
    pub values_end: usize,
}

impl Payload {
    pub fn get(&self, name: &str) -> Option<&Property> {
        self.properties
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn name_of(&self, hash: u64) -> Option<&str> {
        self.names
            .iter()
            .find(|(h, _)| *h == hash)
            .map(|(_, n)| n.as_str())
    }

    pub fn find_path(&self, path: &str) -> Option<&Property> {
        let segs = parse_path(path)?;
        walk(&self.properties, &segs)
    }
}

struct Seg<'a> {
    name: &'a str,
    key: Option<&'a str>,
}

fn parse_path(path: &str) -> Option<Vec<Seg<'_>>> {
    let mut segs = Vec::new();
    for part in path.split('.') {
        let part = part.trim();
        if part.is_empty() {
            return None;
        }
        if let Some(open) = part.find('[') {
            let close = part.find(']')?;
            let mut key = part[open + 1..close].trim();
            if key.len() >= 2
                && ((key.starts_with('"') && key.ends_with('"'))
                    || (key.starts_with('\'') && key.ends_with('\'')))
            {
                key = &key[1..key.len() - 1];
            }
            segs.push(Seg {
                name: &part[..open],
                key: Some(key),
            });
        } else {
            segs.push(Seg {
                name: part,
                key: None,
            });
        }
    }
    Some(segs)
}

fn key_matches(key: &str, value: &Value) -> bool {
    match value {
        Value::Int(v) => key.parse::<u32>().ok() == Some(*v),
        Value::U64(v) => {
            key.parse::<u64>().ok() == Some(*v)
                || key
                    .strip_prefix("0x")
                    .and_then(|hex| u64::from_str_radix(hex, 16).ok())
                    == Some(*v)
        }
        Value::Str(s) | Value::Ref(s) => key == s,
        _ => false,
    }
}

fn walk<'a>(props: &'a [Property], segs: &[Seg]) -> Option<&'a Property> {
    let (seg, rest) = segs.split_first()?;
    let prop = props
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(seg.name))?;
    if rest.is_empty() {
        return Some(prop);
    }
    match (seg.key, &prop.value) {
        (Some(key), Value::Array(items)) => {
            let index: usize = key.parse().ok()?;
            match items.get(index)? {
                Value::Object(o) => walk(&o.properties, rest),
                _ => None,
            }
        }
        (Some(key), Value::Map(pairs)) => {
            let (_, value) = pairs.iter().find(|(k, _)| key_matches(key, k))?;
            match value {
                Value::Object(o) => walk(&o.properties, rest),
                _ => None,
            }
        }
        (None, Value::Object(o)) => walk(&o.properties, rest),
        _ => None,
    }
}

pub const PAYLOAD_VERSION: u32 = 1;
pub const BOD_MAGIC: &[u8] = b"BOD\xFD";

const MAX_DEPTH: u32 = 32;

fn read_names(c: &mut Cursor) -> Result<Vec<(u64, String)>> {
    let count = c.u32()?;
    let _max_len = c.u32()?;
    let mut names = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let hash = c.u64()?;
        let len = c.u32()? as usize;
        let name = String::from_utf8_lossy(c.take(len)?).into_owned();
        names.push((hash, name));
    }
    Ok(names)
}

fn read_name_ref(c: &mut Cursor, names: &mut Vec<String>) -> Result<String> {
    let flag = c.u8()?;
    if flag != 0 {
        let _hash = c.u64()?;
        let len = c.u16()? as usize;
        let name = String::from_utf8_lossy(c.take(len)?).into_owned();
        names.push(name.clone());
        Ok(name)
    } else {
        let index = c.u32()? as usize;
        names
            .get(index)
            .cloned()
            .ok_or_else(|| format!("name index {index} out of range").into())
    }
}

fn parse_value(
    c: &mut Cursor,
    names: &mut Vec<String>,
    depth: u32,
) -> Result<(Value, usize, usize)> {
    if depth > MAX_DEPTH {
        return Err("value nesting too deep".into());
    }
    let tag_at = c.pos;
    let tag = c.u8()?;
    let value = match tag {
        0x00 => Value::Null,
        0x02 => Value::Int(c.u32()?),
        0x03 => {
            let b: [u8; 4] = c.take(4)?.try_into().unwrap();
            Value::Float(f32::from_le_bytes(b))
        }
        0x04 => Value::Bool(c.u8()? != 0),
        0x05 => {
            if c.u8()? != 0xFF {
                return Err("string encoding not supported".into());
            }
            let len = c.u16()? as usize;
            let bytes = c.take(len)?.to_vec();
            Value::Str(String::from_utf8_lossy(&bytes).into_owned())
        }
        0x07 => {
            c.u32()?;
            parse_object_ref(c, names, depth)?
        }
        0x09 | 0x0A => {
            let count = c.u32()?;
            let pairs = c.u8()? != 0;
            if pairs {
                let mut map = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    let (k, _, _) = parse_value(c, names, depth + 1)?;
                    let (v, _, _) = parse_value(c, names, depth + 1)?;
                    map.push((k, v));
                }
                Value::Map(map)
            } else {
                let mut items = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    let (v, _, _) = parse_value(c, names, depth + 1)?;
                    items.push(v);
                }
                Value::Array(items)
            }
        }
        0x0B => {
            let count = c.u32()?;
            let mut items = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let (v, _, _) = parse_value(c, names, depth + 1)?;
                items.push(v);
            }
            Value::Array(items)
        }
        0x0D => Value::U64(c.u64()?),
        0x0F => Value::Ref(read_name_ref(c, names)?),
        0xFC => {
            let id = c.u32()?;
            let hash = c.u64()?;
            Value::Ref(format!("obj:{id}:{hash:016X}"))
        }
        other => {
            return Err(format!("unsupported value tag {other:#x} at {tag_at}").into());
        }
    };
    let end = c.pos;
    let (offset, size) = match tag {
        0x05 => (tag_at + 4, end - (tag_at + 4)),
        _ => (tag_at + 1, end - (tag_at + 1)),
    };
    Ok((value, offset, size))
}

fn parse_object_ref(c: &mut Cursor, names: &mut Vec<String>, depth: u32) -> Result<Value> {
    if depth > MAX_DEPTH {
        return Err("object nesting too deep".into());
    }
    let kind = c.u8()?;
    if kind == 0 {
        return Ok(Value::Object(Box::new(ObjectValue {
            kind,
            reserved: 0,
            class_ref: None,
            properties: Vec::new(),
        })));
    }
    if kind == 4 {
        c.skip(4)?;
    }
    let class_ref = Some(read_name_ref(c, names)?);
    let count = c.u32()? as usize;
    let mut properties = Vec::with_capacity(count);
    for _ in 0..count {
        let name = read_name_ref(c, names)?;
        let (value, offset, size) = parse_value(c, names, depth + 1)?;
        properties.push(Property {
            name,
            hash: 0,
            value,
            offset,
            size,
        });
    }
    Ok(Value::Object(Box::new(ObjectValue {
        kind,
        reserved: 0,
        class_ref,
        properties,
    })))
}

fn parse_v1_payload(payload: &[u8]) -> Result<Payload> {
    let mut c = Cursor::new(payload);
    if c.u32()? != PAYLOAD_VERSION {
        return Err("unsupported OBSP payload version".into());
    }
    let raw_names = read_names(&mut c)?;
    let mut names: Vec<String> = raw_names.iter().map(|(_, n)| n.clone()).collect();
    let name_of = |hash: u64| -> String {
        raw_names
            .iter()
            .find(|(h, _)| *h == hash)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| format!("{hash:#018x}"))
    };
    c.skip(28)?;

    let def_count_at = c.pos;
    let def_count = c.u32()? as usize;
    let mut defs = Vec::with_capacity(def_count);
    let mut properties = Vec::new();
    let mut complete = true;
    for _ in 0..def_count {
        let hash = c.u64()?;
        let flags = c.u8()?;
        let kind = c.u8()?;
        defs.push(Def {
            name: name_of(hash),
            hash,
            flags,
            kind,
        });
        if flags & 8 != 0 {
            match parse_value(&mut c, &mut names, 0) {
                Ok((value, offset, size)) => properties.push(Property {
                    name: name_of(hash),
                    hash,
                    value,
                    offset,
                    size,
                }),
                Err(_) => {
                    complete = false;
                    break;
                }
            }
        }
    }
    let defs_end = c.pos;
    let values_count_at = c.pos;

    if complete {
        let value_count = c.u32()? as usize;
        for _ in 0..value_count {
            let hash = c.u64()?;
            match parse_value(&mut c, &mut names, 0) {
                Ok((value, offset, size)) => properties.push(Property {
                    name: name_of(hash),
                    hash,
                    value,
                    offset,
                    size,
                }),
                Err(_) => {
                    complete = false;
                    break;
                }
            }
        }
    }
    let values_end = c.pos;

    Ok(Payload {
        names: raw_names,
        name_list: names,
        properties,
        defs,
        class_ref: None,
        complete,
        def_count_at,
        defs_end,
        values_count_at,
        values_end,
    })
}

fn parse_bod_payload(payload: &[u8]) -> Result<Payload> {
    let mut c = Cursor::new(payload);
    if c.take(4)? != BOD_MAGIC {
        return Err("not a BOD payload".into());
    }
    let mode = c.u8()?;
    if mode < 3 {
        return Err(format!("BOD name mode {mode} not supported").into());
    }
    c.skip(2)?;
    let _endian = c.u8()?;
    let _name_slots = c.u32()?;
    let _max_name_len = c.u32()?;

    let mut names: Vec<String> = Vec::new();
    let root = parse_object_ref(&mut c, &mut names, 0)?;
    let object = match root {
        Value::Object(o) => *o,
        _ => return Err("BOD root is not an object".into()),
    };
    let complete = c.pos <= payload.len();
    Ok(Payload {
        names: Vec::new(),
        name_list: names,
        properties: object.properties,
        defs: Vec::new(),
        class_ref: object.class_ref,
        complete,
        def_count_at: 0,
        defs_end: 0,
        values_count_at: 0,
        values_end: 0,
    })
}

pub fn parse_payload(payload: &[u8]) -> Result<Payload> {
    if payload.len() >= 4 && &payload[..4] == BOD_MAGIC {
        parse_bod_payload(payload)
    } else {
        parse_v1_payload(payload)
    }
}

#[derive(Debug, Clone)]
pub struct Target {
    pub value: Value,
    pub offset: usize,
    pub size: usize,
}

pub fn resolve_path(payload: &[u8], parsed: &Payload, path: &str) -> Option<Target> {
    let segs = parse_path(path)?;
    let mut names = parsed.name_list.clone();
    resolve_in(&parsed.properties, payload, &mut names, &segs)
}

fn resolve_in(
    props: &[Property],
    payload: &[u8],
    names: &mut Vec<String>,
    segs: &[Seg],
) -> Option<Target> {
    let (seg, rest) = segs.split_first()?;
    let prop = props
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(seg.name))?;
    let mut current = Target {
        value: prop.value.clone(),
        offset: prop.offset,
        size: prop.size,
    };
    if let Some(key) = seg.key {
        let (value, offset, size) = match &current.value {
            Value::Array(_) => {
                let index: usize = key.parse().ok()?;
                // 0x0B is a bare list (no pairs byte); 0x09/0x0A carry one
                let tag = payload
                    .get(current.offset.wrapping_sub(1))
                    .copied()
                    .unwrap_or(0);
                let items = if tag == 0x0B {
                    parse_list_elements(payload, current.offset, names).ok()?
                } else {
                    parse_array_elements(payload, current.offset, names).ok()?
                };
                items.into_iter().nth(index)?
            }
            Value::Map(_) => parse_map_elements(payload, current.offset, names)
                .ok()?
                .into_iter()
                .find(|(k, _, _, _)| key_matches(key, k))
                .map(|(_, value, offset, size)| (value, offset, size))?,
            _ => return None,
        };
        current = Target {
            value,
            offset,
            size,
        };
    }
    if rest.is_empty() {
        return Some(current);
    }
    match &current.value {
        Value::Object(o) => resolve_in(&o.properties, payload, names, rest),
        _ => None,
    }
}

fn parse_array_elements(
    payload: &[u8],
    value_offset: usize,
    names: &mut Vec<String>,
) -> Result<Vec<(Value, usize, usize)>> {
    let mut c = Cursor {
        data: payload,
        pos: value_offset,
    };
    let count = c.u32()? as usize;
    if c.u8()? != 0 {
        return Err("not an array".into());
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(parse_value(&mut c, names, 0)?);
    }
    Ok(out)
}

/// Tag 0x0B list: `u32 count` then elements, no pairs byte.
fn parse_list_elements(
    payload: &[u8],
    value_offset: usize,
    names: &mut Vec<String>,
) -> Result<Vec<(Value, usize, usize)>> {
    let mut c = Cursor {
        data: payload,
        pos: value_offset,
    };
    let count = c.u32()? as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(parse_value(&mut c, names, 0)?);
    }
    Ok(out)
}

fn parse_map_elements(
    payload: &[u8],
    value_offset: usize,
    names: &mut Vec<String>,
) -> Result<Vec<(Value, Value, usize, usize)>> {
    let mut c = Cursor {
        data: payload,
        pos: value_offset,
    };
    let count = c.u32()? as usize;
    if c.u8()? == 0 {
        return Err("not a map".into());
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let (key, _, _) = parse_value(&mut c, names, 0)?;
        let (value, offset, size) = parse_value(&mut c, names, 0)?;
        out.push((key, value, offset, size));
    }
    Ok(out)
}

pub fn patch_value(payload: &mut [u8], prop: &Property, repr: &str) -> Result<()> {
    patch_at(payload, &prop.value, prop.offset, repr)
}

pub fn patch_target(payload: &mut [u8], target: &Target, repr: &str) -> Result<()> {
    patch_at(payload, &target.value, target.offset, repr)
}

pub fn find_property<'a>(props: &'a [Property], name: &str) -> Option<&'a Property> {
    for prop in props {
        if prop.name.eq_ignore_ascii_case(name) {
            return Some(prop);
        }
        match &prop.value {
            Value::Object(o) => {
                if let Some(found) = find_property(&o.properties, name) {
                    return Some(found);
                }
            }
            Value::Array(items) => {
                for item in items {
                    if let Value::Object(o) = item {
                        if let Some(found) = find_property(&o.properties, name) {
                            return Some(found);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

pub fn kind_of(value: &Value) -> Result<u8> {
    match value {
        Value::Int(_) => Ok(0x02),
        Value::Float(_) => Ok(0x03),
        Value::Bool(_) => Ok(0x04),
        Value::Str(_) => Ok(0x05),
        other => Err(format!("property kind not addable: {other}").into()),
    }
}

fn encode_value(kind: u8, repr: &str) -> Result<Vec<u8>> {
    let mut out = vec![kind];
    match kind {
        0x02 => {
            let v: u32 = repr.parse().map_err(|_| format!("not a u32: {repr}"))?;
            out.extend_from_slice(&v.to_le_bytes());
        }
        0x03 => {
            let v: f32 = repr.parse().map_err(|_| format!("not a float: {repr}"))?;
            out.extend_from_slice(&v.to_le_bytes());
        }
        0x04 => {
            let v = match repr.to_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => 1u8,
                "0" | "false" | "no" | "off" => 0u8,
                _ => return Err(format!("not a bool: {repr}").into()),
            };
            out.push(v);
        }
        0x05 => {
            let bytes = repr.as_bytes();
            if bytes.len() > u16::MAX as usize {
                return Err("string too long".into());
            }
            out.push(0xFF);
            out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
            out.extend_from_slice(bytes);
        }
        other => return Err(format!("value kind not supported: {other:#x}").into()),
    }
    Ok(out)
}

pub fn add_property(
    payload: &mut Vec<u8>,
    parsed: &Payload,
    name: &str,
    kind: u8,
    repr: &str,
) -> Result<()> {
    if !parsed.complete {
        return Err("payload did not parse completely; cannot add properties".into());
    }
    if parsed
        .defs
        .iter()
        .any(|d| d.name.eq_ignore_ascii_case(name))
    {
        return Err(format!("property {name} already declared; use obsp-set").into());
    }
    let value_bytes = encode_value(kind, repr)?;

    let mut shift = 0usize;
    let hash = match parsed
        .names
        .iter()
        .find(|(_, n)| n.eq_ignore_ascii_case(name))
    {
        Some((h, _)) => *h,
        None => {
            let hash = crate::hash::crc64(name.as_bytes());
            let name_count = read_u32(payload, 4)?
                .checked_add(1)
                .ok_or("name overflow")?;
            write_u32(payload, 4, name_count)?;
            let len = name.len() as u32;
            if len > read_u32(payload, 8)? {
                write_u32(payload, 8, len)?;
            }
            let names_end = parsed.def_count_at.checked_sub(28).ok_or("bad layout")?;
            let mut record = Vec::with_capacity(12 + name.len());
            record.extend_from_slice(&hash.to_le_bytes());
            record.extend_from_slice(&len.to_le_bytes());
            record.extend_from_slice(name.as_bytes());
            shift = record.len();
            payload.splice(names_end..names_end, record);
            hash
        }
    };

    let def_count_at = parsed.def_count_at + shift;
    let defs_end = parsed.defs_end + shift;
    let values_count_at = parsed.values_count_at + shift;
    let values_end = parsed.values_end + shift;

    let count = read_u32(payload, def_count_at)?
        .checked_add(1)
        .ok_or("def count overflow")?;
    write_u32(payload, def_count_at, count)?;

    let mut def_record = Vec::with_capacity(10);
    def_record.extend_from_slice(&hash.to_le_bytes());
    def_record.push(0);
    def_record.push(kind);
    payload.splice(defs_end..defs_end, def_record);

    let count = read_u32(payload, values_count_at + 10)?
        .checked_add(1)
        .ok_or("value count overflow")?;
    write_u32(payload, values_count_at + 10, count)?;

    let mut value_record = Vec::with_capacity(8 + value_bytes.len());
    value_record.extend_from_slice(&hash.to_le_bytes());
    value_record.extend_from_slice(&value_bytes);
    if values_end + 10 > payload.len() {
        return Err("value list end out of range".into());
    }
    payload.splice(values_end + 10..values_end + 10, value_record);

    Ok(())
}

fn read_u32(data: &[u8], at: usize) -> Result<u32> {
    let b: [u8; 4] = data
        .get(at..at + 4)
        .ok_or("u32 out of range")?
        .try_into()
        .unwrap();
    Ok(u32::from_le_bytes(b))
}

fn write_u32(data: &mut [u8], at: usize, value: u32) -> Result<()> {
    data.get_mut(at..at + 4)
        .ok_or("u32 out of range")?
        .copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn patch_at(payload: &mut [u8], value: &Value, offset: usize, repr: &str) -> Result<()> {
    match value {
        Value::Int(_) => {
            let v: u32 = repr.parse().map_err(|_| format!("not a u32: {repr}"))?;
            set(payload, offset, &v.to_le_bytes())
        }
        Value::Float(_) => {
            let v: f32 = repr.parse().map_err(|_| format!("not a float: {repr}"))?;
            set(payload, offset, &v.to_le_bytes())
        }
        Value::Bool(_) => {
            let v = match repr.to_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => 1u8,
                "0" | "false" | "no" | "off" => 0u8,
                _ => return Err(format!("not a bool: {repr}").into()),
            };
            set(payload, offset, &[v])
        }
        Value::Str(old) => {
            let bytes = repr.as_bytes();
            if bytes.len() > old.len() {
                return Err(
                    format!("string too long: {} bytes (max {})", bytes.len(), old.len()).into(),
                );
            }
            let len_field = offset
                .checked_sub(2)
                .ok_or("string length field out of range")?;
            set(payload, len_field, &(bytes.len() as u16).to_le_bytes())?;
            for (i, b) in bytes.iter().enumerate() {
                payload[offset + i] = *b;
            }
            for i in bytes.len()..old.len() {
                payload[offset + i] = 0;
            }
            Ok(())
        }
        other => Err(format!("property type not patchable: {other}").into()),
    }
}

fn set(payload: &mut [u8], at: usize, bytes: &[u8]) -> Result<()> {
    payload
        .get_mut(at..at + bytes.len())
        .ok_or_else(|| "patch out of range".into())
        .map(|s| s.copy_from_slice(bytes))
}
