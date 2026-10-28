"""Split the CJ body mesh (OBJ Object001) into per-bone rigid chunks.

Bone positions come from the 9-vertex joint boxes in the same OBJ; each body
vertex is assigned to the nearest joint. Renders a colored point cloud to
validate the assignment before any DS2 writing.
"""
import collections
import math
import os
from PIL import Image

OBJ = r'C:\Users\Bigga\AppData\Local\Temp\opencode\cj\source\x\GTA SA - Carl Johnson\CarlJohnson-GTASA.obj'
OUT = r'C:\Users\Bigga\AppData\Local\Temp\opencode\cj_bones'
os.makedirs(OUT, exist_ok=True)

verts = []
faces = []
groups = {}
cur = None
for line in open(OBJ, encoding='utf-8', errors='ignore'):
    if line.startswith('v '):
        verts.append(tuple(float(x) for x in line.split()[1:4]))
    elif line.startswith('g '):
        cur = line[2:].strip()
        groups[cur] = set()
    elif line.startswith('f '):
        idx = [int(t.split('/')[0]) - 1 for t in line.split()[1:]]
        faces.append(idx)
        if cur in groups:
            groups[cur].update(idx)

body = set()
for g, s in groups.items():
    if len(s) > 100:
        body |= s
        body_group = g
print('body group:', body_group, 'verts', len(body))

# bone boxes = groups with 9 verts
bones = {}
for g, s in groups.items():
    if len(s) == 9 and g != body_group:
        pts = [verts[i] for i in s]
        bones[g] = tuple(sum(p[k] for p in pts) / 9 for k in range(3))
print('bones:', len(bones))

# assign body verts to nearest bone center
assign = collections.defaultdict(list)
for vi in sorted(body):
    p = verts[vi]
    best, bd = None, 1e18
    for name, c in bones.items():
        d = sum((p[k] - c[k]) ** 2 for k in range(3))
        if d < bd:
            bd = d
            best = name
    assign[best].append(vi)
print('per-bone assigned verts:')
for name in sorted(assign, key=lambda n: -len(assign[n])):
    print('  %-28s %d' % (name, len(assign[name])))

# render a simple orthographic point cloud with one color per bone
minx = min(v[0] for v in verts)
maxx = max(v[0] for v in verts)
miny = min(v[1] for v in verts)
maxy = max(v[1] for v in verts)
W, H = 640, 800
img = Image.new('RGB', (W, H), (18, 18, 22))
px = img.load()
colors = {}
import random
random.seed(3)
for i, name in enumerate(sorted(bones)):
    colors[name] = (random.randint(60, 255), random.randint(60, 255), random.randint(60, 255))
for name, vlist in assign.items():
    col = colors[name]
    for vi in vlist:
        x, y = verts[vi][0], verts[vi][1]
        ix = int((x - minx) / (maxx - minx) * (W - 1))
        iy = int((maxy - y) / (maxy - miny) * (H - 1))
        px[ix, iy] = col
img.save(os.path.join(OUT, 'bones.png'))
print('wrote', os.path.join(OUT, 'bones.png'))
