"""Stage 1b v4: write CJ's head topology into the visible mask mesh (G6 m4).

The mask main mesh is 432 verts / 2196 indices (IB base 0x4342F9, preceded by
`[u32 ib_bytes][u32 2]`). CJ's head (541 verts / 788 tris) is cluster-decimated
to fit while keeping its own topology and UVs. G6 m3 is restored to stock
(hidden under the mask), G6 m2 brows are collapsed, and G1 keeps the full CJ
head written earlier (maskless fallback).
"""
import struct

OBJ = r'C:\Users\Bigga\AppData\Local\Temp\opencode\cj\source\x\GTA SA - Carl Johnson\CarlJohnson-GTASA.obj'
DEC = r'D:\Dev\Darksiders 2 Modkit\Game\Darksiders II Deathinitive Edition\media\_modkit_work\death.dec'
STOCK = DEC + '.stock'

G6_M2_B, G6_M2_VC = 0x41D32D, 128
G6_M3 = (0x41F72B, 0x42100D, 0x4252C5, 854)
G6_M4_IB, G6_M4_A, G6_M4_B, G6_M4_VC, G6_M4_IC = 0x4342F9, 0x435425, 0x4375E5, 432, 2196
G1_B, G1_VC = 0x17E5CA, 854

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

stock = open(STOCK, 'rb').read()
data = bytearray(open(DEC, 'rb').read())
for off, ln in ((G6_M2_B, G6_M2_VC * 52),
                (G6_M3[0], 3183 * 2), (G6_M3[1], 854 * 20), (G6_M3[2], 854 * 52)):
    data[off:off + ln] = stock[off:off + ln]

dpts = [struct.unpack_from('<3f', stock, G1_B + i * 52) for i in range(G1_VC)]
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


corner_verts, tri_src = {}, []
for corners in head_faces:
    tri = []
    for vi, vti, vni in corners:
        key = (vi, vti, vni)
        if key not in corner_verts:
            p = xf(verts[vi])
            n = normals[vni] if 0 <= vni < len(normals) else (0.0, 0.0, 1.0)
            n = (-n[0], n[2], n[1])
            ln = (sum(v * v for v in n)) ** 0.5 or 1.0
            n = tuple(v / ln for v in n)
            uv = uvs[vti] if 0 <= vti < len(uvs) else (0.0, 0.0)
            corner_verts[key] = (p, n, uv)
        tri.append(key)
    tri_src.append(tri)
print('CJ head corners: %d verts, %d tris' % (len(corner_verts), len(tri_src)))


def decimate(grid):
    order, idx, members = [], {}, {}
    for key, (p, _, _) in corner_verts.items():
        c = (int(p[0] // grid), int(p[1] // grid), int(p[2] // grid))
        if c not in members:
            members[c] = []
            idx[c] = len(order)
            order.append(c)
        members[c].append(key)
    out_v = []
    for c in order:
        ks = members[c]
        k = len(ks)
        cx = sum(corner_verts[x][0][0] for x in ks) / k
        cy = sum(corner_verts[x][0][1] for x in ks) / k
        cz = sum(corner_verts[x][0][2] for x in ks) / k
        best = min(ks, key=lambda x: (corner_verts[x][0][0] - cx) ** 2
                   + (corner_verts[x][0][1] - cy) ** 2 + (corner_verts[x][0][2] - cz) ** 2)
        out_v.append(((cx, cy, cz), corner_verts[best][1], corner_verts[best][2]))
    cid = {}
    for key, (p, _, _) in corner_verts.items():
        cid[key] = idx[(int(p[0] // grid), int(p[1] // grid), int(p[2] // grid))]
    tris, seen = [], set()
    for tri in tri_src:
        a, b, c = (cid[k] for k in tri)
        if a == b or b == c or a == c:
            continue
        t = tuple(sorted((a, b, c)))
        if t in seen:
            continue
        seen.add(t)
        tris.append((a, b, c))
    return out_v, tris


out_v = tris = None
for grid in (0.12, 0.15, 0.18, 0.2, 0.22, 0.25, 0.3, 0.35, 0.4, 0.45, 0.5):
    out_v, tris = decimate(grid)
    if len(out_v) <= G6_M4_VC and len(tris) * 3 <= G6_M4_IC:
        print('decimated grid=%s: %d verts, %d tris' % (grid, len(out_v), len(tris)))
        break
assert out_v is not None and len(out_v) <= G6_M4_VC and len(tris) * 3 <= G6_M4_IC

for i in range(G6_M4_VC):
    p, n, uv = out_v[i] if i < len(out_v) else (dest_c, (0.0, 0.0, 1.0), (0.0, 0.0))
    b = G6_M4_B + i * 52
    struct.pack_into('<3f', data, b, *p)
    struct.pack_into('<f', data, b + 12, 1.0)
    struct.pack_into('<3f', data, b + 16, *n)
    struct.pack_into('<f', data, b + 28, n[2])
    t = (n[1], -n[0], 0.0)
    lt = (sum(v * v for v in t)) ** 0.5 or 1.0
    t = tuple(v / lt for v in t)
    struct.pack_into('<3f', data, b + 32, *t)
    struct.pack_into('<f', data, b + 44, 1.0)
    struct.pack_into('<f', data, b + 48, 0.0)
    a = G6_M4_A + i * 20
    struct.pack_into('<3f', data, a, -uv[1], 0.0, 0.0)
    struct.pack_into('<I', data, a + 12, 0xFFFFFFFF)
    struct.pack_into('<f', data, a + 16, uv[0])

ib = bytearray()
for tri in tris:
    for i in (tri[0], tri[2], tri[1]):
        ib += struct.pack('<H', i)
while len(ib) < G6_M4_IC * 2:
    ib += struct.pack('<HHH', 0, 0, 0)
data[G6_M4_IB:G6_M4_IB + G6_M4_IC * 2] = ib[:G6_M4_IC * 2]
print('wrote m4: %d verts, %d tris (budget %d)' % (len(out_v), len(tris), G6_M4_IC // 3))

cp = [sum(struct.unpack_from('<3f', data, G6_M2_B + i * 52)[k] for i in range(G6_M2_VC)) / G6_M2_VC for k in range(3)]
for i in range(G6_M2_VC):
    struct.pack_into('<3f', data, G6_M2_B + i * 52, *cp)

with open(DEC, 'wb') as f:
    f.write(data)
print('done')
