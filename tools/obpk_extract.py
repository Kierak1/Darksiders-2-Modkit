import argparse
import json
import os
import struct


class FileReader:
    def __init__(self, path):
        self.path = path
        self.size = os.path.getsize(path)
        self.f = open(path, "rb")

    def read_at(self, offset, size):
        self.f.seek(offset)
        return self.f.read(size)


def u8(b, o):
    return b[o]


def u16(b, o):
    return struct.unpack_from("<H", b, o)[0]


def u32(b, o):
    return struct.unpack_from("<I", b, o)[0]


def u64(b, o):
    return struct.unpack_from("<Q", b, o)[0]


class ObpkError(Exception):
    pass


def parse_obpk(path):
    fr = FileReader(path)
    hdr = fr.read_at(0, 30)
    if hdr[0:4] != b"OBPK":
        raise ObpkError("bad magic")
    version = u8(hdr, 4)
    obj_a_off = u32(hdr, 0x09)
    obj_a_size = u32(hdr, 0x0D)
    buf_b_off = u32(hdr, 0x11)
    buf_b_size = u32(hdr, 0x15)
    tbl_c_off = u32(hdr, 0x19)
    streaming = u8(hdr, 0x1D)

    obj_a = fr.read_at(obj_a_off, obj_a_size)
    p = 0

    def ru32():
        nonlocal p
        v = u32(obj_a, p)
        p += 4
        return v

    def rbytes(n):
        nonlocal p
        v = obj_a[p:p + n]
        p += n
        return v

    group_count = ru32()
    entry_count = ru32()
    blob_size = ru32()
    ru32()
    blob = rbytes(blob_size)
    ru32()
    ru32()
    ru32()
    ru32()

    groups = []
    for gi in range(group_count):
        f0 = ru32()
        cnt = ru32()
        off1 = ru32()
        off2 = ru32()
        flags = rbytes(cnt)
        groups.append({"index": f0, "count": cnt, "hash_off": off1, "u16_off": off2, "flags": flags})

    total_entries = sum(g["count"] for g in groups)
    if total_entries != entry_count:
        raise ObpkError("group entries %d != entryCount %d" % (total_entries, entry_count))

    names_present = u8(obj_a, p)
    p += 1
    names = []
    if names_present:
        max_name = ru32()
        for g in groups:
            for i in range(g["count"]):
                h = u64(obj_a, p)
                p += 8
                ln = ru32()
                name = obj_a[p:p + ln].decode("ascii", "replace")
                p += ln
                names.append({"hash": h, "name": name})
    else:
        max_name = 0

    if p > obj_a_size:
        raise ObpkError("object A parse overrun: %d > %d" % (p, obj_a_size))

    tbl_c = fr.read_at(tbl_c_off, entry_count * 4)

    entries = []
    ei = 0
    gei = 0
    for gi, g in enumerate(groups):
        for i in range(g["count"]):
            off = u32(tbl_c, ei * 4)
            h1 = u64(blob, g["hash_off"] + i * 8)
            u1 = u16(blob, g["u16_off"] + i * 4)
            u2 = u16(blob, g["u16_off"] + i * 4 + 2)
            entry = {
                "group": gi,
                "index": ei,
                "flag": g["flags"][i],
                "offset": off,
                "hash": h1,
                "u16a": u1,
                "u16b": u2,
            }
            if ei < len(names):
                entry["name"] = names[ei]["name"]
                entry["name_hash"] = names[ei]["hash"]
            entries.append(entry)
            ei += 1
        gei += g["count"]

    fr.f.close()
    return {
        "path": path,
        "file_size": fr.size,
        "version": version,
        "streaming": streaming,
        "object_a_offset": obj_a_off,
        "object_a_size": obj_a_size,
        "buffer_b_offset": buf_b_off,
        "buffer_b_size": buf_b_size,
        "table_c_offset": tbl_c_off,
        "group_count": group_count,
        "entry_count": entry_count,
        "blob_size": blob_size,
        "max_name_len": max_name,
        "groups": groups,
        "entries": entries,
    }


def main():
    ap = argparse.ArgumentParser(description="OBPK catalog parser")
    ap.add_argument("upak", help="path to .upak file")
    ap.add_argument("--out", help="output JSON path")
    ap.add_argument("--dump-entry", type=int, help="dump raw payload of entry index to .bin")
    args = ap.parse_args()

    info = parse_obpk(args.upak)

    if args.out:
        with open(args.out, "w", encoding="utf-8") as f:
            json.dump(info, f, indent=1, default=lambda x: list(x) if isinstance(x, bytes) else x)

    print("file            :", info["path"])
    print("size            :", info["file_size"])
    print("version         :", info["version"])
    print("groups/entries  : %d / %d" % (info["group_count"], info["entry_count"]))
    print("object A        : 0x%X + 0x%X" % (info["object_a_offset"], info["object_a_size"]))
    print("buffer B        : 0x%X + 0x%X" % (info["buffer_b_offset"], info["buffer_b_size"]))
    print("table C         : 0x%X (%d u32)" % (info["table_c_offset"], info["entry_count"]))
    for e in info["entries"]:
        print("  [%2d] g%d flag=0x%02X off=0x%X u16=(%d,%d) %s" % (
            e["index"], e["group"], e["flag"], e["offset"], e["u16a"], e["u16b"],
            e.get("name", "<unnamed>")))

    if args.dump_entry is not None:
        e = info["entries"][args.dump_entry]
        fr = FileReader(args.upak)
        cap = 1 << 22
        data = fr.read_at(e["offset"], cap)
        fr.f.close()
        out_path = "entry_%03d.bin" % args.dump_entry
        with open(out_path, "wb") as f:
            f.write(data)
        print("dumped %d bytes to %s" % (len(data), out_path))


if __name__ == "__main__":
    main()
