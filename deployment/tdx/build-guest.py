#!/usr/bin/env python3
"""Deterministic immutable UKI + UEFI disk from explicitly hashed input artifacts.
This builds a guest, never fabricates TDX measurements or verifies a hardware boot.
"""
import argparse, hashlib, json, os, pathlib, re, shutil, stat, struct, subprocess, tempfile, uuid, zlib
BASE=pathlib.Path(__file__).resolve().parent
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--inputs',type=pathlib.Path,required=True,help='fetch-inputs.py output/root')
p.add_argument('--executor',type=pathlib.Path,required=True)
p.add_argument('--quote-library',type=pathlib.Path,required=True)
p.add_argument('--public-config',type=pathlib.Path,required=True,help='index, operator, recovery, listen, qgs_port (NO secrets)')
p.add_argument('--out',type=pathlib.Path,required=True)
a=p.parse_args();a.out.mkdir(parents=True,exist_ok=True)
public=json.loads(a.public_config.read_text())
if set(public)!={'index','operator','recovery','listen','qgs_port'} or public['index'] not in range(4):
    raise SystemExit('strict public guest configuration required; never place private keys in guest image')
for name in ('operator','recovery'):
    if len(public[name])!=32 or not all(type(n) is int and 0<=n<=255 for n in public[name]) or not any(public[name]):
        raise SystemExit('32-byte nonzero public '+name+' required')
if not 1<=public['qgs_port']<=65535:raise SystemExit('provider QGS vsock port required')
def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()
kernels=list((a.inputs/'boot').glob('vmlinuz-*'))
if len(kernels)!=1:raise SystemExit('one approved TDX kernel required')
kernel=kernels[0];version=kernel.name.removeprefix('vmlinuz-')
config=(a.inputs/'boot'/('config-'+version)).read_text()
for option in ('INTEL_TDX_GUEST','EFI_STUB','TDX_GUEST_DRIVER'):
    if not re.search(r'^CONFIG_'+option+r'=[ym]$',config,re.M):raise SystemExit('kernel lacks '+option)
with tempfile.TemporaryDirectory(prefix='yami-tdx-build-') as tmp:
    root=pathlib.Path(tmp)
    def copy(src,target,mode=None):
        dest=root/target.lstrip('/');dest.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(src,dest)
        dest.chmod(mode if mode is not None else (src.stat().st_mode&0o777))
    def linked(src,target):
        copy(src,target)
        output=subprocess.check_output(['ldd',str(src.resolve())],text=True,stderr=subprocess.STDOUT)
        if 'not found' in output:raise SystemExit('unresolved guest native dependency')
        for line in output.splitlines():
            m=re.search(r'(?:=>\s*)?(/\S+)\s+\(',line)
            if m:copy(pathlib.Path(m[1]),m[1])
    for d in ('dev','proc','sys','run','tmp','bin','etc/yami','usr/bin','usr/lib/yami','lib','lib64'):(root/d).mkdir(parents=True,exist_ok=True)
    linked(a.executor,'/usr/bin/tdx-executor')
    linked(a.quote_library,'/usr/lib/yami/libtdx_attest.so.1')
    busybox=a.inputs/'usr/bin/busybox'
    if not busybox.exists():busybox=a.inputs/'bin/busybox'
    copy(busybox,'/bin/busybox',0o755)
    copy(BASE/'init','/init',0o755);copy(BASE/'dhcp','/etc/yami/dhcp',0o755)
    # Preserve redistribution notices alongside the measured binaries.
    intel_license=a.quote_library.resolve().parents[4]/'License.txt'
    if not intel_license.exists():raise SystemExit('build quote library in its pinned Intel source tree')
    copy(intel_license,'/usr/share/licenses/intel-dcap.txt',0o644)
    for notice in sorted((a.inputs/'usr/share/doc').glob('*/copyright')):
        copy(notice,'/usr/share/licenses/debian-'+notice.parent.name+'.txt',0o644)
    for name in ('LICENSE','LICENSE.md'):
        notice=BASE.parents[1]/name
        if notice.is_file():copy(notice,'/usr/share/licenses/yaminet.txt',0o644);break

    modules=a.inputs/'usr/lib/modules'/version
    shutil.copytree(modules,root/'usr/lib/modules'/version,symlinks=True)
    (root/'lib/modules').symlink_to('../usr/lib/modules')
    # Busybox depmod is static and writes only to this temporary image tree.
    subprocess.run([str(busybox.resolve()),'depmod','-b',str(root),version],check=True,stdout=subprocess.DEVNULL)
    (root/'etc/resolv.conf').symlink_to('/run/resolv.conf')
    (root/'etc/tdx-attest.conf').write_text('port = '+str(public['qgs_port'])+'\ntimeout = 15\n')
    identity=hashlib.sha256(json.dumps({'domain':'YAMI-TDX-guest-code-v1','executor':digest(a.executor),'quote_library':digest(a.quote_library),'public_config':public},sort_keys=True,separators=(',',':')).encode()).digest()
    guest={k:public[k] for k in ('index','operator','recovery','listen')}
    guest.update(software_identity=list(identity),quote_library='/usr/lib/yami/libtdx_attest.so.1',quote_sha256=list(bytes.fromhex(digest(a.quote_library))))
    (root/'etc/yami/guest.json').write_text(json.dumps(guest,sort_keys=True,separators=(',',':'))+'\n')
    files={}
    for entry in sorted(root.rglob('*')):
        if entry.is_file() and not entry.is_symlink():files[str(entry.relative_to(root))]=digest(entry)
    # newc cpio: stable order, uid/gid/inodes/mtime and zero alignment. No host timestamps.
    archive=bytearray()
    def record(name,mode,data,ino,rmajor=0,rminor=0):
        n=name.encode()+b'\0';fields=[ino,mode,0,0,1,0,len(data),0,0,rmajor,rminor,len(n),0]
        archive.extend(b'070701'+''.join(f'{v:08x}' for v in fields).encode()+n)
        archive.extend(b'\0'*(-len(archive)%4));archive.extend(data);archive.extend(b'\0'*(-len(archive)%4))
    for i,entry in enumerate(sorted(root.rglob('*')),1):
        name=str(entry.relative_to(root));mode=entry.lstat().st_mode
        if entry.is_symlink():data=os.readlink(entry).encode()
        elif entry.is_file():data=entry.read_bytes()
        elif entry.is_dir():data=b''
        else:raise SystemExit('unsupported guest file type')
        record(name,mode,data,i)
    record('dev/console',stat.S_IFCHR|0o600,b'',1000001,5,1)
    record('dev/null',stat.S_IFCHR|0o666,b'',1000002,1,3)
    record('dev/ttyS0',stat.S_IFCHR|0o600,b'',1000003,4,64)
    record('TRAILER!!!',0,b'',0);archive.extend(b'\0'*(-len(archive)%512))
    initrd=a.out/'initrd.cpio';initrd.write_bytes(archive)
    cmdline='console=ttyS0 rdinit=/init net.ifnames=0 panic=-1 mitigations=auto\0'
    (a.out/'cmdline').write_bytes(cmdline.encode())
    (a.out/'os-release').write_text('ID=yamichain-tdx\nPRETTY_NAME="YamiChain TDX Disposable Devnet"\n')
    uki=a.out/'BOOTX64.EFI'
    stub=a.inputs/'usr/lib/systemd/boot/efi/linuxx64.efi.stub'
    stub=a.inputs/'usr/lib/systemd/boot/efi/linuxx64.efi.stub'
    stub_bytes=stub.read_bytes();pe=struct.unpack_from('<I',stub_bytes,0x3c)[0]
    image_base=struct.unpack_from('<Q',stub_bytes,pe+24+24)[0]
    # Give the .initrd its own non-overlapping, page-aligned RVA after the kernel.
    initrd_rva=(0x2000000+kernel.stat().st_size+0xfffff)&~0xfffff
    subprocess.run(['objcopy','--add-section','.osrel='+str(a.out/'os-release'),'--change-section-vma',f'.osrel={image_base+0x20000:#x}',
                    '--add-section','.cmdline='+str(a.out/'cmdline'),'--change-section-vma',f'.cmdline={image_base+0x30000:#x}',
                    '--add-section','.linux='+str(kernel),'--change-section-vma',f'.linux={image_base+0x2000000:#x}',
                    '--add-section','.initrd='+str(initrd),'--change-section-vma',f'.initrd={image_base+initrd_rva:#x}',str(stub),str(uki)],check=True)
    # objcopy stamps COFF time and PE checksum from its wall clock. Both may legally
    # be zero for UEFI images and are normalized before measurement/import.
    image=bytearray(uki.read_bytes());pe=struct.unpack_from('<I',image,0x3c)[0]
    struct.pack_into('<I',image,pe+8,0);struct.pack_into('<I',image,pe+24+64,0);uki.write_bytes(image)
    # Construct a real GPT/ESP disk using pinned mkfs/mtools executables; fixed geometry and IDs.
    # External tools never run against a host block device.
    disk=a.out/'disk.raw';sector=512;start=2048;length=524288;total=start+length+2048
    with disk.open('wb') as f:f.truncate(total*sector)
    guid=uuid.UUID(bytes=hashlib.sha256(uki.read_bytes()).digest()[:16])
    entries=bytearray(128*128)
    entries[:128]=struct.pack('<16s16sQQQ72s',uuid.UUID('c12a7328-f81f-11d2-ba4b-00a0c93ec93b').bytes_le,guid.bytes_le,start,start+length-1,0,'YAMI TDX ESP'.encode('utf-16-le').ljust(72,b'\0'))
    def gpt(current,backup,entry_lba):
        h=bytearray(struct.pack('<8sIIIIQQQQ16sQIII',b'EFI PART',0x10000,92,0,0,current,backup,34,total-34,guid.bytes_le,entry_lba,128,128,zlib.crc32(entries)))
        struct.pack_into('<I',h,16,zlib.crc32(h));return h.ljust(512,b'\0')
    with disk.open('r+b') as f:
        mbr=bytearray(512);mbr[446:462]=struct.pack('<B3sB3sII',0,b'\0\x02\0',0xee,b'\xff'*3,1,min(total-1,0xffffffff));mbr[510:]=b'\x55\xaa';f.write(mbr)
        f.seek(512);f.write(gpt(1,total-1,2));f.write(entries)
        f.seek((total-33)*512);f.write(entries);f.write(gpt(total-1,1,total-33))
    esp=a.out/'esp.img'
    with esp.open('wb') as f:f.truncate(length*sector)
    tools=a.inputs/'usr'
    subprocess.run([str(tools/'sbin/mkfs.fat'),'--invariant','-F','32','-i','59414d49','-n','YAMI_TDX',str(esp)],check=True,stdout=subprocess.DEVNULL)
    env=dict(os.environ,MTOOLS_SKIP_CHECK='1',TZ='UTC',SOURCE_DATE_EPOCH='315532800')
    subprocess.run([str(tools/'bin/mmd'),'-i',str(esp),'::/EFI','::/EFI/BOOT'],check=True,env=env)
    # mtools -m preserves file timestamps. Set a stable DOS-representable timestamp first.
    os.utime(uki,(315532800,315532800))
    subprocess.run([str(tools/'bin/mcopy'),'-m','-i',str(esp),str(uki),'::/EFI/BOOT/BOOTX64.EFI'],check=True,env=env)
    # mmd's directory entries have current timestamps: normalize these through mtools touch.
    subprocess.run([str(tools/'bin/mattrib'),'-i',str(esp),'+r','::/EFI/BOOT/BOOTX64.EFI'],check=True,env=env)
    # Normalize all FAT directory timestamps, including mmd-generated entries.
    fs=bytearray(esp.read_bytes())
    bps=struct.unpack_from('<H',fs,11)[0];spc=fs[13];reserved=struct.unpack_from('<H',fs,14)[0]
    fats=fs[16];fat_sectors=struct.unpack_from('<I',fs,36)[0]
    data_start=(reserved+fats*fat_sectors)*bps
    seen=set()
    def normalize(cluster):
        if cluster in seen:return
        seen.add(cluster);current=cluster
        while 2<=current<0xffffff8:
            offset=data_start+(current-2)*spc*bps
            for pos in range(offset,offset+spc*bps,32):
                if fs[pos] in (0,0xe5) or fs[pos+11]==0x0f:continue
                fs[pos+13]=0
                for field,value in ((14,0),(16,0x21),(18,0x21),(22,0),(24,0x21)):struct.pack_into('<H',fs,pos+field,value)
                if fs[pos+11]&0x10 and fs[pos]!=ord('.'):
                    child=(struct.unpack_from('<H',fs,pos+20)[0]<<16)|struct.unpack_from('<H',fs,pos+26)[0]
                    normalize(child)
            current=struct.unpack_from('<I',fs,reserved*bps+current*4)[0]&0xfffffff
    normalize(struct.unpack_from('<I',fs,44)[0])
    with esp.open('r+b') as f:
        for cluster in sorted(seen):
            offset=data_start+(cluster-2)*spc*bps;f.seek(offset);f.write(fs[offset:offset+spc*bps])
    with disk.open('r+b') as dst,esp.open('rb') as src:
        dst.seek(start*sector)
        while True:
            block=src.read(1024*1024)
            if not block:break
            if any(block):dst.write(block)
            else:dst.seek(len(block),1)
    esp.unlink()
    manifest={'format':'yami-tdx-guest-v1','software_identity':list(identity),'guest_config':guest,'kernel':version,'kernel_sha256':digest(kernel),
              'files':files,'initrd_sha256':digest(initrd),'uki_sha256':digest(uki),'disk_sha256':digest(disk),
              'hardware_tested':False,'production_activation':False,
              'measurement_policy':'MRTD/RTMR values require independently approved provider firmware + measured-boot reference; file hashes are NOT quote measurements'}
    (a.out/'manifest.json').write_text(json.dumps(manifest,sort_keys=True,indent=2)+'\n')
    print(json.dumps({k:manifest[k] for k in ('kernel','software_identity','uki_sha256','disk_sha256','hardware_tested')}))
