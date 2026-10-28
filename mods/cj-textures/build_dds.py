"""Build the CJ diffuse-only texture set.

Only the diffuse blobs get CJ art; every other texture (normal/spec/emissive/
effect/clut) is restored from its `.orig` stock copy so lighting stays intact.
Blob -> name mapping follows the OBPK header resource order (16 = Base_Mask_Diff,
1 = Base_Hair_Diff, 11 = reaperform_diff/body, 12 = reaper_scythe_diff).
"""
import os
import shutil
import struct

from PIL import Image

TEX = r'D:\Dev\Darksiders 2 Modkit\Game\Darksiders II Deathinitive Edition\media\_modkit_work\death_textures'
CJ = r'C:\Users\Bigga\AppData\Local\Temp\opencode\cj\textures'
SLOTS = {1, 2, 3, 11, 12, 16}


def load(name):
    return Image.open(os.path.join(CJ, name)).convert('RGBA')


def scale(src, size):
    return src.resize(size, Image.LANCZOS)


def tile(src, size):
    out = Image.new('RGBA', size)
    tw, th = src.size
    for y in range(0, size[1], th):
        for x in range(0, size[0], tw):
            out.paste(src, (x, y))
    return out


def rgb565(c):
    r, g, b = c
    return ((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)


def expand565(v):
    r, g, b = (v >> 11) & 0x1F, (v >> 5) & 0x3F, v & 0x1F
    return ((r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2))


def bc1_color_block(pixels, force_four=True):
    c0 = rgb565((max(p[0] for p in pixels), max(p[1] for p in pixels), max(p[2] for p in pixels)))
    c1 = rgb565((min(p[0] for p in pixels), min(p[1] for p in pixels), min(p[2] for p in pixels)))
    if c0 == c1:
        return struct.pack('<HHI', c0, c1, 0)
    if force_four and c0 < c1:
        c0, c1 = c1, c0
    p0, p1 = expand565(c0), expand565(c1)
    pal = [p0, p1,
           tuple((2 * p0[i] + p1[i]) // 3 for i in range(3)),
           tuple((p0[i] + 2 * p1[i]) // 3 for i in range(3))]
    idx = 0
    for k, p in enumerate(pixels):
        best = min(range(4), key=lambda j: sum((p[i] - pal[j][i]) ** 2 for i in range(3)))
        idx |= best << (2 * k)
    return struct.pack('<HHI', c0, c1, idx)


def bc3_alpha_block(alphas):
    a0, a1 = max(alphas), min(alphas)
    if a0 == a1:
        return bytes([a0, a1, 0, 0, 0, 0, 0, 0])
    if a0 < a1:
        a0, a1 = a1, a0
    pal = [a0, a1] + [(i * a0 + (7 - i) * a1) // 7 for i in range(1, 7)]
    bits = 0
    for k, a in enumerate(alphas):
        bits |= min(range(8), key=lambda j: abs(a - pal[j])) << (3 * k)
    return bytes([a0, a1]) + bits.to_bytes(6, 'little')


def block_pixels(px, bx, by, w, h):
    return [px[x, y] for y in range(by, min(by + 4, h)) for x in range(bx, min(bx + 4, w))]


def encode_mips(img, fmt, num_mips):
    w, h = img.size
    out = bytearray()
    for _ in range(num_mips):
        rgb, alpha = img.convert('RGB'), (img.getchannel('A') if fmt == 'DXT5' else None)
        px, ap = rgb.load(), (alpha.load() if alpha else None)
        for by in range(0, h, 4):
            for bx in range(0, w, 4):
                pixels = block_pixels(px, bx, by, w, h)
                if fmt == 'DXT5':
                    alphas = [ap[x, y] for y in range(by, min(by + 4, h)) for x in range(bx, min(bx + 4, w))]
                    out += bc3_alpha_block(alphas) + bc1_color_block(pixels, force_four=False)
                else:
                    out += bc1_color_block(pixels)
        if w == 1 and h == 1:
            break
        w, h = max(1, w // 2), max(1, h // 2)
        img = img.resize((w, h), Image.LANCZOS)
    return bytes(out)


def target(idx, size):
    head = load('head.png')
    body = load('upper body.png')
    legs = load('legs.png')
    if idx == 1:
        return scale(head.crop((0, 0, 70, 60)), size)
    if idx == 2:
        return Image.new('RGBA', size, (128, 128, 255, 255))
    if idx == 3:
        return scale(head.crop((55, 95, 92, 128)), size)
    if idx == 11:
        return skin(size)
    if idx == 12:
        return scale(legs, size)
    if idx == 16:
        from PIL import ImageEnhance
        return ImageEnhance.Brightness(scale(head, size)).enhance(1.35)
    raise KeyError(idx)


def skin(size):
    """Procedural CJ skin: base tone with soft variation (no repeated faces)."""
    import random
    random.seed(7)
    small = Image.new('RGB', (256, 256))
    px = small.load()
    for y in range(256):
        for x in range(256):
            n = random.randint(-10, 10)
            px[x, y] = (122 + n, 82 + n, 60 + n)
    return small.resize(size, Image.LANCZOS).convert('RGBA')


def face_fit(size):
    head = load('head.png')
    w, h = size
    canvas = Image.new('RGBA', size, (120, 78, 55, 255))
    face = head.crop((52, 0, 128, 104))
    fw, fh = int(w * 0.86), int(h * 0.86)
    f = face.resize((fw, fh), Image.LANCZOS)
    canvas.paste(f, ((w - fw) // 2, int(h * 0.10)), f)
    hair = head.crop((0, 0, 70, 60)).resize((w, int(h * 0.18)), Image.LANCZOS)
    canvas.paste(hair, (0, 0))
    return canvas


def main():
    names = sorted([n for n in os.listdir(TEX) if n.endswith('.dds')], key=lambda n: int(n.split('_')[1]))
    for name in names:
        idx = int(name.split('_')[1])
        path = os.path.join(TEX, name)
        bak = path + '.orig'
        assert os.path.exists(bak), 'missing' + bak
        if idx not in SLOTS:
            shutil.copyfile(bak, path)
            print('%-28s restored' % name)
            continue
        orig = open(bak, 'rb').read()
        header = orig[:128]
        fourcc = bytes(header[84:88])
        fmt = 'DXT1' if fourcc == b'DXT1' else 'DXT5'
        mips = max(struct.unpack_from('<I', header, 28)[0], 1)
        width = struct.unpack_from('<I', header, 16)[0]
        height = struct.unpack_from('<I', header, 12)[0]
        body = encode_mips(target(idx, (width, height)), fmt, mips)
        expect = len(orig) - 128
        assert len(body) <= expect, (name, len(body), expect)
        body += bytes(expect - len(body))
        open(path, 'wb').write(header + body)
        chk = Image.open(path)
        assert chk.size == (width, height)
        print('%-28s CJ %s %dx%d' % (name, fmt, width, height))


if __name__ == '__main__':
    main()
