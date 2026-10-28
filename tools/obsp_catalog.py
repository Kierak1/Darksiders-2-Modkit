import argparse
import re
import struct


def is_printable(b):
    return all(0x20 <= c <= 0x7E for c in b)


def parse_records(path, start=33, max_records=0):
    """Contiguously parse [u32 len][ascii name][u64 hash] records from `start`."""
    data = open(path, "rb").read()
    n = len(data)
    records = []
    i = start
    errors = 0
    while i + 4 + 8 <= n:
        (name_len,) = struct.unpack_from("<I", data, i)
        if name_len < 3 or name_len > 512:
            break
        name_start = i + 4
        name_end = name_start + name_len
        if not is_printable(data[name_start:name_end]):
            break
        (h,) = struct.unpack_from("<Q", data, name_end)
        name = data[name_start:name_end].decode("ascii", "replace")
        records.append((i, name, h, 4 + name_len + 8))
        i = name_end + 8
        if max_records and len(records) >= max_records:
            break
    return records


def main():
    ap = argparse.ArgumentParser(description="OBSP contiguous name/hash parser")
    ap.add_argument("obsp")
    ap.add_argument("--start", type=int, default=33)
    ap.add_argument("--max", type=int, default=0, help="stop after N records")
    ap.add_argument("--out", help="write TSV")
    args = ap.parse_args()

    rows = parse_records(args.obsp, args.start, args.max)
    print("parsed %d contiguous records (stopped cleanly)" % len(rows))
    lines = ["offset\tname\thash\tspan"]
    for off, name, h, span in rows:
        lines.append("%d\t%s\t0x%016X\t%d" % (off, re.sub(r"[\t\n\r]", ".", name), h, span))
    if args.out:
        with open(args.out, "w", encoding="utf-8") as f:
            f.write("\n".join(lines) + "\n")
        print("wrote", args.out)
    for off, name, h, span in rows[:40]:
        print("off=%8d  %-44s hash=0x%016X" % (off, name, h))


if __name__ == "__main__":
    main()
