import argparse
import re
import struct
import sys


def is_printable(b):
    return all(0x20 <= c <= 0x7E for c in b)


def scan_records(path, max_records=0):
    """Find all valid [u32 len][ascii name][u64 hash] records across the blob.

    Advances by a full record when one is matched; otherwise skips one byte so
    records in every section are caught. Name must be reasonably long and the
    immediately preceding hash must be 'non-trivial' to reduce false positives.
    """
    data = open(path, "rb").read()
    n = len(data)
    records = []
    i = 0
    while i + 12 <= n:
        (ln,) = struct.unpack_from("<I", data, i)
        if 4 <= ln <= 512:
            ns = i + 4
            ne = ns + ln
            if ne + 8 <= n and is_printable(data[ns:ne]):
                (h,) = struct.unpack_from("<Q", data, ne)
                # Non-trivial hash: require both halves non-zero-ish to dodge padding
                if h != 0 and (h >> 32) != 0:
                    name = data[ns:ne].decode("ascii", "replace")
                    if len(name) >= 4:
                        records.append((i, name, h, 4 + ln + 8))
                        i = ne + 8
                        if max_records and len(records) >= max_records:
                            break
                        continue
        i += 1
    return records


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("obsp")
    ap.add_argument("--out", help="TSV output")
    ap.add_argument("--filter", help="case-insensitive keyword substring (repeatable)", action="append")
    ap.add_argument("--max", type=int, default=0)
    args = ap.parse_args()

    rows = scan_records(args.obsp, args.max)
    if args.filter:
        rows = [r for r in rows if any(f.lower() in r[1].lower() for f in args.filter)]
    print("scanned %d records" % len(rows))
    lines = ["offset\tname\thash"]
    for off, name, h, span in rows:
        lines.append("%d\t%s\t0x%016X" % (off, re.sub(r"[\t\n\r]", ".", name), h))
    if args.out:
        with open(args.out, "w", encoding="utf-8") as f:
            f.write("\n".join(lines) + "\n")
        print("wrote", args.out)
    for off, name, h, span in rows[:60]:
        print("off=%8d  %-48s hash=0x%016X" % (off, name, h))


if __name__ == "__main__":
    main()
