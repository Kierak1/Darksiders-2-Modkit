"""Stage 1b: same-size CJ head swap into death.dec's head meshes.

Targets (same counts, so no structural changes):
  G1 m2 `death_maskless` head   IB @0x178A30  A @0x17A312  B @0x17E5CA  854/3183
  G6 m3 `death_head` head       IB @0x41F72B  A @0x42100D  B @0x4252C5  854/3183
  G6 m2 `death_head` mask brows B @0x41D32D (128)   -> collapsed
  G6 m4 `death_head` mask main  B @0x4375E5 (432)   -> collapsed

Stream A = 20 B/vertex {f32 -v, 0, 0, u32 -1, f32 u}; stream B = 52 B/vertex
{pos, 1.0, normal, normal.z, tangent, 1.0, 0.0}. CJ head = OBJ faces bound to
head_* joints (541 unique per-corner verts, 788 tris), fitted into Death's
head bbox, padded with degenerate triangles to the target counts.
"""
import struct

OBJ = r'C:\Users\Bigga\AppData\Local\Temp\opencode\cj\source\x\GTA SA - Carl Johnson\CarlJohnson-GTASA.obj'
DEC = r'D:\Dev\Darksiders 2 Modkit\Game\Darksiders II Deathinitive Edition\media\_modkit_work\death.dec'

G1 = (0x178A30, 0x17A312, 0x17E5CA, 854, 3183)
G6M3 = (0x41F72B, 0x42100D, 0x4252C5, 854, 3183)
COLLAPSE = [(0x41D32D, 128), (0x4375E5, 432)]

verts, uvs, normals, faces = [], [], [], []
groups, cur = {}, None
for line in open(OBJ, encoding='utf-8', errors='ignore'):
    if line.startswith('v '):
        verts.append(tuple(float(x) for x in line.split()[1:4]))
    elif line.startswith('vt '):
        uvs.append(tuple(float(x) for x in line.split()[1:3]))
    elif line.startswith('vn '):
        normals.append(tuple(float(x) for x in line.split()[1:4]))
    elif line.startswith('g '):
        cur = line[2:].strip()
        groups[cur] = set()
    elif line.startswith('f '):
        corners = []
        for t in line.split()[1:]:
            p = (t.split('/') + ['', ''])[:3]
            corners.append((int(p[0]) - 1, int(p[1]) - 1 if p[1] else -1, int(p[2]) - 1 if p[2] else -1))
        faces.append((cur, corners))
        if cur in groups:
            groups[cur].update(c[0] for c in corners)

body_group = max(groups, key=lambda g: len(groups[g]))
bones = {}
for g, s in groups.items():
    if len(s) == 9 and g != body_group:
        pts = [verts[i] for i in s]
        bones[g] = tuple(sum(p[k] for p in pts) / 9 for k in range(3))
head_bones = {g for g in bones if g.startswith('head')}

assign = {}
for vi in groups[body_group]:
    p = verts[vi]
    assign[vi] = min(bones, key=lambda name: sum((p[k] - bones[name][k]) ** 2 for k in range(3)))

head_faces = [c for g, c in faces if g == body_group and all(assign.get(vi) in head_bones for vi, _, _ in c)]

d = open(DEC, 'rb').read()
b_off = G1[2]
dpts = [struct.unpack_from('<3f', d, b_off + i * 52) for i in range(G1[3])]
dest_lo = [min(p[k] for p in dpts) for k in range(3)]
dest_hi = [max(p[k] for p in dpts) for k in range(3)]

used = {vi for c in head_faces for vi, _, _ in c}
cj_lo = [min(verts[i][k] for i in used) for k in range(3)]
cj_hi = [max(verts[i][k] for i in used) for k in range(3)]
scale = (dest_hi[2] - dest_lo[2]) / (cj_hi[1] - cj_lo[1])
center = [(cj_lo[k] + cj_hi[k]) / 2 for k in range(3)]
dest_c = [(dest_lo[k] + dest_hi[k]) / 2 for k in range(3)]


def xf(p):
    x, y, z = p
    return ((-(x - center[0])) * scale + dest_c[0],
            (z - center[2]) * scale + dest_c[1],
            (y - center[1]) * scale + dest_c[2])


key_map, out_verts, tri_list = {}, [], []
for corners in head_faces:
    idx = []
    for key in corners:
        if key not in key_map:
            key_map[key] = len(out_verts)
            out_verts.append(key)
        idx.append(key_map[key])
    tri_list.append(idx)
print('CJ head: %d unique verts, %d tris' % (len(out_verts), len(tri_list)))

recs = []
for vi, vti, vni in out_verts:
    p = xf(verts[vi])
    n = normals[vni] if vni >= 0 else (0.0, 0.0, 1.0)
    n = (-n[0], n[2], n[1])
    ln = (sum(v * v for v in n)) ** 0.5 or 1.0
    n = tuple(v / ln for v in n)
    t = (n[1], -n[0], 0.0)
    lt = (sum(v * v for v in t)) ** 0.5 or 1.0
    t = tuple(v / lt for v in t)
    recs.append((p, n, t, uvs[vti] if 0 <= vti < len(uvs) else (0.0, 0.0)))

data = bytearray(d)
for ib_off, a_off, b_off, vc, ic in (G1, G6M3):
    assert len(out_verts) <= vc and len(tri_list) * 3 <= ic
    ib = bytearray()
    for tri in tri_list:
        for i in (tri[0], tri[2], tri[1]):
            ib += struct.pack('<H', i)
    while len(ib) < ic * 2:
        ib += struct.pack('<HHH', 0, 0, 0)
    data[ib_off:ib_off + ic * 2] = ib[:ic * 2]
    for i in range(vc):
        p, n, t, uv = recs[i] if i < len(recs) else (dest_c, (0.0, 0.0, 1.0), (1.0, 0.0, 0.0), (0.0, 0.0))
        a = a_off + i * 20
        struct.pack_into('<3f', data, a, -uv[1], 0.0, 0.0)
        struct.pack_into('<I', data, a + 12, 0xFFFFFFFF)
        struct.pack_into('<f', data, a + 16, uv[0])
        b = b_off + i * 52
        struct.pack_into('<3f', data, b, *p)
        struct.pack_into('<f', data, b + 12, 1.0)
        struct.pack_into('<3f', data, b + 16, *n)
        struct.pack_into('<f', data, b + 28, n[2])
        struct.pack_into('<3f', data, b + 32, *t)
        struct.pack_into('<f', data, b + 44, 1.0)
        struct.pack_into('<f', data, b + 48, 0.0)
    print('wrote head mesh at %X (%d/%d verts, %d/%d tris)' % (b_off, len(recs), vc, len(tri_list), ic // 3))

for b_off, vc in COLLAPSE:
    center_pt = [sum(struct.unpack_from('<3f', data, b_off + i * 52)[k] for i in range(vc)) / vc for k in range(3)]
    for i in range(vc):
        struct.pack_into('<3f', data, b_off + i * 52, *center_pt)
    print('collapsed mask mesh at %X (%d verts)' % (b_off, vc))

with open(DEC, 'wb') as f:
    f.write(data)
