#!/usr/bin/env python3
"""Validate packaged bytes/layout, not a TDX boot or hardware attestation."""
import argparse,hashlib,json,pathlib,stat,struct,subprocess,zlib
p=argparse.ArgumentParser(description=__doc__);p.add_argument('build',type=pathlib.Path);p.add_argument('--executor',type=pathlib.Path);a=p.parse_args()
m=json.loads((a.build/'manifest.json').read_text())
def sha(b):return hashlib.sha256(b).hexdigest()
image=(a.build/'BOOTX64.EFI').read_bytes();initrd=(a.build/'initrd.cpio').read_bytes()
assert sha(image)==m['uki_sha256'] and sha(initrd)==m['initrd_sha256']
pe=struct.unpack_from('<I',image,60)[0];assert image[pe:pe+4]==b'PE\0\0'
count=struct.unpack_from('<H',image,pe+6)[0];optional=struct.unpack_from('<H',image,pe+20)[0]
base=struct.unpack_from('<Q',image,pe+48)[0];entry=struct.unpack_from('<I',image,pe+40)[0]
sections={};ranges=[]
for i in range(count):
    off=pe+24+optional+i*40;name=image[off:off+8].rstrip(b'\0').decode();size,rva,rawsize,raw=struct.unpack_from('<IIII',image,off+8)
    assert raw+rawsize<=len(image) and rva>0 and size<=rawsize
    sections[name]=image[raw:raw+size];ranges.append((rva,rva+size,name))
for prev,nxt in zip(sorted(ranges),sorted(ranges)[1:]):assert prev[1]<=nxt[0]
assert any(lo<=entry<hi and name=='.text' for lo,hi,name in ranges)
assert sha(sections['.initrd'])==m['initrd_sha256'] and sha(sections['.linux'])==m['kernel_sha256']
files={};devices={};pos=0
while True:
    assert initrd[pos:pos+6]==b'070701'
    fields=[int(initrd[pos+6+i*8:pos+14+i*8],16) for i in range(13)]
    mode,size,namesize=fields[1],fields[6],fields[11]
    name=initrd[pos+110:pos+110+namesize-1].decode();pos=(pos+110+namesize+3)&~3
    data=initrd[pos:pos+size];pos=(pos+size+3)&~3
    if name=='TRAILER!!!':break
    assert not name.startswith('/') and '..' not in pathlib.PurePosixPath(name).parts
    if stat.S_ISREG(mode):files[name]=data
    elif stat.S_ISCHR(mode):devices[name]=(fields[9],fields[10])
assert devices['dev/console']==(5,1) and devices['dev/null']==(1,3)
for name,digest in m['files'].items():assert sha(files[name])==digest,name
config=json.loads(files['etc/yami/guest.json']);assert config==m['guest_config']
assert set(config)=={'index','operator','recovery','software_identity','quote_library','quote_sha256','listen'}
assert 'exec /usr/bin/tdx-executor' in files['init'].decode()
with (a.build/'disk.raw').open('rb') as f:
    f.seek(512);header=bytearray(f.read(512));expected=struct.unpack_from('<I',header,16)[0];struct.pack_into('<I',header,16,0)
    assert zlib.crc32(header[:92])==expected
    f.seek(1024);entries=f.read(16384);assert zlib.crc32(entries)==struct.unpack_from('<I',header,88)[0]
    start=struct.unpack_from('<Q',entries,32)[0]*512
    f.seek(start);b=f.read(512);assert b[510:]==b'\x55\xaa' and b[82:90]==b'FAT32   '
    bps=struct.unpack_from('<H',b,11)[0];spc=b[13];reserved=struct.unpack_from('<H',b,14)[0];fat_sectors=struct.unpack_from('<I',b,36)[0];fats=b[16]
    data_start=start+(reserved+fats*fat_sectors)*bps
    def clusters(first):
        current=first;seen=set()
        while current<0xffffff8:
            assert current>=2 and current not in seen;seen.add(current)
            f.seek(data_start+(current-2)*spc*bps);yield f.read(spc*bps)
            f.seek(start+reserved*bps+current*4);current=struct.unpack('<I',f.read(4))[0]&0xfffffff
    def find(first,name):
        for block in clusters(first):
            for pos in range(0,len(block),32):
                if block[pos:pos+11]==name:
                    c=(struct.unpack_from('<H',block,pos+20)[0]<<16)|struct.unpack_from('<H',block,pos+26)[0]
                    return c,struct.unpack_from('<I',block,pos+28)[0]
        raise AssertionError('missing ESP path '+str(name))
    root=struct.unpack_from('<I',b,44)[0]
    efi,_=find(root,b'EFI        ');boot,_=find(efi,b'BOOT       ');target,length=find(boot,b'BOOTX64 EFI')
    assert sha(b''.join(clusters(target))[:length])==m['uki_sha256']
    f.seek(0);assert sha(f.read())==m['disk_sha256']
negative=False
if a.executor:
    # Run the real executable only in the explicitly hardware-absent test environment.
    assert not any(pathlib.Path(s).exists() for s in ('/dev/tdx_guest','/dev/tdx-guest','/sys/kernel/config/tsm/report'))
    # A missing TDX device must stop it before listening.
    import tempfile
    with tempfile.NamedTemporaryFile(mode='w') as tmp:
        json.dump(config,tmp);tmp.flush()
        p=subprocess.run([str(a.executor.resolve()),tmp.name],capture_output=True,timeout=5)
        assert p.returncode!=0 and b"verification rejected" in p.stderr
        negative=True
print(json.dumps(dict(package_valid=True,hardware_tested=False,tdx_startup_rejected_without_hardware=negative,uki_sha256=m['uki_sha256'],disk_sha256=m['disk_sha256']),indent=2))
