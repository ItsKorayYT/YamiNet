#!/usr/bin/env python3
"""Fetch only SHA256-pinned Debian build inputs. No package installation."""
import hashlib,json,pathlib,subprocess,sys,urllib.request
base=pathlib.Path(__file__).resolve().parent
out=pathlib.Path(sys.argv[1]).resolve();out.mkdir(parents=True,exist_ok=True)
for p in json.loads((base/'inputs.json').read_text())['debian']:
    target=out/pathlib.Path(p['Filename']).name
    if not target.exists():
        with urllib.request.urlopen('https://deb.debian.org/debian/'+p['Filename'],timeout=90) as r:
            target.write_bytes(r.read())
    if hashlib.sha256(target.read_bytes()).hexdigest()!=p['SHA256']:
        raise SystemExit('pinned Debian artifact mismatch: '+target.name)
    subprocess.run(['dpkg-deb','-x',str(target),str(out/'root')],check=True)
print(out/'root')
