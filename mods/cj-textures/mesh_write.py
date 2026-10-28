"""DS2 geometry writer prototype (stage 1).

Commands operate on `media/_modkit_work/death.dec`:
  scale-blade <factor>   scale the G0 scythe-blade positions about their
                         centroid (same vc/ic) and update its header bbox.

Keeps a one-time stock copy at death.dec.stock.
"""
import os
import shutil
import struct
import sys

MEDIA = r'D:\Dev\Darksiders 2 Modkit\Game\Darksiders II Deathinitive Edition\media'
DEC = os.path.join(MEDIA, '_modkit_work', 'death.dec')
STOCK = DEC + '.stock'

# G0 ShaftBlade: header at 0x108B4A, B at 0x13E3D4, 4526 verts, stride 52,
# position at +20.
G0_HDR = 0x108B4A
G0_B = 0x13E3D4
G0_VC = 4526
B_STRIDE = 52
B_POS = 20
BBOX_OFF = 57


def scale_blade(factor):
    data = bytearray(open(DEC, 'rb').read())
    pts = [struct.unpack_from('<3f', data, G0_B + i * B_STRIDE + B_POS) for i in range(G0_VC)]
    cx = sum(p[0] for p in pts) / G0_VC
    cy = sum(p[1] for p in pts) / G0_VC
    cz = sum(p[2] for p in pts) / G0_VC
    lo = [1e30] * 3
    hi = [-1e30] * 3
    for i, p in enumerate(pts):
        q = ((p[0] - cx) * factor + cx, (p[1] - cy) * factor + cy, (p[2] - cz) * factor + cz)
        struct.pack_into('<3f', data, G0_B + i * B_STRIDE + B_POS, *q)
        for k in range(3):
            lo[k] = min(lo[k], q[k])
            hi[k] = max(hi[k], q[k])
    struct.pack_into('<6f', data, G0_HDR + BBOX_OFF, *(lo + hi))
    with open(DEC, 'wb') as f:
        f.write(data)
    print('scaled blade x%s: bbox %s -> %s' % (factor,
          [round(x, 2) for x in lo], [round(x, 2) for x in hi]))


def main():
    if not os.path.exists(STOCK):
        shutil.copyfile(DEC, STOCK)
    cmd = sys.argv[1]
    if cmd == 'scale-blade':
        scale_blade(float(sys.argv[2]))
    else:
        raise SystemExit('unknown command: ' + cmd)


if __name__ == '__main__':
    main()
