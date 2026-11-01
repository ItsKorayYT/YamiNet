#!/usr/bin/env python3
"""Reproducible original pixel art and synthesized PCM. Python standard library only."""
import math, struct, zlib, wave, hashlib, json
from pathlib import Path
root=Path(__file__).resolve().parent
w,h=128,96
pixels=bytearray(w*h*4)
def put(x,y,c):
    if 0<=x<w and 0<=y<h: pixels[(y*w+x)*4:(y*w+x)*4+4]=bytes(c)
for frame in range(4):
    ox=frame*32
    for y in range(32):
        for x in range(32):
            dx=x-15.5;dy=y-15.5;d=math.hypot(dx,dy)
            # Lantern: soft alpha halo, brass frame and flickering warm core.
            if d<15: put(ox+x,y,(255,193,93,int((1-d/15)*70)))
            if 10<=x<=21 and 7<=y<=24: put(ox+x,y,(198,146,73,255))
            if 12<=x<=19 and 10<=y<=21: put(ox+x,y,(255,222+frame*5,135+frame*15,255))
            if ((x in (12,19) and 4<=y<=7) or (y==4 and 12<=x<=19)):put(ox+x,y,(210,173,105,255))
            # Wisp: pulsing teal aura and a diamond heart.
            d2=math.hypot(dx,dy-(frame%2))
            if d2<14:put(ox+x,32+y,(98,245,204,int((1-d2/14)*100)))
            if abs(dx)+abs(dy)<5+frame%2:put(ox+x,32+y,(184,255,227,255))
            # Moth: animated translucent wings and bright eyes.
            wing=8 if frame%2==0 else 4
            if ((dx+7)**2/55+dy**2/(wing*wing)<1 or (dx-7)**2/55+dy**2/(wing*wing)<1):put(ox+x,64+y,(169,113,173,205))
            if abs(dx)<2 and abs(dy)<8:put(ox+x,64+y,(63,44,78,255))
            if y==12 and x in (13,18):put(ox+x,64+y,(255,160,147,255))
def chunk(tag,data):return struct.pack('>I',len(data))+tag+data+struct.pack('>I',zlib.crc32(tag+data))
raw=b''.join(b'\0'+pixels[y*w*4:(y+1)*w*4] for y in range(h))
(root/'sprites.png').write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',w,h,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(raw,9))+chunk(b'IEND',b''))
for name,duration in [('chime',.22),('bump',.20),('marsh',4.)]:
    rate=22050; data=bytearray()
    for i in range(int(rate*duration)):
        t=i/rate
        if name=='chime':v=math.sin(t*2*math.pi*(880+660*t))*math.exp(-t*18)*.4
        elif name=='bump':v=math.sin(t*2*math.pi*(150-300*t))*math.exp(-t*20)*.4
        else:
            # Integer-cycle harmonics and envelope make a quiet seamless loop.
            envelope=math.sin(math.pi*t/duration)**2
            v=(math.sin(t*2*math.pi*220)+.5*math.sin(t*2*math.pi*330)+.3*math.sin(t*2*math.pi*440))*.14*envelope
        data+=struct.pack('<h',int(v*32767))
    with wave.open(str(root/(name+'.wav')),'wb') as f:f.setnchannels(1);f.setsampwidth(2);f.setframerate(rate);f.writeframes(data)
assets=[]
for i,name in enumerate(['sprites.png','chime.wav','bump.wav','marsh.wav']):
    path=root/name;data=path.read_bytes();assets.append(dict(id=i,key=path.stem,kind=path.suffix[1:],bytes=len(data),sha256=hashlib.sha256(data).hexdigest()))
(root/'game.yami-game.json').write_text(json.dumps(dict(version=2,abi=2,title='Lantern Drift',description='Guide a lantern through a moonlit marsh. Gather 24 lights, avoid the moths, and find your way home.',module='lantern-drift.wasm',assets=assets),indent=2)+'\n')
