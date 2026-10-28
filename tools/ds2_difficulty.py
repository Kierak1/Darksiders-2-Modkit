#!/usr/bin/env python3
"""Difficulty scaling mod — patch the DifficultyLevel*Scales multipliers in
media/scripts.obsp (Darksiders II Deathinitive Edition).

The exe reads these properties lazily via CRC64 (see docs/difficulty-script-modding.md);
their values are float arrays (one per difficulty) stored right after the
property name in the gameplay-data instance. This tool locates the *active*
instance (the last one that carries >=3 float values) and rewrites the floats.

Usage:
  python ds2_difficulty.py health 2.0 damage 2.0
  python ds2_difficulty.py health 2.5
  python ds2_difficulty.py --restore            # restore from .bak
  python ds2_difficulty.py --dump               # show current scale values w/o patching
"""
import os
import shutil
import struct
import sys

GAME_MEDIA = os.path.join(os.path.dirname(__file__), "..", "Game",
                          "Darksiders II Deathinitive Edition", "media")
OBSP = os.path.join(GAME_MEDIA, "scripts.obsp")

PROPS = {
    "health": b"DifficultyLevelHealthScales",
    "damage": b"DifficultyLevelDamageScales",
    "pressure": b"DifficultyLevelCombatPressureScales",
    "maxpressure": b"DifficultyLevelMaxPressureScales",
}


def instances(data, name):
    out, start = [], 0
    while True:
        i = data.find(name, start)
        if i < 0:
            break
        out.append(i)
        start = i + 1
    return out


def float_offsets_after(data, name, span=24, count=3):
    """Return up to `count` float offsets after the name with plausible
    multiplier values (0.5 <= v <= 100)."""
    n = len(name)
    res = []
    for j in range(0, span):
        off = j + n
        if off + 4 > len(data):
            break
        v = struct.unpack("<f", data[off:off + 4])[0]
        if 0.5 <= v <= 100.0:
            res.append((off, v))
        if len(res) >= count:
            break
    return res


def plausible(v):
    return 0.5 <= v <= 100.0


def active_instance(data, name):
    """Pick the instance with the most plausible multiplier floats right after it."""
    best = (None, [])
    for o in instances(data, name):
        rel = []
        for j in range(0, 28):
            off = o + len(name) + j
            if off + 4 > len(data) or off - 1 < 0:
                break
            # True multiplier floats are immediately preceded by a 0x03 marker
            # byte; this rejects shifted/marker-alias floats.
            if data[off - 1] != 0x03:
                continue
            v = struct.unpack("<f", data[off:off + 4])[0]
            if plausible(v):
                rel.append((off, v))
        if len(rel) > len(best[1]):
            best = (o, rel)
    return best


def dump():
    data = open(OBSP, "rb").read()
    for key, name in PROPS.items():
        o, rel = active_instance(data, name)
        if o is None:
            print("%-12s  (no instance found)" % key)
            continue
        vals = ["%.2f@0x%X" % (v, off) for off, v in rel]
        print("%-12s  name@0x%X  values: %s" % (key, o, ", ".join(vals)))


def patch(specs):
    if not os.path.exists(OBSP):
        raise SystemExit("scripts.obsp not found: %s" % OBSP)
    bak = OBSP + ".bak"
    if not os.path.exists(bak):
        shutil.copy2(OBSP, bak)
        print("backup ->", bak)
    data = bytearray(open(OBSP, "rb").read())

    for key, mult in specs.items():
        if key not in PROPS:
            raise SystemExit("unknown prop %r (choose %s)" % (key, ", ".join(PROPS)))
        o, rel = active_instance(data, PROPS[key])
        if o is None:
            print("  ! no instance for %s" % key)
            continue
        # patch every float >= 0.5 in the run
        patched = 0
        for off, v in rel:
            struct.pack_into("<f", data, off, float(mult))
            patched += 1
        print("  %-11s -> x%s (%d values @ 0x%X)" % (key, mult, patched, o))

    open(OBSP, "wb").write(data)
    print("patched scripts.obsp (restore with --restore)")


def main():
    args = [a for a in sys.argv[1:]]
    if "--restore" in args:
        bak = OBSP + ".bak"
        if not os.path.exists(bak):
            raise SystemExit("no backup found")
        shutil.copy2(bak, OBSP)
        print("restored from", bak)
        return
    if "--dump" in args or not args:
        dump()
        return
    specs = {}
    i = 0
    while i < len(args):
        k = args[i].lower()
        if k == "--dump":
            dump(); return
        v = args[i + 1] if i + 1 < len(args) else None
        if v is None or k not in PROPS:
            raise SystemExit("usage: ds2_difficulty.py [health|damage|pressure|maxpressure] <mult> [...]")
        specs[k] = v
        i += 2
    patch(specs)


if __name__ == "__main__":
    main()
