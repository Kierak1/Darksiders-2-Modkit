//! `worlds.bin` + `worldnodes64.bin` parsers (world/level table + node data).
//!
//! Reversed from `sub_14082C3B0` (loads both files on PC),
//! `sub_14082D438` (worlds table) and `sub_14082D834` (nodes):
//!
//! `worlds.bin`: `u8 magic=3, u8 version, u32 count`, then per world:
//! `u32 nameLen, name, u64 hash, u32 a, u32 b, u32 n1, n1×{u32,u32},
//!  u32 n2, n2×{u32,u32}, u8 flag`.
//!
//! `worldnodes64.bin`: `u8 magic=2, u8 version, u32 count`, then per world:
//! `u64 hash, u32 n1, u32 n2, n1×16B, n2×16B`.

use crate::Result;
use std::path::Path;

fn u32_at(d: &[u8], o: usize) -> Result<u32> {
    d.get(o..o + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| format!("truncated u32 at {o:#X}").into())
}

fn u64_at(d: &[u8], o: usize) -> Result<u64> {
    d.get(o..o + 8)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| format!("truncated u64 at {o:#X}").into())
}

#[derive(Debug, Clone)]
pub struct World {
    pub name: String,
    pub hash: u64,
    pub a: u32,
    pub b: u32,
    pub pairs1: Vec<(u32, u32)>,
    pub pairs2: Vec<(u32, u32)>,
    pub flag: u8,
    /// File offset of the u32 `a` field (for blob offset fixups).
    pub a_at: usize,
    /// File offset of the u32 `b` field.
    pub b_at: usize,
    /// File offset of the first `pairs1` entry (`u32 offset, u32 size` each).
    pub pairs1_at: usize,
    /// File offset of the first `pairs2` entry.
    pub pairs2_at: usize,
}

#[derive(Debug, Clone)]
pub struct NodeSet {
    pub hash: u64,
    /// first 16-byte entry array (4 x u32)
    pub entries1: Vec<[u32; 4]>,
    /// second 16-byte entry array (4 x u32)
    pub entries2: Vec<[u32; 4]>,
}

pub fn parse_worlds(data: &[u8]) -> Result<Vec<World>> {
    if data.len() < 6 || data[0] != 3 {
        return Err("not a worlds.bin (magic != 3)".into());
    }
    let count = u32_at(data, 2)? as usize;
    let mut o = 6usize;
    let mut worlds = Vec::with_capacity(count);
    for _ in 0..count {
        let name_len = u32_at(data, o)? as usize;
        o += 4;
        if name_len > 128 || o + name_len + 16 > data.len() {
            return Err(format!("bad name length {name_len} at {o:#X}").into());
        }
        let name = String::from_utf8_lossy(&data[o..o + name_len]).into_owned();
        o += name_len;
        let hash = u64_at(data, o)?;
        o += 8;
        let a_at = o;
        let a = u32_at(data, o)?;
        let b_at = o + 4;
        let b = u32_at(data, o + 4)?;
        o += 8;
        let n1 = u32_at(data, o)? as usize;
        o += 4;
        let pairs1_at = o;
        let mut pairs1 = Vec::with_capacity(n1);
        for _ in 0..n1 {
            pairs1.push((u32_at(data, o)?, u32_at(data, o + 4)?));
            o += 8;
        }
        let n2 = u32_at(data, o)? as usize;
        o += 4;
        let pairs2_at = o;
        let mut pairs2 = Vec::with_capacity(n2);
        for _ in 0..n2 {
            pairs2.push((u32_at(data, o)?, u32_at(data, o + 4)?));
            o += 8;
        }
        let flag = *data
            .get(o)
            .ok_or_else(|| format!("truncated flag at {o:#X}"))?;
        o += 1;
        worlds.push(World {
            name,
            hash,
            a,
            b,
            pairs1,
            pairs2,
            flag,
            a_at,
            b_at,
            pairs1_at,
            pairs2_at,
        });
    }
    Ok(worlds)
}

pub fn parse_nodes(data: &[u8]) -> Result<Vec<NodeSet>> {
    if data.len() < 6 || data[0] != 2 {
        return Err("not a worldnodes file (magic != 2)".into());
    }
    let count = u32_at(data, 2)? as usize;
    let mut o = 6usize;
    let mut sets = Vec::with_capacity(count);
    for _ in 0..count {
        let hash = u64_at(data, o)?;
        o += 8;
        let n1 = u32_at(data, o)? as usize;
        let n2 = u32_at(data, o + 4)? as usize;
        o += 8;
        let mut entries1 = Vec::with_capacity(n1);
        let mut entries2 = Vec::with_capacity(n2);
        for _ in 0..n1 {
            let mut e = [0u32; 4];
            for (k, v) in e.iter_mut().enumerate() {
                *v = u32_at(data, o + k * 4)?;
            }
            entries1.push(e);
            o += 16;
        }
        for _ in 0..n2 {
            let mut e = [0u32; 4];
            for (k, v) in e.iter_mut().enumerate() {
                *v = u32_at(data, o + k * 4)?;
            }
            entries2.push(e);
            o += 16;
        }
        sets.push(NodeSet {
            hash,
            entries1,
            entries2,
        });
    }
    Ok(sets)
}

pub fn load(media_dir: &Path) -> Result<(Vec<World>, Vec<NodeSet>)> {
    let worlds = parse_worlds(&std::fs::read(media_dir.join("worlds.bin"))?)?;
    let nodes = parse_nodes(&std::fs::read(media_dir.join("worldnodes64.bin"))?)?;
    Ok((worlds, nodes))
}

/// The world's serialized BOD tree inside `worlds.bin` (`a..a+b`).
pub fn world_blob<'a>(file: &'a [u8], w: &World) -> Result<&'a [u8]> {
    let start = w.a as usize;
    let end = start + w.b as usize;
    file.get(start..end).ok_or_else(|| {
        format!(
            "world {} blob {:#X}..{:#X} out of range",
            w.name, start, end
        )
        .into()
    })
}

/// A placed world object (waypoint, spawnpoint, StaticObject, chest, ...)
/// found in one of the world's BOD blobs.
#[derive(Debug, Clone)]
pub struct PlacedObject {
    /// offset of the blob in `worlds.bin`
    pub blob_offset: usize,
    pub blob_size: usize,
    /// property path inside the blob, e.g. `Root.Objects[3]`
    pub path: String,
    pub class: String,
    pub name: String,
    pub position: [f32; 3],
    pub rotation: Option<[f32; 3]>,
}

fn float3(v: &crate::obsp::payload::Value) -> Option<[f32; 3]> {
    use crate::obsp::payload::Value;
    if let Value::Array(items) = v {
        if items.len() >= 3 {
            let mut out = [0f32; 3];
            for (i, item) in items.iter().take(3).enumerate() {
                if let Value::Float(f) = item {
                    out[i] = *f;
                } else {
                    return None;
                }
            }
            return Some(out);
        }
    }
    None
}

fn prop_str(props: &[crate::obsp::payload::Property], name: &str) -> Option<String> {
    use crate::obsp::payload::Value;
    props
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            Value::Str(s) | Value::Ref(s) => Some(s.clone()),
            _ => None,
        })
}

fn walk_objects(
    props: &[crate::obsp::payload::Property],
    path: &str,
    blob_offset: usize,
    blob_size: usize,
    out: &mut Vec<PlacedObject>,
) {
    use crate::obsp::payload::Value;
    for p in props {
        let child_path = if path.is_empty() {
            p.name.clone()
        } else {
            format!("{path}.{}", p.name)
        };
        match &p.value {
            Value::Object(o) => {
                let pos = o
                    .properties
                    .iter()
                    .find(|q| q.name == "Position")
                    .and_then(|q| float3(&q.value));
                if let Some(position) = pos {
                    let rotation = o
                        .properties
                        .iter()
                        .find(|q| q.name == "Rotation")
                        .and_then(|q| float3(&q.value));
                    let name = prop_str(&o.properties, "Name")
                        .or_else(|| prop_str(&o.properties, "ObjectName"))
                        .or_else(|| prop_str(&o.properties, "Comment"))
                        .or_else(|| prop_str(&o.properties, "EncounterName"))
                        .unwrap_or_else(|| o.class_ref.clone().unwrap_or_default());
                    out.push(PlacedObject {
                        blob_offset,
                        blob_size,
                        path: child_path.clone(),
                        class: o.class_ref.clone().unwrap_or_default(),
                        name,
                        position,
                        rotation,
                    });
                }
                walk_objects(&o.properties, &child_path, blob_offset, blob_size, out);
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    if let Value::Object(o) = item {
                        let item_path = format!("{child_path}[{i}]");
                        let pos = o
                            .properties
                            .iter()
                            .find(|q| q.name == "Position")
                            .and_then(|q| float3(&q.value));
                        if let Some(position) = pos {
                            let rotation = o
                                .properties
                                .iter()
                                .find(|q| q.name == "Rotation")
                                .and_then(|q| float3(&q.value));
                            let name = prop_str(&o.properties, "Name")
                                .or_else(|| prop_str(&o.properties, "ObjectName"))
                                .or_else(|| prop_str(&o.properties, "Comment"))
                                .or_else(|| prop_str(&o.properties, "EncounterName"))
                                .unwrap_or_else(|| o.class_ref.clone().unwrap_or_default());
                            out.push(PlacedObject {
                                blob_offset,
                                blob_size,
                                path: item_path.clone(),
                                class: o.class_ref.clone().unwrap_or_default(),
                                name,
                                position,
                                rotation,
                            });
                        }
                        walk_objects(&o.properties, &item_path, blob_offset, blob_size, out);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Every positioned object in the world's BOD blobs (`pairs1` + `pairs2`).
pub fn collect_objects(file: &[u8], w: &World) -> Result<Vec<PlacedObject>> {
    let mut out = Vec::new();
    for (offset, size) in w.pairs1.iter().chain(w.pairs2.iter()) {
        let start = *offset as usize;
        let len = *size as usize;
        let Some(blob) = file.get(start..start + len) else {
            continue;
        };
        let Ok(payload) = crate::obsp::payload::parse_payload(blob) else {
            continue;
        };
        walk_objects(&payload.properties, "", start, len, &mut out);
    }
    Ok(out)
}

/// A world region row (`RegionData[]` in the world's `a` blob).
#[derive(Debug, Clone)]
pub struct Region {
    pub id: u32,
    pub name: String,
}

fn prop_int(props: &[crate::obsp::payload::Property], name: &str) -> Option<u32> {
    use crate::obsp::payload::Value;
    props
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            Value::Int(v) => Some(*v),
            Value::Float(v) => Some(*v as u32),
            _ => None,
        })
}

/// Region table (`RegionData[]`) from the world's `a` blob.
pub fn regions(file: &[u8], w: &World) -> Result<Vec<Region>> {
    use crate::obsp::payload::Value;
    let blob = world_blob(file, w)?;
    let payload = crate::obsp::payload::parse_payload(blob)?;
    let mut out = Vec::new();
    for p in &payload.properties {
        if p.name != "RegionData" {
            continue;
        }
        if let Value::Array(items) = &p.value {
            for item in items {
                if let Value::Object(o) = item {
                    if let (Some(id), Some(name)) = (
                        prop_int(&o.properties, "ID"),
                        prop_str(&o.properties, "Name"),
                    ) {
                        out.push(Region { id, name });
                    }
                }
            }
        }
    }
    Ok(out)
}

fn walk_all_objects(
    props: &[crate::obsp::payload::Property],
    f: &mut impl FnMut(&crate::obsp::payload::ObjectValue),
) {
    use crate::obsp::payload::Value;
    for p in props {
        match &p.value {
            Value::Object(o) => {
                f(o);
                walk_all_objects(&o.properties, f);
            }
            Value::Array(items) => {
                for item in items {
                    if let Value::Object(o) = item {
                        f(o);
                        walk_all_objects(&o.properties, f);
                    }
                }
            }
            _ => {}
        }
    }
}

fn point_dist2(p: [f32; 3], q: [f32; 3]) -> f32 {
    (0..3).map(|k| (p[k] - q[k]).powi(2)).sum()
}

/// Squared distance to an AABB and whether the point is inside it.
fn box_dist2(center: [f32; 3], size: [f32; 3], point: [f32; 3]) -> (f32, bool) {
    let mut d2 = 0.0;
    let mut inside = true;
    for k in 0..3 {
        let lo = center[k] - size[k] * 0.5;
        let hi = center[k] + size[k] * 0.5;
        if point[k] < lo {
            d2 += (lo - point[k]).powi(2);
            inside = false;
        } else if point[k] > hi {
            d2 += (point[k] - hi).powi(2);
            inside = false;
        }
    }
    (d2, inside)
}

/// Region id whose cell contains `point`, else the region of the nearest
/// cell/portal referenced by the world's `pairs2` blobs.
pub fn nearest_region(file: &[u8], w: &World, point: [f32; 3]) -> Option<u32> {
    let mut inside: Option<(f32, u32)> = None;
    let mut any: Option<(f32, u32)> = None;
    let mut consider = |id: u32, d2: f32, is_inside: bool| {
        if is_inside && inside.is_none_or(|(bd, _)| d2 < bd) {
            inside = Some((d2, id));
        }
        if any.is_none_or(|(bd, _)| d2 < bd) {
            any = Some((d2, id));
        }
    };
    for (offset, size) in &w.pairs2 {
        let start = *offset as usize;
        let len = *size as usize;
        let Some(blob) = file.get(start..start + len) else {
            continue;
        };
        let Ok(payload) = crate::obsp::payload::parse_payload(blob) else {
            continue;
        };
        walk_all_objects(&payload.properties, &mut |o| {
            let p = &o.properties;
            if let Some(id) = prop_int(p, "RegionID") {
                if let Some(pos) = p
                    .iter()
                    .find(|q| q.name == "Position")
                    .and_then(|q| float3(&q.value))
                {
                    let (d2, is_inside) = match p
                        .iter()
                        .find(|q| q.name == "Size")
                        .and_then(|q| float3(&q.value))
                    {
                        Some(sz) => box_dist2(pos, sz, point),
                        None => (point_dist2(pos, point), false),
                    };
                    consider(id, d2, is_inside);
                }
            } else if let Some(a) = prop_int(p, "RegionA") {
                if let Some(pos) = p
                    .iter()
                    .find(|q| q.name == "Position")
                    .and_then(|q| float3(&q.value))
                {
                    let d2 = point_dist2(pos, point);
                    consider(a, d2, false);
                    if let Some(b) = prop_int(p, "RegionB") {
                        consider(b, d2, false);
                    }
                }
            }
        });
    }
    inside.or(any).map(|(_, id)| id)
}

fn write_u32_at(data: &mut [u8], at: usize, value: u32) -> Result<()> {
    data.get_mut(at..at + 4)
        .ok_or_else(|| format!("u32 write at {at:#X} out of range").into())
        .map(|b| b.copy_from_slice(&value.to_le_bytes()))
}

/// Byte span of the element's tagged value including its tag byte. Structural
/// edits are only implemented for object values (tag 0x07).
fn object_span(blob: &[u8], target: &crate::obsp::payload::Target) -> Result<(usize, usize)> {
    let tag = target
        .offset
        .checked_sub(1)
        .and_then(|i| blob.get(i))
        .copied()
        .ok_or("element tag out of range")?;
    if tag != 0x07 {
        return Err(format!("element tag {tag:#x} is not an object").into());
    }
    let end = target.offset + target.size;
    if end > blob.len() {
        return Err("element span out of range".into());
    }
    Ok((target.offset - 1, end))
}

/// Split `Root.Objects[3]` into (`Root.Objects`, 3).
fn parent_list(path: &str) -> Result<(String, usize)> {
    let open = path.rfind('[').ok_or("path has no list index")?;
    let close = path.rfind(']').ok_or("path has no list index")?;
    let index: usize = path[open + 1..close]
        .parse()
        .map_err(|_| format!("bad list index in {path}"))?;
    Ok((path[..open].to_string(), index))
}

fn list_target(
    blob: &[u8],
    parsed: &crate::obsp::payload::Payload,
    parent_path: &str,
) -> Result<crate::obsp::payload::Target> {
    let target = crate::obsp::payload::resolve_path(blob, parsed, parent_path)
        .ok_or_else(|| format!("list {parent_path} not found"))?;
    match target.value {
        crate::obsp::payload::Value::Array(_) => Ok(target),
        _ => Err(format!("{parent_path} is not an array").into()),
    }
}

/// `(element count, offset of the first element)` of an array/list value.
fn list_body(blob: &[u8], list: &crate::obsp::payload::Target) -> Result<(usize, usize)> {
    let tag = blob.get(list.offset.wrapping_sub(1)).copied().unwrap_or(0);
    let body = match tag {
        0x0B => list.offset + 4,
        0x09 | 0x0A => list.offset + 5,
        other => return Err(format!("unsupported list tag {other:#x}").into()),
    };
    Ok((u32_at(blob, list.offset)? as usize, body))
}

/// Structural equality ignoring the byte-span fields of nested properties.
fn same_value(a: &crate::obsp::payload::Value, b: &crate::obsp::payload::Value) -> bool {
    use crate::obsp::payload::Value;
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            x.kind == y.kind
                && x.reserved == y.reserved
                && x.class_ref == y.class_ref
                && same_props(&x.properties, &y.properties)
        }
        (Value::Array(u), Value::Array(v)) => {
            u.len() == v.len() && u.iter().zip(v).all(|(x, y)| same_value(x, y))
        }
        (Value::Map(u), Value::Map(v)) => {
            u.len() == v.len()
                && u.iter()
                    .zip(v)
                    .all(|((k1, v1), (k2, v2))| same_value(k1, k2) && same_value(v1, v2))
        }
        _ => a == b,
    }
}

fn same_props(a: &[crate::obsp::payload::Property], b: &[crate::obsp::payload::Property]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.name == y.name && same_value(&x.value, &y.value))
}

/// Re-parse the edited blob and prove the edit did only what was intended:
/// every top-level property outside the edited list must be unchanged and the
/// list itself must equal the expected model. This catches broken index-based
/// name references (BOD stores names inline or as indices into a parse-order
/// vector).
fn verify_edit(
    old: &crate::obsp::payload::Payload,
    new_blob: &[u8],
    parent_path: &str,
    expected: &crate::obsp::payload::Value,
) -> Result<()> {
    let new = crate::obsp::payload::parse_payload(new_blob)?;
    let top = parent_path.split('.').next().unwrap_or(parent_path);
    for p in &old.properties {
        if p.name.eq_ignore_ascii_case(top) {
            continue;
        }
        match new.properties.iter().find(|q| q.name == p.name) {
            Some(q) if same_value(&q.value, &p.value) => {}
            Some(_) => return Err(format!("edit changed unrelated property {}", p.name).into()),
            None => return Err(format!("edit dropped property {}", p.name).into()),
        }
    }
    let got = crate::obsp::payload::resolve_path(new_blob, &new, parent_path)
        .ok_or_else(|| format!("{parent_path} missing after edit"))?;
    if !same_value(&got.value, expected) {
        return Err("edit corrupted the payload (name references?)".into());
    }
    Ok(())
}

/// Replace the blob at `blob_offset`/`blob_size` with `new_blob` and patch
/// every offset in the world table that follows the splice point.
fn replace_blob(
    file: &mut Vec<u8>,
    blob_offset: usize,
    blob_size: usize,
    new_blob: Vec<u8>,
    splice_at: usize,
) -> Result<()> {
    if blob_offset + blob_size > file.len() {
        return Err("blob range out of bounds".into());
    }
    let worlds = parse_worlds(file)?;
    let delta = new_blob.len() as i64 - blob_size as i64;
    file.splice(blob_offset..blob_offset + blob_size, new_blob);
    let shifted = |v: u32| -> Result<u32> {
        u32::try_from(v as i64 + delta)
            .map_err(|_| "worlds.bin offset out of range after edit".into())
    };
    for w in &worlds {
        if w.a as usize == blob_offset {
            write_u32_at(file, w.b_at, shifted(w.b)?)?;
        }
        if w.a as usize >= splice_at {
            write_u32_at(file, w.a_at, shifted(w.a)?)?;
        }
        for (base, pairs) in [(w.pairs1_at, &w.pairs1), (w.pairs2_at, &w.pairs2)] {
            for (i, &(off, size)) in pairs.iter().enumerate() {
                let at = base + i * 8;
                let new_off = if off as usize >= splice_at {
                    shifted(off)?
                } else {
                    off
                };
                let new_size = if off as usize == blob_offset {
                    shifted(size)?
                } else {
                    size
                };
                if new_off != off {
                    write_u32_at(file, at, new_off)?;
                }
                if new_size != size {
                    write_u32_at(file, at + 4, new_size)?;
                }
            }
        }
    }
    Ok(())
}

/// Remove the object at `path` (e.g. `Root.Objects[3]`) from its list inside
/// the blob at `blob_offset`/`blob_size`, shifting `worlds.bin` offsets.
pub fn delete_object(
    file: &mut Vec<u8>,
    blob_offset: usize,
    blob_size: usize,
    path: &str,
) -> Result<()> {
    let mut blob = file
        .get(blob_offset..blob_offset + blob_size)
        .ok_or("blob range out of bounds")?
        .to_vec();
    let parsed = crate::obsp::payload::parse_payload(&blob)?;
    let elem = crate::obsp::payload::resolve_path(&blob, &parsed, path)
        .ok_or_else(|| format!("object {path} not found"))?;
    let (start, end) = object_span(&blob, &elem)?;
    let (parent_path, index) = parent_list(path)?;
    let list = list_target(&blob, &parsed, &parent_path)?;
    let (count, _) = list_body(&blob, &list)?;
    if index >= count {
        return Err(format!("{path}: index {index} >= count {count}").into());
    }
    let mut expected = list.value.clone();
    if let crate::obsp::payload::Value::Array(items) = &mut expected {
        items.remove(index);
    }
    blob.drain(start..end);
    write_u32_at(&mut blob, list.offset, (count - 1) as u32)?;
    verify_edit(&parsed, &blob, &parent_path, &expected)?;
    replace_blob(file, blob_offset, blob_size, blob, blob_offset + start)
}

/// Clone the object at `path`, set its `Position` to `position`, and append it
/// to the same list. Returns the new object's path (`...[{count}]`).
pub fn spawn_object(
    file: &mut Vec<u8>,
    blob_offset: usize,
    blob_size: usize,
    path: &str,
    position: [f32; 3],
) -> Result<String> {
    use crate::obsp::payload::Value;
    let mut blob = file
        .get(blob_offset..blob_offset + blob_size)
        .ok_or("blob range out of bounds")?
        .to_vec();
    let parsed = crate::obsp::payload::parse_payload(&blob)?;
    let elem = crate::obsp::payload::resolve_path(&blob, &parsed, path)
        .ok_or_else(|| format!("object {path} not found"))?;
    let (start, end) = object_span(&blob, &elem)?;
    let (parent_path, index) = parent_list(path)?;
    let list = list_target(&blob, &parsed, &parent_path)?;
    let (count, body) = list_body(&blob, &list)?;
    if index >= count {
        return Err(format!("{path}: index {index} >= count {count}").into());
    }

    let mut copy = blob[start..end].to_vec();
    let mut expected = list.value.clone();
    {
        let Value::Array(items) = &mut expected else {
            return Err("list is not an array".into());
        };
        let mut item = items
            .get(index)
            .cloned()
            .ok_or_else(|| format!("{path} not in list"))?;
        let Value::Object(obj) = &mut item else {
            return Err(format!("{path} is not an object").into());
        };
        let prop = obj
            .properties
            .iter_mut()
            .find(|p| p.name.eq_ignore_ascii_case("Position"))
            .ok_or("template object has no Position")?;
        let Value::Array(vals) = &mut prop.value else {
            return Err("template Position is not an array".into());
        };
        if vals.len() < 3 {
            return Err("template Position has fewer than 3 floats".into());
        }
        for k in 0..3 {
            let sub = crate::obsp::payload::resolve_path(
                &blob,
                &parsed,
                &format!("{path}.Position[{k}]"),
            )
            .ok_or("template Position element not found")?;
            let rel = sub
                .offset
                .checked_sub(start)
                .ok_or("Position offset before element")?;
            copy.get_mut(rel..rel + 4)
                .ok_or("Position patch out of range")?
                .copy_from_slice(&position[k].to_le_bytes());
            vals[k] = Value::Float(position[k]);
        }
        items.push(item);
    }

    let insert_at = if count == 0 {
        body
    } else {
        let last = crate::obsp::payload::resolve_path(
            &blob,
            &parsed,
            &format!("{parent_path}[{}]", count - 1),
        )
        .ok_or("last list element not found")?;
        last.offset + last.size
    };
    let new_path = format!("{parent_path}[{count}]");
    blob.splice(insert_at..insert_at, copy);
    write_u32_at(&mut blob, list.offset, (count + 1) as u32)?;
    verify_edit(&parsed, &blob, &parent_path, &expected)?;
    replace_blob(file, blob_offset, blob_size, blob, blob_offset + insert_at)?;
    Ok(new_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sample_worlds_record() {
        let mut d = vec![3u8, 0];
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&3u32.to_le_bytes());
        d.extend_from_slice(b"abc");
        d.extend_from_slice(&0x1122334455667788u64.to_le_bytes());
        d.extend_from_slice(&7u32.to_le_bytes());
        d.extend_from_slice(&8u32.to_le_bytes());
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&9u32.to_le_bytes());
        d.extend_from_slice(&10u32.to_le_bytes());
        d.extend_from_slice(&2u32.to_le_bytes());
        d.extend_from_slice(&11u32.to_le_bytes());
        d.extend_from_slice(&12u32.to_le_bytes());
        d.extend_from_slice(&13u32.to_le_bytes());
        d.extend_from_slice(&14u32.to_le_bytes());
        d.push(1);
        let w = parse_worlds(&d).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].name, "abc");
        assert_eq!(w[0].hash, 0x1122334455667788);
        assert_eq!(w[0].pairs1, vec![(9, 10)]);
        assert_eq!(w[0].pairs2, vec![(11, 12), (13, 14)]);
        assert_eq!(w[0].flag, 1);
    }

    #[test]
    fn parse_sample_nodes_record() {
        let mut d = vec![2u8, 0];
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&0xAAu64.to_le_bytes());
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&0u32.to_le_bytes());
        for v in [1u32, 2, 3, 4] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        let n = parse_nodes(&d).unwrap();
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].hash, 0xAA);
        assert_eq!(n[0].entries1, vec![[1, 2, 3, 4]]);
        assert!(n[0].entries2.is_empty());
    }

    fn name_ref(name: &str) -> Vec<u8> {
        let mut v = vec![1u8];
        v.extend_from_slice(&crate::hash::crc64(name.as_bytes()).to_le_bytes());
        v.extend_from_slice(&(name.len() as u16).to_le_bytes());
        v.extend_from_slice(name.as_bytes());
        v
    }

    fn obj_bytes(pos: [f32; 3]) -> Vec<u8> {
        let mut v = vec![0x07u8];
        v.extend_from_slice(&0u32.to_le_bytes());
        v.push(1);
        v.extend(name_ref("spawnpoint"));
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend(name_ref("Position"));
        v.push(0x0B);
        v.extend_from_slice(&3u32.to_le_bytes());
        for c in pos {
            v.push(0x03);
            v.extend_from_slice(&c.to_le_bytes());
        }
        v
    }

    fn bod_blob(objs: &[[f32; 3]]) -> Vec<u8> {
        let mut v = b"BOD\xFD".to_vec();
        v.extend_from_slice(&[4, 0, 1, 0]);
        v.extend_from_slice(&4u32.to_le_bytes());
        v.extend_from_slice(&64u32.to_le_bytes());
        v.push(1);
        v.extend(name_ref("WorldGroup"));
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend(name_ref("Root"));
        v.push(0x07);
        v.extend_from_slice(&0u32.to_le_bytes());
        v.push(1);
        v.extend(name_ref("WorldGroup"));
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend(name_ref("Objects"));
        v.push(0x09);
        v.extend_from_slice(&(objs.len() as u32).to_le_bytes());
        v.push(0);
        for o in objs {
            v.extend(obj_bytes(*o));
        }
        v
    }

    fn worlds_file(names: &[&str], blobs: &[Vec<u8>]) -> Vec<u8> {
        let mut out = vec![3u8, 0];
        out.extend_from_slice(&(names.len() as u32).to_le_bytes());
        let mut pair_at = Vec::new();
        for name in names {
            out.extend_from_slice(&(name.len() as u32).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&0xAAu64.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&1u32.to_le_bytes());
            pair_at.push(out.len());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.push(0);
        }
        for (at, blob) in pair_at.iter().zip(blobs) {
            let off = out.len() as u32;
            out[*at..*at + 4].copy_from_slice(&off.to_le_bytes());
            out[*at + 4..*at + 8].copy_from_slice(&(blob.len() as u32).to_le_bytes());
            out.extend_from_slice(blob);
        }
        out
    }

    #[test]
    fn delete_and_spawn_objects_fix_offsets() {
        let blob0 = bod_blob(&[[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
        let blob1 = bod_blob(&[[7.0, 8.0, 9.0]]);
        let mut file = worlds_file(&["w0", "w1"], &[blob0, blob1]);

        let worlds = parse_worlds(&file).unwrap();
        let (off0, size0) = worlds[0].pairs2[0];
        let (off1, size1) = worlds[1].pairs2[0];

        delete_object(&mut file, off0 as usize, size0 as usize, "Root.Objects[0]").unwrap();
        let worlds = parse_worlds(&file).unwrap();
        let objs0 = collect_objects(&file, &worlds[0]).unwrap();
        assert_eq!(objs0.len(), 1);
        assert_eq!(objs0[0].position, [4.0, 5.0, 6.0]);
        let (off1b, size1b) = worlds[1].pairs2[0];
        assert_ne!(off1b, off1, "later blob must shift");
        assert_eq!(size1b, size1);
        let objs1 = collect_objects(&file, &worlds[1]).unwrap();
        assert_eq!(objs1.len(), 1);
        assert_eq!(objs1[0].position, [7.0, 8.0, 9.0]);

        let after_delete = file.clone();
        let (off0b, size0b) = worlds[0].pairs2[0];
        let new_path = spawn_object(
            &mut file,
            off0b as usize,
            size0b as usize,
            "Root.Objects[0]",
            [10.0, 11.0, 12.0],
        )
        .unwrap();
        assert_eq!(new_path, "Root.Objects[1]");
        let worlds = parse_worlds(&file).unwrap();
        let objs0 = collect_objects(&file, &worlds[0]).unwrap();
        assert_eq!(objs0.len(), 2);
        assert_eq!(objs0[1].position, [10.0, 11.0, 12.0]);

        let (off0c, size0c) = worlds[0].pairs2[0];
        delete_object(&mut file, off0c as usize, size0c as usize, &new_path).unwrap();
        assert_eq!(
            file, after_delete,
            "spawn+delete must round-trip byte-identical"
        );
    }
}
