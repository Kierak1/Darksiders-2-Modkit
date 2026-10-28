import argparse
import os
import re
import struct
import sys


def is_printable(b):
    return all(0x20 <= c <= 0x7E for c in b)


def scan_strings(path, min_len=3, max_len=512):
    """Scan for [u32 len][ascii name][u64 hash] records in an OBSP data blob."""
    data = open(path, "rb").read()
    results = []
    i = 0
    n = len(data)
    while i + 8 < n:
        (name_len,) = struct.unpack_from("<I", data, i)
        if min_len <= name_len <= max_len:
            start = i + 4
            end = start + name_len
            if end + 8 <= n and is_printable(data[start:end]):
                (h,) = struct.unpack_from("<Q", data, end)
                name = data[start:end].decode("ascii", "replace")
                results.append((i, name, h, end + 8 - i))
                i = end + 8
                continue
        i += 1
    return results


def main():
    ap = argparse.ArgumentParser(description="OBSP name/hash string-table scanner")
    ap.add_argument("obsp", help="path to scripts.obsp")
    ap.add_argument("--out", help="write CSV catalog")
    ap.add_argument("--filter", help="case-insensitive substring filter")
    ap.add_argument("--max", type=int, default=0, help="stop after N matches (0=all)")
    args = ap.parse_args()

    rows = scan_strings(args.obsp)
    if args.filter:
        rows = [r for r in rows if args.filter.lower() in r[1].lower()]
    if args.max:
        rows = rows[: args.max]

    print("found %d string records" % len(rows))
    lines = []
    for off, name, h, span in rows:
        lines.append("%d\t%s\t0x%016X\t%d" % (off, re.sub(r"[\x00-\x1F\x7F]", ".", name), h, span))
        print("off=%8d  %-48s hash=0x%016X" % (off, name, h))
    if args.out:
        with open(args.out, "w", encoding="utf-8") as f:
            f.write("offset\tname\thash\tspan\n")
            f.write("\n".join(lines))


if __name__ == "__main__":
    main()
