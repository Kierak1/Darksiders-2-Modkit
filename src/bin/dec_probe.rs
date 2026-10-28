//! Dev probe: dump Bone_Head sub-mesh bboxes / collapse state from a .dec.
//! Usage: cargo run --release --bin dec_probe -- <path.dec>

use std::env;
use std::fs;

struct Mesh {
    name: &'static str,
    ib: usize,
    b: usize,
    vc: usize,
    ic: usize,
}

/// A UV pair reader: `(bytes, offset) -> (u, v)`.
type UvReader = fn(&[u8], usize) -> (f32, f32);

const MESHES: [Mesh; 6] = [
    Mesh {
        name: "G1 m1 (eyes?)",
        ib: 0x177FB1,
        b: 0x178252,
        vc: 28,
        ic: 96,
    },
    Mesh {
        name: "G1 m2 (maskless head)",
        ib: 0x178A30,
        b: 0x17E5CA,
        vc: 854,
        ic: 3183,
    },
    Mesh {
        name: "G6 m1 (eyes?)",
        ib: 0x41B905,
        b: 0x41BBF9,
        vc: 28,
        ic: 96,
    },
    Mesh {
        name: "G6 m2 (brows)",
        ib: 0x41C3D7,
        b: 0x41D32D,
        vc: 128,
        ic: 681,
    },
    Mesh {
        name: "G6 m3 (head/hair)",
        ib: 0x41F72B,
        b: 0x4252C5,
        vc: 854,
        ic: 3183,
    },
    Mesh {
        name: "G6 m4 (mask main)",
        ib: 0x4342F9,
        b: 0x4375E5,
        vc: 432,
        ic: 2196,
    },
];

fn f32_at(d: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

fn u16_at(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(d[o..o + 2].try_into().unwrap())
}

/// Print `[u16 len][ascii]` records (the inline name-ref format used by the
/// serialized material/mesh records).
fn scan_strings(d: &[u8]) {
    let mut i = 2usize;
    while i + 2 < d.len() {
        let len = u16_at(d, i) as usize;
        if (3..=64).contains(&len) && i + 2 + len <= d.len() {
            let s = &d[i + 2..i + 2 + len];
            if s.iter().all(|&b| (32..127).contains(&b)) && s[0].is_ascii_alphabetic() {
                println!("{:#09X} len {:3} {}", i, len, String::from_utf8_lossy(s));
                i += 2 + len;
                continue;
            }
        }
        i += 1;
    }
}

/// Find the longest `{u64 hash, u32 len, name}` chain (brute force over the
/// first 1 MiB) and print the records in order.
fn scan_nametable(d: &[u8]) {
    let mut best: (usize, usize) = (0, 0);
    for start in 0..d.len().min(1 << 20) {
        let mut p = start;
        let mut n = 0usize;
        while p + 12 <= d.len() {
            let len = u32::from_le_bytes(d[p + 8..p + 12].try_into().unwrap()) as usize;
            if !(2..=120).contains(&len) || p + 12 + len > d.len() {
                break;
            }
            let s = &d[p + 12..p + 12 + len];
            if !(s.iter().all(|&b| (32..127).contains(&b)) && s[0].is_ascii_alphanumeric()) {
                break;
            }
            n += 1;
            p += 12 + len;
        }
        if n > best.1 {
            best = (start, n);
        }
    }
    let (start, n) = best;
    println!("name table at {:#X}: {} records", start, n);
    let mut p = start;
    let mut i = 0usize;
    while i < n {
        let hash = u64::from_le_bytes(d[p..p + 8].try_into().unwrap());
        let len = u32::from_le_bytes(d[p + 8..p + 12].try_into().unwrap()) as usize;
        let s = String::from_utf8_lossy(&d[p + 12..p + 12 + len]);
        println!("{:4} {:#018X} {}", i, hash, s);
        p += 12 + len;
        i += 1;
    }
}

/// Map armor.obpk entry names to their decompressed-payload mesh geometry.
/// `obpk` is the (single or chunked) OBPK, `dec` its flat decompressed payload.
fn scan_armor(obpk: &[u8], dec_path: &str, wanted: &[String]) {
    let chunks = ds2_modkit::pack::obpk::scan_chunks(obpk).expect("chunk scan");
    let (start, n) = longest_chain(obpk);
    let mut recs = Vec::new();
    let mut p = start;
    for _ in 0..n {
        let len = u32::from_le_bytes(obpk[p + 8..p + 12].try_into().unwrap()) as usize;
        recs.push(String::from_utf8_lossy(&obpk[p + 12..p + 12 + len]).into_owned());
        p += 12 + len;
    }
    println!("{} chunks, {} names", chunks.len(), recs.len());
    assert_eq!(chunks.len(), recs.len(), "chunk/name count mismatch");
    let d = fs::read(dec_path).expect("read dec");
    let mut off = vec![0usize; chunks.len() + 1];
    for (i, c) in chunks.iter().enumerate() {
        off[i + 1] = off[i] + c.usize_;
    }
    for name in wanted {
        let hits: Vec<usize> = recs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.as_str() == name)
            .map(|(i, _)| i)
            .collect();
        if hits.is_empty() {
            println!("{name}: not found");
            continue;
        }
        for idx in hits {
            let lo = off[idx];
            let chunk = &d[lo..off[idx + 1]];
            println!(
                "{name}: rec {idx} chunk {} bytes at dec {:#X}",
                chunk.len(),
                lo
            );
            match find_mesh(chunk) {
                Some((dh, vc, ic, b)) => {
                    let mut blo = [f32::MAX; 3];
                    let mut bhi = [f32::MIN; 3];
                    let mut bad = 0usize;
                    for i in 0..vc {
                        let o = b + i * 52;
                        if o + 52 > chunk.len() {
                            bad += 1;
                            continue;
                        }
                        let pos = [f32_at(chunk, o), f32_at(chunk, o + 4), f32_at(chunk, o + 8)];
                        for k in 0..3 {
                            blo[k] = blo[k].min(pos[k]);
                            bhi[k] = bhi[k].max(pos[k]);
                        }
                    }
                    println!(
                        "  data {:#X} vc {vc} ic {ic} B {:#X} bbox x {:.1}..{:.1} y {:.1}..{:.1} z {:.1}..{:.1} bad {bad}",
                        lo + dh,
                        lo + b,
                        blo[0], bhi[0], blo[1], bhi[1], blo[2], bhi[2]
                    );
                }
                None => println!("  no mesh header found"),
            }
        }
    }
}

/// Longest `{u64 hash, u32 len, name}` chain in `d`'s first MiB.
fn longest_chain(d: &[u8]) -> (usize, usize) {
    let mut best: (usize, usize) = (0, 0);
    for start in 0..d.len().min(1 << 20) {
        let mut p = start;
        let mut n = 0usize;
        while p + 12 <= d.len() {
            let len = u32::from_le_bytes(d[p + 8..p + 12].try_into().unwrap()) as usize;
            if !(2..=120).contains(&len) || p + 12 + len > d.len() {
                break;
            }
            let s = &d[p + 12..p + 12 + len];
            if !(s.iter().all(|&b| (32..127).contains(&b)) && s[0].is_ascii_alphanumeric()) {
                break;
            }
            n += 1;
            p += 12 + len;
        }
        if n > best.1 {
            best = (start, n);
        }
    }
    best
}

/// Find the `[0, vc, 0, ic, 0, ic*2, 2]` data header in a mesh chunk; returns
/// (header offset, vc, ic, B stream offset).
fn find_mesh(chunk: &[u8]) -> Option<(usize, usize, usize, usize)> {
    let lim = chunk.len().min(1 << 20);
    for o in 0..lim.saturating_sub(28) {
        let v = |k: usize| {
            u32::from_le_bytes(chunk[o + k * 4..o + k * 4 + 4].try_into().unwrap()) as usize
        };
        if v(0) != 0 || v(2) != 0 || v(4) != 0 || v(6) != 2 {
            continue;
        }
        let (vc, ic) = (v(1), v(3));
        if vc == 0 || vc > 200_000 || ic == 0 || ic > 2_000_000 || v(5) != ic * 2 {
            continue;
        }
        let b = o + 28 + ic * 2 + vc * 20 + 4;
        if b + vc * 52 > chunk.len() {
            continue;
        }
        // validate the B stream: f32 at +12 of every 16th record is 1.0
        let mut ok = true;
        for i in (0..vc).step_by(16) {
            if (f32_at(chunk, b + i * 52 + 12) - 1.0).abs() > 1e-4 {
                ok = false;
                break;
            }
        }
        if ok {
            return Some((o, vc, ic, b));
        }
    }
    None
}

fn parse_num(s: &str) -> usize {
    if let Some(hex) = s.strip_prefix("0x") {
        usize::from_str_radix(hex, 16).unwrap()
    } else {
        s.parse().unwrap()
    }
}

/// Print candidate `[u32;7]` mesh data headers in a byte range.
fn mesh_scan(chunk: &[u8], base: usize) {
    for o in 0..chunk.len().saturating_sub(28) {
        let v = |k: usize| u32::from_le_bytes(chunk[o + k * 4..o + k * 4 + 4].try_into().unwrap());
        let (vc, ic) = (v(1), v(3));
        if vc == 0
            || vc > 200_000
            || ic == 0
            || ic > 2_000_000
            || v(5) != ic.wrapping_mul(2)
            || !(1..=4).contains(&v(6))
        {
            continue;
        }
        println!(
            "{:#X}: [{}, {vc}, {}, {ic}, {}, {}, {}]",
            base + o,
            v(0),
            v(2),
            v(4),
            v(5),
            v(6)
        );
    }
}

/// Find maximal runs of 52-byte body B records ({u32 id, pos@4, 1.0@16,
/// normal@20, normal.z@32, tangent@36, 1.0@48}).
fn b_runs(chunk: &[u8], base: usize) {
    let ok = |o: usize| -> bool {
        o + 52 <= chunk.len()
            && (f32_at(chunk, o + 16) - 1.0).abs() < 1e-4
            && (f32_at(chunk, o + 48) - 1.0).abs() < 1e-4
    };
    let mut o = 0usize;
    while o + 52 <= chunk.len() {
        if !ok(o) {
            o += 1;
            continue;
        }
        let start = o;
        let mut n = 0usize;
        while ok(o) {
            n += 1;
            o += 52;
        }
        if n >= 4 {
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for i in 0..n {
                let p = [
                    f32_at(chunk, start + i * 52 + 4),
                    f32_at(chunk, start + i * 52 + 8),
                    f32_at(chunk, start + i * 52 + 12),
                ];
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            println!(
                "B run at {:#X}: {n} verts, bbox x {:.1}..{:.1} y {:.1}..{:.1} z {:.1}..{:.1}",
                base + start,
                lo[0],
                hi[0],
                lo[1],
                hi[1],
                lo[2],
                hi[2]
            );
        }
    }
}

/// Print sub-mesh descriptors: `u8 01, 5×u32, 6×f32 bbox, FF u16 name`.
fn descriptors(chunk: &[u8], base: usize) {
    for o in 0..chunk.len().saturating_sub(64) {
        if chunk[o] != 1 {
            continue;
        }
        let v =
            |k: usize| u32::from_le_bytes(chunk[o + 1 + k * 4..o + 5 + k * 4].try_into().unwrap());
        let vc = v(0) as usize;
        if vc == 0 || vc > 100_000 || v(1) != 64 {
            continue;
        }
        let name_off = o + 45;
        if chunk[name_off] != 0xFF {
            continue;
        }
        let len =
            u16::from_le_bytes(chunk[name_off + 1..name_off + 3].try_into().unwrap()) as usize;
        if len == 0 || len > 40 || name_off + 3 + len > chunk.len() {
            continue;
        }
        let name = &chunk[name_off + 3..name_off + 3 + len];
        if !name.iter().all(|&b| (32..127).contains(&b)) {
            continue;
        }
        let f = |k: usize| f32_at(chunk, o + 21 + k * 4);
        println!(
            "{:#X} flag {o:6} vc {vc:6} five [{}, {}, {}, {}, {}] bbox x {:.1}..{:.1} y {:.1}..{:.1} z {:.1}..{:.1} {}",
            base + o,
            v(0), v(1), v(2), v(3), v(4),
            f(0), f(3), f(1), f(4), f(2), f(5),
            String::from_utf8_lossy(name)
        );
    }
}

/// Print a parsed BOD/OBSP payload tree (classes and scalar properties).
fn dump_bod(d: &[u8], off: usize, max_items: usize, depth: usize) {
    use ds2_modkit::obsp::payload as obsp_payload;
    let limit = d.len().saturating_sub(off).min(1 << 20);
    let blob = &d[off..off + limit];
    let Ok(p) = obsp_payload::parse_payload(blob) else {
        println!("{off:#X}: not a payload");
        return;
    };
    println!(
        "{off:#X}: class {:?} complete {} props {} defs_end {:#X} values_end {:#X}",
        p.class_ref,
        p.complete,
        p.properties.len(),
        p.defs_end,
        p.values_end
    );
    fn walk(
        props: &[ds2_modkit::obsp::payload::Property],
        blob: &[u8],
        indent: usize,
        max_items: usize,
        depth: usize,
    ) {
        use ds2_modkit::obsp::payload::Value;
        let pad = "  ".repeat(indent);
        for q in props {
            match &q.value {
                Value::Object(o) => {
                    println!(
                        "{pad}{} = obj({}) [{} props]",
                        q.name,
                        o.class_ref.clone().unwrap_or_default(),
                        o.properties.len()
                    );
                    if indent / 2 < depth {
                        walk(&o.properties, blob, indent + 1, max_items, depth);
                    }
                }
                Value::Array(items) => {
                    println!("{pad}{} = [{} items]", q.name, items.len());
                    if indent / 2 < depth {
                        for (i, item) in items.iter().take(max_items).enumerate() {
                            if let Value::Object(o) = item {
                                println!(
                                    "{pad}  [{i}] obj({}) [{} props]",
                                    o.class_ref.clone().unwrap_or_default(),
                                    o.properties.len()
                                );
                                walk(&o.properties, blob, indent + 2, max_items, depth);
                            } else {
                                println!("{pad}  [{i}] {item}");
                            }
                        }
                        if items.len() > max_items {
                            println!("{pad}  ... {} more", items.len() - max_items);
                        }
                    }
                }
                v => {
                    let raw = q
                        .offset
                        .checked_sub(1)
                        .map(|s| {
                            let e = (s + 24).min(blob.len());
                            blob[s..e]
                                .iter()
                                .map(|b| format!("{b:02x}"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .unwrap_or_default();
                    println!("{pad}{} = {v}   [{raw}]", q.name);
                }
            }
        }
    }
    walk(&p.properties, blob, 0, max_items, depth);
}

fn half_to_f32(h: u16) -> f32 {
    let sign = (h as u32 >> 15) << 31;
    let exp = ((h >> 10) & 0x1F) as u32;
    let mant = (h & 0x3FF) as u32;
    let bits = match exp {
        0 if mant == 0 => sign,
        0 => {
            let mut e = 127 - 15 + 1i32;
            let mut m = mant;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | ((e as u32) << 23) | ((m & 0x3FF) << 13)
        }
        0x1F => sign | 0x7F80_0000 | (mant << 13),
        _ => sign | ((exp + 127 - 15) << 23) | (mant << 13),
    };
    f32::from_bits(bits)
}

/// Scan a batch's tail for a per-vertex 2-component stream (f32, f16 or u16
/// pairs), ranked by mean component distance across the mesh's edges. A real
/// UV stream varies smoothly between connected vertices, so it ranks far
/// below packed/normal data.
fn uv_scan(d: &[u8], off: usize, vc: usize, ic: usize, ib: usize, pos: usize, end: usize) {
    let tail = pos + vc * 12;
    if tail >= end || ib + ic * 2 > d.len() || pos + vc * 12 > end {
        println!("bad ranges: tail {tail:#X} end {end:#X}");
        return;
    }
    let tail_size = end - tail;
    println!("off {off:#X} vc {vc} ic {ic} ib {ib:#X} pos {pos:#X} tail {tail:#X}..{end:#X} ({tail_size} B)");
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for t in (0..ic.saturating_sub(2)).step_by(3) {
        let a = u16_at(d, ib + t * 2) as usize;
        let b = u16_at(d, ib + (t + 1) * 2) as usize;
        let c = u16_at(d, ib + (t + 2) * 2) as usize;
        if a >= vc || b >= vc || c >= vc {
            continue;
        }
        for (x, y) in [(a, b), (b, c), (c, a)] {
            edges.push((x.min(y), x.max(y)));
        }
    }
    edges.sort_unstable();
    edges.dedup();
    println!("{} edges", edges.len());

    // kind: (name, stride, reader)
    let kinds: [(&str, usize, UvReader); 3] = [
        ("f32", 8, |d, o| (f32_at(d, o), f32_at(d, o + 4))),
        ("f16", 4, |d, o| {
            (half_to_f32(u16_at(d, o)), half_to_f32(u16_at(d, o + 2)))
        }),
        ("u16", 4, |d, o| {
            (
                u16_at(d, o) as f32 / 65535.0,
                u16_at(d, o + 2) as f32 / 65535.0,
            )
        }),
    ];
    for (name, stride, read) in kinds {
        let mut best: Vec<(f64, usize)> = Vec::new();
        let mut base = 0usize;
        while base + vc * stride <= tail_size {
            let mut ok = true;
            for i in (0..vc).step_by((vc / 16).max(1)) {
                let (a, b) = read(d, tail + base + i * stride);
                if !a.is_finite() || !b.is_finite() {
                    ok = false;
                    break;
                }
            }
            if ok {
                let mut total = 0f64;
                let mut valid = true;
                for &(x, y) in &edges {
                    let (ax, ay) = read(d, tail + base + x * stride);
                    let (bx, by) = read(d, tail + base + y * stride);
                    if !ax.is_finite() || !ay.is_finite() || !bx.is_finite() || !by.is_finite() {
                        valid = false;
                        break;
                    }
                    total += (((ax - bx) as f64).powi(2) + ((ay - by) as f64).powi(2)).sqrt();
                }
                if valid {
                    best.push((total / edges.len() as f64, base));
                }
            }
            base += 4;
        }
        best.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        for (mean, b) in best.iter().take(4) {
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for i in 0..vc {
                let (a, c) = read(d, tail + b + i * stride);
                lo = lo.min(a).min(c);
                hi = hi.max(a).max(c);
            }
            println!(
                "{name} base +{b} ({:#X}) mean {mean:.4} range {lo:.3}..{hi:.3}",
                tail + b
            );
        }
        if best.is_empty() {
            println!("{name}: no NaN-free window fits ({vc} verts)");
        }
    }
}

/// Find which (stride, component offset) of a f32 stream varies smoothly
/// across mesh edges. Interleaved vertex records store u and v as separate
/// components (the character format does), so scan scalar components.
fn component_scan(
    edges: &[(usize, usize)],
    tail: usize,
    tail_size: usize,
    vc: usize,
    d: &[u8],
    kind: &str,
) {
    let strides = [
        4usize, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 48, 52, 56, 60, 64, 68, 72,
    ];
    let step = if kind == "f32" { 4 } else { 2 };
    let read: fn(&[u8], usize) -> f32 = match kind {
        "f16" => |d, o| half_to_f32(u16_at(d, o)),
        "u16" => |d, o| u16_at(d, o) as f32 / 65535.0,
        _ => |d, o| f32_at(d, o),
    };
    let mut best: Vec<(f64, f64, usize, usize)> = Vec::new(); // mean, finite_frac, stride, off
    for stride in strides {
        if vc * stride > tail_size {
            continue;
        }
        let mut off = 0usize;
        while off + step <= stride {
            let mut total = 0f64;
            let mut used = 0usize;
            for &(x, y) in edges {
                let a = read(d, tail + off + x * stride);
                let b = read(d, tail + off + y * stride);
                if !a.is_finite() || !b.is_finite() {
                    continue;
                }
                total += (a - b).abs() as f64;
                used += 1;
            }
            let frac = used as f64 / edges.len() as f64;
            if used > 0 && frac > 0.7 {
                best.push((total / used as f64, frac, stride, off));
            }
            off += step;
        }
    }
    best.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    for (mean, frac, stride, off) in best.iter().take(14) {
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for i in 0..vc {
            let v = read(d, tail + off + i * stride) as f64;
            if v.is_finite() {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        println!(
            "stride {stride:3} off +{off:3} mean {mean:.4} finite {frac:.2} range {lo:.3}..{hi:.3}"
        );
    }
    if best.is_empty() {
        println!("no smooth {kind} component found");
    }
}

fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: dec_probe <path.dec> [strings]");
    let mode = env::args().nth(2).unwrap_or_else(|| "meshes".into());
    let d = fs::read(&path).expect("read");
    println!("{} ({} bytes)", path, d.len());
    if mode == "strings" {
        scan_strings(&d);
        return;
    }
    if mode == "nametable" {
        scan_nametable(&d);
        return;
    }
    if mode == "bod" {
        let off = parse_num(env::args().nth(3).expect("offset").as_str());
        let max_items: usize = env::args().nth(4).and_then(|s| s.parse().ok()).unwrap_or(4);
        let depth: usize = env::args().nth(5).and_then(|s| s.parse().ok()).unwrap_or(3);
        dump_bod(&d, off, max_items, depth);
        return;
    }
    if mode == "meshscan" {
        let off = parse_num(env::args().nth(3).expect("offset").as_str());
        let len = parse_num(env::args().nth(4).expect("len").as_str());
        mesh_scan(&d[off..off + len], off);
        return;
    }
    if mode == "bruns" {
        let off = parse_num(env::args().nth(3).expect("offset").as_str());
        let len = parse_num(env::args().nth(4).expect("len").as_str());
        b_runs(&d[off..off + len], off);
        return;
    }
    if mode == "desc" {
        let off = parse_num(env::args().nth(3).expect("offset").as_str());
        let len = parse_num(env::args().nth(4).expect("len").as_str());
        descriptors(&d[off..off + len], off);
        return;
    }
    if mode == "uvscan" {
        let n = |k: usize| parse_num(env::args().nth(k).expect("arg").as_str());
        uv_scan(&d, n(3), n(4), n(5), n(6), n(7), n(8));
        return;
    }
    if mode == "compscan" {
        let n = |k: usize| parse_num(env::args().nth(k).expect("arg").as_str());
        let (vc, ic, ib, pos, end) = (n(4), n(5), n(6), n(7), n(8));
        let tail = pos + vc * 12;
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for t in (0..ic.saturating_sub(2)).step_by(3) {
            let (a, b, c) = (
                u16_at(&d, ib + t * 2) as usize,
                u16_at(&d, ib + (t + 1) * 2) as usize,
                u16_at(&d, ib + (t + 2) * 2) as usize,
            );
            if a >= vc || b >= vc || c >= vc {
                continue;
            }
            for (x, y) in [(a, b), (b, c), (c, a)] {
                edges.push((x.min(y), x.max(y)));
            }
        }
        edges.sort_unstable();
        edges.dedup();
        println!(
            "tail {tail:#X}..{end:#X} ({} B), {} edges",
            end - tail,
            edges.len()
        );
        let kind = env::args().nth(9).unwrap_or_else(|| "f32".into());
        component_scan(&edges, tail, end.saturating_sub(tail), vc, &d, &kind);
        return;
    }
    if mode == "armor" {
        let dec_path = env::args()
            .nth(3)
            .expect("usage: dec_probe <obpk> armor <armor.dec> <name...>");
        let names: Vec<String> = env::args().skip(4).collect();
        scan_armor(&d, &dec_path, &names);
        return;
    }
    for m in &MESHES {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        let mut distinct = std::collections::HashSet::new();
        for i in 0..m.vc {
            let o = m.b + i * 52;
            let p = [f32_at(&d, o), f32_at(&d, o + 4), f32_at(&d, o + 8)];
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
            distinct.insert((p[0].to_bits(), p[1].to_bits(), p[2].to_bits()));
        }
        let mut maxi = 0u16;
        let mut tris = 0usize;
        let mut zero_tris = 0usize;
        for t in 0..m.ic / 3 {
            let (a, b, c) = (
                u16_at(&d, m.ib + t * 6),
                u16_at(&d, m.ib + t * 6 + 2),
                u16_at(&d, m.ib + t * 6 + 4),
            );
            maxi = maxi.max(a).max(b).max(c);
            if a == b || b == c || a == c {
                zero_tris += 1;
            } else {
                tris += 1;
            }
        }
        println!(
            "{}: vc {} ic {} bbox x {:.2}..{:.2} y {:.2}..{:.2} z {:.2}..{:.2} distinct {} live_tris {} degen {} max_idx {}",
            m.name, m.vc, m.ic, lo[0], hi[0], lo[1], hi[1], lo[2], hi[2],
            distinct.len(), tris, zero_tris, maxi
        );
        // sample first vertices
        for i in [0usize, 1, 2] {
            let o = m.b + i * 52;
            println!(
                "    v{i}: ({:.2}, {:.2}, {:.2})",
                f32_at(&d, o),
                f32_at(&d, o + 4),
                f32_at(&d, o + 8)
            );
        }
    }
}
