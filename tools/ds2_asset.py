#!/usr/bin/env python3
"""Extract + repack a named asset (OBPK) from media/maps.upak via manifest.bin.

Usage:
  python ds2_asset.py extract characters/death            # -> _modkit_work/death.{obpk,dec,hdr} + textures/
  python ds2_asset.py repack  characters/death             # -> _modkit_work/death_repacked.obpk
  python ds2_asset.py list                                  # dump assets with offsets/magics
"""
import os
import struct
import sys

sys.path.insert(0, r"D:\dr2\Darksiders Hook")
from modkit_tools import obpk, textures

GAME_MEDIA = os.path.join(os.path.dirname(__file__), "..", "Game",
                          "Darksiders II Deathinitive Edition", "media")
WORK = os.path.join(GAME_MEDIA, "_modkit_work")
MANIFEST = os.path.join(GAME_MEDIA, "manifest.bin")


def parse_files():
    data = open(MANIFEST, "rb").read()
    ptr = 0
    version = struct.unpack("<I", data[ptr:ptr+4])[0]; ptr += 4 + 40
    np = struct.unpack("<I", data[ptr:ptr+4])[0]; ptr += 8
    paths = []
    for _ in range(np):
        ptr += 8
        ln = struct.unpack("<I", data[ptr:ptr+4])[0]; ptr += 4
        paths.append(data[ptr:ptr+ln].decode("ascii", "ignore")); ptr += ln
    npk = struct.unpack("<I", data[ptr:ptr+4])[0]; ptr += 4
    pkgs = []
    for _ in range(npk):
        ptr += 1
        ln = struct.unpack("<H", data[ptr:ptr+2])[0]; ptr += 2
        pkgs.append(data[ptr:ptr+ln].decode("ascii", "ignore")); ptr += ln
    nf = struct.unpack("<I", data[ptr:ptr+4])[0]; ptr += 4
    files = []
    for _ in range(nf):
        fl = data[ptr]; ptr += 1
        name = data[ptr:ptr+fl].decode("ascii", "ignore"); ptr += fl
        ptr += 8
        us0, us1 = struct.unpack("<HH", data[ptr:ptr+4]); ptr += 4
        al = data[ptr]; ptr += 1 + 2*al
        uc0, uc1, b1, b2 = data[ptr:ptr+4]; ptr += 4
        off = struct.unpack("<Q", data[ptr:ptr+8])[0]; ptr += 8
        files.append({"name": name, "path": paths[us1], "pkg": uc1, "offset": off})
    return pkgs, files


def find_asset(name):
    pkgs, files = parse_files()
    nm = name.replace("\\", "/").lower()
    for f in files:
        full = os.path.join(f["path"], f["name"]).replace("\\", "/").lower()
        if full == nm or f["name"].lower() == nm:
            return f
    raise SystemExit("asset not found in manifest: %s" % name)


def carve(f):
    upak = os.path.join(GAME_MEDIA, pkgs_name(f) + ".upak")
    with open(upak, "rb") as src:
        src.seek(f["offset"])
        raw = src.read(size_of(f))
    os.makedirs(WORK, exist_ok=True)
    out = os.path.join(WORK, f["name"].replace("\\", "_") + ".obpk")
    open(out, "wb").write(raw)
    return out


def pkgs_name(f):
    pkgs, files = parse_files()
    return pkgs[f["pkg"]]


def size_of(f):
    pkgs, files = parse_files()
    by = sorted([x for x in files if x["pkg"] == f["pkg"]], key=lambda x: x["offset"])
    idx = [x for x in by].index([x for x in by if x["name"] == f["name"] and x["path"] == f["path"]][0])
    if idx < len(by) - 1:
        return by[idx+1]["offset"] - f["offset"]
    upak = os.path.join(GAME_MEDIA, pkgs_name(f) + ".upak")
    return os.path.getsize(upak) - f["offset"]


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "list"
    if cmd == "list":
        pkgs, files = parse_files()
        for f in files:
            if f["pkg"] != 0xFF:
                print("off=0x%-9X %s" % (f["offset"], os.path.join(f["path"], f["name"])))
        return
    asset = sys.argv[2]
    f = find_asset(asset)
    print("asset:", os.path.join(f["path"], f["name"]), "pkg", pkgs_name(f), "off 0x%X" % f["offset"])
    if cmd == "extract":
        obpk_path = carve(f)
        dec = os.path.join(WORK, f["name"].replace("\\", "_") + ".dec")
        obpk.decompress_obpk(obpk_path, dec)
        tex = os.path.join(WORK, f["name"].replace("\\", "_") + "_textures")
        textures.extract_dds_textures(dec, tex)
        print("textures ->", tex)
    elif cmd == "repack":
        base = os.path.join(WORK, f["name"].replace("\\", "_"))
        dec = base + ".dec"
        obpk_src = base + ".obpk"
        in_tex = os.path.join(os.path.dirname(dec), f["name"].replace("\\", "_") + "_textures")
        # inject relies on textures_dir next to .dec named "textures"; copy in
        import shutil
        td = os.path.join(os.path.dirname(dec), "textures")
        if os.path.isdir(in_tex) and not os.path.isdir(td):
            shutil.copytree(in_tex, td)
        obpk.compress_obpk(dec, base + "_repacked.obpk", orig_obpk_path=obpk_src)
    else:
        raise SystemExit("unknown cmd")


if __name__ == "__main__":
    main()
