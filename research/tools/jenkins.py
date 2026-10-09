def hashlittle(data: bytes, seed=0):
    M = 0xffffffff
    rot = lambda x,k: ((x << k) | (x >> (32-k))) & M
    length = len(data)
    a = b = c = (0xdeadbeef + length + seed) & M
    i = 0
    while length - i > 12:
        a = (a + int.from_bytes(data[i:i+4],'little')) & M
        b = (b + int.from_bytes(data[i+4:i+8],'little')) & M
        c = (c + int.from_bytes(data[i+8:i+12],'little')) & M
        a = (a - c) & M; a ^= rot(c,4);  c = (c + b) & M
        b = (b - a) & M; b ^= rot(a,6);  a = (a + c) & M
        c = (c - b) & M; c ^= rot(b,8);  b = (b + a) & M
        a = (a - c) & M; a ^= rot(c,16); c = (c + b) & M
        b = (b - a) & M; b ^= rot(a,19); a = (a + c) & M
        c = (c - b) & M; c ^= rot(b,4);  b = (b + a) & M
        i += 12
    if length - i == 0:
        return c
    tail = data[i:] + b'\0'*12
    a = (a + int.from_bytes(tail[0:4],'little')) & M
    b = (b + int.from_bytes(tail[4:8],'little')) & M
    c = (c + int.from_bytes(tail[8:12],'little')) & M
    c ^= b; c = (c - rot(b,14)) & M
    a ^= c; a = (a - rot(c,11)) & M
    b ^= a; b = (b - rot(a,25)) & M
    c ^= b; c = (c - rot(b,16)) & M
    a ^= c; a = (a - rot(c,4)) & M
    b ^= a; b = (b - rot(a,14)) & M
    c ^= b; c = (c - rot(b,24)) & M
    return c
