"""Untrusted Intel PCS collateral retrieval; only wallet-local QVL can accept it."""
import json, pathlib, re, subprocess, urllib.parse, urllib.request
ROOT=pathlib.Path(__file__).resolve().parents[2]
ADMIN=ROOT/'experiments/yamichain-tee-m20/target/release/tdx-admin'

def native(value):
    p=subprocess.run([str(ADMIN)],input=json.dumps(value),text=True,capture_output=True,check=True,timeout=30)
    return json.loads(p.stdout)

def retrieve(quote,trusted):
    chain=native(dict(op='collateral-chain',quote=quote,qvl_library=trusted['qvl_library'],qvl_sha256=trusted['qvl_sha256']))['pck_chain']
    cert=chain.split('-----END CERTIFICATE-----',1)[0]+'-----END CERTIFICATE-----\n'
    der=subprocess.run(['openssl','x509','-outform','DER'],input=cert.encode(),capture_output=True,check=True).stdout
    oid=bytes.fromhex('060a2a864886f84d010d0104')
    occurrences=[];start=0
    while True:
        i=der.find(oid,start)
        if i<0:break
        start=i+len(oid)
        if der[start:start+2]==b'\x04\x06':occurrences.append(der[start+2:start+8].hex())
    if len(occurrences)!=1:raise ValueError('cannot identify quote FMSPC')
    issuer=subprocess.run(['openssl','x509','-noout','-issuer'],input=cert.encode(),capture_output=True,check=True).stdout
    if b'Processor' in issuer:ca='processor'
    elif b'Platform' in issuer:ca='platform'
    else:raise ValueError('unsupported PCK CA')
    def fetch(url):
        with urllib.request.urlopen(url,timeout=15) as r:
            body=r.read(1024*1024+1)
            if len(body)>1024*1024:raise ValueError('collateral limit')
            return body,r.headers
    sgx='https://api.trustedservices.intel.com/sgx/certification/v4/'
    tdx='https://api.trustedservices.intel.com/tdx/certification/v4/'
    root,_=fetch(sgx+'rootcacrl')
    crl,ch=fetch(sgx+'pckcrl?ca='+ca+'&encoding=pem')
    tcb,th=fetch(tdx+'tcb?fmspc='+occurrences[0])
    qe,qh=fetch(tdx+'qe/identity')
    def signed_body(raw,key):
        text=raw.decode();m=re.search(r'"'+re.escape(key)+r'"\s*:\s*',text)
        if not m:raise ValueError('missing signed collateral object')
        _,end=json.JSONDecoder().raw_decode(text[m.end():])
        return text[m.end():m.end()+end]
    tcb_body=signed_body(tcb,'tcbInfo');qe_body=signed_body(qe,'enclaveIdentity')
    tcb=json.loads(tcb);qe=json.loads(qe)
    def hdr(h,name):
        value=h.get(name)
        if not value:raise ValueError('missing Intel collateral '+name)
        return urllib.parse.unquote(value)
    return dict(pck_crl_issuer_chain=hdr(ch,'SGX-PCK-CRL-Issuer-Chain'),
                root_ca_crl=root.decode() if root.startswith(b'-----BEGIN') else root.hex(),
                pck_crl=crl.decode() if crl.startswith(b'-----BEGIN') else crl.hex(),
                tcb_info_issuer_chain=hdr(th,'TCB-Info-Issuer-Chain'),
                tcb_info=tcb_body,tcb_info_signature=tcb['signature'],
                qe_identity_issuer_chain=hdr(qh,'SGX-Enclave-Identity-Issuer-Chain'),
                qe_identity=qe_body,qe_identity_signature=qe['signature'])
